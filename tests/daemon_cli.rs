#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_a2amx");

fn a2amx(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command.arg("--home").arg(home).args(args);
    command
}

fn run(home: &Path, args: &[&str]) -> Output {
    a2amx(home, args)
        .stdin(Stdio::null())
        .output()
        .expect("run a2amx")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Ends any daemon left in the state dir, so a failing test leaves no process.
struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = run(self.0, &["stop", "--yes"]);
    }
}

fn wait_exit(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("poll child") {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

#[test]
fn background_start_detaches_and_logs() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);

    let started = Instant::now();
    let output = run(&home, &["daemon", "--background"]);

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(stdout(&output).starts_with("listening on 127.0.0.1:"));
    assert!(run(&home, &["list"]).status.success());
    let mode = std::fs::metadata(home.join("daemon.log"))
        .expect("daemon.log exists")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn background_refuses_when_already_running() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());

    let second = run(&home, &["daemon", "--background"]);

    assert!(!second.status.success());
    assert!(stderr(&second).contains("already running"));
}

#[test]
fn stop_ends_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());

    let stop = run(&home, &["stop", "--yes"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "stopped\n");
    assert!(!run(&home, &["list"]).status.success());
}

#[test]
fn stop_without_a_daemon_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");

    let stop = run(&home, &["stop"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "no daemon running\n");
}

#[test]
fn stop_refuses_to_end_sessions_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let refused = run(&home, &["stop"]);

    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("1 running session"));
    assert!(run(&home, &["list"]).status.success());
    let forced = run(&home, &["stop", "--yes"]);
    assert!(forced.status.success(), "stderr: {}", stderr(&forced));
}

#[test]
fn foreground_daemon_honors_stop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let mut daemon = a2amx(&home, &["daemon", "--listen", "127.0.0.1:0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .expect("spawn foreground daemon");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !run(&home, &["list"]).status.success() {
        assert!(Instant::now() < deadline, "daemon did not come up");
        std::thread::sleep(Duration::from_millis(100));
    }

    let stop = run(&home, &["stop", "--yes"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    let status = wait_exit(&mut daemon, Duration::from_secs(15));
    if status.is_none() {
        let _ = daemon.kill();
    }
    assert!(status.expect("daemon exited").success());
}

#[test]
fn background_reports_a_startup_failure() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("not-a-directory");
    std::fs::write(&home, b"").unwrap();

    let output = run(&home, &["daemon", "--background"]);

    assert!(!output.status.success());
    let message = stderr(&output);
    assert!(
        message.contains("state path is not a directory")
            || message.contains("daemon exited during startup"),
        "stderr: {message}"
    );
}

/// An executable named `claude` that only sleeps, so no real harness starts.
fn fake_claude(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("claude");
    std::fs::write(&path, "#!/bin/sh\nexec sleep 30\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn harness_is_inferred_from_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "--background"]).status.success());

    let created = run(&home, &["new", "--detach", "--", claude.to_str().unwrap()]);

    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert!(stdout(&run(&home, &["list", "--details"])).contains("--mcp-config"));
}

#[test]
fn explicit_generic_harness_wins_over_inference() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "--background"]).status.success());

    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            claude.to_str().unwrap(),
        ],
    );

    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert!(!stdout(&run(&home, &["list", "--details"])).contains("--mcp-config"));
}
