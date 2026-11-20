use super::*;

#[tokio::test]
async fn attach_viewer_sends_no_resize_frame() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let hello = fake_recv_frame(&mut stream)
            .await?
            .context("missing hello")?;
        anyhow::ensure!(
            matches!(
                serde_json::from_slice::<Request>(&hello)?,
                Request::Hello { .. }
            ),
            "expected hello request"
        );
        fake_send_frame(&mut stream, &serde_json::to_vec(&Response::Ok)?).await?;
        let attach = fake_recv_frame(&mut stream)
            .await?
            .context("missing attach")?;
        let Request::Attach { viewer, .. } = serde_json::from_slice(&attach)? else {
            anyhow::bail!("expected attach request");
        };
        anyhow::ensure!(viewer, "attach_viewer did not set viewer mode");
        fake_send_frame(&mut stream, &serde_json::to_vec(&Response::Attached)?).await?;
        let next =
            tokio::time::timeout(Duration::from_millis(250), fake_recv_frame(&mut stream)).await;
        Ok::<_, anyhow::Error>(matches!(next, Ok(Ok(Some(_)))))
    });

    let attachment = Client::connect_addr(address, "fake-token")
        .await?
        .attach_viewer("s1", false, 80, 24, 0)
        .await?;
    let resize_or_other_frame = server.await??;
    assert!(
        !resize_or_other_frame,
        "attach_viewer wrote an unexpected frame after Attach"
    );
    drop(attachment);
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_toggle_blocks_input_shows_overlay_and_resizes() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;

    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    let raw_before_resize = attached.raw_output().len();
    attached.resize(90, 24)?;
    eventually(|| {
        let frames = daemon.frames();
        let changed = attached.raw_output().len() > raw_before_resize;
        let has_resize = frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { cols: 90, rows: 20 }));
        async move { (changed && has_resize).then_some(()) }
    })
    .await;
    assert_eq!(
        daemon
            .frames()
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
            .count(),
        1,
        "viewer resize did not send exactly one daemon frame: {:?}",
        daemon.frames()
    );
    attached.send(b"\x1b[<0;10")?;
    attached.send(b";5M")?;

    let mode_frame_count = daemon
        .frames()
        .iter()
        .filter(|frame| matches!(frame, ClientFrame::Mode { .. }))
        .count();
    attached.send(b"x")?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(b"\x1b[200~paste\x1b[201~")?;
    attached.send(&[2, b'r'])?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(b"\x1b[<0;10")?;
    attached.send(b";5M")?;
    eventually(|| {
        let absent = !attached.screen_text().contains("VIEW MODE");
        async move { absent.then_some(()) }
    })
    .await;
    assert!(
        daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Redraw)),
        "overlay expiry did not request a repaint: {:?}",
        daemon.frames()
    );
    attached.send(b"\x1b[O")?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b")?;
    eventually(|| {
        let screen = attached.screen_text();
        async move {
            (screen.contains("ALPHA") && screen.contains("VIEW") && !screen.contains("VIEW MODE"))
                .then_some(())
        }
    })
    .await;
    let frames = daemon.frames();
    assert_eq!(
        frames
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Mode { .. }))
            .count(),
        mode_frame_count
    );
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Release)),
        "viewer forwarded a release frame: {frames:?}"
    );
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer forwarded an input frame: {frames:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
async fn assert_viewer_after_stays_interactive(after: &str) -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", after, "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(b"x")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"x"))
                .then_some(())
        }
    })
    .await;
    let observe_until = Instant::now() + Duration::from_millis(2500);
    eventually(|| {
        let screen = attached.screen_text();
        let no_mode = !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }));
        async move {
            (Instant::now() >= observe_until && screen.contains("ALPHA") && no_mode).then_some(())
        }
    })
    .await;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_idle_switches_once_and_toggle_redraws_after_overlay() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "1", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"x")?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        let ready = frames
            .iter()
            .position(|frame| matches!(frame, ClientFrame::Mode { viewer: false }))
            .is_some_and(|left| {
                matches!(frames.get(left + 1), Some(ClientFrame::Resize { .. }))
                    && matches!(frames.get(left + 2), Some(ClientFrame::Redraw))
            });
        async move { ready.then_some(()) }
    })
    .await;
    eventually(|| {
        let absent = !attached.screen_text().contains("VIEW MODE");
        async move { absent.then_some(()) }
    })
    .await;

    let frames = daemon.frames();
    let entered = frames
        .iter()
        .position(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
        .expect("automatic viewer mode frame");
    let left = frames
        .iter()
        .enumerate()
        .find_map(|(index, frame)| {
            (index > entered && matches!(frame, ClientFrame::Mode { viewer: false }))
                .then_some(index)
        })
        .expect("toggle back mode frame");
    assert!(left > entered);
    assert!(matches!(
        frames.get(left + 1),
        Some(ClientFrame::Resize { .. })
    ));
    assert!(matches!(frames.get(left + 2), Some(ClientFrame::Redraw)));
    assert!(
        frames[entered + 1..left]
            .iter()
            .all(|frame| !matches!(frame, ClientFrame::Mode { .. }))
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_idle_switch_ignores_daemon_data() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    daemon.schedule_data_after(Duration::from_millis(2500));
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "3", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("scheduled daemon data", WAIT)?;
    tokio::time::timeout(
        Duration::from_secs(2),
        eventually(|| {
            let frames = daemon.frames();
            async move {
                frames
                    .iter()
                    .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                    .then_some(())
            }
        }),
    )
    .await?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_focus_report_forwards_without_resetting_idle_clock() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "3", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(b"x")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"x"))
                .then_some(())
        }
    })
    .await;
    let focus_deadline = Instant::now() + Duration::from_millis(2500);
    eventually(|| async move { (Instant::now() >= focus_deadline).then_some(()) }).await;
    attached.send(b"\x1b[I")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"\x1b[I"))
                .then_some(())
        }
    })
    .await;
    tokio::time::timeout(
        Duration::from_secs(2),
        eventually(|| {
            let frames = daemon.frames();
            async move {
                frames
                    .iter()
                    .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                    .then_some(())
            }
        }),
    )
    .await?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_focus_report_does_not_show_overlay() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    assert!(!attached.screen_text().contains("VIEW MODE"));
    let raw_before = attached.raw_output().len();
    attached.send(b"\x1b[I")?;
    let showed_overlay = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            let has_overlay = {
                let raw = attached.raw_output();
                raw.len() > raw_before
                    && raw[raw_before..]
                        .windows(b"VIEW MODE".len())
                        .any(|window| window == b"VIEW MODE")
            };
            if has_overlay {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    assert!(!showed_overlay, "focus report showed the viewer overlay");
    assert!(!attached.screen_text().contains("VIEW MODE"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_after_zero_and_maximum_never_switch() -> anyhow::Result<()> {
    assert_viewer_after_stays_interactive("0").await?;
    assert_viewer_after_stays_interactive("18446744073709551615").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_status_toggle_same_top_rows_sends_resize_and_redraws() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(Some(0)).await?;
    daemon.hide_session_teams();
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    let frames_before_hide = daemon.frames().len();

    let raw_before_hide = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let raw_changed = attached.raw_output().len() > raw_before_hide;
        let frames = daemon.frames();
        let has_resize = frames[frames_before_hide..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 24 }));
        async move { (raw_changed && has_resize).then_some(()) }
    })
    .await;

    let frames_before_show = daemon.frames().len();
    let raw_before_show = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let has_marker = {
            let raw = attached.raw_output();
            raw.len() > raw_before_show
                && raw
                    .windows(b"\x1b[1;30;43m VIEW \x1b[0m".len())
                    .any(|window| window == b"\x1b[1;30;43m VIEW \x1b[0m")
        };
        let frames = daemon.frames();
        let has_resize = frames[frames_before_show..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 22 }));
        async move { (has_marker && has_resize).then_some(()) }
    })
    .await;
    assert_eq!(daemon.attach_viewers().len(), 1);
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_picker_switch_and_status_toggle_keep_viewer_attach() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b")?;
    eventually(|| {
        let screen = attached.screen_text();
        async move { (screen.contains("ALPHA") && screen.contains("VIEW")).then_some(()) }
    })
    .await;

    attached.send(&[2, b'n'])?;
    attached.wait_for_text("BETA", WAIT)?;
    eventually(|| {
        let attaches = daemon.attach_viewers();
        async move { (attaches.len() >= 2).then_some(()) }
    })
    .await;
    attached.send(&[2, b's'])?;
    eventually(|| {
        let attaches = daemon.attach_viewers();
        async move { (attaches.len() >= 3).then_some(()) }
    })
    .await;

    let attaches = daemon.attach_viewers();
    assert!(!attaches[0].1);
    assert!(
        attaches.iter().skip(1).all(|(_, viewer)| *viewer),
        "{attaches:?}"
    );
    let frames = daemon.frames();
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { .. })),
        "viewer switch or status toggle resized: {frames:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_switch_sends_corrective_resize_when_team_rows_refused() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(Some(0)).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    let frames_before_switch = daemon.frames().len();
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[B\r")?;
    attached.wait_for_text("BETA", WAIT)?;
    eventually(|| {
        let frames = daemon.frames();
        let has_resize = frames[frames_before_switch..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { cols: 80, rows: 22 }));
        async move { has_resize.then_some(()) }
    })
    .await;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

async fn assert_viewer_mouse_is_not_forwarded(top_rows: Option<u16>) -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(top_rows).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    let resize_count = daemon
        .frames()
        .iter()
        .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
        .count();
    if top_rows == Some(0) {
        attached.send(&[2, b's'])?;
        eventually(|| {
            let count = daemon
                .frames()
                .iter()
                .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
                .count();
            async move { (count == resize_count + 1).then_some(()) }
        })
        .await;
    }
    let expected_resize_count = resize_count + usize::from(top_rows == Some(0));

    let raw_before_mouse = attached.raw_output().to_vec();
    attached.send(b"\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<35;10;5M")?;
    attached.send(b"\x1b[<64;10;5M")?;
    eventually(|| {
        let changed = attached.raw_output().len() > raw_before_mouse.len();
        async move { changed.then_some(()) }
    })
    .await;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    let raw_after_mouse = attached.raw_output();
    let suffix = &raw_after_mouse[raw_before_mouse.len()..];
    assert!(
        !suffix
            .windows(b"VIEW MODE".len())
            .any(|window| window == b"VIEW MODE"),
        "complete mouse report showed the viewer overlay in raw output"
    );
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    assert!(
        !attached.screen_text().contains("VIEW MODE"),
        "mouse report showed the viewer overlay"
    );
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer forwarded a mouse frame: {:?}",
        daemon.frames()
    );
    assert_eq!(
        daemon
            .frames()
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
            .count(),
        expected_resize_count,
        "viewer status toggle or mouse path resized the daemon: {:?}",
        daemon.frames()
    );
    attached.send(b"q")?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_team_click_acts_on_release_and_keeps_viewer_mode() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    let attaches = daemon.attach_viewers();
    assert_eq!(attaches.get(1).map(|(_, viewer)| *viewer), Some(true));
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer team click forwarded input: {:?}",
        daemon.frames()
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_overlay_uses_the_configured_prefix_name() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--prefix", "C-space", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[0, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"x")?;
    attached.wait_for_text("Ctrl-Space", WAIT)?;
    assert!(!attached.screen_text().contains("Ctrl-B"));
    attached.send(&[0, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_mouse_reports_are_dropped_with_and_without_team_rows() -> anyhow::Result<()> {
    assert_viewer_mouse_is_not_forwarded(None).await?;
    assert_viewer_mouse_is_not_forwarded(Some(0)).await?;
    Ok(())
}
