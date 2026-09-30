//! S5 acceptance tests for the real `a2amx` binary under a PTY.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::pty::PtyHarness;

const WAIT: Duration = Duration::from_secs(10);

fn cli_args(home: &std::path::Path, command: &[&str]) -> Vec<String> {
    let mut args = vec!["--home".to_owned(), home.to_string_lossy().into_owned()];
    args.extend(command.iter().map(|arg| (*arg).to_owned()));
    args
}

fn run_cli(home: &std::path::Path, command: &[&str]) -> anyhow::Result<(String, i32)> {
    let args = cli_args(home, command);
    let mut pty = PtyHarness::spawn(&args, home)?;
    let code = pty.wait_exit(WAIT)?;
    Ok((pty.screen_text(), code))
}

fn run_binary(
    home: &Path,
    command: &[&str],
    environment: &[(&str, &str)],
) -> anyhow::Result<Output> {
    let mut process = Command::new(env!("CARGO_BIN_EXE_a2amx"));
    process
        .args(cli_args(home, command))
        .envs(environment.iter().copied())
        .stdin(Stdio::null());
    Ok(process.output()?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_list_attach_detach_and_literal_prefix_use_real_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;

    let (created, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf ready; cat"],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let (listed, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(listed.contains("ID  STATE"), "list output: {listed:?}");
    assert!(
        listed.contains("s1") && listed.contains("running"),
        "list output: {listed:?}"
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"typed\n")?;
    attached.wait_for_text("typed", WAIT)?;
    attached.send(&[0, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );

    let (after_detach, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(after_detach.contains("s1") && after_detach.contains("running"));

    // Ctrl-Space twice is a literal NUL, not a prefix command. The child
    // prints the received byte in hex and stays alive so the screen is observable.
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "stty raw -echo; dd if=/dev/tty bs=1 count=1 2>/dev/null | od -An -t x1; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s2"), "new output: {created:?}");
    let mut literal = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    literal.send(&[0, 0])?;
    literal.wait_for_text("00", WAIT)?;
    literal.send(&[0, b'd'])?;
    literal.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(literal.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scroll_mode_parses_split_sequences_and_returns_to_live_bottom() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script =
        "i=1; while [ $i -le 30 ]; do printf 'line%02d\\r\\n' $i; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("line30", WAIT)?;
    attached.send(&[0, b'['])?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;

    // Send PageUp one byte at a time; the parser must retain the split CSI.
    for byte in b"\x1b[5~" {
        attached.send(&[*byte])?;
    }
    attached.wait_for_text("line01", WAIT)?;
    attached.send(b"qafter-scroll-input\n")?;
    attached.wait_for_text("after-scroll-input", WAIT)?;
    attached.send(&[0, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_switches_sessions_and_exit_status_is_reported() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf '%s%s' o ne; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf '%s%s' t wo; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut blocker = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    blocker.wait_for_text("two", WAIT)?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("one", WAIT)?;

    // Entering and selecting the current row cancels and redraws the live session.
    attached.send(&[0, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;

    // A held controller makes the first switch fail; the picker remains open.
    attached.send(&[0, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[B\r")?;
    attached.wait_for_text("attached elsewhere", WAIT)?;
    blocker.send(&[0, b'd'])?;
    blocker.wait_for_text("[detached from s2]", WAIT)?;
    blocker.wait_exit(WAIT)?;
    attached.send(b"\r")?;
    attached.wait_for_text("two", WAIT)?;
    attached.send(&[0, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    let (_, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf bye; exit 7"],
    )?;
    assert_eq!(code, 0);
    let mut exited = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s3"]), dir.path())?;
    exited.wait_for_text("[session s3 exited with code 7]", WAIT)?;
    assert_eq!(exited.wait_exit(WAIT)?, 0);
    assert!(
        exited
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );
    Ok(())
}

#[test]
fn attach_without_terminal_is_rejected_with_status_one() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["attach", "s1"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "a2amx: attach needs a terminal on stdin\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_precedence_and_nonterminal_new_use_default_size() -> anyhow::Result<()> {
    let (first_dir, _first_daemon) = common::start_daemon().await;
    let second_dir = tempfile::tempdir()?;
    let _second_daemon = a2amx::daemon::Daemon::start(a2amx::daemon::DaemonConfig {
        state_dir: second_dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse()?],
    })
    .await?;
    let first_home = first_dir.path().to_string_lossy().into_owned();
    let second_home = second_dir.path().to_string_lossy().into_owned();
    let environment = [
        ("A2AMX_HOME", first_home.as_str()),
        ("XDG_STATE_HOME", second_home.as_str()),
    ];

    let created = run_binary(
        first_dir.path(),
        &["new", "--detach", "--", "sh", "-c", "sleep 30"],
        &environment,
    )?;
    assert_eq!(created.status.code(), Some(0));
    assert_eq!(String::from_utf8(created.stdout)?, "s1\n");

    let listed = run_binary(first_dir.path(), &["list"], &environment)?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    assert!(listing.starts_with("ID  STATE    ATTACHED  SIZE   COMMAND\n"));
    assert!(listing.contains("s1  running  no        80x24  sh -c sleep 30\n"));

    let explicit = run_binary(second_dir.path(), &["list"], &environment)?;
    assert_eq!(explicit.status.code(), Some(0));
    let explicit_listing = String::from_utf8(explicit.stdout)?;
    assert!(explicit_listing.starts_with("ID  STATE  ATTACHED  SIZE  COMMAND\n"));
    assert!(!explicit_listing.contains("s1"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_prefix_and_sigwinch_resize_reach_attached_session() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn_with_env(
        &cli_args(dir.path(), &["attach", "s1"]),
        dir.path(),
        &[("A2AMX_PREFIX", "C-a")],
    )?;
    attached.wait_for_text("24 80", WAIT)?;
    attached.resize(40, 10)?;
    attached.wait_for_text("10 40", WAIT)?;
    attached.send(&[1, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_failure_restores_the_outer_terminal_before_returning_an_error() -> anyhow::Result<()>
{
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "cat"])?;
    assert_eq!(code, 0);
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    let script = "stty -g > \"$BEFORE\"; \"$A2AMX_BIN\" --home \"$A2AMX_HOME\" attach s1 > /dev/full; status=$?; stty -g > \"$AFTER\"; printf 'status=%s\\n' \"$status\"";
    let mut terminal = PtyHarness::spawn_program(
        "sh",
        &["-c".into(), script.into()],
        dir.path(),
        &[
            ("A2AMX_BIN", env!("CARGO_BIN_EXE_a2amx")),
            ("BEFORE", before.to_str().unwrap()),
            ("AFTER", after.to_str().unwrap()),
        ],
    )?;
    terminal.wait_for_text("status=1", WAIT)?;
    assert_eq!(terminal.wait_exit(WAIT)?, 0);
    assert_eq!(std::fs::read(before)?, std::fs::read(after)?);
    assert!(terminal.screen_text().contains("a2amx:"));
    Ok(())
}

#[test]
fn invalid_prefix_reports_the_named_reason_as_an_a2amx_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["--prefix", "C-m", "list"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.starts_with("a2amx:"));
    assert!(stderr.contains("CR"));
    Ok(())
}
