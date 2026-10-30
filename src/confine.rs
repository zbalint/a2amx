//! Opt-in bubblewrap confinement for PTY sessions.
//!
//! This module owns every bubblewrap argument and probe.  Callers pass plain
//! paths and a harness profile; the resulting [`Plan`] is the sole source for
//! mounts, the launch argv, the notice, and dry-run visibility.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};

use crate::harness::Harness;

pub const MAX_LIST_ENTRIES: usize = 64;
const MAX_PATH_BYTES: usize = 4096;
const MAX_ARG_BYTES: usize = 128 * 1024;
const MAX_ARGV_BYTES: usize = 256 * 1024;
const NOTICE_LIMIT: usize = 12 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub const CLOSING_NOTICE: &str = "Do not try to get around these limits, whether through other programs, services or sockets, through other agents, or by editing configuration to loosen them. If your task needs access they do not allow, stop that part and tell the person running this session which path or access you need and why. Other errors, such as a missing file or a failing build or test, are ordinary: handle them as usual.";

const ALWAYS_UNSET: [&str; 6] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "WSL_INTEROP",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];
const CONTAINER_UNSET: [&str; 6] = [
    "DOCKER_HOST",
    "DOCKER_CONTEXT",
    "CONTAINER_HOST",
    "CONTAINER_CONNECTION",
    "CONTAINERD_ADDRESS",
    "BUILDKIT_HOST",
];
const CONTAINER_UNSET_EXTRA: [&str; 1] = ["LIBVIRT_DEFAULT_URI"];
const CONTAINER_HIDES: [&str; 15] = [
    "/run/docker.sock",
    "/run/podman/podman.sock",
    "/run/containerd/containerd.sock",
    "/run/containerd/containerd.sock.ttrpc",
    "/run/buildkit/buildkitd.sock",
    "/run/crio/crio.sock",
    "/run/k3s/containerd/containerd.sock",
    "/run/libvirt/libvirt-sock",
    "/run/snapd.socket",
    "/run/snapd-snap.socket",
    "/var/snap/lxd/common/lxd/unix.socket",
    "/var/lib/lxd/unix.socket",
    "/var/lib/incus/unix.socket",
    "~/.docker/desktop/docker.sock",
    "~/.docker/run/docker.sock",
];
const DEFAULT_HIDES: [&str; 24] = [
    "~/.ssh",
    "~/.gnupg",
    "~/.aws",
    "~/.config/gcloud",
    "~/.kube",
    "~/.docker",
    "~/.netrc",
    "~/.local/share/keyrings",
    "~/.config/chromium",
    "~/.config/google-chrome",
    "~/.mozilla/firefox",
    "~/.config/gh",
    "~/.git-credentials",
    "~/.config/git/credentials",
    "~/.npmrc",
    "~/.pypirc",
    "~/.cargo/credentials",
    "~/.cargo/credentials.toml",
    "~/.azure",
    "~/.terraform.d",
    "~/.vault-token",
    "~/.pgpass",
    "~/.password-store",
    "~/.config/op",
];
const DEFAULT_HIDES_EXTRA: [&str; 4] = [
    "~/.cache/huggingface/token",
    "~/.config/rclone",
    "~/.s3cfg",
    "~/.claude/ide",
];
const MANDATORY_ROOTS: [&str; 2] = ["/run/WSL", "/mnt/wslg"];

/// Client-selected confinement options.  The wire module re-exports this
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct ConfineRequest {
    #[serde(default)]
    pub write: Vec<String>,
    #[serde(default)]
    pub hide: Vec<String>,
    #[serde(default)]
    pub unhide: Vec<String>,
    #[serde(default)]
    pub no_default_hides: bool,
    #[serde(default)]
    pub allow_containers: bool,
    #[serde(default)]
    pub team_file: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub home: PathBuf,
    pub state_dir: PathBuf,
    pub runtime_dir: PathBuf,
    pub empty_file: PathBuf,
}

pub struct Plan {
    bwrap: PathBuf,
    empty_file: PathBuf,
    state_dir: PathBuf,
    pub cwd: PathBuf,
    pub harness: Harness,
    pub writable: Vec<PathBuf>,
    pub readonly: Vec<PathBuf>,
    pub protected: Vec<PathBuf>,
    pub hidden: Vec<PathBuf>,
    pub skipped: Vec<PathBuf>,
    pub allow_containers: bool,
    pub mandatory_hidden: Vec<PathBuf>,
    pub default_hidden: Vec<PathBuf>,
    pub container_hidden: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MountKind {
    Writable,
    ReadOnly,
    Empty,
    Tmpfs,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Mount {
    path: PathBuf,
    kind: MountKind,
}

fn canonical_existing(path: &Path, label: &str) -> Result<PathBuf> {
    if !path.is_absolute() {
        bail!(
            "{label} must resolve to an absolute path: {}",
            path.display()
        );
    }
    fs::canonicalize(path)
        .with_context(|| format!("cannot canonicalize {label} {}", path.display()))
}

fn canonical_if_exists(path: &Path) -> Option<PathBuf> {
    path.is_absolute()
        .then(|| fs::canonicalize(path).ok())
        .flatten()
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    is_within(left, right) || is_within(right, left)
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_within(path: &Path, root: &Path) -> bool {
    path == root || path.starts_with(root)
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

fn check_entry_count(values: &[String], label: &str) -> Result<()> {
    if values.len() > MAX_LIST_ENTRIES {
        bail!("{label} has more than {MAX_LIST_ENTRIES} entries");
    }
    for value in values {
        if value.len() > MAX_PATH_BYTES {
            bail!("{label} path exceeds {MAX_PATH_BYTES} bytes");
        }
    }
    Ok(())
}

/// Canonicalize and validate a path that may become writable.
pub fn check_write_allowed(path: &Path, home: &Path, state_dir: &Path) -> Result<PathBuf> {
    let path = canonical_existing(path, "writable path")?;
    let home = canonical_existing(home, "home")?;
    let state_dir = canonical_existing(state_dir, "state directory")?;
    if path == Path::new("/") {
        bail!("writable path / is not allowed");
    }
    if path == home {
        bail!("writable path $HOME is not allowed");
    }
    if paths_overlap(&path, &state_dir) {
        bail!(
            "writable path {} overlaps the state directory",
            path.display()
        );
    }
    if paths_overlap(&path, Path::new("/run/WSL")) || paths_overlap(&path, Path::new("/mnt/wslg")) {
        bail!(
            "writable path {} overlaps a mandatory hidden path",
            path.display()
        );
    }
    Ok(path)
}

fn check_write_runtime_allowed(path: &Path, runtime_dir: &Path) -> Result<()> {
    let runtime_dir = canonical_existing(runtime_dir, "runtime directory")?;
    if paths_overlap(path, &runtime_dir) {
        bail!(
            "writable path {} overlaps the runtime directory",
            path.display()
        );
    }
    Ok(())
}

/// Canonicalize and validate a path that may become visible read-only.
pub fn check_unhide_allowed(
    path: &Path,
    state_dir: &Path,
    runtime_dir: &Path,
    home: &Path,
) -> Result<PathBuf> {
    let path = canonical_existing(path, "unhide path")?;
    let state_dir = canonical_existing(state_dir, "state directory")?;
    let runtime_dir = canonical_existing(runtime_dir, "runtime directory")?;
    let home = canonical_existing(home, "home")?;
    if path == home || is_within(&home, &path) {
        bail!("unhide path {} is home or contains home", path.display());
    }
    if paths_overlap(&path, &state_dir) {
        bail!(
            "unhide path {} overlaps the state directory",
            path.display()
        );
    }
    if paths_overlap(&path, &runtime_dir) {
        bail!(
            "unhide path {} overlaps the runtime directory",
            path.display()
        );
    }
    for mandatory in MANDATORY_ROOTS {
        if paths_overlap(&path, Path::new(mandatory)) {
            bail!(
                "unhide path {} overlaps a mandatory hidden path",
                path.display()
            );
        }
    }
    Ok(path)
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_owned();
    }
    path.strip_prefix("~/")
        .map_or_else(|| PathBuf::from(path), |tail| home.join(tail))
}

fn user_path(path: &str, home: &Path) -> PathBuf {
    expand_home(path, home)
}

fn add_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

fn harness_data(harness: Harness, home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    match harness {
        Harness::Claude => {
            let root = home.join(".claude");
            for name in [
                "projects",
                "todos",
                "shell-snapshots",
                "session-env",
                "sessions",
                "statsig",
                "file-history",
                "paste-cache",
                "debug",
                "cache",
                "telemetry",
                "plans",
                "backups",
            ] {
                dirs.push(root.join(name));
            }
            files.extend([
                root.join("history.jsonl"),
                root.join("stats-cache.json"),
                root.join(".credentials.json"),
                home.join(".claude.json"),
            ]);
        }
        Harness::Omp => {
            let root = home.join(".omp");
            for name in [
                "cache",
                "logs",
                "run",
                "agent/sessions",
                "agent/projects",
                "agent/cache",
                "agent/blobs",
            ] {
                dirs.push(root.join(name));
            }
            let parent = root.join("agent");
            let mut databases = BTreeSet::new();
            if let Ok(entries) = fs::read_dir(&parent) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path
                        .file_name()
                        .and_then(OsStr::to_str)
                        .is_some_and(|name| name.ends_with(".db"))
                    {
                        databases.insert(path);
                    }
                }
            }
            for database in databases {
                files.push(database.clone());
                files.push(PathBuf::from(format!("{}-wal", database.display())));
                files.push(PathBuf::from(format!("{}-shm", database.display())));
            }
            let stats = root.join("stats.db");
            if stats.is_file() {
                files.extend([
                    stats.clone(),
                    PathBuf::from(format!("{}-wal", stats.display())),
                    PathBuf::from(format!("{}-shm", stats.display())),
                ]);
            }
        }
        Harness::Generic | Harness::Codex => {}
    }
    (dirs, files)
}
fn validate_harness_data_paths(
    harness: Harness,
    home: &Path,
    state_dir: &Path,
    runtime_dir: &Path,
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
) -> Result<()> {
    let roots = match harness {
        Harness::Claude => vec![home.join(".claude")],
        Harness::Omp => vec![home.join(".omp"), home.join(".omp/agent")],
        Harness::Generic | Harness::Codex => Vec::new(),
    };
    for path in roots
        .into_iter()
        .chain(data_dirs.iter().cloned())
        .chain(data_files.iter().cloned())
    {
        if let Some(path) = canonical_if_exists(&path) {
            check_write_allowed(&path, home, state_dir)?;
            check_write_runtime_allowed(&path, runtime_dir)?;
        }
    }
    Ok(())
}
/// Create the existing-harness data directories that are writable by a plan.
pub fn ensure_data_dirs(harness: Harness, home: &Path) -> Result<()> {
    let root = match harness {
        Harness::Claude => home.join(".claude"),
        Harness::Omp => home.join(".omp"),
        Harness::Generic | Harness::Codex => return Ok(()),
    };
    if !root.is_dir() {
        return Ok(());
    }
    let (dirs, _) = harness_data(harness, home);
    for directory in dirs {
        if directory.exists() {
            if !directory.is_dir() {
                bail!(
                    "harness data path is not a directory: {}",
                    directory.display()
                );
            }
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        } else {
            fs::create_dir_all(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Create only OMP's SQLite sidecars; never create a credential or database file.
pub fn ensure_data_files(harness: Harness, home: &Path) -> Result<()> {
    if harness != Harness::Omp || !home.join(".omp").is_dir() {
        return Ok(());
    }
    let (_, files) = harness_data(Harness::Omp, home);
    let dbs = files
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(OsStr::to_str)
                .is_some_and(|name| name.ends_with(".db"))
        })
        .collect::<Vec<_>>();
    for db in dbs {
        if !db.is_file() {
            continue;
        }
        for suffix in ["-wal", "-shm"] {
            let sidecar = PathBuf::from(format!("{}{}", db.display(), suffix));
            if !sidecar.exists() {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .open(&sidecar)?;
            }
            fs::set_permissions(&sidecar, fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(())
}

fn parse_core_hooks_path(common: &Path, worktree: &Path) -> Option<PathBuf> {
    let config = common.join("config");
    let text = fs::read_to_string(config).ok()?;
    let mut in_core = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_core = line.eq_ignore_ascii_case("[core]");
            continue;
        }
        if !in_core {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        if key.trim().eq_ignore_ascii_case("hooksPath") {
            let value = value.trim().trim_matches(['"', '\'']);
            let path = PathBuf::from(value);
            return Some(if path.is_absolute() {
                path
            } else {
                worktree.join(path)
            });
        }
    }
    None
}

fn git_common_dir(cwd: &Path, home: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut current = cwd.to_owned();
    loop {
        if current == home {
            return None;
        }
        let marker = current.join(".git");
        if marker.is_dir() {
            return fs::canonicalize(marker)
                .ok()
                .map(|common| (common, current));
        }
        if marker.is_file() {
            let text = fs::read_to_string(&marker).ok()?;
            let target = text
                .lines()
                .find_map(|line| line.trim().strip_prefix("gitdir:"))?
                .trim();
            let gitdir = PathBuf::from(target);
            let gitdir = if gitdir.is_absolute() {
                gitdir
            } else {
                current.join(gitdir)
            };
            let gitdir = fs::canonicalize(gitdir).ok()?;
            let common = match fs::read_to_string(gitdir.join("commondir")) {
                Ok(relative) => {
                    let path = PathBuf::from(relative.trim());
                    if path.is_absolute() {
                        path
                    } else {
                        gitdir.join(path)
                    }
                }
                Err(_) => gitdir,
            };
            let common = fs::canonicalize(common).ok()?;
            return (common != home).then_some((common, current));
        }
        let parent = current.parent()?.to_owned();
        if parent == current {
            return None;
        }
        current = parent;
    }
}

fn plan_empty_file(host: &Host, state_dir: &Path) -> Result<PathBuf> {
    let candidate_name = host
        .empty_file
        .file_name()
        .ok_or_else(|| anyhow!("empty file path has no file name"))?;
    if candidate_name != "confine-empty" {
        bail!("empty file must be named confine-empty");
    }
    let parent = host
        .empty_file
        .parent()
        .ok_or_else(|| anyhow!("empty file path has no parent"))?;
    let parent = canonical_existing(parent, "empty file directory")?;
    if parent != state_dir {
        bail!("empty file must be inside the state directory");
    }
    let candidate = parent.join(candidate_name);
    if candidate.exists() {
        let path = canonical_existing(&candidate, "empty file")?;
        if !is_within(&path, state_dir) || fs::metadata(&path)?.len() != 0 {
            bail!("confinement empty file must be an empty state file");
        }
        Ok(path)
    } else {
        Ok(candidate)
    }
}
fn add_canonical_existing(
    source: &Path,
    label: &str,
    skipped: &mut Vec<PathBuf>,
) -> Option<PathBuf> {
    match fs::canonicalize(source) {
        Ok(path) => Some(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            add_unique(skipped, source.to_owned());
            None
        }
        Err(_) => {
            add_unique(skipped, source.to_owned());
            tracing::debug!(path = %source.display(), %label, "confinement path skipped");
            None
        }
    }
}

fn add_validated_write(
    path: &Path,
    host: &Host,
    writable: &mut Vec<PathBuf>,
    skipped: &mut Vec<PathBuf>,
) -> Result<Option<PathBuf>> {
    let Some(path) = add_canonical_existing(path, "write", skipped) else {
        return Ok(None);
    };
    let path = check_write_allowed(&path, &host.home, &host.state_dir)?;
    check_write_runtime_allowed(&path, &host.runtime_dir)?;
    add_unique(writable, path.clone());
    Ok(Some(path))
}

fn add_optional_hide(path: &Path, hidden: &mut Vec<PathBuf>, skipped: &mut Vec<PathBuf>) {
    if let Some(path) = add_canonical_existing(path, "hide", skipped) {
        add_unique(hidden, path);
    }
}

fn add_harness_data_mounts(
    harness: Harness,
    home: &Path,
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    writable: &mut Vec<PathBuf>,
    skipped: &mut Vec<PathBuf>,
) -> Result<()> {
    if harness == Harness::Generic || harness == Harness::Codex {
        return Ok(());
    }
    let root = match harness {
        Harness::Claude => home.join(".claude"),
        Harness::Omp => home.join(".omp"),
        Harness::Generic | Harness::Codex => unreachable!(),
    };
    if !root.is_dir() {
        return Ok(());
    }
    for path in data_dirs {
        match fs::canonicalize(path) {
            Ok(path) => add_unique(writable, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if fs::symlink_metadata(path).is_ok() {
                    bail!(
                        "harness data path cannot be canonicalized: {}",
                        path.display()
                    );
                }
                add_unique(writable, path.clone());
            }
            Err(error) => return Err(error.into()),
        }
    }
    for path in data_files {
        if let Some(path) = add_canonical_existing(path, "harness data", skipped) {
            add_unique(writable, path);
            continue;
        }
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        let Some(base) = name
            .strip_suffix("-wal")
            .or_else(|| name.strip_suffix("-shm"))
        else {
            continue;
        };
        let database = path.with_file_name(base);
        if database.is_file() {
            add_unique(writable, path.clone());
        }
    }
    Ok(())
}

/// Reject harness profiles whose control plane runs outside the confined PTY.
pub fn check_harness(harness: Harness) -> Result<()> {
    if harness == Harness::Codex {
        bail!(
            "confinement is not supported for Codex sessions yet: its tools run outside the session process"
        );
    }
    Ok(())
}

/// Build the complete mount plan.  Paths are canonicalized before every
/// containment comparison and missing operator paths are retained in
/// `skipped` for dry-run reporting but never passed to bubblewrap.
pub fn plan(request: &ConfineRequest, harness: Harness, cwd: &Path, host: &Host) -> Result<Plan> {
    check_request_limits(request, harness)?;
    let mut builder = PlanBuilder::new(host, harness)?;
    let cwd = canonical_existing(cwd, "cwd")?;
    if !cwd.is_dir() {
        bail!("cwd is not a directory: {}", cwd.display());
    }
    let empty_file = plan_empty_file(host, &builder.state_dir)?;
    builder.bind_workspace(&cwd)?;
    builder.bind_harness_data()?;
    builder.bind_operator_writes(request)?;
    builder.hide_mandatory();
    builder.hide_defaults(request);
    builder.hide_containers(request);
    builder.release_operator_writes(request)?;
    builder.apply_operator_hides(request)?;
    builder.finish(request, cwd, empty_file)
}

fn check_request_limits(request: &ConfineRequest, harness: Harness) -> Result<()> {
    check_entry_count(&request.write, "confine_write")?;
    check_entry_count(&request.hide, "confine_hide")?;
    check_entry_count(&request.unhide, "confine_unhide")?;
    if request
        .team_file
        .as_deref()
        .is_some_and(|path| path.len() > MAX_PATH_BYTES)
    {
        bail!("team_file path exceeds {MAX_PATH_BYTES} bytes");
    }
    check_harness(harness)
}

/// The mount lists while the jobs of `plan` fill them.
struct PlanBuilder<'a> {
    host: &'a Host,
    harness: Harness,
    home: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
    writable: Vec<PathBuf>,
    readonly: Vec<PathBuf>,
    hidden: Vec<PathBuf>,
    skipped: Vec<PathBuf>,
    explicit_writes: BTreeSet<PathBuf>,
    mandatory_hidden: Vec<PathBuf>,
    default_hidden: Vec<PathBuf>,
    container_hidden: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
enum HideKind {
    Default,
    Container,
}

impl<'a> PlanBuilder<'a> {
    fn new(host: &'a Host, harness: Harness) -> Result<Self> {
        Ok(Self {
            host,
            harness,
            home: canonical_existing(&host.home, "home")?,
            state_dir: canonical_existing(&host.state_dir, "state directory")?,
            runtime_dir: canonical_existing(&host.runtime_dir, "runtime directory")?,
            writable: Vec::new(),
            readonly: Vec::new(),
            hidden: Vec::new(),
            skipped: Vec::new(),
            explicit_writes: BTreeSet::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
        })
    }

    fn bind_workspace(&mut self, cwd: &Path) -> Result<()> {
        let _ = add_validated_write(cwd, self.host, &mut self.writable, &mut self.skipped)?;
        if let Some((common, worktree)) = git_common_dir(cwd, &self.home) {
            let _ = add_validated_write(&common, self.host, &mut self.writable, &mut self.skipped)?;
            self.protect_git_metadata(&common, &worktree);
        }
        Ok(())
    }

    fn protect_git_metadata(&mut self, common: &Path, worktree: &Path) {
        for name in ["hooks", "config", "config.worktree"] {
            if let Ok(path) = common.join(name).canonicalize() {
                add_unique(&mut self.readonly, path);
            }
        }
        if let Some(path) =
            parse_core_hooks_path(common, worktree).and_then(|path| path.canonicalize().ok())
        {
            add_unique(&mut self.readonly, path);
        }
    }

    fn bind_harness_data(&mut self) -> Result<()> {
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        validate_harness_data_paths(
            self.harness,
            &self.home,
            &self.state_dir,
            &self.runtime_dir,
            &data_dirs,
            &data_files,
        )?;
        add_harness_data_mounts(
            self.harness,
            &self.home,
            &data_dirs,
            &data_files,
            &mut self.writable,
            &mut self.skipped,
        )
    }

    fn bind_operator_writes(&mut self, request: &ConfineRequest) -> Result<()> {
        for path in &request.write {
            let source = user_path(path, &self.home);
            if let Some(path) =
                add_validated_write(&source, self.host, &mut self.writable, &mut self.skipped)?
            {
                self.explicit_writes.insert(path);
            }
        }
        if let Some(team_file) = &request.team_file
            && let Some(path) = add_canonical_existing(
                &user_path(team_file, &self.home),
                "team file",
                &mut self.skipped,
            )
            && self.writable.iter().any(|write| is_within(&path, write))
        {
            add_unique(&mut self.readonly, path);
        }
        Ok(())
    }

    fn hide_mandatory(&mut self) {
        for path in [self.runtime_dir.clone(), self.state_dir.clone()] {
            add_unique(&mut self.mandatory_hidden, path);
        }
        for path in MANDATORY_ROOTS {
            if let Some(path) = canonical_if_exists(Path::new(path)) {
                add_unique(&mut self.mandatory_hidden, path);
            } else {
                add_unique(&mut self.skipped, PathBuf::from(path));
            }
        }
        for path in &self.mandatory_hidden {
            add_unique(&mut self.hidden, path.clone());
        }
    }

    fn hide_defaults(&mut self, request: &ConfineRequest) {
        if request.no_default_hides {
            return;
        }
        for name in DEFAULT_HIDES.into_iter().chain(DEFAULT_HIDES_EXTRA) {
            self.hide_or_skip(user_path(name, &self.home), HideKind::Default);
        }
        if self.harness != Harness::Claude {
            self.hide_or_skip(
                self.home.join(".claude/.credentials.json"),
                HideKind::Default,
            );
        }
        if self.harness != Harness::Codex {
            self.hide_or_skip(self.home.join(".codex/auth.json"), HideKind::Default);
        }
        if let Some(path) = canonical_if_exists(Path::new("/run/dbus")) {
            add_unique(&mut self.default_hidden, path.clone());
            add_unique(&mut self.hidden, path);
        }
    }

    fn hide_containers(&mut self, request: &ConfineRequest) {
        if request.allow_containers {
            return;
        }
        for entry in CONTAINER_HIDES {
            let source = if let Some(relative) = entry.strip_prefix("~/") {
                self.home.join(relative)
            } else {
                PathBuf::from(entry)
            };
            self.hide_or_skip(source, HideKind::Container);
        }
    }

    fn hide_or_skip(&mut self, source: PathBuf, kind: HideKind) {
        if let Some(path) = canonical_if_exists(&source) {
            match kind {
                HideKind::Default => add_unique(&mut self.default_hidden, path.clone()),
                HideKind::Container => add_unique(&mut self.container_hidden, path.clone()),
            }
            add_unique(&mut self.hidden, path);
        } else {
            add_unique(&mut self.skipped, source);
        }
    }

    fn check_container_socket(
        &self,
        path: &Path,
        option: &str,
        allow_containers: bool,
    ) -> Result<()> {
        if !allow_containers
            && self
                .container_hidden
                .iter()
                .any(|socket| is_within(path, socket))
        {
            bail!(
                "{option} path {} requires --confine-allow-containers",
                path.display()
            );
        }
        Ok(())
    }

    fn release_operator_writes(&mut self, request: &ConfineRequest) -> Result<()> {
        for path in &self.explicit_writes {
            self.check_container_socket(path, "confine_write", request.allow_containers)?;
            self.hidden.retain(|existing| existing != path);
            self.default_hidden.retain(|existing| existing != path);
        }
        Ok(())
    }

    fn apply_operator_hides(&mut self, request: &ConfineRequest) -> Result<()> {
        for path in &request.hide {
            add_optional_hide(
                &user_path(path, &self.home),
                &mut self.hidden,
                &mut self.skipped,
            );
        }
        for path in &request.unhide {
            self.unhide(path, request.allow_containers)?;
        }
        Ok(())
    }

    fn unhide(&mut self, path: &str, allow_containers: bool) -> Result<()> {
        let source = user_path(path, &self.home);
        if let Some(path) = add_canonical_existing(&source, "unhide", &mut self.skipped) {
            check_unhide_allowed(&path, &self.state_dir, &self.runtime_dir, &self.home)?;
            self.check_container_socket(&path, "confine_unhide", allow_containers)?;
            self.hidden.retain(|existing| existing != &path);
            self.default_hidden.retain(|existing| existing != &path);
            self.container_hidden.retain(|existing| existing != &path);
            add_unique(&mut self.readonly, path);
        }
        Ok(())
    }

    fn finish(
        mut self,
        request: &ConfineRequest,
        cwd: PathBuf,
        empty_file: PathBuf,
    ) -> Result<Plan> {
        self.readonly
            .retain(|path| !self.explicit_writes.contains(path));
        let protected = protected_paths(&self.readonly, &self.writable);
        self.readonly.sort();
        self.writable.sort();
        self.hidden.sort();
        self.mandatory_hidden.sort();
        self.default_hidden.sort();
        self.container_hidden.sort();
        self.skipped.sort();

        let plan = Plan {
            bwrap: PathBuf::from("bwrap"),
            empty_file,
            state_dir: self.state_dir.clone(),
            cwd,
            harness: self.harness,
            writable: self.writable,
            readonly: self.readonly,
            protected,
            hidden: self.hidden,
            skipped: self.skipped,
            allow_containers: request.allow_containers,
            mandatory_hidden: self.mandatory_hidden,
            default_hidden: self.default_hidden,
            container_hidden: self.container_hidden,
        };
        // Force the expensive argument-size validation while still in the
        // daemon-side preflight path, before a PTY can be created.
        plan.wrap(vec!["true".to_owned()])?;
        Ok(plan)
    }
}

fn protected_paths(readonly: &[PathBuf], writable: &[PathBuf]) -> Vec<PathBuf> {
    let mut protected = Vec::new();
    let mut protected_set = BTreeSet::new();
    for path in readonly {
        if writable.iter().any(|write| is_within(path, write))
            && protected_set.insert(path_key(path))
        {
            protected.push(path.clone());
        }
    }
    protected.sort();
    protected
}

impl Plan {
    pub fn with_bwrap(mut self, path: PathBuf) -> Self {
        self.bwrap = path;
        self
    }

    pub fn is_confined(&self) -> bool {
        true
    }

    fn mounts(&self) -> Vec<Mount> {
        let mut mounts = BTreeMap::<PathBuf, MountKind>::new();
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
        if self.harness == Harness::Omp {
            let omp = self.state_dir.join("omp");
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
                        MountKind::Tmpfs | MountKind::Empty => 0,
                        MountKind::Writable | MountKind::ReadOnly => 1,
                    };
                    rank(left.kind).cmp(&rank(right.kind))
                })
                .then_with(|| left.path.cmp(&right.path))
        });
        mounts
    }

    fn env_unsets(&self) -> impl Iterator<Item = &'static str> {
        ALWAYS_UNSET
            .into_iter()
            .chain(
                (!self.allow_containers)
                    .then_some(CONTAINER_UNSET.iter().copied())
                    .into_iter()
                    .flatten(),
            )
            .chain(
                (!self.allow_containers)
                    .then_some(CONTAINER_UNSET_EXTRA.iter().copied())
                    .into_iter()
                    .flatten(),
            )
    }

    /// Produce `bwrap <options> -- <argv>` with deterministic operation order.
    pub fn wrap(&self, argv: Vec<String>) -> Result<Vec<String>> {
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
        let mounts = self.mounts();
        let mut remounts = Vec::new();
        for mount in mounts {
            match mount.kind {
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
                MountKind::Empty => {
                    wrapped.extend([
                        "--ro-bind".into(),
                        path_key(&self.empty_file),
                        path_key(&mount.path),
                    ]);
                }
                MountKind::Tmpfs => {
                    wrapped.extend(["--tmpfs".into(), path_key(&mount.path)]);
                    remounts.push(mount.path);
                }
            }
        }
        for path in remounts {
            wrapped.extend(["--remount-ro".into(), path_key(&path)]);
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
}

fn command_with_timeout(path: &Path, args: &[String]) -> Result<()> {
    let mut child = Command::new(path)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot run {}", path.display()))?;
    let deadline = Instant::now() + PROBE_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let mut message = format!("{} exited with {status}", path.display());
            if let Some(mut stderr) = child.stderr.take() {
                use std::io::Read;
                let mut bytes = Vec::new();
                stderr.read_to_end(&mut bytes).ok();
                if !bytes.is_empty() {
                    message.push_str(": ");
                    message.push_str(&String::from_utf8_lossy(&bytes));
                }
            }
            bail!("{message}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} timed out after {} seconds",
                path.display(),
                PROBE_TIMEOUT.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Resolve a bwrap executable from a daemon-owned PATH-style value.
pub fn resolve_bwrap(path_var: Option<&OsStr>) -> Result<PathBuf> {
    let path_var = path_var
        .map(OsStr::to_os_string)
        .or_else(|| std::env::var_os("PATH"))
        .ok_or_else(|| anyhow!("PATH is not set; cannot find bubblewrap"))?;
    let path = PathBuf::from(path_var);
    if path.as_os_str().is_empty() {
        bail!("PATH is empty; cannot find bubblewrap");
    }
    if path.components().count() > 1 && path.is_file() {
        return fs::canonicalize(&path).context("canonicalizing bwrap path");
    }
    let candidate_name = if path.is_file() {
        path.clone()
    } else {
        PathBuf::from("bwrap")
    };
    if candidate_name.is_absolute() {
        return fs::canonicalize(&candidate_name).context("canonicalizing bwrap path");
    }
    for entry in std::env::split_paths(&path) {
        let entry = if entry.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            entry
        };
        let candidate = entry.join(&candidate_name);
        if candidate.is_file() {
            let metadata = fs::metadata(&candidate)?;
            if metadata.permissions().mode() & 0o111 != 0 {
                return fs::canonicalize(candidate).context("canonicalizing bwrap path");
            }
        }
    }
    bail!("bubblewrap was not found in PATH")
}

/// Run version and a real base-mount probe.  A failure is fatal; callers must
/// not fall back to an unwrapped PTY after this returns an error.
pub fn preflight(bwrap: &Path) -> Result<()> {
    let bwrap = canonical_existing(bwrap, "bubblewrap")?;
    command_with_timeout(&bwrap, &["--version".into()])?;
    let probe = vec![
        "--die-with-parent".into(),
        "--unshare-user".into(),
        "--unshare-ipc".into(),
        "--unshare-pid".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--size".into(),
        "1073741824".into(),
        "--tmpfs".into(),
        "/tmp".into(),
        "--".into(),
        "true".into(),
    ];
    command_with_timeout(&bwrap, &probe)
}

/// Create the host-side empty regular file used for hidden files.
pub fn ensure_empty_file(state_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(state_dir)?;
    let path = state_dir.join("confine-empty");
    if !path.exists() {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o400)
            .open(&path)?;
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400))?;
    Ok(path)
}

fn capped_paths(paths: &[PathBuf]) -> String {
    let shown = paths.len().min(20);
    let mut output = paths[..shown]
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    if paths.len() > shown {
        if !output.is_empty() {
            output.push_str(", ");
        }
        output.push_str(&format!("and {} more", paths.len() - shown));
    }
    output
}

fn truncate_before_closing(mut text: String) -> String {
    let suffix_len = CLOSING_NOTICE.len() + 2;
    if text.len() + suffix_len > NOTICE_LIMIT {
        let mut keep = NOTICE_LIMIT.saturating_sub(suffix_len);
        while !text.is_char_boundary(keep) {
            keep -= 1;
        }
        text.truncate(keep);
    }
    text.push_str("\n\n");
    text.push_str(CLOSING_NOTICE);
    text
}

/// Render the operator-facing notice from the exact mount plan.
pub fn notice(plan: &Plan) -> String {
    let mut text = String::new();
    text.push_str("This session is confined.\n");
    text.push_str("Writable locations: ");
    text.push_str(&capped_paths(&plan.writable));
    text.push_str(".\n");
    text.push_str("Read-only protected paths: ");
    text.push_str(&capped_paths(&plan.protected));
    text.push_str(".\n");
    text.push_str("Hidden locations (a hidden directory looks empty; a hidden file looks empty): ");
    text.push_str(&capped_paths(&plan.hidden));
    text.push_str(".\n");
    if !plan.skipped.is_empty() {
        text.push_str("Skipped missing paths: ");
        text.push_str(&capped_paths(&plan.skipped));
        text.push_str(".\n");
    }
    text.push_str(
        "Everything else is read-only. /tmp is private to this session and discarded when it ends; paths under a peer's /tmp cannot be read.\n",
    );
    text.push_str(if plan.allow_containers {
        "Container engines are allowed.\n"
    } else {
        "Container engines are restricted.\n"
    });
    text.push_str(
        "A Read-only file system error is authoritative where this notice and the system disagree.",
    );
    truncate_before_closing(text)
}

/// Build a wrapped command and, where the harness has a system prompt channel,
/// append the notice.  Generic sessions intentionally receive no notice.
pub fn launch_argv(
    plan: &Plan,
    notice: &str,
    argv: Vec<String>,
    harness: Harness,
) -> Result<Vec<String>> {
    let wrapped = plan.wrap(argv)?;
    match harness {
        Harness::Claude | Harness::Omp => {
            crate::harness::append_system_prompt(wrapped, notice, harness, &plan.cwd)
        }
        Harness::Generic => Ok(wrapped),
        Harness::Codex => bail!(
            "confinement is not supported for Codex sessions yet: its tools run outside the session process"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_notice_is_always_present() {
        let plan = Plan {
            bwrap: PathBuf::from("bwrap"),
            empty_file: PathBuf::from("/dev/null"),
            state_dir: PathBuf::from("/state"),
            cwd: PathBuf::from("/workspace"),
            harness: Harness::Generic,
            writable: (0..100).map(|n| PathBuf::from(format!("/w/{n}"))).collect(),
            readonly: Vec::new(),
            protected: Vec::new(),
            hidden: Vec::new(),
            skipped: Vec::new(),
            allow_containers: false,
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
        };
        let text = notice(&plan);
        assert!(text.len() <= NOTICE_LIMIT);
        assert!(text.ends_with(CLOSING_NOTICE));
    }
}
