//! Shared helpers for integration tests. Include with `mod common;`.

// Test-only code: clippy's allow-*-in-tests does not cover a shared helper module.
#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

pub mod pty;

use a2amx::client::Client;
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::Limits;
use a2amx::wire::{Request, Response};
use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;
use tempfile::TempDir;

/// Start a daemon in a fresh temp state dir on an OS-chosen loopback port, so
/// concurrent tests never collide. Keep the `TempDir` alive for the test's duration.
pub async fn start_daemon() -> (TempDir, Daemon) {
    let dir = tempfile::tempdir().expect("temp state dir");
    let daemon = start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    (dir, daemon)
}

pub async fn start_daemon_in(dir: &Path, host_name: Option<&str>, limits: Limits) -> Daemon {
    Daemon::start(DaemonConfig {
        state_dir: dir.to_path_buf(),
        listen: vec!["127.0.0.1:0".parse().expect("loopback address")],
        host_name: host_name.map(str::to_owned),
        limits,
    })
    .await
    .expect("daemon starts")
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
            name: name.map(str::to_owned),
            harness,
            deliver: Some(deliver),
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
