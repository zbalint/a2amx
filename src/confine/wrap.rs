use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::grant;
use crate::harness::Harness;
use crate::wire::ConfineInfo;

use super::{ALWAYS_UNSET, CODEX_SOCKET_SUBDIR, CREDENTIAL_ENV, Plan, granted_env, path_key};

const MAX_ARG_BYTES: usize = 128 * 1024;
const MAX_ARGV_BYTES: usize = 256 * 1024;
const BWRAP_PREFIX: [&str; 15] = [
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
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountKind {
    Overlay,
    Writable,
    ReadOnly,
    ReadOnlyTry,
    Empty,
    Tmpfs,
}

impl MountKind {
    fn layer(self) -> u8 {
        match self {
            Self::Overlay | Self::Tmpfs | Self::Empty => 0,
            Self::Writable | Self::ReadOnly | Self::ReadOnlyTry => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    kind: MountKind,
}

impl Mount {
    fn bwrap_args(&self, confine_empty: &Path) -> Vec<String> {
        match self.kind {
            MountKind::Overlay => {
                // shortcut: tmpfs upper layer, no size bound; bubblewrap offers none with --tmp-overlay, a disk-backed upper or a quota if it ever does.
                vec![
                    "--overlay-src".into(),
                    path_key(&self.path),
                    "--tmp-overlay".into(),
                    path_key(&self.path),
                ]
            }
            MountKind::Writable => {
                vec!["--bind".into(), path_key(&self.path), path_key(&self.path)]
            }
            MountKind::ReadOnly => vec![
                "--ro-bind".into(),
                path_key(&self.path),
                path_key(&self.path),
            ],
            MountKind::ReadOnlyTry => vec![
                "--ro-bind-try".into(),
                path_key(&self.path),
                path_key(&self.path),
            ],
            MountKind::Empty => vec![
                "--ro-bind".into(),
                path_key(confine_empty),
                path_key(&self.path),
            ],
            MountKind::Tmpfs => vec!["--tmpfs".into(), path_key(&self.path)],
        }
    }
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
        mounts.extend(
            self.overlay_parents
                .iter()
                .map(|path| (path.clone(), MountKind::Overlay)),
        );
        mounts.extend(self.hidden.iter().map(|path| {
            let kind = if path.is_dir() {
                MountKind::Tmpfs
            } else {
                MountKind::Empty
            };
            (path.clone(), kind)
        }));
        mounts.extend(
            self.writable
                .iter()
                .map(|path| (path.clone(), MountKind::Writable)),
        );
        mounts.extend(
            self.readonly
                .iter()
                .map(|path| (path.clone(), MountKind::ReadOnly)),
        );
        mounts.extend(
            self.protected
                .iter()
                .map(|path| (path.clone(), MountKind::ReadOnly)),
        );
        mounts.extend(
            self.overlay_protected
                .iter()
                .map(|path| (path.clone(), MountKind::ReadOnlyTry)),
        );
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
                .then_with(|| left.kind.layer().cmp(&right.kind.layer()))
                .then_with(|| left.path.cmp(&right.path))
        });
        mounts
    }

    pub(super) fn unset_credentials(&self) -> impl Iterator<Item = &'static str> {
        let released = granted_env(&self.grants);
        CREDENTIAL_ENV.iter().copied().filter(move |name| {
            !released.contains(name) && !self.keep_env.iter().any(|kept| kept == name)
        })
    }

    fn env_unsets(&self) -> impl Iterator<Item = &'static str> {
        let given = self
            .grants
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        ALWAYS_UNSET
            .iter()
            .copied()
            .chain(
                grant::all()
                    .iter()
                    .filter(|grant| !given.contains(grant.name))
                    .flat_map(|grant| grant.env.iter().copied())
                    .filter(|name| !CREDENTIAL_ENV.contains(name)),
            )
            .chain(self.unset_credentials())
            .filter(|name| seen.insert(*name))
            .collect::<Vec<_>>()
            .into_iter()
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
        if argv.iter().any(|a| a.len() > MAX_ARG_BYTES) {
            bail!("session command argument exceeds {MAX_ARG_BYTES} bytes");
        }
        let mut wrapped = Vec::with_capacity(argv.len() + 32);
        wrapped.push(self.bwrap.to_string_lossy().into_owned());
        wrapped.extend(BWRAP_PREFIX.into_iter().map(str::to_owned));
        wrapped.extend(
            self.env_unsets()
                .flat_map(|variable| ["--unsetenv".into(), variable.into()]),
        );
        let confine_empty = self.layout.confine_empty();
        let mounts = self.mounts();
        wrapped.extend(
            mounts
                .iter()
                .flat_map(|mount| mount.bwrap_args(&confine_empty)),
        );
        if app_server && let Some(dir) = &self.codex_dir {
            wrapped.extend(["--ro-bind".into(), path_key(dir), path_key(dir)]);
        }
        wrapped.extend(
            mounts
                .iter()
                .filter(|mount| mount.kind == MountKind::Tmpfs)
                .flat_map(|mount| ["--remount-ro".into(), path_key(&mount.path)]),
        );
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
