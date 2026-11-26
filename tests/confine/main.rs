#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "../common/mod.rs"]
mod common;

use anyhow::Context as _;
use std::ffi::{OsStr, OsString};
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc as std_mpsc;

use a2amx::client::Client;
use a2amx::confine::{self, ConfineRequest, Host};
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::grant;
use a2amx::harness::Harness;
use a2amx::messaging::Limits;
use a2amx::team;
use a2amx::wire::{ClientFrame, Request, Response};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{Duration, timeout};

struct Fixture {
    _root: TempDir,
    home: PathBuf,
    state: PathBuf,
    runtime: PathBuf,
    cwd: PathBuf,
    empty: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("tempdir");
        let home = root.path().join("home");
        let state = root.path().join("state");
        let runtime = root.path().join("runtime");
        let cwd = root.path().join("workspace");
        for path in [&home, &state, &runtime, &cwd] {
            std::fs::create_dir_all(path).expect("fixture directory");
        }
        a2amx::layout::Layout::new(state.clone())
            .ensure()
            .expect("layout directories");
        std::fs::create_dir_all(home.join(".aws")).expect("aws directory");
        std::fs::create_dir_all(home.join(".ssh")).expect("ssh directory");
        std::fs::write(home.join(".ssh/config"), "Host example\n").expect("ssh config");
        let empty = a2amx::layout::Layout::new(state.clone()).confine_empty();
        std::fs::write(&empty, []).expect("empty file");
        Self {
            _root: root,
            home,
            state,
            runtime,
            cwd,
            empty,
        }
    }

    fn host(&self) -> Host {
        Host {
            home: self.home.clone(),
            layout: a2amx::layout::Layout::new(self.state.clone()),
            runtime_dir: self.runtime.clone(),
        }
    }
}

fn bwrap_path_dir(target: &Path) -> TempDir {
    let dir = tempfile::tempdir().expect("bwrap directory");
    symlink(target, dir.path().join("bwrap")).expect("bwrap symlink");
    dir
}

fn stub_path_dir(exit_code: i32) -> TempDir {
    let dir = tempfile::tempdir().expect("stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(&path, format!("#!/bin/sh\nexit {exit_code}\n")).expect("stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("stub permissions");
    dir
}
fn probe_fail_stub_path_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("probe stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(
        &path,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then exit 0; fi\nexit 42\n",
    )
    .expect("probe stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("probe stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("probe stub permissions");
    dir
}

fn bwrap_probe_fail_stub_path_dir() -> TempDir {
    let dir = tempfile::tempdir().expect("bwrap probe stub directory");
    let path = dir.path().join("bwrap");
    std::fs::write(
        &path,
        "#!/bin/sh\ncase \"$1\" in\n--version|--ignore-signal*) exit 0 ;;\n*) exit 42 ;;\nesac\n",
    )
    .expect("bwrap probe stub");
    let mut permissions = std::fs::metadata(&path)
        .expect("bwrap probe stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&path, permissions).expect("bwrap probe stub permissions");
    dir
}

fn require_bwrap() -> bool {
    std::env::var_os("A2AMX_REQUIRE_BWRAP").is_some_and(|value| value == "1")
}

async fn start_confine_daemon(state: &Path, bwrap_dir: &Path) -> anyhow::Result<Daemon> {
    start_confine_daemon_with_env(state, bwrap_dir, None).await
}

async fn start_confine_daemon_with_env(
    state: &Path,
    bwrap_dir: &Path,
    env_path: Option<PathBuf>,
) -> anyhow::Result<Daemon> {
    Daemon::start(DaemonConfig {
        root: state.to_owned(),
        listen: vec!["127.0.0.1:0".parse()?],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(OsString::from(bwrap_dir)),
        env_path,
    })
    .await
    .context("start test daemon")
}

fn confined_shell_request(
    cwd: &Path,
    script: &str,
    env: Vec<(String, String)>,
    options: ConfineRequest,
) -> Request {
    Request::Confine {
        confine: options,
        request: Box::new(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cols: 80,
            rows: 24,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env,
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("confined-test".into()),
            harness: Harness::Generic,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    }
}

fn populate_claude_overlay(fixture: &Fixture, credential: bool) {
    let root = fixture.home.join(".claude");
    std::fs::create_dir_all(&root).expect("Claude root");
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
        "plugins",
    ] {
        std::fs::create_dir_all(root.join(name)).expect("Claude data directory");
    }
    std::fs::write(root.join("history.jsonl"), "history").expect("history");
    std::fs::write(root.join("stats-cache.json"), "stats").expect("stats");
    if credential {
        std::fs::write(root.join(".credentials.json"), "credentials").expect("credentials");
    }
    std::fs::write(root.join("settings.json"), "settings").expect("settings");
    let target = fixture._root.path().join("external-hooks");
    std::fs::create_dir_all(&target).expect("hooks target");
    symlink(&target, root.join("hooks")).expect("hooks symlink");
}

fn populate_omp_overlay(fixture: &Fixture) {
    let root = fixture.home.join(".omp");
    let agent = root.join("agent");
    std::fs::create_dir_all(agent.join("sessions")).expect("OMP sessions");
    std::fs::create_dir_all(root.join("extensions")).expect("OMP extensions");
    std::fs::write(agent.join("a.db"), "agent").expect("agent db");
    std::fs::write(root.join("stats.db"), "stats").expect("stats db");
    std::fs::write(root.join("config.yml"), "config").expect("OMP config");
}

mod bwrap_probe_messages;
mod git_config_case;
mod git_dirs_and_roots;
mod grant_dir_reasons;
mod grant_writes_and_create;
mod grants_links_and_env;
mod grants_runtime;
mod harness_config;
mod harnesses;
mod limits_and_hides;
mod notices_and_preflight;
mod overlay_symlinked_agent;
mod paths_and_git_config;
mod plan_basics;
mod real_bwrap;
mod wrap_and_sockets;
