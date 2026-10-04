#![allow(clippy::expect_used, clippy::unwrap_used)]
mod common;
use common::pty::PtyHarness;
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

fn run_owned(home: &Path, args: &[String]) -> Output {
    a2amx(home, &[])
        .args(args)
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

fn screen_contains_until(home: &Path, args: &[&str], expected: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run(home, args);
        if !output.status.success()
            || stdout(&output).contains(expected)
            || Instant::now() >= deadline
        {
            return output;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ends any daemon left in the state dir, so a failing test leaves no process.
struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = run(self.0, &["daemon", "stop", "--yes"]);
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
    let output = run(&home, &["daemon", "start"]);

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
    assert!(run(&home, &["daemon", "start"]).status.success());

    let second = run(&home, &["daemon", "start"]);

    assert!(!second.status.success());
    assert!(stderr(&second).contains("already running"));
}

#[test]
fn stop_ends_the_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let stop = run(&home, &["daemon", "stop", "--yes"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "stopped\n");
    assert!(!run(&home, &["list"]).status.success());
}

#[test]
fn stop_without_a_daemon_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");

    let stop = run(&home, &["daemon", "stop"]);

    assert!(stop.status.success(), "stderr: {}", stderr(&stop));
    assert_eq!(stdout(&stop), "no daemon running\n");
}

#[test]
fn stop_refuses_to_end_sessions_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));

    let refused = run(&home, &["daemon", "stop"]);

    assert!(!refused.status.success());
    assert!(stderr(&refused).contains("1 running session"));
    assert!(run(&home, &["list"]).status.success());
    let forced = run(&home, &["daemon", "stop", "--yes"]);
    assert!(forced.status.success(), "stderr: {}", stderr(&forced));
}

#[test]
fn foreground_daemon_honors_stop() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let mut daemon = a2amx(
        &home,
        &["daemon", "start", "--foreground", "--listen", "127.0.0.1:0"],
    )
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .spawn()
    .expect("spawn foreground daemon");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !run(&home, &["list"]).status.success() {
        assert!(Instant::now() < deadline, "daemon did not come up");
        std::thread::sleep(Duration::from_millis(100));
    }

    let stop = run(&home, &["daemon", "stop", "--yes"]);

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

    let output = run(&home, &["daemon", "start"]);

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

fn fake_reset_composer(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake-reset-composer.py");
    std::fs::write(
        &path,
        r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
    else:
        print("got:" + line, flush=True)
"#,
    )
    .expect("write fake reset composer");
    path
}

fn fake_timeout_composer(dir: &Path) -> std::path::PathBuf {
    let path = dir.join("fake-timeout-composer.py");
    std::fs::write(
        &path,
        r#"import sys
sys.stdout.write("\x1b[?2004h")
sys.stdout.flush()
for raw in sys.stdin.buffer:
    line = raw.decode(errors="replace").replace("\x1b[200~", "").replace("\x1b[201~", "").rstrip("\r\n")
    if line == "/clear":
        print("marker:/clear", flush=True)
    elif line == "/second":
        print("got:/second", flush=True)
        sys.stdout.write("\x1b[?2004l")
        sys.stdout.flush()
"#,
    )
    .expect("write fake timeout composer");
    path
}

#[test]
fn reset_uses_default_clear_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
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
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(stdout(&reset), "reset s1: 1 step\n");
    let screen = screen_contains_until(&home, &["screen", "worker"], "marker:/clear");
    assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    assert!(stdout(&screen).contains("marker:/clear"));
}

#[test]
fn reset_rejects_a_draft_without_typing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
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
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let args = vec![
        "--home".to_owned(),
        home.to_string_lossy().into_owned(),
        "attach".to_owned(),
        "worker".to_owned(),
    ];
    let mut terminal = PtyHarness::spawn(&args, &home).unwrap();
    terminal
        .wait_for_text("worker", Duration::from_secs(10))
        .unwrap();
    terminal.send(b"draft").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let reset = run(&home, &["reset", "worker"]);
    assert!(!reset.status.success());
    assert!(
        stderr(&reset).contains("draft_present"),
        "{}",
        stderr(&reset)
    );
    drop(terminal);
}

#[test]
fn reset_reports_not_ready_exited_and_unknown_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let not_ready = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "not-ready",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(not_ready.status.success(), "stderr: {}", stderr(&not_ready));
    let reset = run(&home, &["reset", "not-ready"]);
    assert!(!reset.status.success());
    assert!(stderr(&reset).contains("not_ready"), "{}", stderr(&reset));
    assert!(stderr(&reset).contains("step 1 of 1"), "{}", stderr(&reset));

    let exited = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "exited",
            "--harness",
            "generic",
            "--",
            "true",
        ],
    );
    assert!(exited.status.success(), "stderr: {}", stderr(&exited));
    let reset = run(&home, &["reset", "exited"]);
    assert!(!reset.status.success());
    assert!(stderr(&reset).contains("exited"), "{}", stderr(&reset));

    let reset = run(&home, &["reset", "missing"]);
    assert!(!reset.status.success());
    assert!(
        stderr(&reset).contains("unknown_session"),
        "{}",
        stderr(&reset)
    );
}

#[test]
fn reset_times_out_when_second_step_stays_not_ready() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_timeout_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/second",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(!reset.status.success(), "stdout: {}", stdout(&reset));
    let message = stderr(&reset);
    assert!(message.contains("step_timeout"), "{message}");
    assert!(message.contains("step 2 of 2"), "{message}");
    let screen = screen_contains_until(&home, &["screen", "worker"], "marker:/clear");
    assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    let text = stdout(&screen);
    assert!(text.contains("marker:/clear"), "{text:?}");
    assert!(text.contains("got:/second"), "{text:?}");
    let retry = run(&home, &["reset", "worker"]);
    assert!(!retry.status.success());
    assert!(!stderr(&retry).contains("busy"), "{}", stderr(&retry));
    assert!(stderr(&retry).contains("not_ready"), "{}", stderr(&retry));
}

#[test]
fn reset_runs_configured_steps_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/prewalk restart",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let reset = run(&home, &["reset", "worker"]);
    assert!(
        reset.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&reset),
        stderr(&reset)
    );
    assert_eq!(stdout(&reset), "reset s1: 2 steps\n");
    let screen = screen_contains_until(&home, &["screen", "worker"], "got:/prewalk restart");
    assert!(screen.status.success(), "stderr: {}", stderr(&screen));
    let text = stdout(&screen);
    assert!(text.contains("marker:/clear"), "{text:?}");
    assert!(text.contains("got:/prewalk restart"), "{text:?}");
    assert!(
        text.find("marker:/clear") < text.find("got:/prewalk restart"),
        "{text:?}"
    );
}

#[test]
fn harness_is_inferred_from_the_command() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");

    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());

    let created = run(&home, &["new", "--detach", "--", claude.to_str().unwrap()]);

    assert!(created.status.success(), "stderr: {}", stderr(&created));
    assert!(stdout(&run(&home, &["list", "--details"])).contains("--mcp-config"));
}
#[test]
fn new_rejects_invalid_reset_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let cases = [
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                "clear".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "must start with '/'",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                "/bad\ncommand".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "control characters",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--reset".to_owned(),
                format!("/{}", "x".repeat(200)),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "at most 200 bytes",
        ),
        (
            vec![
                "new".to_owned(),
                "--detach".to_owned(),
                "--control-from".to_owned(),
                "Bad_Name".to_owned(),
                "--".to_owned(),
                "true".to_owned(),
            ],
            "invalid name",
        ),
    ];
    for (args, expected) in cases {
        let output = run_owned(&home, &args);
        assert!(!output.status.success(), "{args:?}");
        assert!(
            stderr(&output).contains(expected),
            "{expected}: {}",
            stderr(&output)
        );
    }

    let mut too_many = vec!["new".to_owned(), "--detach".to_owned()];
    for _ in 0..9 {
        too_many.push("--reset".to_owned());
        too_many.push("/clear".to_owned());
    }
    too_many.extend(["--".to_owned(), "true".to_owned()]);
    let output = run_owned(&home, &too_many);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("at most 8 steps"));
}

#[test]
fn reset_rejects_a_concurrent_request_as_busy() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let fixture = fake_reset_composer(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--harness",
            "generic",
            "--reset",
            "/clear",
            "--reset",
            "/second",
            "--",
            "python3",
            "-u",
            fixture.to_str().unwrap(),
        ],
    );
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let first = a2amx(&home, &["reset", "worker"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn first reset");
    std::thread::sleep(Duration::from_millis(200));
    let second = run(&home, &["reset", "worker"]);
    assert!(
        !second.status.success(),
        "second reset unexpectedly succeeded"
    );
    assert!(stderr(&second).contains("busy"), "{}", stderr(&second));
    let first = first.wait_with_output().expect("wait first reset");
    assert!(first.status.success(), "stderr: {}", stderr(&first));
    assert_eq!(stdout(&first), "reset s1: 2 steps\n");
}

#[test]
fn explicit_generic_harness_wins_over_inference() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let claude = fake_claude(dir.path());
    assert!(run(&home, &["daemon", "start"]).status.success());

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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    let before_size = before_row.split_whitespace().nth(9).unwrap().to_owned();

    let output = screen_until(&home, &["screen", "s1"], "attached\n");
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "attached\n");

    let after = stdout(&run(&home, &["list"]));
    let after_row = after.lines().find(|line| line.starts_with("s1 ")).unwrap();
    assert_eq!(after_row.split_whitespace().nth(5), Some("yes"));
    assert_eq!(
        after_row.split_whitespace().nth(9),
        Some(before_size.as_str())
    );
}

#[test]
fn screen_unknown_session_matches_kill_error() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

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
    assert!(run(&home, &["daemon", "start"]).status.success());
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
    assert!(run(&home, &["daemon", "start"]).status.success());
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

#[test]
fn daemon_status_reports_address_and_session_counts() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let address = std::fs::read_to_string(home.join("addr"))
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .to_owned();
    let running = run(&home, &["new", "--detach", "--", "sh", "-c", "sleep 30"]);
    assert!(running.status.success(), "stderr: {}", stderr(&running));
    let exited = run(&home, &["new", "--detach", "--", "sh", "-c", "exit 7"]);
    assert!(exited.status.success(), "stderr: {}", stderr(&exited));
    let listed = screen_contains_until(&home, &["list"], "exited");
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));

    let status = run(&home, &["daemon", "status"]);

    assert!(status.status.success(), "stderr: {}", stderr(&status));
    assert_eq!(
        stdout(&status),
        format!("running\nlistening on {address}\nsessions: 1 running, 1 exited\n")
    );
    assert_eq!(stderr(&status), "");
}

#[test]
fn removed_daemon_spellings_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let background = ["--", "background"].concat();
    let top_level_stop = ["st", "op"].concat();
    for args in [
        vec!["daemon"],
        vec!["daemon", background.as_str()],
        vec![top_level_stop.as_str()],
    ] {
        let output = run(&home, &args);
        assert!(!output.status.success(), "accepted {args:?}");
    }
}

#[test]
fn kill_running_session_requires_confirmation_without_a_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh", "-c", "sleep 30"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = stdout(&created).trim().to_owned();

    let refused = run(&home, &["kill", &id]);

    assert!(!refused.status.success());
    assert_eq!(
        stderr(&refused),
        format!("a2amx: refusing to kill: session {id} is running; pass --yes to end it\n")
    );
    let listing = stdout(&run(&home, &["list"]));
    assert!(listing.contains(&id));
    assert!(listing.contains("running"));
    let forced = run(&home, &["kill", &id, "--yes"]);
    assert!(forced.status.success(), "stderr: {}", stderr(&forced));
}

#[test]
fn kill_exited_session_skips_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let created = run(&home, &["new", "--detach", "--", "sh", "-c", "exit 7"]);
    assert!(created.status.success(), "stderr: {}", stderr(&created));
    let id = stdout(&created).trim().to_owned();
    let listed = screen_contains_until(&home, &["list"], "exited");
    assert!(listed.status.success(), "stderr: {}", stderr(&listed));

    let killed = run(&home, &["kill", &id]);

    assert!(killed.status.success(), "stderr: {}", stderr(&killed));
}

#[test]
fn daemon_status_without_a_daemon_is_quiet_failure() {
    let dir = tempfile::tempdir().unwrap();
    let output = run(dir.path(), &["daemon", "status"]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(stdout(&output), "not running\n");
    assert_eq!(stderr(&output), "");
}

struct ExitFixture {
    _dir: tempfile::TempDir,
    home: std::path::PathBuf,
    record: std::path::PathBuf,
}

impl ExitFixture {
    fn start(mode: &str, harness: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Self {
            home: dir.path().join("state"),
            record: dir.path().join("bytes"),
            _dir: dir,
        };
        let script = fixture._dir.path().join("exit.py");
        std::fs::write(
            &script,
            r#"import os, sys, time, tty
tty.setraw(0)
record = open(sys.argv[1], "wb", buffering=0)
mode = sys.argv[2]
cols = os.get_terminal_size().columns
sys.stdout.write("\x1b[?2004h\x1b[?25h\x1b[2J\x1b[1;1Hfixture-ready")
if mode != "not-ready":
    sys.stdout.write("\x1b[3;1H" + "─" * cols + "\x1b[5;1H" + "─" * cols + "\x1b[4;1H❯\u00a0")
sys.stdout.flush()
first = None
while True:
    byte = os.read(0, 1)
    record.write(byte)
    if byte == b"\x04":
        if mode == "first":
            sys.exit(0)
        if mode == "two":
            now = time.monotonic()
            if first is not None and now - first <= 0.6:
                sys.exit(0)
            first = now
"#,
        )
        .unwrap();
        let started = a2amx(&fixture.home, &["daemon", "start"])
            .env("RUST_LOG", "info")
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert!(started.status.success(), "{}", stderr(&started));
        let created = run(
            &fixture.home,
            &[
                "new",
                "--detach",
                "--name",
                "worker",
                "--harness",
                harness,
                "--no-channel",
                "--",
                "sh",
                "-c",
                "exec python3 -u \"$1\" \"$2\" \"$3\"",
                "fixture",
                script.to_str().unwrap(),
                fixture.record.to_str().unwrap(),
                mode,
            ],
        );
        assert!(created.status.success(), "{}", stderr(&created));
        let screen = screen_contains_until(&fixture.home, &["screen", "worker"], "fixture-ready");
        assert!(
            stdout(&screen).contains("fixture-ready"),
            "{}",
            stderr(&screen)
        );
        fixture
    }

    fn bytes(&self) -> Vec<u8> {
        std::fs::read(&self.record).unwrap()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.join("daemon.log")).unwrap()
    }
}

impl Drop for ExitFixture {
    fn drop(&mut self) {
        let _ = run(&self.home, &["daemon", "stop", "--yes"]);
    }
}

#[test]
fn kill_gracefully_exits_on_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
    assert!(!stdout(&run(&fixture.home, &["list"])).contains("worker"));
}

#[test]
fn kill_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes", "--now"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_sends_claude_two_key_exit_sequence() {
    let fixture = ExitFixture::start("two", "claude");
    let started = Instant::now();
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "two-key exit took {:?}",
        started.elapsed()
    );
    assert_eq!(fixture.bytes(), b"\x04\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_falls_back_after_graceful_timeout() {
    let fixture = ExitFixture::start("ignore", "claude");
    let started = Instant::now();
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert!(fixture.bytes().starts_with(b"\x04\x04"));
    assert!(
        fixture.log().contains("outcome=\"timeout\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_skips_ctrl_d_when_claude_is_not_ready() {
    let fixture = ExitFixture::start("not-ready", "claude");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        fixture
            .log()
            .contains("outcome=\"skipped\" code=\"not_ready\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn kill_generic_session_uses_hup_without_ctrl_d() {
    let fixture = ExitFixture::start("first", "generic");
    let killed = run(&fixture.home, &["kill", "worker", "--yes"]);
    assert!(killed.status.success(), "{}", stderr(&killed));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("graceful exit"),
        "{}",
        fixture.log()
    );
}

#[test]
fn daemon_stop_gracefully_exits_idle_session() {
    let fixture = ExitFixture::start("first", "claude");
    let stopped = run(&fixture.home, &["daemon", "stop", "--yes"]);
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn daemon_stop_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let stopped = run(&fixture.home, &["daemon", "stop", "--now", "--yes"]);
    assert!(stopped.status.success(), "{}", stderr(&stopped));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn team_down_gracefully_exits_idle_session() {
    let fixture = ExitFixture::start("first", "claude");
    let down = run(&fixture.home, &["team", "down", "worker"]);
    assert!(down.status.success(), "{}", stderr(&down));
    assert_eq!(fixture.bytes(), b"\x04");
    assert!(
        fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn team_down_now_skips_ctrl_d() {
    let fixture = ExitFixture::start("first", "claude");
    let down = run(&fixture.home, &["team", "down", "--now", "worker"]);
    assert!(down.status.success(), "{}", stderr(&down));
    assert!(fixture.bytes().is_empty());
    assert!(
        !fixture.log().contains("outcome=\"exited\""),
        "{}",
        fixture.log()
    );
}

#[test]
fn new_rejects_the_daemon_sender_name() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let output = run(
        &home,
        &["new", "--detach", "--name", "a2amx-daemon", "--", "true"],
    );
    assert!(!output.status.success(), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("reserved for the daemon"),
        "{}",
        stderr(&output)
    );
    let self_watch = run(
        &home,
        &[
            "new", "--detach", "--name", "self", "--watch", "self", "--", "true",
        ],
    );
    assert!(!self_watch.status.success(), "{}", stdout(&self_watch));
    assert!(
        stderr(&self_watch).contains("watches itself"),
        "{}",
        stderr(&self_watch)
    );
}

#[test]
fn watching_session_receives_a_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());

    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 3",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));

    let messages = screen_contains_until(
        &home,
        &["messages", "--session", "watcher"],
        "peer exited: peer (code 3)",
    );
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    assert!(
        stdout(&messages).contains("a2amx-daemon@"),
        "{}",
        stdout(&messages)
    );
}

#[test]
fn two_watchers_each_receive_one_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    for watcher in ["watcher-a", "watcher-b"] {
        let name_args = vec![
            "new".to_owned(),
            "--detach".to_owned(),
            "--name".to_owned(),
            watcher.to_owned(),
            "--harness".to_owned(),
            "generic".to_owned(),
            "--watch".to_owned(),
            "peer".to_owned(),
            "--".to_owned(),
            "sh".to_owned(),
            "-c".to_owned(),
            "sleep 30".to_owned(),
        ];
        let output = run_owned(&home, &name_args);
        assert!(output.status.success(), "stderr: {}", stderr(&output));
    }
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 7",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    for watcher in ["watcher-a", "watcher-b"] {
        let messages = screen_contains_until(
            &home,
            &["messages", "--session", watcher],
            "peer exited: peer (code 7)",
        );
        assert!(messages.status.success(), "stderr: {}", stderr(&messages));
        assert_eq!(
            stdout(&messages)
                .lines()
                .filter(|line| line.contains("peer exited: peer (code 7)"))
                .count(),
            1,
            "{}",
            stdout(&messages)
        );
    }
}

#[test]
fn daemon_ended_peer_does_not_trigger_an_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("running"))
    {
        assert!(Instant::now() < deadline, "peer did not become running");
        std::thread::sleep(Duration::from_millis(20));
    }
    let killed = run(&home, &["kill", "peer", "--yes", "--now"]);
    assert!(killed.status.success(), "stderr: {}", stderr(&killed));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn exited_watcher_does_not_receive_a_peer_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sh",
            "-c",
            "exit 0",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("watcher") && line.contains("exited(0)"))
    {
        assert!(Instant::now() < deadline, "watcher did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn team_up_forwards_watch_entries() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    let team = dir.path().join("team.toml");
    std::fs::write(
        &team,
        r#"
[[session]]
name = "watcher"
command = ["sh", "-c", "sleep 30"]
watch = ["peer"]

[[session]]
name = "peer"
command = ["sh", "-c", "exit 5"]
"#,
    )
    .unwrap();
    assert!(run(&home, &["daemon", "start"]).status.success());
    let file = team.to_string_lossy().into_owned();
    let up = run_owned(
        &home,
        &[
            "team".to_owned(),
            "up".to_owned(),
            "--detach".to_owned(),
            "--file".to_owned(),
            file,
        ],
    );
    assert!(up.status.success(), "stderr: {}", stderr(&up));
    let messages = screen_contains_until(
        &home,
        &["messages", "--session", "watcher"],
        "peer exited: peer (code 5)",
    );
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
}

#[test]
fn non_watching_session_stays_silent_when_peer_exits() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let silent = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "silent",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(silent.status.success(), "stderr: {}", stderr(&silent));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "exit 4",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("exited(4)"))
    {
        assert!(Instant::now() < deadline, "peer did not exit");
        std::thread::sleep(Duration::from_millis(20));
    }
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "silent"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn team_down_peer_does_not_trigger_an_exit_message() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "watcher",
            "--harness",
            "generic",
            "--watch",
            "peer",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "peer",
            "--harness",
            "generic",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(10);
    while !stdout(&run(&home, &["list"]))
        .lines()
        .any(|line| line.contains("peer") && line.contains("running"))
    {
        assert!(Instant::now() < deadline, "peer did not become running");
        std::thread::sleep(Duration::from_millis(20));
    }
    let down = run(&home, &["team", "down", "--now", "peer"]);
    assert!(down.status.success(), "stderr: {}", stderr(&down));
    for _ in 0..20 {
        let messages = run(&home, &["messages", "--session", "watcher"]);
        assert!(
            !stdout(&messages).contains("peer exited: peer"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn idle_watcher_receives_heartbeat_for_busy_peer() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; while :; do printf 'peer output\\n'; sleep 1; done",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let messages = screen_contains_until(&home, &["messages", "--session", "arch"], "heartbeat");
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    assert!(
        stdout(&messages).contains("a2amx-daemon@"),
        "{}",
        stdout(&messages)
    );
}

#[test]
fn idle_watcher_skips_heartbeat_for_ready_peer() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    std::thread::sleep(Duration::from_millis(10_500));
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let messages = run(&home, &["messages", "--session", "arch"]);
        assert!(
            !stdout(&messages).contains("heartbeat"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn held_delivery_keeps_at_most_one_heartbeat_open() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--deliver",
            "hold",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let messages = screen_contains_until(&home, &["messages", "--session", "arch"], "heartbeat");
    assert!(messages.status.success(), "stderr: {}", stderr(&messages));
    std::thread::sleep(Duration::from_secs(5));
    let messages = run(&home, &["messages", "--session", "arch"]);
    let count = stdout(&messages)
        .lines()
        .filter(|line| line.contains("heartbeat"))
        .count();
    assert_eq!(count, 1, "{}", stdout(&messages));
}

#[test]
fn new_heartbeat_validates_interval_and_requires_watch() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let missing_watch = run(
        &home,
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--heartbeat",
            "30m",
            "--",
            "sleep",
            "60",
        ],
    );
    assert!(!missing_watch.status.success());
    assert!(
        stderr(&missing_watch).contains("heartbeat needs a non-empty watch"),
        "{}",
        stderr(&missing_watch)
    );
    for value in ["0s", "25h"] {
        let invalid = run(
            &home,
            &[
                "new",
                "--detach",
                "--harness",
                "generic",
                "--heartbeat",
                value,
                "--watch",
                "dev",
                "--",
                "sleep",
                "60",
            ],
        );
        assert!(!invalid.status.success());
        assert!(
            stderr(&invalid).contains("invalid heartbeat"),
            "{value}: {}",
            stderr(&invalid)
        );
    }
}

#[test]
fn not_ready_watcher_does_not_receive_heartbeat() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("state");
    let _cleanup = Cleanup(&home);
    assert!(run(&home, &["daemon", "start"]).status.success());
    let watcher = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "arch",
            "--harness",
            "generic",
            "--watch",
            "dev",
            "--heartbeat",
            "2s",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(watcher.status.success(), "stderr: {}", stderr(&watcher));
    let peer = run(
        &home,
        &[
            "new",
            "--detach",
            "--name",
            "dev",
            "--harness",
            "generic",
            "--",
            "sh",
            "-c",
            "sleep 60",
        ],
    );
    assert!(peer.status.success(), "stderr: {}", stderr(&peer));
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let messages = run(&home, &["messages", "--session", "arch"]);
        assert!(
            !stdout(&messages).contains("heartbeat"),
            "{}",
            stdout(&messages)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
