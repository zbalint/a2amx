use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::grant;
use crate::harness::Harness;
use crate::wire::ConfineInfo;

use super::{ALWAYS_UNSET, CODEX_SOCKET_SUBDIR, CREDENTIAL_ENV, Plan, granted_env, path_key};

const MAX_ARG_BYTES: usize = 128 * 1024;
const MAX_ARGV_BYTES: usize = 256 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountKind {
    Overlay,
    Writable,
    ReadOnly,
    ReadOnlyTry,
    Empty,
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    kind: MountKind,
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

impl Plan {
    pub fn layout(&self) -> &crate::layout::Layout {
        &self.layout
    }

    pub fn with_bwrap(mut self, path: PathBuf) -> Self {
        self.bwrap = path;
        self
    }

    pub fn with_codex_dir(mut self, path: PathBuf) -> Self {
        self.codex_dir = Some(path);
        self
    }

    pub fn is_confined(&self) -> bool {
        true
    }
    pub fn confine_info(&self) -> ConfineInfo {
        ConfineInfo {
            auto: self.auto,
            grants: self.effective_grants.clone(),
            write: self.write.clone(),
            reasons: self.reasons.clone(),
            hide: self.hide.clone(),
            unhide: self.unhide.clone(),
            no_default_hides: self.no_default_hides,
        }
    }

    fn mounts(&self) -> Vec<Mount> {
        let mut mounts = BTreeMap::<PathBuf, MountKind>::new();
        for path in &self.overlay_parents {
            mounts.insert(path.clone(), MountKind::Overlay);
        }
        for path in &self.hidden {
            let kind = if path.is_dir() {
                MountKind::Tmpfs
            } else {
                MountKind::Empty
            };
            mounts.insert(path.clone(), kind);
        }
        for path in &self.writable {
            mounts.insert(path.clone(), MountKind::Writable);
        }
        for path in &self.readonly {
            mounts.insert(path.clone(), MountKind::ReadOnly);
        }
        for path in &self.protected {
            mounts.insert(path.clone(), MountKind::ReadOnly);
        }
        for path in &self.overlay_protected {
            mounts.insert(path.clone(), MountKind::ReadOnlyTry);
        }
        if self.harness == Harness::Omp {
            let omp = self.layout.omp_dir();
            if omp.exists() {
                mounts.insert(omp, MountKind::ReadOnly);
            }
        }
        let mut mounts = mounts
            .into_iter()
            .map(|(path, kind)| Mount { path, kind })
            .collect::<Vec<_>>();
        mounts.sort_by(|left, right| {
            path_depth(&left.path)
                .cmp(&path_depth(&right.path))
                .then_with(|| {
                    let rank = |kind| match kind {
                        MountKind::Overlay | MountKind::Tmpfs | MountKind::Empty => 0,
                        MountKind::Writable | MountKind::ReadOnly | MountKind::ReadOnlyTry => 1,
                    };
                    rank(left.kind).cmp(&rank(right.kind))
                })
                .then_with(|| left.path.cmp(&right.path))
        });
        mounts
    }

    fn env_unsets(&self) -> impl Iterator<Item = &'static str> {
        let given = self
            .grants
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let released = granted_env(&self.grants);
        let mut unset = ALWAYS_UNSET.to_vec();
        for grant in grant::all()
            .iter()
            .filter(|grant| !given.contains(grant.name))
        {
            for name in grant.env {
                if CREDENTIAL_ENV.contains(name) || unset.contains(name) {
                    continue;
                }
                unset.push(name);
            }
        }
        for name in CREDENTIAL_ENV {
            if !released.contains(name)
                && !self.keep_env.iter().any(|kept| kept == name)
                && !unset.contains(&name)
            {
                unset.push(name);
            }
        }
        unset.into_iter()
    }

    /// Produce `bwrap <options> -- <argv>` with deterministic operation order.
    pub fn wrap(&self, argv: Vec<String>) -> Result<Vec<String>> {
        self.wrap_with_codex_bind(argv, false)
    }

    pub fn wrap_app_server(&self, argv: Vec<String>) -> Result<Vec<String>> {
        self.wrap_with_codex_bind(argv, true)
    }

    fn wrap_with_codex_bind(&self, argv: Vec<String>, app_server: bool) -> Result<Vec<String>> {
        if argv.is_empty() {
            bail!("session command is empty");
        }
        for argument in &argv {
            if argument.len() > MAX_ARG_BYTES {
                bail!("session command argument exceeds {MAX_ARG_BYTES} bytes");
            }
        }
        let mut wrapped = Vec::with_capacity(argv.len() + 32);
        wrapped.push(self.bwrap.to_string_lossy().into_owned());
        wrapped.extend(
            [
                "--die-with-parent",
                "--unshare-user",
                "--unshare-ipc",
                "--unshare-pid",
                "--ro-bind",
                "/",
                "/",
                "--dev",
                "/dev",
                "--proc",
                "/proc",
                "--size",
                "1073741824",
                "--tmpfs",
                "/tmp",
            ]
            .into_iter()
            .map(str::to_owned),
        );
        for variable in self.env_unsets() {
            wrapped.push("--unsetenv".into());
            wrapped.push(variable.into());
        }
        let mut remounts = Vec::new();
        for mount in self.mounts() {
            match mount.kind {
                MountKind::Overlay => {
                    // shortcut: tmpfs upper layer, no size bound; bubblewrap offers none with --tmp-overlay, a disk-backed upper or a quota if it ever does.
                    wrapped.extend([
                        "--overlay-src".into(),
                        path_key(&mount.path),
                        "--tmp-overlay".into(),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::Writable => {
                    wrapped.extend([
                        "--bind".into(),
                        path_key(&mount.path),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::ReadOnly => {
                    wrapped.extend([
                        "--ro-bind".into(),
                        path_key(&mount.path),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::ReadOnlyTry => {
                    wrapped.extend([
                        "--ro-bind-try".into(),
                        path_key(&mount.path),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::Empty => {
                    wrapped.extend([
                        "--ro-bind".into(),
                        path_key(&self.layout.confine_empty()),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::Tmpfs => {
                    wrapped.extend(["--tmpfs".into(), path_key(&mount.path)]);
                    remounts.push(mount.path);
                }
            }
        }
        if app_server && let Some(dir) = &self.codex_dir {
            wrapped.extend(["--ro-bind".into(), path_key(dir), path_key(dir)]);
        }
        for path in remounts {
            wrapped.extend(["--remount-ro".into(), path_key(&path)]);
        }
        if let Some(dir) = &self.codex_dir {
            let socket_dir = dir.join(CODEX_SOCKET_SUBDIR);
            wrapped.extend([
                if app_server { "--bind" } else { "--ro-bind" }.into(),
                path_key(&socket_dir),
                format!("/tmp/codex-daemon-{}", rustix::process::geteuid().as_raw()),
            ]);
        }
        wrapped.extend(["--chdir".into(), path_key(&self.cwd), "--".into()]);
        wrapped.extend(argv);
        let bytes = wrapped.iter().map(|value| value.len() + 1).sum::<usize>();
        if bytes > MAX_ARGV_BYTES {
            bail!("bubblewrap argument vector exceeds {MAX_ARGV_BYTES} bytes");
        }
        Ok(wrapped)
    }

    pub fn hidden_paths(&self) -> &[PathBuf] {
        &self.hidden
    }

    pub fn skipped_paths(&self) -> &[PathBuf] {
        &self.skipped
    }
    pub fn overlay_parents(&self) -> &[PathBuf] {
        &self.overlay_parents
    }

    pub fn unsaved_files(&self) -> Vec<PathBuf> {
        self.unsaved_files.clone()
    }

    pub fn without_overlays(mut self, reason: &str) -> Self {
        self.overlay_parents.clear();
        self.overlay_protected.clear();
        self.overlay_notice = Some(format!(
            "harness login cannot be renewed in this session: {reason}; bubblewrap 0.11 or newer is needed (or open the harness directory with `write`)."
        ));
        self
    }
}
