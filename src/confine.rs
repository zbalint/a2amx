//! Opt-in bubblewrap confinement for PTY sessions.
//!
//! This module owns every bubblewrap argument and probe.  Callers pass plain
//! paths and a harness profile; the resulting [`Plan`] is the sole source for
//! mounts, the launch argv, the notice, and dry-run visibility.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::grant;
use crate::harness::Harness;
use crate::wire::GrantReason;

mod builder;
mod bwrap;
mod git;
mod grant_dirs;
mod harness_data;
mod hides;
mod launch;
mod wrap;

pub use builder::{PlanTarget, plan, plan_with_path, plan_with_preflight};
pub use bwrap::{
    ensure_empty_file, find_executable, preflight, preflight_env, resolve_bwrap,
    supports_tmp_overlay,
};
pub use git::git_common_dir;
pub use grant_dirs::ensure_grant_dirs;
pub use harness_data::{ensure_data_dirs, ensure_data_files};
pub use hides::default_hide_names;
pub use launch::{dry_run_lines, launch_argv, notice};

pub const MAX_LIST_ENTRIES: usize = 64;
/// Subdirectory that holds a confined Codex app-server's socket.
pub const CODEX_SOCKET_SUBDIR: &str = "sock";
pub const MAX_PATH_BYTES: usize = 4096;
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

fn is_home_entry(entry: &str) -> bool {
    entry == "~" || entry.starts_with("~/")
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
    if entry.starts_with('~') && !is_home_entry(entry) {
        bail!("{option} entry {entry:?}: only ~ and ~/... are understood, not another user's home");
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub home: PathBuf,
    pub layout: crate::layout::Layout,
    pub runtime_dir: PathBuf,
}

pub struct Plan {
    bwrap: PathBuf,
    layout: crate::layout::Layout,
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
    pub hide: Vec<String>,
    pub unhide: Vec<String>,
    pub no_default_hides: bool,
    pub keep_env: Vec<String>,
    pub mandatory_hidden: Vec<PathBuf>,
    pub default_hidden: Vec<PathBuf>,
    pub container_hidden: Vec<PathBuf>,
    overlay_parents: Vec<PathBuf>,
    overlay_protected: Vec<PathBuf>,
    unsaved_files: Vec<PathBuf>,
    overlay_notice: Option<String>,
    harness_config: Option<PathBuf>,
    codex_dir: Option<PathBuf>,
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
fn overlaps_for(explicit: bool, path: &Path, other: &Path) -> bool {
    if explicit {
        is_within(path, other)
    } else {
        paths_overlap(path, other)
    }
}

fn check_absolute_entry(option: &str, entry: &str) -> Result<()> {
    if !is_home_entry(entry) && !Path::new(entry).is_absolute() {
        bail!("{option} entry {entry:?} must be an absolute path or start with ~/");
    }
    Ok(())
}

/// Canonicalize and validate a path that may become writable.
pub fn check_write_allowed(
    path: &Path,
    home: &Path,
    root: &Path,
    explicit: bool,
) -> Result<PathBuf> {
    let path = canonical_existing(path, "writable path")?;
    let home = canonical_existing(home, "home")?;
    let root = canonical_existing(root, "a2amx directory")?;
    if path == Path::new("/") {
        bail!("writable path / is not allowed");
    }
    if !explicit && is_within(&home, &path) {
        bail!("writable path $HOME is not allowed");
    }
    if overlaps_for(explicit, &path, &root) {
        bail!(
            "writable path {} overlaps the a2amx directory",
            path.display()
        );
    }
    for mandatory in MANDATORY_ROOTS {
        if overlaps_for(explicit, &path, Path::new(mandatory)) {
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
    if overlaps_for(explicit, path, &runtime_dir) {
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
    root: &Path,
    runtime_dir: &Path,
    home: &Path,
) -> Result<PathBuf> {
    let path = canonical_existing(path, "unhide path")?;
    let root = canonical_existing(root, "a2amx directory")?;
    let runtime_dir = canonical_existing(runtime_dir, "runtime directory")?;
    let home = canonical_existing(home, "home")?;
    if path == home || is_within(&home, &path) {
        bail!("unhide path {} is home or contains home", path.display());
    }
    if paths_overlap(&path, &root) {
        bail!(
            "unhide path {} overlaps the a2amx directory",
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

/// The mount lists while the jobs of `plan` fill them.
struct PlanBuilder<'a> {
    host: &'a Host,
    harness: Harness,
    home: PathBuf,
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
    layout: crate::layout::Layout,
    harness_config: Option<PathBuf>,
}

fn granted_env(grants: &[String]) -> BTreeSet<&'static str> {
    let given = grants.iter().map(String::as_str).collect::<BTreeSet<_>>();
    grant::all()
        .iter()
        .filter(|grant| given.contains(grant.name))
        .flat_map(|grant| grant.env.iter().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
