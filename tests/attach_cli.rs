//! S5 acceptance tests for the real `a2amx` binary under a PTY.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{Request, Response};
use common::pty::PtyHarness;
use common::{eventually, new_agent};

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
    assert!(listed.contains("ID  NAME"), "list output: {listed:?}");
    assert!(
        listed.contains("s1") && listed.contains("running"),
        "list output: {listed:?}"
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"typed\n")?;
    attached.wait_for_text("typed", WAIT)?;
    attached.send(&[2, b'd'])?;
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

    // Ctrl-B twice is a literal 0x02, not a prefix command. The child
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
    literal.send(&[2, 2])?;
    literal.wait_for_text("02", WAIT)?;
    literal.send(&[2, b'd'])?;
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
    attached.send(&[2, b'['])?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;

    // Send PageUp one byte at a time; the parser must retain the split CSI.
    for byte in b"\x1b[5~" {
        attached.send(&[*byte])?;
    }
    attached.wait_for_text("line01", WAIT)?;
    attached.send(b"qafter-scroll-input\n")?;
    attached.wait_for_text("after-scroll-input", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_wheel_scrolls_three_lines_and_q_returns_to_live() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script =
        "i=1; while [ $i -le 100 ]; do printf 'line-%03d\\r\\n' $i; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("line-100", WAIT)?;
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1000h\x1b[?1006h".len())
            .any(|window| window == b"\x1b[?1000h\x1b[?1006h")
    );
    assert!(!attached.screen_text().contains("line-076"));
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("line-076", WAIT)?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    assert!(!attached.screen_text().contains("line-100"));
    attached.send(b"\x1b[<65;10;5M")?;
    attached.wait_for_text("line-100", WAIT)?;
    assert!(attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(b"q")?;
    attached.wait_for_text("s1@", WAIT)?;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_click_reports_are_dropped_without_entering_scroll_mode() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "stty raw -echo; cat -v",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("s1@", WAIT)?;
    attached.send(b"\x1b[<65;10;5M")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(b"\x1b[<0;10;5M\x1b[<0;10;5m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    assert!(!attached.screen_text().contains("^[[<0;10;5M"));
    attached.send(b"x")?;
    attached.wait_for_text("x", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_mouse_reporting_forwards_wheel_reports() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h\\033[?1006h'; printf ready; stty raw -echo; cat -v";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("^[[<64;10;5M", WAIT)?;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_reporting_reset_returns_to_wheel_capture() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h\\033[?1006h'; printf on; stty raw -echo; read trigger; printf '\\033[?1000l\\033[?1006l'; printf off; cat -v";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("on", WAIT)?;
    attached.send(b"x\n")?;
    attached.wait_for_text("off", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    assert!(!attached.screen_text().contains("^[[<64;10;5M"));
    attached.send(b"q")?;
    attached.wait_for_text("s1@", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_does_not_capture_session_already_in_drag_mode() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf '\\033[?1002h'; printf ready; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    assert!(
        !attached
            .raw_output()
            .windows(b"\x1b[?1000h".len())
            .any(|window| window == b"\x1b[?1000h")
    );
    assert!(
        !attached
            .raw_output()
            .windows(b"\x1b[?1006h".len())
            .any(|window| window == b"\x1b[?1006h")
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_wheel_is_dropped_while_session_picker_is_open() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf ready; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"\x1b")?;
    attached.wait_for_text("ready", WAIT)?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(&[2, b'd'])?;
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
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    for header in ["NAME", "HARNESS", "PENDING", "HELD", "QUOTA", "CWD"] {
        attached.wait_for_text(header, WAIT)?;
    }
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;

    // A held controller makes the first switch fail; the picker remains open.
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[B\r")?;
    attached.wait_for_text("attached elsewhere", WAIT)?;
    blocker.send(&[2, b'd'])?;
    blocker.wait_for_text("[detached from s2]", WAIT)?;
    blocker.wait_exit(WAIT)?;
    attached.send(b"\r")?;
    attached.wait_for_text("two", WAIT)?;
    attached.send(&[2, b'd'])?;
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_clips_long_cwd_to_narrow_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let cwd_root = tempfile::tempdir()?;
    let long_cwd = cwd_root
        .path()
        .join("fictional-picker-working-directory")
        .join("tail-of-picker-path");
    std::fs::create_dir_all(&long_cwd)?;

    let mut client = Client::connect(dir.path()).await?;
    let response = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "stty size; trap 'stty size' WINCH; while :; do read line; done".into(),
            ],
            cols: 80,
            rows: 24,
            cwd: Some(long_cwd.to_string_lossy().into_owned()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: None,
            heartbeat: None,
        })
        .await?;
    let Response::Created { session } = response else {
        anyhow::bail!("new session response was not Created");
    };

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", &session]), dir.path())?;
    attached.wait_for_text("23 80", WAIT)?;
    attached.resize(90, 10)?;
    attached.wait_for_text("9 90", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    let screen = attached.screen_text();
    for line in screen.lines() {
        assert!(
            line.chars().count() <= 90,
            "picker screen line exceeded width: {line:?}"
        );
    }
    assert!(
        screen
            .lines()
            .any(|line| line.contains("…-picker-path (current)")),
        "picker did not left-shorten CWD: {screen:?}"
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
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
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
    assert!(listing.starts_with(
        "ID  NAME  HARNESS  STATE    ACTIVITY  ATTACHED  PENDING  HELD  QUOTA  SIZE\n"
    ));
    assert!(
        listing.contains(
            "s1  -     generic  running  working   no        0        -     -      80x24\n"
        )
    );

    let explicit = run_binary(second_dir.path(), &["list"], &environment)?;
    assert_eq!(explicit.status.code(), Some(0));
    let explicit_listing = String::from_utf8(explicit.stdout)?;
    assert!(
        explicit_listing.starts_with(
            "ID  NAME  HARNESS  STATE  ACTIVITY  ATTACHED  PENDING  HELD  QUOTA  SIZE\n"
        )
    );
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
    attached.wait_for_text("23 80", WAIT)?;
    attached.resize(40, 10)?;
    attached.wait_for_text("9 40", WAIT)?;
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
async fn wait_for_file(path: &Path, lines: usize) -> String {
    let path = path.to_owned();
    eventually(move || {
        let path = path.clone();
        async move {
            let content = std::fs::read_to_string(path).ok()?;
            (content.lines().count() == lines).then_some(content)
        }
    })
    .await
}

async fn wait_for_message_state(home: &Path, id: &str, state: &str) {
    let home = home.to_owned();
    let id = id.to_owned();
    let state = state.to_owned();
    eventually(move || {
        let home = home.clone();
        let id = id.clone();
        let state = state.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Status { message } =
                client.request(Request::MessageStatus { id }).await.ok()?
            else {
                return None;
            };
            (message.state == state).then_some(())
        }
    })
    .await;
}

async fn wait_for_held(home: &Path, session: &str, held: bool) {
    let home = home.to_owned();
    let session = session.to_owned();
    eventually(move || {
        let home = home.clone();
        let session = session.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Sessions { sessions } = client.request(Request::List).await.ok()? else {
                return None;
            };
            sessions
                .into_iter()
                .find(|summary| summary.id == session)
                .filter(|summary| summary.held == held)
                .map(|_| ())
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_session_appears_in_the_list_table() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let output = run_binary(dir.path(), &["list"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID  NAME        HARNESS  STATE    ACTIVITY  ATTACHED  PENDING  HELD  QUOTA  SIZE\n"
    ));
    assert!(listing.contains(
        "s1  agent-plan  generic  running  working   no        0        -     -      80x24\n"
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_new_wires_mcp_and_optional_peer_authorization() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let first = tempfile::NamedTempFile::new_in(dir.path())?;
    let first_path = first.path().to_string_lossy().into_owned();
    let first_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &first_path)],
    )?;
    assert_eq!(first_output.status.code(), Some(0));
    let first_args = wait_for_file(first.path(), 8).await;
    let first_args: Vec<&str> = first_args.lines().collect();
    assert_eq!(first_args.len(), 8);
    assert_eq!(first_args[0], "--mcp-config");
    let config: serde_json::Value = serde_json::from_str(first_args[1])?;
    let expected_exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_a2amx"))?
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        config["mcpServers"]["a2amx"]["command"],
        serde_json::Value::String(expected_exe.clone())
    );
    assert_eq!(
        config["mcpServers"]["a2amx"]["args"],
        serde_json::json!(["mcp"])
    );
    assert_eq!(first_args[2], "--allowedTools");
    assert_eq!(
        first_args[3],
        "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session"
    );
    assert_eq!(first_args[4], "--append-system-prompt");
    assert_eq!(first_args[5], a2amx::harness::PEER_AUTHORIZATION_PROMPT);
    assert_eq!(first_args[6], "--settings");
    let settings: serde_json::Value = serde_json::from_str(first_args[7])?;
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        format!("'{expected_exe}' hook")
    );

    let second = tempfile::NamedTempFile::new_in(dir.path())?;
    let second_path = second.path().to_string_lossy().into_owned();
    let second_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--no-authorize-peers",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &second_path)],
    )?;
    assert_eq!(second_output.status.code(), Some(0));
    let second_args = wait_for_file(second.path(), 6).await;
    let second_args: Vec<&str> = second_args.lines().collect();
    assert_eq!(second_args.len(), 6);
    assert_eq!(
        second_args,
        vec![
            "--mcp-config",
            first_args[1],
            "--allowedTools",
            "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session",
            "--settings",
            first_args[7],
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_input_sets_hold_and_prefix_release_clears_it() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "stty raw -echo; cat",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(b"x")?;
    wait_for_held(dir.path(), "s1", true).await;
    let listed = run_binary(dir.path(), &["list", "--details"], &[])?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    assert!(listing.contains(&format!(
        "s1  -     generic  running  busy      yes       0        human_draft  -      80x23  {}  sh -c stty raw -echo; cat\n",
        std::env::current_dir()?.display(),
    )));

    attached.send(&[2, b'r'])?;
    wait_for_held(dir.path(), "s1", false).await;
    let (listing, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(listing.contains("s1  -     generic  running  working   yes       0        -"));

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_cli_prints_submitted_and_undeliverable_rows() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let recipient_output = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-review",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; stty raw -echo; cat >/dev/null",
        ],
    )?;
    assert_eq!(recipient_output.1, 0);
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Parser issue".into(),
            message: "I found the regression.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    wait_for_message_state(dir.path(), "m_1", "submitted").await;

    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Second".into(),
            message: "Another issue.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_2".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Trailing ".into(),
            message: "Subjects print as stored.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_3".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    assert_eq!(
        admin
            .request(Request::Kill {
                session: "s2".into(),
                now: true,
            })
            .await?,
        Response::Ok
    );
    wait_for_message_state(dir.path(), "m_2", "undeliverable").await;
    wait_for_message_state(dir.path(), "m_3", "undeliverable").await;

    let output = run_binary(dir.path(), &["messages"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID   FROM               TO                   STATE          DETAIL            SUBJECT\n"
    ));
    assert!(listing.contains("m_1  agent-plan@host-a  agent-review@host-a  submitted      -                 Parser issue\n"));
    assert!(listing.contains(
        "m_2  agent-plan@host-a  agent-review@host-a  undeliverable  recipient_exited  Second\n"
    ));
    assert!(listing.contains(
        "m_3  agent-plan@host-a  agent-review@host-a  undeliverable  recipient_exited  Trailing \n"
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_is_silent_on_success_and_reports_errors() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _, _) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    assert_eq!(
        sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Pending".into(),
                message: "Cancel me.".into(),
            })
            .await?,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: Some("deliver_hold".into()),
            recipient_quota: None
        }
    );

    let cancelled = run_binary(dir.path(), &["cancel", "m_1"], &[])?;
    assert_eq!(cancelled.status.code(), Some(0));
    assert!(cancelled.stdout.is_empty());
    assert!(cancelled.stderr.is_empty());

    let missing = run_binary(dir.path(), &["cancel", "m_99"], &[])?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr)?,
        "a2amx: unknown message m_99\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_line_toggles_and_shows_the_session_address() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            script,
        ],
    )?;
    assert_eq!(code, 0);
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("23 80", WAIT)?;
    attached.wait_for_text("agent-plan@", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(40, 10)?;
    attached.wait_for_text("9 40", WAIT)?;
    let after_toggle = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    if let Err(error) = attached.wait_for_text("10 40", WAIT) {
        eprintln!(
            "post-toggle outer PTY bytes: {:?}",
            String::from_utf8_lossy(&attached.raw_output()[after_toggle..]),
        );
        return Err(error);
    }
    assert!(
        !attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(50, 12)?;
    attached.wait_for_text("12 50", WAIT)?;
    attached.send(&[2, b's'])?;
    attached.wait_for_text("11 50", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test]
async fn output_during_backpressured_snapshot_reaches_live_attachment_without_redraw()
-> anyhow::Result<()> {
    use a2amx::wire::{ClientFrame, ServerFrame, encode_frame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    tokio::time::timeout(WAIT, async {
        let (dir, daemon) = common::start_daemon().await;
        let gate_path = dir.path().join("output-gate");
        let listener = tokio::net::UnixListener::bind(&gate_path)?;
        let script = r#"
import os, socket, sys, tty
tty.setraw(0)
gate = socket.socket(socket.AF_UNIX)
gate.connect(sys.argv[1])
def synced_write(data):
    sys.stdout.buffer.write(data + b'\x1b[6n')
    sys.stdout.buffer.flush()
    while os.read(0, 1) != b'R':
        pass
pair = b'\x1b[38;2;1;2;3mX\x1b[38;2;4;5;6mX'
synced_write(pair * 131072)
gate.sendall(b'ready')
gate.recv(1)
synced_write(b'\x1b[1;1H\x1b[0mOUTPUT-AFTER-SNAPSHOT')
gate.sendall(b'printed')
gate.recv(1)
"#;
        let mut control = Client::connect(dir.path()).await?;
        let Response::Created { session } = control
            .request(Request::NewSession {
                argv: vec![
                    "python3".into(),
                    "-c".into(),
                    script.into(),
                    gate_path.to_string_lossy().into_owned(),
                ],
                cols: 512,
                rows: 512,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("render-regression".into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                heartbeat: None,
            })
            .await?
        else {
            panic!("session creation");
        };
        let (mut gate, _) = listener.accept().await?;
        let mut ready = [0; 5];
        gate.read_exact(&mut ready).await?;
        assert_eq!(&ready, b"ready");

        // Alternating RGB cells make the snapshot exceed Linux's TCP send buffer.
        // A small receive window leaves the daemon sending it while the child writes.
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_recv_buffer_size(16384)?;
        let mut attachment = socket.connect(daemon.addrs()[0]).await?;
        let token = std::fs::read_to_string(dir.path().join("admin.token"))?;
        for (request, expected) in [
            (Request::Hello { token }, Response::Ok),
            (
                Request::Attach {
                    session,
                    force: false,
                    cols: 512,
                    rows: 512,
                    status: false,
                },
                Response::Attached,
            ),
        ] {
            attachment
                .write_all(&encode_frame(&serde_json::to_vec(&request)?)?)
                .await?;
            let length = attachment.read_u32().await?;
            let mut payload = vec![0; length as usize];
            attachment.read_exact(&mut payload).await?;
            assert_eq!(serde_json::from_slice::<Response>(&payload)?, expected);
        }
        gate.write_all(b"p").await?;
        let mut printed = [0; 7];
        gate.read_exact(&mut printed).await?;
        assert_eq!(&printed, b"printed");
        // Let the already-woken delivery observer consume the shared notification.
        assert!(matches!(
            control.request(Request::List).await?,
            Response::Sessions { .. }
        ));
        tokio::task::yield_now().await;
        tokio::time::timeout(WAIT, async {
            loop {
                let length = attachment.read_u32().await?;
                let mut payload = vec![0; length as usize];
                attachment.read_exact(&mut payload).await?;
                let ServerFrame::Data(data) = ServerFrame::decode(&payload)? else {
                    panic!("child remains live");
                };
                if data
                    .windows(b"OUTPUT-AFTER-SNAPSHOT".len())
                    .any(|window| window == b"OUTPUT-AFTER-SNAPSHOT")
                {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        attachment
            .write_all(&encode_frame(&ClientFrame::Detach.encode())?)
            .await?;
        gate.write_all(b"x").await?;
        daemon.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}
