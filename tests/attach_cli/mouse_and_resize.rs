use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_row_press_on_another_member_released_on_the_grid_does_not_switch()
-> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("demo-beta", WAIT)?;

    let screen = attached.screen_text();
    let team_row = screen
        .lines()
        .next()
        .context("missing team row on screen")?;
    let beta_offset = team_row
        .find("demo-beta")
        .context("demo-beta not found on team row")?;
    let beta_col = team_row[..beta_offset].chars().count() + 1;

    let press = format!("\x1b[<0;{beta_col};1M");
    let release = format!("\x1b[<0;{beta_col};5m");
    attached.send(press.as_bytes())?;
    attached.send(release.as_bytes())?;

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert_eq!(daemon.attach_viewers(), vec![("s1".to_owned(), false)]);
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "unexpected Input frame: {:?}",
        daemon.frames()
    );
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn grid_press_released_on_the_team_row_is_forwarded_shifted() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("alpha@host-a", WAIT)?;

    attached.send(b"\x1b[<0;10;5M\x1b[<0;10;1m")?;
    eventually(|| {
        let inputs: Vec<Vec<u8>> = daemon
            .frames()
            .into_iter()
            .filter_map(|frame| match frame {
                ClientFrame::Input(bytes) => Some(bytes),
                _ => None,
            })
            .collect();
        async move { (inputs.len() >= 2).then_some(inputs) }
    })
    .await;

    let inputs: Vec<Vec<u8>> = daemon
        .frames()
        .into_iter()
        .filter_map(|frame| match frame {
            ClientFrame::Input(bytes) => Some(bytes),
            _ => None,
        })
        .collect();
    assert_eq!(
        inputs,
        vec![b"\x1b[<0;10;3M".to_vec(), b"\x1b[<0;10;1m".to_vec()]
    );
    assert_eq!(daemon.attach_viewers(), vec![("s1".to_owned(), false)]);

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resize_in_live_scroll_and_picker_modes_sends_the_pty_rows() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    daemon.schedule_data_after(Duration::from_millis(2500));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("alpha@host-a", WAIT)?;

    // Live case (width 60, rows 12 -> pty rows 8)
    let raw_before_live_resize = attached.raw_output().len();
    attached.resize(60, 12)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Resize { cols: 60, rows: 8 }))
                .then_some(())
        }
    })
    .await;
    let raw_after_live = &attached.raw_output()[raw_before_live_resize..];
    assert!(
        !raw_after_live
            .windows(b"alpha@host-a".len())
            .any(|w| w == b"alpha@host-a"),
        "live resize unexpectedly repainted status bar immediately"
    );
    attached.wait_for_text("scheduled daemon data", WAIT)?;
    attached.wait_for_text("alpha@host-a", WAIT)?;

    // Scroll case (width 61, rows 12 -> pty rows 8)
    attached.send(&[2, b'['])?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.resize(61, 12)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Resize { cols: 61, rows: 8 }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"q")?;
    attached.wait_for_text("ALPHA", WAIT)?;

    // Picker case (width 62, rows 12 -> pty rows 8)
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.resize(62, 12)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Resize { cols: 62, rows: 8 }))
                .then_some(())
        }
    })
    .await;
    let full_header =
        "a2amx sessions: Enter attach/fold, Esc or q cancel, / filter, Space fold, j/k move";
    attached.wait_for_raw_contains(&full_header.as_bytes()[..62], WAIT)?;
    assert!(
        !attached.screen_text().contains(full_header),
        "picker header was not clipped to 62 columns: {:?}",
        attached.screen_text()
    );

    attached.send(b"\x1b")?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_toggle_when_team_rows_are_refused_reattaches_on_every_show() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(Some(0)).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 22 }))
                .then_some(())
        }
    })
    .await;

    let frames_before_hide = daemon.frames().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames[frames_before_hide..]
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 24 }))
                .then_some(())
        }
    })
    .await;
    assert_eq!(daemon.attach_viewers().len(), 1);

    let frames_before_show = daemon.frames().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let viewers = daemon.attach_viewers();
        let frames = daemon.frames();
        let has_resize = frames[frames_before_show..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 22 }));
        async move { (viewers.len() == 2 && has_resize).then_some(()) }
    })
    .await;

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    Ok(())
}
