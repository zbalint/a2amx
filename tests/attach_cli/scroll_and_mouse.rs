use super::*;

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

#[test]
fn raw_wait_observes_late_bytes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = [
        "-c".to_owned(),
        "printf first; sleep 1; printf second; sleep 30".to_owned(),
    ];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    pty.wait_for_text("first", WAIT)?;
    pty.wait_for_raw_contains(b"second", WAIT)?;
    Ok(())
}

#[test]
fn raw_wait_times_out_with_diagnostics() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = ["-c".to_owned(), "sleep 30".to_owned()];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    let error = pty
        .wait_for_raw_contains(b"ne\x01ver", Duration::from_millis(300))
        .expect_err("missing raw output should time out");
    let message = error.to_string();
    assert!(message.contains("timed out"), "error: {message}");
    assert!(message.contains("ne\\x01ver"), "error: {message}");
    assert!(message.contains("raw output:"), "error: {message}");
    Ok(())
}

#[test]
fn raw_wait_accepts_empty_needle() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = ["-c".to_owned(), "printf first; sleep 30".to_owned()];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    pty.wait_for_text("first", WAIT)?;
    pty.wait_for_raw_contains(b"", Duration::from_millis(300))?;
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
    attached.wait_for_raw_contains(b"\x1b[?1000h\x1b[?1006h", WAIT)?;
    attached.expect_screen_lacks("line-076")?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("line-077", WAIT)?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.expect_screen_lacks("line-100")?;
    attached.send(b"\x1b[<65;10;5M")?;
    attached.wait_for_text("line-100", WAIT)?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.expect_screen_lacks("[scroll: q to exit]")?;
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
    attached.wait_for_text("@host-a", WAIT)?;
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
    attached.wait_for_text("@host-a", WAIT)?;
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
    attached.expect_raw_lacks(b"\x1b[?1000h")?;
    attached.expect_raw_lacks(b"\x1b[?1006h")?;
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
async fn typed_filter_is_not_polluted_by_a_wheel_report_with_session_mouse_on() -> anyhow::Result<()>
{
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h'; printf '\\033[?1006h'; printf ready; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"/")?;
    attached.send(b"\x1b[<64;10;5M")?;
    // The marker proves the report bytes were consumed before the absence check.
    attached.send(b"zq")?;
    attached.wait_for_text("zq", WAIT)?;
    attached.expect_screen_lacks("4;10;5M")?;
    attached.send(&[0x1b])?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    attached.send(&[0x1b])?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wheel_scrolls_in_scroll_mode_with_session_mouse_on() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h'; printf '\\033[?1006h'; i=1; while [ $i -le 100 ]; do printf 'line-%03d\\r\\n' $i; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("line-100", WAIT)?;
    attached.send(&[2, b'['])?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.expect_screen_lacks("line-077")?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("line-077", WAIT)?;
    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.expect_screen_lacks("[scroll: q to exit]")?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}
