//! Shared helpers for integration tests. Include with `mod common;`.

// Test-only code: clippy's allow-*-in-tests does not cover a shared helper module.
#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

pub mod fake_codex;
pub mod fake_discord;
pub mod pty;

use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{Limits, RemoteMode, TeamScope};
use a2amx::wire::{Request, Response};
use std::net::SocketAddr;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;
pub fn new_session_request(name: Option<&str>, argv: &[&str]) -> Request {
    new_session_request_with_remote(name, argv, None)
}

pub fn new_session_request_with_remote(
    name: Option<&str>,
    argv: &[&str],
    remote: Option<RemoteMode>,
) -> Request {
    Request::NewSession {
        argv: argv.iter().map(|argument| (*argument).to_owned()).collect(),
        cols: 40,
        rows: 10,
        cwd: None,
        env: vec![],
        reset: vec![],
        control_from: vec![],
        watch: vec![],
        activity: None,
        heartbeat: None,
        name: name.map(str::to_owned),
        harness: Harness::Generic,
        deliver: Some(Deliver::Hold),
        team: None,
        role: None,
        remote,
    }
}

pub async fn start_confined_daemon_with_limits(limits: Limits) -> Option<(TempDir, Daemon)> {
    let real_bwrap = a2amx::confine::resolve_bwrap(None).ok()?;
    let dir = tempfile::tempdir().expect("temp state dir");
    let bwrap_dir = dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).expect("bwrap directory");
    symlink(real_bwrap, bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let daemon = Daemon::start(DaemonConfig {
        root: dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse().expect("loopback address")],
        host_name: Some("host-a".into()),
        limits,
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await
    .expect("confined daemon starts");
    Some((dir, daemon))
}

pub async fn start_confined_daemon() -> Option<(TempDir, Daemon)> {
    start_confined_daemon_with_limits(Limits::default()).await
}

pub async fn create_confined_with_team(
    admin: &mut Client,
    fixture: &Path,
    name: &str,
    remote: Option<RemoteMode>,
    team: Option<TeamScope>,
) -> String {
    let response = admin
        .request(Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec!["sleep".into(), "30".into()],
                cols: 40,
                rows: 10,
                cwd: Some(fixture.to_string_lossy().into_owned()),
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some(name.into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                team,
                role: None,
                remote,
            }),
        })
        .await
        .expect("create confined session");
    match response {
        Response::Created { session } => session,
        response => panic!("confined session creation failed: {response:?}"),
    }
}

pub async fn create_confined_with_token(
    admin: &mut Client,
    fixture: &Path,
    name: &str,
    remote: Option<RemoteMode>,
    team: Option<TeamScope>,
    harness: Harness,
    deliver: Option<Deliver>,
) -> (String, String) {
    let response = admin
        .request(Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf %s \"$A2AMX_TOKEN\" > token; exec sleep 90".into(),
                ],
                cols: 40,
                rows: 10,
                cwd: Some(fixture.to_string_lossy().into_owned()),
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some(name.into()),
                harness,
                deliver,
                team,
                role: None,
                remote,
            }),
        })
        .await
        .expect("create confined token session");
    let session = match response {
        Response::Created { session } => session,
        response => panic!("confined token session creation failed: {response:?}"),
    };
    let token_path = fixture.join("token");
    let token = eventually(|| async {
        let token = std::fs::read_to_string(&token_path).ok()?;
        (token.len() == 64).then_some(token)
    })
    .await;
    (session, token)
}

pub async fn create_confined(
    admin: &mut Client,
    fixture: &Path,
    name: &str,
    remote: Option<RemoteMode>,
) -> String {
    create_confined_with_team(admin, fixture, name, remote, None).await
}

pub async fn gateway(dir: &Path, address: SocketAddr) -> Client {
    let token = std::fs::read_to_string(dir.join("run/gateway.token")).expect("gateway token");
    Client::connect_addr(address, token.trim())
        .await
        .expect("gateway connection")
}

pub fn error_message(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        response => panic!("expected error response, got {response:?}"),
    }
}

pub fn failed_code(response: Response) -> String {
    match response {
        Response::Failed { code, .. } => code,
        response => panic!("expected failed response, got {response:?}"),
    }
}

pub fn failed_message(response: Response) -> String {
    match response {
        Response::Failed { message, .. } => message,
        response => panic!("expected failed response, got {response:?}"),
    }
}

pub fn team_scope(name: &str) -> TeamScope {
    TeamScope {
        name: name.into(),
        private: false,
        allow: vec![],
        agents: vec![],
    }
}

pub async fn issue_code(admin: &mut Client, team: &str) -> String {
    let response = admin
        .request(Request::IssueLeaseCode { team: team.into() })
        .await
        .unwrap();
    let Response::LeaseCode { code, .. } = response else {
        panic!("expected lease code, got {response:?}");
    };
    code
}

pub async fn issue_and_redeem(admin: &mut Client, gateway: &mut Client, team: &str) {
    let code = issue_code(admin, team).await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Ok
    ));
}

pub fn limits_with(lease_secs: u64, code_secs: u64) -> Limits {
    Limits {
        lease_secs,
        code_secs,
        ..Limits::default()
    }
}

pub fn keep_coverage_env(command: &mut Command) -> &mut Command {
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

pub struct DaemonProcess {
    child: Child,
    state: PathBuf,
}

impl Drop for DaemonProcess {
    fn drop(&mut self) {
        let _ = Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(&self.state)
            .args(["daemon", "stop", "--yes"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(_) => break,
            }
        }
        if self
            .child
            .try_wait()
            .map_or(true, |status| status.is_none())
        {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

/// Run the real daemon with only the environment needed by its child sessions.
pub fn spawn_daemon_process(state: &Path, home: &Path, runtime: &Path) -> DaemonProcess {
    let path = std::env::var_os("PATH").expect("PATH");
    let child = keep_coverage_env(
        Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(state)
            .args(["daemon", "start", "--foreground", "--listen", "127.0.0.1:0"])
            .env_clear(),
    )
    .env("PATH", path)
    .env("HOME", home)
    .env("XDG_RUNTIME_DIR", runtime)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .expect("spawn foreground daemon");
    let mut process = DaemonProcess {
        child,
        state: state.to_owned(),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = process.child.try_wait().expect("poll foreground daemon") {
            panic!("foreground daemon exited during startup: {status}");
        }
        let ready = Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(state)
            .arg("list")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("check foreground daemon");
        if ready.success() {
            return process;
        }
        assert!(
            Instant::now() < deadline,
            "foreground daemon did not become ready within 10 seconds"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn list_cells<'a>(listing: &'a str, id: &str) -> Vec<&'a str> {
    listing
        .lines()
        .find(|line| line.split_whitespace().next() == Some(id))
        .unwrap_or_else(|| panic!("session {id} not found in list"))
        .split_whitespace()
        .collect()
}

/// Start a daemon in a fresh temp state dir on an OS-chosen loopback port, so
/// concurrent tests never collide. Keep the `TempDir` alive for the test's duration.
pub async fn start_daemon() -> (TempDir, Daemon) {
    let dir = tempfile::tempdir().expect("temp state dir");
    let daemon = start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    (dir, daemon)
}

pub async fn start_daemon_in(dir: &Path, host_name: Option<&str>, limits: Limits) -> Daemon {
    // Restart tests start a daemon right after shutting one down on the same dir. Under load
    // the old flock can outlive `shutdown` for a moment (suspected: a PTY child forked by a
    // parallel test inherits the fd until it execs), so retry that one error briefly.
    // shortcut: fixed 5s budget; find who holds the lock if this ever needs more.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let bwrap_dir = dir.join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).expect("bwrap path directory");
    loop {
        let started = Daemon::start(DaemonConfig {
            root: dir.to_path_buf(),
            listen: vec!["127.0.0.1:0".parse().expect("loopback address")],
            host_name: host_name.map(str::to_owned),
            limits,
            bwrap_path: Some(bwrap_dir.clone().into_os_string()),
            env_path: None,
        })
        .await;
        match started {
            Err(error)
                if error
                    .to_string()
                    .contains("another a2amx daemon is running")
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            other => return other.expect("daemon starts"),
        }
    }
}

pub async fn eventually<F, Fut, T>(mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Option<T>>,
{
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(value) = check().await {
                return value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition becomes true")
}

pub async fn new_agent(
    admin: &mut Client,
    dir: &Path,
    name: Option<&str>,
    harness: Harness,
    deliver: Deliver,
) -> (String, String, SocketAddr) {
    new_team_agent(admin, dir, name, harness, deliver, None, &[], &[]).await
}
#[allow(clippy::too_many_arguments)] // Locked spec requires this public integration-test seam.
pub async fn new_team_agent(
    admin: &mut Client,
    dir: &Path,
    name: Option<&str>,
    harness: Harness,
    deliver: Deliver,
    team: Option<TeamScope>,
    watch: &[&str],
    control_from: &[&str],
) -> (String, String, SocketAddr) {
    let output = tempfile::NamedTempFile::new_in(dir).expect("credential capture path");
    let response = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; sleep 30".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.path().to_string_lossy().into_owned())],
            reset: vec![],
            control_from: control_from.iter().map(|name| (*name).to_owned()).collect(),
            watch: watch.iter().map(|name| (*name).to_owned()).collect(),
            name: name.map(str::to_owned),
            harness,
            deliver: Some(deliver),
            team,
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .expect("create credential session");
    let Response::Created { session } = response else {
        panic!("session creation failed");
    };
    let credentials = eventually(|| async {
        let text = std::fs::read_to_string(output.path()).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let addr = lines.next()?.parse::<SocketAddr>().ok()?;
        (token.len() == 64).then_some((token, addr))
    })
    .await;
    (session, credentials.0, credentials.1)
}
