use super::*;

#[tokio::test]
async fn attach_top_rows_acknowledges_and_resets_origin() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("run/admin.token")).unwrap();
    for top_rows in [Some(9), Some(8), Some(1), Some(0), None] {
        let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
        socket
            .write_all(
                &encode_frame(
                    &serde_json::to_vec(&Request::Hello {
                        token: token.clone(),
                    })
                    .unwrap(),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(raw_response(&mut socket).await, Response::Ok);
        let mut request =
            serde_json::json!({"type":"attach","session":id,"force":true,"cols":40,"rows":5});
        if let Some(top_rows) = top_rows {
            request["top_rows"] = top_rows.into();
        }
        socket
            .write_all(&encode_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
            .await
            .unwrap();
        let response = raw_response(&mut socket).await;
        if top_rows == Some(9) {
            assert!(matches!(response, Response::Error { .. }));
            continue;
        }
        if let Some(rows @ 1..=8) = top_rows {
            assert_eq!(response, Response::AttachedTop { top_rows: rows });
        } else {
            assert_eq!(response, Response::Attached);
        }
        let length = socket.read_u32().await.unwrap();
        let mut frame = vec![0; length as usize];
        socket.read_exact(&mut frame).await.unwrap();
        let ServerFrame::Data(bytes) = ServerFrame::decode(&frame).unwrap() else {
            panic!("expected data")
        };
        if top_rows == Some(1) {
            assert!(bytes.starts_with(b"\x1b[2;1H\x1b[J"));
        } else if top_rows == Some(8) {
            assert!(bytes.starts_with(b"\x1b[9;1H\x1b[J"));
        } else {
            assert!(bytes.starts_with(b"\x1b[2J\x1b[H"));
        }
    }
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn resize_and_redraw_preserve_top_rows_origin() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("run/admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    let request = Request::Attach {
        session: id,
        force: false,
        cols: 40,
        rows: 5,
        status: false,
        top_rows: 1,
        viewer: false,
    };
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::AttachedTop { top_rows: 1 }
    );
    let length = socket.read_u32().await.unwrap();
    let mut initial = vec![0; length as usize];
    socket.read_exact(&mut initial).await.unwrap();
    assert!(matches!(
        ServerFrame::decode(&initial).unwrap(),
        ServerFrame::Data(_)
    ));
    for request in [
        ClientFrame::Resize { cols: 42, rows: 6 },
        ClientFrame::Redraw,
    ] {
        socket
            .write_all(&encode_frame(&request.encode()).unwrap())
            .await
            .unwrap();
        let length = socket.read_u32().await.unwrap();
        let mut frame = vec![0; length as usize];
        socket.read_exact(&mut frame).await.unwrap();
        let ServerFrame::Data(bytes) = ServerFrame::decode(&frame).unwrap() else {
            panic!("expected data after {request:?}");
        };
        assert!(
            bytes.starts_with(b"\x1b[2;1H\x1b[J"),
            "after {request:?}: {bytes:?}"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn input_resize_takeover_detach_and_disconnect_preserve_session() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    assert!(matches!(next(&mut first).await, ServerFrame::Data(_)));
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    first
        .send(ClientFrame::Input(b"consumer-input\n".to_vec()))
        .await
        .unwrap();
    wait_text(&mut first, &mut emu, "consumer-input").await;
    let error = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        format!("session {id} is attached elsewhere (use --force to take over)")
    );
    let mut second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 60, 8)
        .await
        .unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match first.recv().await.unwrap() {
                Some(ServerFrame::Data(_)) => continue,
                Some(ServerFrame::Detached(reason)) => break reason,
                frame => panic!("expected take-over notice, got {frame:?}"),
            }
        }
    })
    .await
    .expect("take-over notice within deadline");
    assert_eq!(reason, "taken over by another attachment");
    assert!(first.recv().await.unwrap().is_none());
    assert!(matches!(next(&mut second).await, ServerFrame::Data(_)));
    second
        .send(ClientFrame::Resize { cols: 30, rows: 6 })
        .await
        .unwrap();
    let mut resized = Emulator::new(Size { cols: 30, rows: 6 });
    match next(&mut second).await {
        ServerFrame::Data(bytes) => {
            resized.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&resized).contains("consumer-input"));
    eventually_sessions(&mut client, |s| {
        s[0].cols == 30 && s[0].rows == 6 && s[0].attached
    })
    .await;
    second.send(ClientFrame::Detach).await.unwrap();
    eventually_sessions(&mut client, |s| !s[0].attached).await;
    let third = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    drop(third);
    eventually_sessions(&mut client, |s| !s[0].attached && s[0].exit_code.is_none()).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn detached_queries_environment_and_final_output_are_preserved() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let out = dir.path().join("reply");
    let id = create(&mut client, &["sh", "-c", "stty raw -echo; printf '\\033[c'; head -c 5 > \"$OUT\"; printf '%s:%s:final' \"$TERM\" \"$COLORTERM\"; exit 7"],
        vec![("OUT".into(), out.to_str().unwrap().into()), ("TERM".into(), "wrong".into()), ("COLORTERM".into(), "wrong".into())]).await;
    let exited = eventually_sessions(&mut client, |s| s[0].exit_code == Some(7)).await;
    assert_eq!(exited[0].id, id);
    assert_eq!(std::fs::read(out).unwrap(), b"\x1b[?6c");
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    match next(&mut attachment).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        frame => panic!("snapshot first, not {frame:?}"),
    }
    assert!(text(&emu).contains("xterm-256color:truecolor:final"));
    assert_eq!(next(&mut attachment).await, ServerFrame::Exit(7));
    assert!(attachment.recv().await.unwrap().is_none());
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(list(&mut client).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_exit_sends_last_screen_before_status_and_retains_it() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &["sh", "-c", "read line; printf 'last-screen'; exit 23"],
        vec![],
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    attachment
        .send(ClientFrame::Input(b"go\n".to_vec()))
        .await
        .unwrap();
    loop {
        match next(&mut attachment).await {
            ServerFrame::Data(bytes) => {
                emu.feed(&bytes);
            }
            ServerFrame::Exit(code) => {
                assert_eq!(code, 23);
                assert!(text(&emu).contains("last-screen"));
                break;
            }
            frame => panic!("{frame:?}"),
        }
    }
    assert_eq!(list(&mut client).await[0].exit_code, Some(23));
    let mut late = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    match next(&mut late).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("last-screen"));
    assert_eq!(next(&mut late).await, ServerFrame::Exit(23));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn detached_output_snapshot_and_scroll_history_are_model_renders() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "printf hi; read line; i=1; while [ \"$i\" -le 30 ]; do printf '\\r\\nline%02d' \"$i\"; i=$((i + 1)); done; read line"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut first, &mut emu, "hi").await;
    first.send(ClientFrame::Detach).await.unwrap();
    eventually_sessions(&mut client, |s| !s[0].attached).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    match next(&mut attached).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("hi"));
    attached
        .send(ClientFrame::Input(b"go\n".to_vec()))
        .await
        .unwrap();
    wait_text(&mut attached, &mut emu, "line30").await;
    attached
        .send(ClientFrame::Scroll(Scroll::Top))
        .await
        .unwrap();
    loop {
        match next(&mut attached).await {
            ServerFrame::Data(bytes) => {
                emu.feed(&bytes);
                if text(&emu).contains("line01") {
                    break;
                }
            }
            f => panic!("{f:?}"),
        }
    }
    assert!(!emu.screen().cursor.visible);
    attached
        .send(ClientFrame::Scroll(Scroll::Bottom))
        .await
        .unwrap();
    wait_text(&mut attached, &mut emu, "line30").await;
    attached.send(ClientFrame::Redraw).await.unwrap();
    assert!(matches!(next(&mut attached).await, ServerFrame::Data(_)));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn output_drains_when_the_attached_client_never_reads() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &[
            "sh",
            "-c",
            "read line; head -c 20000000 /dev/zero | tr '\\0' x",
        ],
        vec![],
    )
    .await;
    let mut stalled = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    stalled
        .send(ClientFrame::Input(b"start\n".to_vec()))
        .await
        .unwrap();
    let sessions = eventually_sessions(&mut client, |s| s[0].exit_code == Some(0)).await;
    assert_eq!(sessions[0].exit_code, Some(0));
    drop(stalled);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn input_backpressure_preserves_bytes_and_keeps_control_requests_responsive() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let out = dir.path().join("input");
    let start = dir.path().join("start");
    let id = create(&mut client,
        &["sh", "-c", "stty raw -echo; printf ready; while [ ! -e \"$START\" ]; do :; done; head -c 8388608 > \"$OUT\""],
        vec![("OUT".into(), out.to_str().unwrap().into()), ("START".into(), start.to_str().unwrap().into())]).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut attached, &mut emulator, "ready").await;
    let sending = tokio::spawn(async move {
        attached
            .send(ClientFrame::Input(vec![0xab; 8 * 1024 * 1024]))
            .await
            .unwrap();
        attached
    });
    let sessions = tokio::time::timeout(Duration::from_secs(2), list(&mut client))
        .await
        .unwrap();
    assert_eq!(sessions[0].exit_code, None);
    std::fs::write(start, b"go").unwrap();
    let mut attached = tokio::time::timeout(Duration::from_secs(10), sending)
        .await
        .unwrap()
        .unwrap();
    loop {
        match next(&mut attached).await {
            ServerFrame::Exit(code) => {
                assert_eq!(code, 0);
                break;
            }
            ServerFrame::Data(_) => {}
            frame => panic!("{frame:?}"),
        }
    }
    let bytes = std::fs::read(out).unwrap();
    assert_eq!(bytes.len(), 8 * 1024 * 1024);
    assert!(bytes.iter().all(|byte| *byte == 0xab));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn repeated_force_takeovers_close_each_previous_controller_before_registration() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    next(&mut first).await;
    let mut second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 60, 8)
        .await
        .unwrap();
    assert_eq!(
        next(&mut first).await,
        ServerFrame::Detached("taken over by another attachment".into())
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(10), first.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    next(&mut second).await;
    let mut third = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 30, 6)
        .await
        .unwrap();
    assert_eq!(
        next(&mut second).await,
        ServerFrame::Detached("taken over by another attachment".into())
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(10), second.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    next(&mut third).await;
    third
        .send(ClientFrame::Input(b"last-controller\n".to_vec()))
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size { cols: 30, rows: 6 });
    wait_text(&mut third, &mut emulator, "last-controller").await;
    let sessions = list(&mut client).await;
    assert!(sessions[0].attached);
    assert_eq!((sessions[0].cols, sessions[0].rows), (30, 6));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn large_final_snapshot_is_split_at_the_data_limit_and_precedes_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let Response::Created { session: id } = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "head -c 131072 /dev/zero | tr '\\0' x".into(),
            ],
            cols: 512,
            rows: 256,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    else {
        panic!("created session");
    };
    eventually_sessions(&mut client, |sessions| sessions[0].exit_code == Some(0)).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 512, 256)
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size {
        cols: 512,
        rows: 256,
    });
    let mut total = 0;
    loop {
        match next(&mut attached).await {
            ServerFrame::Data(bytes) => {
                assert!(bytes.len() <= 65536);
                total += bytes.len();
                emulator.feed(&bytes);
            }
            ServerFrame::Exit(code) => {
                assert_eq!(code, 0);
                break;
            }
            frame => panic!("{frame:?}"),
        }
    }
    assert!(total > 65536);
    assert!(emulator.screen().cells.iter().all(|cell| cell.ch == 'x'));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn takeover_waits_for_received_input_to_enqueue_under_backpressure() {
    let (dir, daemon) = common::start_daemon().await;
    let release = dir.path().join("release-input");
    let mut control = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut control,
        &[
            "sh",
            "-c",
            "stty raw -echo; printf ready; while [ ! -f \"$RELEASE\" ]; do :; done; cat >/dev/null",
        ],
        vec![("RELEASE".into(), release.to_str().unwrap().into())],
    )
    .await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut screen = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut first, &mut screen, "ready").await;
    let mut input = Box::pin(first.send(ClientFrame::Input(vec![b'x'; 20_000_000])));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), input.as_mut())
            .await
            .is_err(),
        "input should be backpressured while the child does not read"
    );
    let force_client = Client::connect(dir.path()).await.unwrap();
    let mut forced = Box::pin(force_client.attach(&id, true, 60, 8));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), forced.as_mut())
            .await
            .is_err(),
        "takeover must not discard a received input frame waiting for queue space"
    );
    tokio::task::spawn_blocking(move || std::fs::write(release, b"resume"))
        .await
        .unwrap()
        .unwrap();
    let mut second = tokio::time::timeout(Duration::from_secs(10), forced.as_mut())
        .await
        .expect("takeover completes once the child drains input")
        .unwrap();
    drop(input);
    assert!(matches!(next(&mut second).await, ServerFrame::Data(_)));
    assert!(list(&mut control).await[0].attached);
    daemon.shutdown().await.unwrap();
}
