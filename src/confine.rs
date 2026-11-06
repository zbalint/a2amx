//! Opt-in bubblewrap confinement for PTY sessions.
//!
//! This module owns every bubblewrap argument and probe.  Callers pass plain
//! paths and a harness profile; the resulting [`Plan`] is the sole source for
//! mounts, the launch argv, the notice, and dry-run visibility.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rustix::fs::OFlags;

use anyhow::{Context, Result, anyhow, bail};

use crate::grant;
use crate::harness::Harness;
use crate::wire::{ConfineInfo, GrantReason};

pub const MAX_LIST_ENTRIES: usize = 64;
pub const MAX_PATH_BYTES: usize = 4096;
const MAX_ARG_BYTES: usize = 128 * 1024;
const MAX_ARGV_BYTES: usize = 256 * 1024;
const NOTICE_LIMIT: usize = 12 * 1024;
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub const ENV_PATH: &str = "/usr/bin/env";

pub const CLOSING_NOTICE: &str = "Do not try to get around these limits, whether through other programs, services or sockets, through other agents, or by editing configuration to loosen them. If your task needs access they do not allow, stop that part and tell the person running this session which path or access you need and why. Other errors, such as a missing file or a failing build or test, are ordinary: handle them as usual.";

const ALWAYS_UNSET: [&str; 6] = [
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "GPG_AGENT_INFO",
    "WSL_INTEROP",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];
const CREDENTIAL_ENV: [&str; 29] = [
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITLAB_TOKEN",
    "GITLAB_PRIVATE_TOKEN",
    "CI_JOB_TOKEN",
    "BITBUCKET_TOKEN",
    "NPM_TOKEN",
    "NODE_AUTH_TOKEN",
    "PYPI_TOKEN",
    "TWINE_PASSWORD",
    "CARGO_REGISTRY_TOKEN",
    "CARGO_REGISTRIES_CRATES_IO_TOKEN",
    "AZURE_CLIENT_SECRET",
    "ARM_CLIENT_SECRET",
    "VAULT_TOKEN",
    "HF_TOKEN",
    "HUGGING_FACE_HUB_TOKEN",
    "SONAR_TOKEN",
    "SENTRY_AUTH_TOKEN",
    "DIGITALOCEAN_ACCESS_TOKEN",
    "HCLOUD_TOKEN",
    "CLOUDFLARE_API_TOKEN",
    "NETLIFY_AUTH_TOKEN",
    "VERCEL_TOKEN",
    "FLY_API_TOKEN",
    "HEROKU_API_KEY",
    "PGPASSWORD",
    "MYSQL_PWD",
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
    pub grants: Vec<String>,
    #[serde(default)]
    pub auto: bool,
    #[serde(default)]
    pub reasons: Vec<GrantReason>,
    #[serde(default)]
    pub keep_env: Vec<String>,
    #[serde(default)]
    pub team_file: Option<String>,
}

impl ConfineRequest {
    /// Merge detected grants into this request and record only newly added grants.
    pub fn apply_auto(&mut self, found: Vec<(&'static str, String)>) {
        let mut added = Vec::new();
        for (grant, reason) in found {
            if !self.grants.iter().any(|current| current == grant) {
                self.grants.push(grant.to_owned());
                added.push(GrantReason {
                    grant: grant.to_owned(),
                    reason,
                });
            }
        }
        self.auto = true;
        self.reasons = added;
    }

    /// Return grants that were explicitly requested rather than auto-detected.
    pub fn explicit_grants(&self) -> Vec<String> {
        self.grants
            .iter()
            .filter(|grant| !self.reasons.iter().any(|reason| reason.grant == **grant))
            .cloned()
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub option: &'static str,
    pub path: PathBuf,
    pub reason: String,
}

/// Reject a confinement entry that no later stage would understand.
pub fn check_entry_syntax(option: &str, entry: &str) -> Result<()> {
    if entry.is_empty() {
        bail!("{option} entry is empty");
    }
    if entry.starts_with('$') {
        bail!(
            "{option} entry {entry:?}: environment variables are not expanded; write the path out, or use ~/ for your home"
        );
    }
    if entry.starts_with('~') && entry != "~" && !entry.starts_with("~/") {
        bail!("{option} entry {entry:?}: only ~ and ~/... are understood, not another user's home");
    }
    Ok(())
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
    pub unresolved: Vec<Unresolved>,
    pub grants: Vec<String>,
    pub effective_grants: Vec<String>,
    pub auto: bool,
    pub reasons: Vec<GrantReason>,
    pub write: Vec<String>,
    pub keep_env: Vec<String>,
    pub mandatory_hidden: Vec<PathBuf>,
    pub default_hidden: Vec<PathBuf>,
    pub container_hidden: Vec<PathBuf>,
    overlay_parents: Vec<PathBuf>,
    overlay_protected: Vec<PathBuf>,
    unsaved_files: Vec<PathBuf>,
    overlay_notice: Option<String>,
    harness_config: Option<PathBuf>,
}

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
fn check_absolute_entry(option: &str, entry: &str) -> Result<()> {
    if entry != "~" && !entry.starts_with("~/") && !Path::new(entry).is_absolute() {
        bail!("{option} entry {entry:?} must be an absolute path or start with ~/");
    }
    Ok(())
}

/// Canonicalize and validate a path that may become writable.
pub fn check_write_allowed(
    path: &Path,
    home: &Path,
    state_dir: &Path,
    explicit: bool,
) -> Result<PathBuf> {
    let path = canonical_existing(path, "writable path")?;
    let home = canonical_existing(home, "home")?;
    let state_dir = canonical_existing(state_dir, "state directory")?;
    if path == Path::new("/") {
        bail!("writable path / is not allowed");
    }
    if !explicit && (path == home || is_within(&home, &path)) {
        bail!("writable path $HOME is not allowed");
    }
    let state_overlap = if explicit {
        is_within(&path, &state_dir)
    } else {
        paths_overlap(&path, &state_dir)
    };
    if state_overlap {
        bail!(
            "writable path {} overlaps the state directory",
            path.display()
        );
    }
    for mandatory in MANDATORY_ROOTS {
        let mandatory = Path::new(mandatory);
        let overlap = if explicit {
            is_within(&path, mandatory)
        } else {
            paths_overlap(&path, mandatory)
        };
        if overlap {
            bail!(
                "writable path {} overlaps a mandatory hidden path",
                path.display()
            );
        }
    }
    Ok(path)
}

fn check_write_runtime_allowed(path: &Path, runtime_dir: &Path, explicit: bool) -> Result<()> {
    let runtime_dir = canonical_existing(runtime_dir, "runtime directory")?;
    let overlap = if explicit {
        is_within(path, &runtime_dir)
    } else {
        paths_overlap(path, &runtime_dir)
    };
    if overlap {
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
fn symlink_component(path: &Path) -> Option<PathBuf> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Some(current);
        }
    }
    None
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

fn harness_overlay_parents(
    home: &Path,
    data_files: &[PathBuf],
    writable: &[PathBuf],
    hidden: &[PathBuf],
    readonly: &[PathBuf],
    protected: &[PathBuf],
) -> Vec<PathBuf> {
    let mut parents = Vec::new();
    for file in data_files {
        let Some(parent) = file.parent().and_then(canonical_if_exists) else {
            continue;
        };
        if parent == home || !parent.is_dir() {
            continue;
        }
        if [writable, hidden, readonly, protected]
            .into_iter()
            .any(|paths| paths.iter().any(|path| is_within(&parent, path)))
        {
            continue;
        }
        add_unique(&mut parents, parent);
    }
    parents.sort();
    parents
}

fn overlay_protected_entries(
    parents: &[PathBuf],
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    writable: &[PathBuf],
    readonly: &[PathBuf],
    protected: &[PathBuf],
    hidden: &[PathBuf],
) -> Vec<PathBuf> {
    let mut entries_protected = Vec::new();
    for parent in parents {
        let Ok(entries) = fs::read_dir(parent) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_symlink()
                || (!metadata.file_type().is_file() && !metadata.file_type().is_dir())
            {
                continue;
            }
            let excluded = data_dirs
                .iter()
                .chain(data_files)
                .chain(writable)
                .chain(readonly)
                .chain(protected)
                .chain(parents)
                .chain(hidden)
                .any(|other| {
                    other == &path
                        || canonical_if_exists(other).is_some_and(|canonical| canonical == path)
                });
            if !excluded {
                add_unique(&mut entries_protected, path);
            }
        }
    }
    entries_protected.sort();
    entries_protected
}

fn unsaved_harness_files(data_files: &[PathBuf], overlay_parents: &[PathBuf]) -> Vec<PathBuf> {
    let mut unsaved = Vec::new();
    for file in data_files {
        if file.exists() {
            continue;
        }
        let Some(parent) = file.parent().and_then(canonical_if_exists) else {
            continue;
        };
        if !overlay_parents.contains(&parent) {
            continue;
        }
        let Some(name) = file.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if let Some(base) = name
            .strip_suffix("-wal")
            .or_else(|| name.strip_suffix("-shm"))
            && file.with_file_name(base).is_file()
        {
            continue;
        }
        add_unique(&mut unsaved, file.clone());
    }
    unsaved.sort();
    unsaved
}

fn validate_harness_data_paths(
    harness: Harness,
    home: &Path,
    state_dir: &Path,
    runtime_dir: &Path,
    data_dirs: &[PathBuf],
    data_files: &[PathBuf],
    explicit_writes: &BTreeSet<PathBuf>,
) -> Result<()> {
    let expected_sources = match harness {
        Harness::Claude => vec![home.join(".claude")],
        Harness::Omp => vec![home.join(".omp/agent"), home.join(".omp")],
        Harness::Generic | Harness::Codex => Vec::new(),
    };
    let expected_roots = expected_sources
        .iter()
        .filter_map(|source| fs::canonicalize(source).ok().map(|root| (source, root)))
        .collect::<Vec<_>>();
    let home_file = home.join(".claude.json");
    let outside_error = |path: &Path, target: &Path, root: &Path| -> anyhow::Error {
        anyhow!(
            "harness data path {} resolves to {}, outside {}; remove the link, keep the data inside {}, or open it with write = [\"{}\"] if that is intended",
            path.display(),
            target.display(),
            root.display(),
            root.display(),
            target.display()
        )
    };
    for path in expected_sources
        .iter()
        .cloned()
        .chain(data_dirs.iter().cloned())
        .chain(data_files.iter().cloned())
    {
        let Some(target) = canonical_if_exists(&path) else {
            continue;
        };
        let explicit = explicit_writes.contains(&target);
        if !explicit {
            if harness == Harness::Claude && path == home_file {
                let is_file = fs::metadata(&target).is_ok_and(|metadata| metadata.is_file());
                if !is_file || target.parent() != Some(home) {
                    return Err(outside_error(&path, &target, home));
                }
            } else {
                let Some((_, root)) = expected_roots
                    .iter()
                    .find(|(source, _)| is_within(&path, source))
                else {
                    return Err(outside_error(&path, &target, home));
                };
                if !is_within(&target, root) {
                    return Err(outside_error(&path, &target, root));
                }
            }
        }
        check_write_allowed(&target, home, state_dir, explicit)?;
        check_write_runtime_allowed(&target, runtime_dir, explicit)?;
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
struct GrantDirScope {
    home: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
    hidden: Vec<PathBuf>,
}

fn grant_dir_scope(
    request: &ConfineRequest,
    harness: Harness,
    host: &Host,
) -> Option<GrantDirScope> {
    let home = fs::canonicalize(&host.home).ok()?;
    let state_dir = fs::canonicalize(&host.state_dir).ok()?;
    let runtime_dir = fs::canonicalize(&host.runtime_dir).ok()?;
    let mut hidden = if request.no_default_hides {
        Vec::new()
    } else {
        default_hide_sources_for(&home, harness)
    };
    hidden.extend(
        request
            .hide
            .iter()
            .map(|path| user_path(path, &home))
            .filter_map(|path| canonical_if_exists(&path)),
    );
    let unhidden = request
        .unhide
        .iter()
        .map(|path| user_path(path, &home))
        .filter_map(|path| canonical_if_exists(&path))
        .collect::<Vec<_>>();
    hidden.retain(|path| !unhidden.iter().any(|unhidden| unhidden == path));
    Some(GrantDirScope {
        home,
        state_dir,
        runtime_dir,
        hidden,
    })
}

fn grant_dir_problem(source: &Path, scope: &GrantDirScope) -> Option<String> {
    if source.exists() {
        if let Ok(canonical) = fs::canonicalize(source)
            && (paths_overlap(&canonical, &scope.state_dir)
                || paths_overlap(&canonical, &scope.runtime_dir)
                || scope
                    .hidden
                    .iter()
                    .any(|hidden| is_within(&canonical, hidden)))
        {
            return Some("path overlaps protected location".into());
        }
        if let Some(path) = symlink_component(source) {
            return Some(format!("symlink component: {}", path.display()));
        }
        return None;
    }
    if let Some(path) = symlink_component(source) {
        return Some(format!("symlink component: {}", path.display()));
    }
    let Some(ancestor) = source.ancestors().find(|path| path.exists()) else {
        return Some("no existing ancestor".into());
    };
    let Ok(ancestor) = fs::canonicalize(ancestor) else {
        return Some("ancestor cannot be canonicalized".into());
    };
    if !is_within(&ancestor, &scope.home)
        || paths_overlap(&ancestor, &scope.state_dir)
        || paths_overlap(&ancestor, &scope.runtime_dir)
        || (ancestor != scope.home
            && check_write_allowed(&ancestor, &scope.home, &scope.state_dir, false).is_err())
        || scope
            .hidden
            .iter()
            .any(|hidden| is_within(&ancestor, hidden))
    {
        return Some("ancestor is not writable".into());
    }
    None
}

/// Best-effort creation for the tool directories requested by grants.
///
/// The planner repeats the path checks and records missing paths as skipped in
/// dry runs; this phase only creates safe directories for a real daemon start.
pub fn ensure_grant_dirs(
    request: &ConfineRequest,
    harness: Harness,
    host: &Host,
    create: bool,
) -> Vec<Unresolved> {
    let Some(scope) = grant_dir_scope(request, harness, host) else {
        return Vec::new();
    };
    let mut unresolved = Vec::new();
    for name in &request.grants {
        let Ok(grant) = grant::lookup(name) else {
            continue;
        };
        for relative in grant.create {
            let source = user_path(relative, &scope.home);
            if let Some(reason) = grant_dir_problem(&source, &scope) {
                unresolved.push(Unresolved {
                    option: "grant",
                    path: source,
                    reason,
                });
            } else if create
                && !source.exists()
                && let Err(error) = fs::create_dir_all(&source)
            {
                unresolved.push(Unresolved {
                    option: "grant",
                    path: source,
                    reason: error.to_string(),
                });
            }
        }
    }
    unresolved
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

fn parse_core_hooks_path(common: &Path, worktree: &Path) -> Result<Option<PathBuf>> {
    let config = common.join("config");
    let Ok(file) = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NONBLOCK | OFlags::NOCTTY).bits() as i32)
        .open(&config)
    else {
        return Ok(None);
    };
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to stat git config {}", config.display()))?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    let read = file
        .take((GIT_CONFIG_PLAN_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("failed to read git config {}", config.display()))?;
    if read > GIT_CONFIG_PLAN_LIMIT {
        bail!(
            "git config {} is larger than {} bytes; the confinement plan cannot check its hooksPath",
            config.display(),
            GIT_CONFIG_PLAN_LIMIT
        );
    }
    let text = String::from_utf8_lossy(&bytes);
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
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("hooksPath") {
            let value = value.trim().trim_matches(['"', '\'']);
            let path = PathBuf::from(value);
            return Ok(Some(if path.is_absolute() {
                path
            } else {
                worktree.join(path)
            }));
        }
    }
    Ok(None)
}

const POINTER_LIMIT: usize = 4096;
// The planner fails closed on oversize; auto detection has a separate, smaller cap.
const GIT_CONFIG_PLAN_LIMIT: usize = 1024 * 1024;

fn read_pointer_text(path: &Path) -> Option<String> {
    if !fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    let read = fs::File::open(path)
        .ok()?
        .take((POINTER_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if read > POINTER_LIMIT {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn is_git_dir_shape(common: &Path) -> bool {
    fs::metadata(common.join("HEAD")).is_ok_and(|metadata| metadata.is_file())
        && fs::metadata(common.join("objects")).is_ok_and(|metadata| metadata.is_dir())
}

/// Returns the canonical common git directory and worktree root, or `None` for no repository, a home-directory repository, or an unusable pointer; `cwd` must be canonical.
pub fn git_common_dir(cwd: &Path, home: &Path) -> Option<(PathBuf, PathBuf)> {
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
            let text = read_pointer_text(&marker)?;
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
            let commondir = gitdir.join("commondir");
            let common = match fs::metadata(&commondir) {
                Ok(metadata) if metadata.is_file() => {
                    let relative = read_pointer_text(&commondir)?;
                    let path = PathBuf::from(relative.trim());
                    if path.is_absolute() {
                        path
                    } else {
                        gitdir.join(path)
                    }
                }
                Ok(_) => return None,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => gitdir,
                Err(_) => return None,
            };
            let common = fs::canonicalize(common).ok()?;
            if common == home || !is_git_dir_shape(&common) {
                return None;
            }
            return Some((common, current));
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
    explicit: bool,
) -> Result<Option<PathBuf>> {
    let Some(path) = add_canonical_existing(path, "write", skipped) else {
        return Ok(None);
    };
    let path = check_write_allowed(&path, &host.home, &host.state_dir, explicit)?;
    check_write_runtime_allowed(&path, &host.runtime_dir, explicit)?;
    add_unique(writable, path.clone());
    Ok(Some(path))
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

/// Build the complete mount plan using the daemon's current `PATH`.
pub fn plan(request: &ConfineRequest, harness: Harness, cwd: &Path, host: &Host) -> Result<Plan> {
    plan_with_path(request, harness, cwd, host, std::env::var_os("PATH"))
}

/// Build the complete mount plan with an explicit daemon `PATH` value.
pub fn plan_with_path(
    request: &ConfineRequest,
    harness: Harness,
    cwd: &Path,
    host: &Host,
    path_var: Option<OsString>,
) -> Result<Plan> {
    plan_with_preflight(request, harness, cwd, host, path_var, Vec::new())
}

/// Build a plan after the grant-directory preflight.
pub fn plan_with_preflight(
    request: &ConfineRequest,
    harness: Harness,
    cwd: &Path,
    host: &Host,
    path_var: Option<OsString>,
    preflight: Vec<Unresolved>,
) -> Result<Plan> {
    check_request_limits(request, harness)?;
    crate::grant::validate(&request.grants)?;
    let mut builder = PlanBuilder::new(host, harness)?;
    builder.unresolved.extend(preflight);
    let cwd = canonical_existing(cwd, "cwd")?;
    if !cwd.is_dir() {
        bail!("cwd is not a directory: {}", cwd.display());
    }
    let empty_file = plan_empty_file(host, &builder.state_dir)?;
    builder.bind_workspace(&cwd)?;
    builder.bind_harness_data(request)?;
    let mut effective_grants = request.grants.clone();
    if matches!(harness, Harness::Claude | Harness::Omp)
        && !request.grants.iter().any(|grant| grant == "harness-config")
    {
        let root = match harness {
            Harness::Claude => builder.home.join(".claude"),
            Harness::Omp => builder.home.join(".omp"),
            Harness::Generic | Harness::Codex => unreachable!(),
        };
        let root = canonical_if_exists(&root);
        let operator_root = root.as_ref().is_some_and(|root| {
            request
                .hide
                .iter()
                .chain(&request.unhide)
                .map(|path| user_path(path, &builder.home))
                .filter_map(|path| canonical_if_exists(&path))
                .any(|path| path.as_path() == root.as_path())
        });
        if operator_root {
            if let Some(root) = root {
                builder.unresolved.push(Unresolved {
                    option: "harness-config",
                    path: root,
                    reason: "operator hide or unhide of the harness root".into(),
                });
            }
        } else {
            effective_grants.push("harness-config".into());
        }
    }
    builder.bind_harness_config(&effective_grants)?;
    builder.bind_operator_writes(request, path_var.as_deref())?;
    builder.hide_mandatory();
    builder.hide_defaults(request);
    builder.hide_containers(&request.grants);
    builder.apply_grant_unhides(&request.grants, request.no_default_hides)?;
    builder.apply_grant_stand_ins(&request.grants);
    builder.release_operator_writes()?;
    builder.apply_operator_hides(request)?;
    builder.protect_system_paths(path_var.as_deref());
    builder.finish(request, cwd, empty_file, effective_grants)
}

fn check_request_limits(request: &ConfineRequest, harness: Harness) -> Result<()> {
    check_entry_count(&request.write, "write")?;
    check_entry_count(&request.hide, "hide")?;
    check_entry_count(&request.unhide, "unhide")?;
    check_entry_count(&request.grants, "grants")?;
    check_entry_count(&request.keep_env, "keep_env")?;
    let mut seen_keep_env = BTreeSet::new();
    for name in &request.keep_env {
        if !CREDENTIAL_ENV.contains(&name.as_str()) {
            if let Some(grant) = grant::all()
                .iter()
                .find(|grant| grant.env.contains(&name.as_str()))
            {
                bail!(
                    "keep_env {name:?} is not unset by default, so there is nothing to keep; it returns with the {} grant",
                    grant.name
                );
            }
            bail!("keep_env {name:?} is not unset by default, so there is nothing to keep");
        }
        if !seen_keep_env.insert(name.as_str()) {
            bail!("keep_env {name:?} is repeated");
        }
    }

    for (option, entries) in [
        ("write", &request.write),
        ("hide", &request.hide),
        ("unhide", &request.unhide),
    ] {
        for entry in entries {
            check_entry_syntax(option, entry)?;
            check_absolute_entry(option, entry)?;
        }
    }
    if let Some(path) = request.team_file.as_deref() {
        if path.len() > MAX_PATH_BYTES {
            bail!("team_file path exceeds {MAX_PATH_BYTES} bytes");
        }
        check_absolute_entry("team_file", path)?;
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
    unresolved: Vec<Unresolved>,
    explicit_writes: BTreeSet<PathBuf>,
    implicit_writes: Vec<(PathBuf, &'static str)>,
    mandatory_hidden: Vec<PathBuf>,
    default_hidden: Vec<PathBuf>,
    container_hidden: Vec<PathBuf>,
    container_sockets: Vec<(PathBuf, &'static str)>,
    grant_unhides: Vec<(PathBuf, &'static str)>,
    harness_config: Option<PathBuf>,
}

struct SocketRow<'a> {
    name: &'static str,
    sockets: &'a [&'a str],
}

fn container_socket_hides(
    table: &[SocketRow<'_>],
    given: &[String],
    home: &Path,
) -> (Vec<(PathBuf, &'static str)>, Vec<PathBuf>) {
    let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut granted_targets = BTreeSet::new();
    for row in table.iter().filter(|row| given.contains(row.name)) {
        for socket in row.sockets {
            let source = expand_home(socket, home);
            if let Some(path) = canonical_if_exists(&source) {
                granted_targets.insert(path);
            }
        }
    }

    let mut hidden = Vec::new();
    let mut skipped = Vec::new();
    for row in table.iter().filter(|row| !given.contains(row.name)) {
        for socket in row.sockets {
            let source = expand_home(socket, home);
            let Some(path) = canonical_if_exists(&source) else {
                add_unique(&mut skipped, source);
                continue;
            };
            if granted_targets.contains(&path) {
                continue;
            }
            if !hidden.iter().any(|(existing, _)| existing == &path) {
                hidden.push((path, row.name));
            }
        }
    }
    (hidden, skipped)
}

fn default_hide_sources_for(home: &Path, harness: Harness) -> Vec<PathBuf> {
    let mut sources = DEFAULT_HIDES
        .into_iter()
        .chain(DEFAULT_HIDES_EXTRA)
        .map(|name| user_path(name, home))
        .collect::<Vec<_>>();
    if harness != Harness::Claude {
        sources.push(home.join(".claude/.credentials.json"));
    }
    if harness != Harness::Codex {
        sources.push(home.join(".codex/auth.json"));
    }
    sources.push(PathBuf::from("/run/dbus"));
    sources
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
            unresolved: Vec::new(),
            explicit_writes: BTreeSet::new(),
            implicit_writes: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            container_sockets: Vec::new(),
            grant_unhides: Vec::new(),
            harness_config: None,
        })
    }

    fn session_harness_root(&self) -> Option<PathBuf> {
        match self.harness {
            Harness::Claude => Some(self.home.join(".claude")),
            Harness::Omp => Some(self.home.join(".omp")),
            Harness::Generic | Harness::Codex => None,
        }
    }

    fn bind_workspace(&mut self, cwd: &Path) -> Result<()> {
        if let Some(path) =
            add_validated_write(cwd, self.host, &mut self.writable, &mut self.skipped, false)?
        {
            self.implicit_writes.push((path, "working directory"));
        }
        if let Some((common, worktree)) = git_common_dir(cwd, &self.home) {
            if let Some(path) = add_validated_write(
                &common,
                self.host,
                &mut self.writable,
                &mut self.skipped,
                false,
            )? {
                self.implicit_writes.push((path, "git directory"));
            }
            self.protect_git_metadata(&common, &worktree)?;
        }
        Ok(())
    }

    fn protect_git_metadata(&mut self, common: &Path, worktree: &Path) -> Result<()> {
        for name in ["hooks", "config", "config.worktree"] {
            if let Ok(path) = common.join(name).canonicalize() {
                add_unique(&mut self.readonly, path);
            }
        }
        if let Some(path) = parse_core_hooks_path(common, worktree)?
            && let Ok(path) = path.canonicalize()
        {
            add_unique(&mut self.readonly, path);
        }
        Ok(())
    }

    fn bind_harness_data(&mut self, request: &ConfineRequest) -> Result<()> {
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let explicit_writes = request
            .write
            .iter()
            .filter_map(|entry| canonical_if_exists(&user_path(entry, &self.home)))
            .collect::<BTreeSet<_>>();
        validate_harness_data_paths(
            self.harness,
            &self.home,
            &self.state_dir,
            &self.runtime_dir,
            &data_dirs,
            &data_files,
            &explicit_writes,
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

    fn bind_harness_config(&mut self, grants: &[String]) -> Result<()> {
        let own_config = grants.iter().any(|name| {
            grant::all()
                .iter()
                .any(|row| row.name == name && row.own_config)
        });
        if !own_config {
            return Ok(());
        }
        let Some(root) = self.session_harness_root() else {
            bail!("confinement grant \"harness-config\" needs a Claude or OMP session")
        };
        let Some(root) = add_canonical_existing(&root, "harness config", &mut self.skipped) else {
            return Ok(());
        };
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let data_paths = data_dirs
            .into_iter()
            .chain(data_files)
            .filter_map(|path| {
                let path = canonical_if_exists(&path).unwrap_or(path);
                is_within(&path, &root).then_some(path)
            })
            .collect::<Vec<_>>();
        self.writable
            .retain(|path| !data_paths.iter().any(|data| data == path));
        self.skipped
            .retain(|path| !data_paths.iter().any(|data| data == path));
        self.harness_config = Some(root.clone());
        add_unique(&mut self.writable, root);
        Ok(())
    }

    fn resolve_operator_path(&mut self, option: &'static str, source: &Path) -> Option<PathBuf> {
        match fs::canonicalize(source) {
            Ok(path) => Some(path),
            Err(error) => {
                add_unique(&mut self.skipped, source.to_owned());
                self.unresolved.push(Unresolved {
                    option,
                    path: source.to_owned(),
                    reason: if error.kind() == std::io::ErrorKind::NotFound {
                        "not found".into()
                    } else {
                        error.kind().to_string()
                    },
                });
                tracing::debug!(
                    path = %source.display(),
                    %option,
                    error = %error,
                    "operator confinement path unresolved"
                );
                None
            }
        }
    }

    fn bind_operator_writes(
        &mut self,
        request: &ConfineRequest,
        path_var: Option<&OsStr>,
    ) -> Result<()> {
        for path in &request.write {
            let source = user_path(path, &self.home);
            if let Some(source) = self.resolve_operator_path("write", &source)
                && let Some(path) = add_validated_write(
                    &source,
                    self.host,
                    &mut self.writable,
                    &mut self.skipped,
                    true,
                )?
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
        self.protect_system_paths(path_var);
        Ok(())
    }

    fn protect_system_paths(&mut self, path_var: Option<&OsStr>) {
        if let Some(path) = canonical_if_exists(&self.home.join(".a2amx"))
            && is_within(&path, &self.home)
            && self.writable.iter().any(|write| is_within(&path, write))
        {
            add_unique(&mut self.readonly, path);
        }
        if let Some(path_var) = path_var {
            for source in std::env::split_paths(path_var) {
                if !source.is_absolute() || !source.is_dir() {
                    continue;
                }
                let Some(path) = canonical_if_exists(&source) else {
                    continue;
                };
                if is_within(&path, &self.home)
                    && self.writable.iter().any(|write| is_within(&path, write))
                {
                    add_unique(&mut self.readonly, path);
                }
            }
        }
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

    fn default_hide_sources(&self) -> Vec<PathBuf> {
        default_hide_sources_for(&self.home, self.harness)
    }

    fn hide_defaults(&mut self, request: &ConfineRequest) {
        if request.no_default_hides {
            return;
        }
        for source in self.default_hide_sources() {
            if source == Path::new("/run/dbus") {
                if let Some(path) = canonical_if_exists(&source) {
                    add_unique(&mut self.default_hidden, path.clone());
                    add_unique(&mut self.hidden, path);
                }
            } else {
                self.hide_or_skip(source);
            }
        }
    }

    fn check_grant_path_writable(&self, grant: &str, entry: &str, path: &Path) -> Result<PathBuf> {
        let path = check_write_allowed(path, &self.home, &self.state_dir, false)?;
        check_write_runtime_allowed(&path, &self.runtime_dir, false)?;
        if let Some(root) = canonical_if_exists(&self.home.join(".a2amx"))
            && is_within(&path, &root)
        {
            bail!("confinement grant \"{grant}\" path {entry} is inside ~/.a2amx");
        }
        if self.harness_config.is_none()
            && let Some(root) = self
                .session_harness_root()
                .and_then(|root| canonical_if_exists(&root))
            && is_within(&path, &root)
        {
            bail!(
                "confinement grant \"{grant}\" path {entry} is inside the harness root {}",
                root.display()
            );
        }
        Ok(path)
    }

    fn apply_grant_unhides(&mut self, given: &[String], no_default_hides: bool) -> Result<()> {
        let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
        let own_paths = grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
            .flat_map(|grant| grant.unhides)
            .map(|path| user_path(path, &self.home))
            .collect::<BTreeSet<_>>();
        let default_hide_sources = if no_default_hides {
            Vec::new()
        } else {
            self.default_hide_sources()
        };
        for grant in grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
        {
            for name in grant.unhides {
                let source = user_path(name, &self.home);
                if let Some(path) = symlink_component(&source) {
                    bail!(
                        "confinement grant \"{}\" path {} passes through the symlink {}; name the real path with a write entry instead",
                        grant.name,
                        name,
                        path.display()
                    );
                }
                let Some(path) = canonical_if_exists(&source) else {
                    add_unique(&mut self.skipped, source);
                    continue;
                };
                let path = self.check_grant_path_writable(grant.name, name, &path)?;
                for hidden_source in &default_hide_sources {
                    if own_paths.contains(hidden_source) {
                        continue;
                    }
                    let Some(hidden) = canonical_if_exists(hidden_source) else {
                        continue;
                    };
                    if is_within(&path, &hidden) {
                        bail!(
                            "confinement grant \"{}\" path {} resolves to the hidden path {}",
                            grant.name,
                            name,
                            hidden.display()
                        );
                    }
                }
                for hidden in self
                    .mandatory_hidden
                    .iter()
                    .chain(self.container_hidden.iter())
                {
                    if paths_overlap(&path, hidden) {
                        bail!(
                            "confinement grant \"{}\" path {} resolves to the hidden path {}",
                            grant.name,
                            name,
                            hidden.display()
                        );
                    }
                }
                self.hidden.retain(|existing| existing != &path);
                self.default_hidden.retain(|existing| existing != &path);
                add_unique(&mut self.writable, path.clone());
                self.grant_unhides.push((path, grant.name));
            }
        }
        Ok(())
    }

    fn apply_grant_stand_ins(&mut self, given: &[String]) {
        let given = given.iter().map(String::as_str).collect::<BTreeSet<_>>();
        for grant in grant::all()
            .iter()
            .filter(|grant| given.contains(grant.name))
        {
            for name in grant.stand_ins {
                let source = PathBuf::from(name);
                let Some(path) = canonical_if_exists(&source) else {
                    add_unique(&mut self.skipped, source);
                    continue;
                };
                add_unique(&mut self.hidden, path);
            }
        }
    }

    fn hide_containers(&mut self, given: &[String]) {
        let table = grant::all()
            .iter()
            .map(|grant| SocketRow {
                name: grant.name,
                sockets: grant.sockets,
            })
            .collect::<Vec<_>>();
        let (hidden, skipped) = container_socket_hides(&table, given, &self.home);
        for (path, owner) in hidden {
            add_unique(&mut self.container_hidden, path.clone());
            add_unique(&mut self.hidden, path.clone());
            self.container_sockets.push((path, owner));
        }
        for path in skipped {
            add_unique(&mut self.skipped, path);
        }
    }

    fn hide_or_skip(&mut self, source: PathBuf) {
        if let Some(path) = canonical_if_exists(&source) {
            add_unique(&mut self.default_hidden, path.clone());
            add_unique(&mut self.hidden, path);
        } else {
            add_unique(&mut self.skipped, source);
        }
    }

    fn check_container_socket(&self, path: &Path, option: &str) -> Result<()> {
        if let Some((_, grant)) = self
            .container_sockets
            .iter()
            .find(|(socket, _)| is_within(path, socket))
        {
            bail!(
                "{option} path {} requires the {grant} grant (--confine-grant {grant}, or grants = [\"{grant}\"])",
                path.display()
            );
        }
        Ok(())
    }

    fn release_operator_writes(&mut self) -> Result<()> {
        for path in &self.explicit_writes {
            self.check_container_socket(path, "write")?;
            self.hidden.retain(|existing| existing != path);
            self.default_hidden.retain(|existing| existing != path);
        }
        Ok(())
    }

    fn apply_operator_hides(&mut self, request: &ConfineRequest) -> Result<()> {
        for path in &request.hide {
            let source = user_path(path, &self.home);
            if let Some(path) = self.resolve_operator_path("hide", &source) {
                add_unique(&mut self.hidden, path);
            }
        }
        for path in &request.unhide {
            self.unhide(path)?;
        }
        Ok(())
    }

    fn unhide(&mut self, path: &str) -> Result<()> {
        let source = user_path(path, &self.home);
        if let Some(path) = self.resolve_operator_path("unhide", &source) {
            check_unhide_allowed(&path, &self.state_dir, &self.runtime_dir, &self.home)?;
            self.check_container_socket(&path, "unhide")?;
            self.hidden.retain(|existing| existing != &path);
            self.default_hidden.retain(|existing| existing != &path);
            self.container_hidden.retain(|existing| existing != &path);
            if !self
                .grant_unhides
                .iter()
                .any(|(granted, _)| granted == &path)
            {
                add_unique(&mut self.readonly, path);
            }
        }
        Ok(())
    }

    fn check_implicit_writes_not_hidden(&self) -> Result<()> {
        let protected_hides = self
            .default_hidden
            .iter()
            .chain(&self.mandatory_hidden)
            .chain(&self.container_hidden);
        let mut offenders = Vec::new();
        for (path, source) in &self.implicit_writes {
            if self.explicit_writes.contains(path) {
                continue;
            }
            for hidden in protected_hides.clone() {
                if is_within(path, hidden) {
                    offenders.push(format!(
                        "{source} {} resolves to the hidden path {}; open it with write = [\"{}\"] if that is intended",
                        path.display(),
                        hidden.display(),
                        path.display()
                    ));
                }
            }
        }
        if offenders.is_empty() {
            Ok(())
        } else {
            bail!("{}", offenders.join("; "));
        }
    }

    fn finish(
        mut self,
        request: &ConfineRequest,
        cwd: PathBuf,
        empty_file: PathBuf,
        mut effective_grants: Vec<String>,
    ) -> Result<Plan> {
        self.readonly
            .retain(|path| !self.explicit_writes.contains(path));
        if let Some(root) = self.harness_config.as_ref() {
            if self.hidden.iter().any(|path| path == root) {
                bail!(
                    "confinement grant \"harness-config\" conflicts with the hide of {}",
                    root.display()
                );
            }
            if self.readonly.iter().any(|path| path == root) {
                bail!(
                    "confinement grant \"harness-config\" conflicts with the unhide of {}",
                    root.display()
                );
            }
        }
        for (path, grant) in &self.grant_unhides {
            if self.hidden.iter().any(|existing| existing == path) {
                bail!(
                    "confinement grant \"{grant}\" conflicts with the hide of {}",
                    path.display()
                );
            }
        }
        self.check_implicit_writes_not_hidden()?;
        let protected = protected_paths(&self.readonly, &self.writable);
        let (data_dirs, data_files) = harness_data(self.harness, &self.home);
        let overlay_parents = harness_overlay_parents(
            &self.home,
            &data_files,
            &self.writable,
            &self.hidden,
            &self.readonly,
            &protected,
        );
        let overlay_protected = overlay_protected_entries(
            &overlay_parents,
            &data_dirs,
            &data_files,
            &self.writable,
            &self.readonly,
            &protected,
            &self.hidden,
        );
        let unsaved_files = unsaved_harness_files(&data_files, &overlay_parents);
        self.readonly.sort();
        self.writable.sort();
        self.hidden.sort();
        self.mandatory_hidden.sort();
        self.default_hidden.sort();
        self.container_hidden.sort();
        self.skipped.sort();
        self.unresolved.sort_by(|left, right| {
            left.option
                .cmp(right.option)
                .then_with(|| left.path.cmp(&right.path))
        });

        let mut grants = request.grants.clone();
        grants.sort();
        effective_grants.sort();
        effective_grants.dedup();
        let mut keep_env = request.keep_env.clone();
        keep_env.sort();
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
            unresolved: self.unresolved,
            grants,
            effective_grants,
            auto: request.auto,
            reasons: request.reasons.clone(),
            write: request.write.clone(),
            keep_env,
            mandatory_hidden: self.mandatory_hidden,
            default_hidden: self.default_hidden,
            container_hidden: self.container_hidden,
            overlay_parents,
            overlay_protected,
            unsaved_files,
            overlay_notice: None,
            harness_config: self.harness_config,
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
fn granted_env(grants: &[String]) -> BTreeSet<&'static str> {
    let given = grants.iter().map(String::as_str).collect::<BTreeSet<_>>();
    grant::all()
        .iter()
        .filter(|grant| given.contains(grant.name))
        .flat_map(|grant| grant.env.iter().copied())
        .collect()
}

impl Plan {
    pub fn with_bwrap(mut self, path: PathBuf) -> Self {
        self.bwrap = path;
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

/// Find the first executable named `name` in absolute entries of a PATH value.
pub fn find_executable(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    for entry in std::env::split_paths(path_var) {
        if !entry.is_absolute() {
            continue;
        }
        let candidate = entry.join(name);
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
            continue;
        }
        if let Ok(path) = fs::canonicalize(candidate) {
            return Some(path);
        }
    }
    None
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
    if path.is_absolute() && path.components().count() > 1 && path.is_file() {
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
    let Some(candidate_name) = candidate_name.to_str() else {
        bail!("bubblewrap was not found in PATH");
    };
    if let Some(path) = find_executable(candidate_name, path.as_os_str()) {
        return Ok(path);
    }
    bail!("bubblewrap was not found in PATH")
}

fn preflight_args() -> Vec<String> {
    vec![
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
    ]
}

/// Run version and a real base-mount probe.  A failure is fatal; callers must
/// not fall back to an unwrapped PTY after this returns an error.
pub fn preflight(bwrap: &Path) -> Result<()> {
    let bwrap = canonical_existing(bwrap, "bubblewrap")?;
    command_with_timeout(&bwrap, &["--version".into()])?;
    command_with_timeout(&bwrap, &preflight_args())
}

/// Check whether bubblewrap supports the temporary overlays used for harness files.
pub fn supports_tmp_overlay(bwrap: &Path, parents: &[PathBuf]) -> Result<(), String> {
    if parents.is_empty() {
        return Ok(());
    }
    let bwrap = canonical_existing(bwrap, "bubblewrap").map_err(|error| error.to_string())?;
    let mut probe = preflight_args();
    let Some(mut separator) = probe.iter().position(|argument| argument == "--") else {
        return Err("invalid preflight probe".into());
    };
    for parent in parents {
        probe.splice(
            separator..separator,
            [
                "--overlay-src".to_owned(),
                path_key(parent),
                "--tmp-overlay".to_owned(),
                path_key(parent),
            ],
        );
        separator += 4;
    }
    command_with_timeout(&bwrap, &probe).map_err(|error| {
        let text = error.to_string();
        text.split_once(": ")
            .map(|(_, stderr)| stderr.lines().next().unwrap_or(stderr))
            .unwrap_or_else(|| {
                text.lines()
                    .next()
                    .unwrap_or("bubblewrap overlay probe failed")
            })
            .to_owned()
    })
}

/// Verify that `env` supports the signal-disposition flags used for Generic sessions.
pub fn preflight_env(env: &Path) -> Result<()> {
    let env_path = env.to_string_lossy().into_owned();
    let probe = vec![
        "--ignore-signal=INT".into(),
        env_path.clone(),
        "--default-signal=INT".into(),
        env_path,
    ];
    let requirement = "confinement of a Generic session needs env with --ignore-signal and --default-signal: \
         GNU coreutils 8.31 or newer, or uutils";
    command_with_timeout(env, &probe).map_err(|error| anyhow!("{requirement}: {error:#}"))
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

/// Render the confinement lines in a team's dry-run report.
pub fn dry_run_lines(plan: &Plan, resolved_cwd: &Path, accepted: &BTreeSet<String>) -> String {
    let mode = if plan.auto { "auto" } else { "manual" };
    let mut output = format!("  confinement: {mode}\n");
    if plan.auto {
        for reason in &plan.reasons {
            output.push_str(&format!("  {}: {}\n", reason.grant, reason.reason));
        }
        let not_accepted = plan
            .reasons
            .iter()
            .filter(|reason| !accepted.contains(&reason.grant))
            .map(|reason| reason.grant.as_str())
            .collect::<Vec<_>>();
        if !not_accepted.is_empty() {
            output.push_str(&format!(
                "  not yet accepted: {}\n",
                not_accepted.join(", ")
            ));
        }
    }
    if !plan.grants.is_empty() {
        output.push_str(&format!("  grants: {}\n", plan.grants.join(", ")));
    }
    if !plan.keep_env.is_empty() {
        output.push_str(&format!("  keep_env: {}\n", plan.keep_env.join(", ")));
    }
    for path in plan.writable.iter().filter(|path| **path != resolved_cwd) {
        output.push_str(&format!("  writable: {}\n", path.display()));
    }
    for path in &plan.protected {
        output.push_str(&format!("  protected: {}\n", path.display()));
    }
    for path in plan.hidden_paths() {
        output.push_str(&format!("  hidden: {}\n", path.display()));
    }
    for path in plan.overlay_parents() {
        output.push_str(&format!("  overlay: {}\n", path.display()));
    }
    for path in plan.unsaved_files() {
        output.push_str(&format!("  unsaved: {}\n", path.display()));
    }
    for unresolved in &plan.unresolved {
        output.push_str(&format!(
            "  unresolved: {} ({}: {})\n",
            unresolved.path.display(),
            unresolved.option,
            unresolved.reason
        ));
    }
    for path in plan
        .skipped_paths()
        .iter()
        .filter(|path| !plan.unresolved.iter().any(|item| item.path == **path))
    {
        output.push_str(&format!("  hidden: {} (skipped missing)\n", path.display()));
    }
    output
}

/// Render the operator-facing notice from the exact mount plan.
pub fn notice(plan: &Plan) -> String {
    let writable: Cow<'_, [PathBuf]> = match plan.writable.iter().position(|path| path == &plan.cwd)
    {
        Some(0) | None => Cow::Borrowed(&plan.writable),
        Some(index) => {
            let mut reordered = Vec::with_capacity(plan.writable.len());
            reordered.push(plan.cwd.clone());
            reordered.extend(
                plan.writable
                    .iter()
                    .enumerate()
                    .filter(|(position, _)| *position != index)
                    .map(|(_, path)| path.clone()),
            );
            Cow::Owned(reordered)
        }
    };
    let mut text = String::new();
    text.push_str("This session is confined.\n");
    text.push_str("Writable locations: ");
    text.push_str(&capped_paths(&writable));
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
    for path in &plan.overlay_parents {
        text.push_str(&format!(
            "{}: writable, but only the harness's data files are saved; other writes are discarded at exit.\n",
            path.display()
        ));
    }
    if !plan.unsaved_files.is_empty() {
        text.push_str("Files not present at start, writes to them are not saved: ");
        text.push_str(&capped_paths(&plan.unsaved_files));
        text.push_str(".\n");
    }
    if let Some(reason) = &plan.overlay_notice {
        text.push_str(reason);
        text.push('\n');
    }
    if let Some(path) = &plan.harness_config {
        let source = if plan.grants.iter().any(|grant| grant == "harness-config") {
            "grant harness-config"
        } else {
            "default for this harness"
        };
        text.push_str(&format!(
            "{}: harness config, writable and persistent ({}).\n",
            path.display(),
            source
        ));
    }
    if !plan.grants.is_empty() {
        text.push_str("Granted: ");
        text.push_str(&plan.grants.join(", "));
        text.push_str(".\n");
    }
    let released = granted_env(&plan.grants);
    let unset_credentials = CREDENTIAL_ENV
        .iter()
        .copied()
        .filter(|name| !released.contains(*name) && !plan.keep_env.iter().any(|kept| kept == *name))
        .collect::<Vec<_>>();
    text.push_str("Credential environment variables are unset: ");
    text.push_str(&unset_credentials.join(", "));
    if !plan.keep_env.is_empty() {
        text.push_str("; kept: ");
        text.push_str(&plan.keep_env.join(", "));
    }
    text.push_str(".\n");
    text.push_str(
        "Everything else is read-only. /tmp is private to this session and discarded when it ends; paths under a peer's /tmp cannot be read.\n",
    );
    let granted = plan
        .grants
        .iter()
        .filter_map(|name| grant::all().iter().find(|grant| grant.name == name))
        .filter(|grant| !grant.sockets.is_empty())
        .map(|grant| grant.name)
        .collect::<Vec<_>>();
    if granted.is_empty() {
        text.push_str("Container engines are restricted.\n");
    } else {
        text.push_str(&format!(
            "Container engines allowed: {}. Other container engines are restricted.\n",
            granted.join(", ")
        ));
    }
    text.push_str(
        "A Read-only file system error is authoritative where this notice and the system disagree.",
    );
    truncate_before_closing(text)
}

fn ctrl_c_wrapper(wrapped_inner: Vec<String>) -> Vec<String> {
    let mut wrapped = Vec::with_capacity(wrapped_inner.len() + 2);
    wrapped.push(ENV_PATH.to_owned());
    wrapped.push("--ignore-signal=INT".into());
    // shortcut: the fixed outer prefix is 33 bytes with NULs, far below ARG_MAX; no second size check.
    wrapped.extend(wrapped_inner);
    wrapped
}

/// Build a wrapped command and, where the harness has a system prompt channel,
/// append the notice.  Generic sessions intentionally receive no notice.
pub fn launch_argv(
    plan: &Plan,
    notice: &str,
    argv: Vec<String>,
    harness: Harness,
) -> Result<Vec<String>> {
    match harness {
        Harness::Generic => {
            let mut inner = Vec::with_capacity(argv.len() + 2);
            inner.push(ENV_PATH.to_owned());
            inner.push("--default-signal=INT".into());
            inner.extend(argv);
            Ok(ctrl_c_wrapper(plan.wrap(inner)?))
        }
        Harness::Claude | Harness::Omp => {
            let wrapped = plan.wrap(argv)?;
            crate::harness::append_system_prompt(wrapped, notice, harness, &plan.cwd)
        }
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
            unresolved: Vec::new(),
            grants: Vec::new(),
            effective_grants: Vec::new(),
            auto: false,
            reasons: Vec::new(),
            write: Vec::new(),
            keep_env: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            overlay_parents: Vec::new(),
            overlay_protected: Vec::new(),
            unsaved_files: Vec::new(),
            overlay_notice: None,
            harness_config: None,
        };
        let text = notice(&plan);
        assert!(text.len() <= NOTICE_LIMIT);
        assert_eq!(text.matches("/w/0").count(), 1);
        assert!(text.ends_with(CLOSING_NOTICE));
    }
    #[test]
    fn granted_socket_target_wins_over_an_ungranted_alias() {
        let root = tempfile::tempdir().expect("socket fixture directory");
        let granted = root.path().join("granted.sock");
        let alias = root.path().join("alias.sock");
        std::fs::write(&granted, "socket").expect("granted socket");
        std::os::unix::fs::symlink(&granted, &alias).expect("alias socket");
        let granted_path = granted.to_string_lossy().into_owned();
        let alias_path = alias.to_string_lossy().into_owned();
        let granted_sockets = [granted_path.as_str()];
        let alias_sockets = [alias_path.as_str()];
        let table = [
            SocketRow {
                name: "granted",
                sockets: &granted_sockets,
            },
            SocketRow {
                name: "alias",
                sockets: &alias_sockets,
            },
        ];
        let given = vec!["granted".to_owned()];
        let (hidden, skipped) = container_socket_hides(&table, &given, root.path());
        assert!(hidden.is_empty());
        assert!(skipped.is_empty());
    }
    #[test]
    fn credential_grant_unhides_are_default_hide_sources() {
        let defaults = DEFAULT_HIDES
            .into_iter()
            .chain(DEFAULT_HIDES_EXTRA)
            .collect::<BTreeSet<_>>();
        let credential_grants = [
            "gh",
            "git-credentials",
            "ssh",
            "aws",
            "gcloud",
            "azure",
            "kube",
            "terraform",
            "npm-token",
            "pypi-token",
            "cargo-token",
            "netrc",
            "huggingface-token",
        ];
        for grant in grant::all()
            .iter()
            .filter(|grant| credential_grants.contains(&grant.name))
        {
            for path in grant.unhides {
                assert!(
                    defaults.contains(path),
                    "{} unhide path {path} is not a default hide",
                    grant.name
                );
            }
        }
    }

    #[test]
    fn grant_socket_and_environment_names_are_unique() {
        let mut sockets = BTreeSet::new();
        let mut environment = BTreeSet::new();
        for grant in grant::all() {
            for socket in grant.sockets {
                assert!(sockets.insert(*socket), "duplicate socket {socket}");
            }
            for name in grant.env {
                assert!(!ALWAYS_UNSET.contains(name), "always-unset name {name}");
                assert!(
                    !CREDENTIAL_ENV.contains(name) || !grant.unhides.is_empty(),
                    "credential environment name {name} needs a credential grant path"
                );
                assert!(
                    environment.insert(*name),
                    "duplicate environment name {name}"
                );
            }
        }
        for name in CREDENTIAL_ENV {
            for prefix in [
                "ANTHROPIC_",
                "OPENAI_",
                "GEMINI_",
                "GOOGLE_API_KEY",
                "OPENROUTER_",
                "A2AMX_",
            ] {
                assert!(
                    !name.starts_with(prefix),
                    "excluded environment prefix {prefix}: {name}"
                );
            }
            assert!(!ALWAYS_UNSET.contains(&name), "always-unset name {name}");
        }
        for name in [
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
            "AWS_SESSION_TOKEN",
            "GOOGLE_APPLICATION_CREDENTIALS",
        ] {
            assert!(
                !CREDENTIAL_ENV.contains(&name),
                "harness credential {name} must remain set"
            );
        }
    }

    #[test]
    fn apply_auto_keeps_requested_grants_and_records_only_additions() {
        let mut request = ConfineRequest {
            grants: vec!["cache".to_owned()],
            ..ConfineRequest::default()
        };

        request.apply_auto(vec![
            ("cache", "already requested".to_owned()),
            ("cargo", "detected cargo files".to_owned()),
        ]);

        assert_eq!(request.grants, ["cache", "cargo"]);
        assert!(request.auto);
        assert_eq!(
            request.reasons,
            [GrantReason {
                grant: "cargo".to_owned(),
                reason: "detected cargo files".to_owned()
            }]
        );
    }

    #[test]
    fn apply_auto_sets_auto_when_no_grant_is_added() {
        let mut request = ConfineRequest {
            grants: vec!["cache".to_owned()],
            ..ConfineRequest::default()
        };

        request.apply_auto(vec![("cache", "already requested".to_owned())]);

        assert!(request.auto);
        assert!(request.reasons.is_empty());
    }

    #[test]
    fn explicit_grants_exclude_auto_added_grants() {
        let request = ConfineRequest {
            grants: vec!["manual".to_owned(), "cache".to_owned()],
            reasons: vec![GrantReason {
                grant: "cache".to_owned(),
                reason: "detected cache".to_owned(),
            }],
            ..ConfineRequest::default()
        };

        assert_eq!(request.explicit_grants(), ["manual"]);
    }

    fn dry_run_plan() -> Plan {
        Plan {
            bwrap: PathBuf::from("bwrap"),
            empty_file: PathBuf::from("/dev/null"),
            state_dir: PathBuf::from("/state"),
            cwd: PathBuf::from("/plan-cwd"),
            harness: Harness::Generic,
            writable: vec![PathBuf::from("/resolved-cwd"), PathBuf::from("/other")],
            readonly: Vec::new(),
            protected: Vec::new(),
            hidden: Vec::new(),
            skipped: vec![PathBuf::from("/missing")],
            unresolved: Vec::new(),
            grants: vec!["cache".to_owned()],
            effective_grants: Vec::new(),
            auto: true,
            reasons: vec![GrantReason {
                grant: "cache".to_owned(),
                reason: "detected cache".to_owned(),
            }],
            write: Vec::new(),
            keep_env: Vec::new(),
            mandatory_hidden: Vec::new(),
            default_hidden: Vec::new(),
            container_hidden: Vec::new(),
            overlay_parents: Vec::new(),
            overlay_protected: Vec::new(),
            unsaved_files: Vec::new(),
            overlay_notice: None,
            harness_config: None,
        }
    }

    #[test]
    fn dry_run_lines_list_auto_grants_not_yet_accepted() {
        let text = dry_run_lines(
            &dry_run_plan(),
            Path::new("/resolved-cwd"),
            &BTreeSet::new(),
        );

        assert!(text.contains("  confinement: auto\n"));
        assert!(text.contains("  cache: detected cache\n"));
        assert!(text.contains("  not yet accepted: cache\n"));
        assert!(text.contains("  hidden: /missing (skipped missing)\n"));
    }

    #[test]
    fn dry_run_lines_uses_resolved_cwd_when_filtering_writable_paths() {
        let text = dry_run_lines(
            &dry_run_plan(),
            Path::new("/resolved-cwd"),
            &BTreeSet::new(),
        );

        assert!(!text.contains("  writable: /resolved-cwd\n"));
        assert!(text.contains("  writable: /other\n"));
    }

    #[test]
    fn dry_run_lines_only_lists_unaccepted_reason_for_mixed_set() {
        let mut plan = dry_run_plan();
        plan.reasons.push(GrantReason {
            grant: "cargo".to_owned(),
            reason: "detected Cargo files".to_owned(),
        });
        let accepted = BTreeSet::from(["cache".to_owned()]);

        let text = dry_run_lines(&plan, Path::new("/resolved-cwd"), &accepted);

        assert!(text.contains("  not yet accepted: cargo\n"));
        assert!(!text.contains("  not yet accepted: cache"));
    }
}
