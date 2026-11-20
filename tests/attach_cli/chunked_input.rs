use super::*;

async fn wait_for_file_bytes(path: &Path, expected: &[u8]) {
    eventually(|| {
        let actual = std::fs::read(path).ok();
        async move { (actual.as_deref() == Some(expected)).then_some(()) }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn command_and_following_keys_in_one_write_reach_the_picker() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;

    let (created1, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf ALPHA; exec cat",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created1.contains("s1"), "new output: {created1:?}");

    let (created2, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf BETA; exec cat"],
    )?;
    assert_eq!(code, 0);
    assert!(created2.contains("s2"), "new output: {created2:?}");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;

    attached.send(&[2, b'w', b'j', b'\r'])?;
    attached.wait_for_text("BETA", WAIT)?;

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scroll_command_and_quit_in_one_write_return_to_live() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let capture = dir.path().join("scroll_quit.bin");
    let script = format!(
        "printf READY; stty raw -echo; exec cat > {}",
        capture.display()
    );

    let (created, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", &script])?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("READY", WAIT)?;

    attached.send(&[2, b'[', b'q'])?;
    attached.send(b"Z\n")?;
    wait_for_file_bytes(&capture, b"Z\n").await;
    assert_eq!(std::fs::read(&capture)?, b"Z\n");

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn detach_drops_the_rest_of_the_write() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let capture = dir.path().join("detach_drop.bin");
    let script = format!(
        "printf READY; stty raw -echo; exec cat > {}",
        capture.display()
    );

    let (created, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", &script])?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("READY", WAIT)?;

    attached.send(b"A")?;
    wait_for_file_bytes(&capture, b"A").await;

    attached.send(&[2, b'd', b'x', b'y', b'z'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(std::fs::read(&capture)?, b"A");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn literal_prefix_then_command_letter_in_one_write_is_forwarded() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let capture = dir.path().join("literal_prefix.bin");
    let script = format!(
        "printf READY; stty raw -echo; exec cat > {}",
        capture.display()
    );

    let (created, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", &script])?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("READY", WAIT)?;

    attached.send(&[2, 2, b'd'])?;
    wait_for_file_bytes(&capture, &[2, b'd']).await;

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bracketed_paste_with_the_prefix_byte_is_forwarded_verbatim() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let capture = dir.path().join("paste_prefix.bin");
    let script = format!(
        "printf READY; stty raw -echo; exec cat > {}",
        capture.display()
    );

    let (created, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", &script])?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("READY", WAIT)?;

    let paste = b"\x1b[200~a\x02db\x1b[201~";
    attached.send(paste)?;
    wait_for_file_bytes(&capture, paste).await;

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}
