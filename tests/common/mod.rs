//! Shared helpers for integration tests. Include with `mod common;`.

// Test-only code: clippy's allow-*-in-tests does not cover a shared helper module.
#![allow(clippy::expect_used, clippy::unwrap_used, dead_code)]

pub mod pty;

use a2amx::daemon::{Daemon, DaemonConfig};
use tempfile::TempDir;

/// Start a daemon in a fresh temp state dir on an OS-chosen loopback port, so
/// concurrent tests never collide. Keep the `TempDir` alive for the test's duration.
pub async fn start_daemon() -> (TempDir, Daemon) {
    let dir = tempfile::tempdir().expect("temp state dir");
    let daemon = Daemon::start(DaemonConfig {
        state_dir: dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse().expect("loopback address")],
    })
    .await
    .expect("daemon starts");
    (dir, daemon)
}
