#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;

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

fn screen_until(home: &Path, args: &[&str], expected: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run(home, args);
        if !output.status.success() || stdout(&output) == expected || Instant::now() >= deadline {
            return output;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
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

#[test]
fn screen_prints_trimmed_visible_rows() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'first  \\n\\nthird   \\n\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "first\n\nthird\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "first\n\nthird\n");
}

#[test]
fn screen_resolves_session_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'named\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "worker"], "named\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "named\n");
}

#[test]
fn screen_rows_keeps_only_the_last_lines() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'one\\ntwo\\nthree\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1", "--rows", "2"], "two\nthree\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "two\nthree\n");
}

#[test]
fn screen_reads_the_active_alternate_screen() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?1049hALT\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "ALT\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "ALT\n");
}

#[test]
fn screen_does_not_resize_or_detach_an_attached_session() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'attached\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let attach_args = vec!["attach".to_owned(), "s1".to_owned()];
    let mut attached = common::pty::PtyHarness::spawn(&attach_args, &home).unwrap();
    attached
        .wait_for_text("attached", Duration::from_secs(10))
        .unwrap();
    let before = stdout(&run(&home, &["list"]));
    let before_row = before.lines().find(|line| line.starts_with("s1 ")).unwrap();
    let before_size = before_row.split_whitespace().nth(8).unwrap().to_owned();

    let output = screen_until(&home, &["screen", "s1"], "attached\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "attached\n");

    let after = stdout(&run(&home, &["list"]));
    let after_row = after.lines().find(|line| line.starts_with("s1 ")).unwrap();
    assert_eq!(after_row.split_whitespace().nth(4), Some("yes"));
    assert_eq!(
        after_row.split_whitespace().nth(8),
        Some(before_size.as_str())
    );
}

#[test]
fn screen_unknown_session_matches_kill_error() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());

    let kill = run(&home, &["kill", "missing"]);
    let screen = run(&home, &["screen", "missing"]);
    assert!(!kill.status.success());
    assert!(!screen.status.success());
    assert_eq!(stderr(&screen), stderr(&kill));
}

#[test]
fn screen_reads_the_last_screen_after_exit() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'exited\\n'; exit 7",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let listed = stdout(&run(&home, &["list"]));
        if listed
            .lines()
            .find(|line| line.starts_with("s1 "))
            .is_some_and(|line| {
                line.split_whitespace()
                    .nth(3)
                    .is_some_and(|state| state.starts_with("exited("))
            })
        {
            break;
        }
        assert!(Instant::now() < deadline, "session did not exit: {listed}");
        std::thread::sleep(Duration::from_millis(20));
    }

    let output = screen_until(&home, &["screen", "s1"], "exited\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "exited\n");
}

#[test]
fn screen_wide_character_is_not_duplicated() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "--background"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf 'A界B\\n'; sleep 30",
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let output = screen_until(&home, &["screen", "s1"], "A界B\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "A界B\n");
}
