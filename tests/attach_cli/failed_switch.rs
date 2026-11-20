use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_enter_on_a_session_that_refuses_keeps_the_picker_open_with_the_error()
-> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    daemon.set_refuse_attach(Some("s2"));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;

    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"j")?;
    attached.send(b"\r")?;
    attached.wait_for_text("session refused", WAIT)?;
    assert!(
        attached.screen_text().contains("a2amx sessions:"),
        "picker closed on refused attach: {:?}",
        attached.screen_text()
    );

    attached.send(b"\x1b")?;
    attached.wait_for_text("ALPHA", WAIT)?;
    assert_eq!(
        daemon.attach_viewers(),
        vec![("s1".to_owned(), false), ("s1".to_owned(), false)]
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_next_to_a_session_that_refuses_restores_silently_and_keeps_input_on_the_current_one()
-> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    daemon.set_refuse_attach(Some("s2"));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("alpha@host-a", WAIT)?;

    let frames_before_switch = daemon.frames().len();
    attached.send(&[2, b'n'])?;
    eventually(|| {
        let viewers = daemon.attach_viewers();
        async move { (viewers.len() >= 2).then_some(()) }
    })
    .await;
    attached.wait_for_text("ALPHA", WAIT)?;

    let frames_after_refusal = daemon.frames().len().max(frames_before_switch);
    attached.send(b"hello")?;
    eventually(|| {
        let frames = daemon.frames();
        let bytes: Vec<u8> = frames[frames_after_refusal..]
            .iter()
            .filter_map(|frame| match frame {
                ClientFrame::Input(payload) => Some(payload.as_slice()),
                _ => None,
            })
            .flatten()
            .copied()
            .collect();
        async move { (bytes == b"hello").then_some(()) }
    })
    .await;

    let screen = attached.screen_text();
    assert!(
        !screen.contains("session refused"),
        "unexpected error on screen: {screen:?}"
    );
    assert!(
        !screen.contains("BETA"),
        "unexpected BETA on screen: {screen:?}"
    );
    assert_eq!(
        daemon.attach_viewers(),
        vec![("s1".to_owned(), false), ("s1".to_owned(), false)]
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    Ok(())
}
