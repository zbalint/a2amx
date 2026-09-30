// Shared test helpers are not themselves annotated with #[test].
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::os::unix::fs::PermissionsExt;

use a2amx::daemon::{Daemon, DaemonConfig};
use std::time::Duration;

use a2amx::client::{Attachment, Client};
use a2amx::emulator::{Emulator, Scroll, Size};
use a2amx::wire::{ClientFrame, Request, Response, ServerFrame, SessionSummary, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn create(client: &mut Client, argv: &[&str], env: Vec<(String, String)>) -> String {
    match client
        .request(Request::NewSession {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            cols: 40,
            rows: 5,
            cwd: None,
            env,
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

async fn list(client: &mut Client) -> Vec<SessionSummary> {
    match client.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("unexpected list response: {response:?}"),
    }
}

async fn eventually_sessions(
    client: &mut Client,
    predicate: impl Fn(&[SessionSummary]) -> bool,
) -> Vec<SessionSummary> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let sessions = list(client).await;
            if predicate(&sessions) {
                return sessions;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("session condition within deadline")
}

fn text(emulator: &Emulator) -> String {
    emulator.screen().cells.iter().map(|cell| cell.ch).collect()
}

async fn next(attachment: &mut Attachment) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(10), attachment.recv())
        .await
        .expect("stream deadline")
        .unwrap()
        .expect("stream frame")
}

async fn wait_text(attachment: &mut Attachment, emulator: &mut Emulator, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !text(emulator).contains(expected) {
            match attachment.recv().await.unwrap().expect("data before close") {
                ServerFrame::Data(bytes) => {
                    emulator.feed(&bytes);
                }
                frame => panic!("expected text before {frame:?}"),
            }
        }
    })
    .await
    .expect("screen text within deadline");
}

async fn raw_response(stream: &mut TcpStream) -> Response {
    let size = stream.read_u32().await.unwrap() as usize;
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn state_is_owner_only_and_exclusively_locked() {
    let (dir, daemon) = common::start_daemon().await;
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in ["admin.token", "addr"] {
        assert_eq!(
            std::fs::metadata(dir.path().join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let second = Daemon::start(DaemonConfig {
        state_dir: dir.path().to_owned(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
    })
    .await;
    assert_eq!(
        second.err().unwrap().to_string(),
        "another a2amx daemon is running for this state dir"
    );
    daemon.shutdown().await.unwrap();
    assert!(!dir.path().join("addr").exists());
}

#[tokio::test]
async fn every_connection_requires_a_valid_hello() {
    let (dir, daemon) = common::start_daemon().await;
    let addr = daemon.addrs()[0];
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::List).unwrap()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        raw_response(&mut socket).await,
        Response::Error { .. }
    ));
    assert_eq!(
        socket.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    let error = Client::connect_addr(addr, "wrong-token")
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("admin.token"));
    assert!(!error.contains("wrong-token"));
    let mut silent = TcpStream::connect(addr).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(7), raw_response(&mut silent))
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(
        silent.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    let mut valid = Client::connect(dir.path()).await.unwrap();
    assert_eq!(list(&mut valid).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_spawns_and_zero_sizes_leave_no_sessions() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for (argv, cols, rows) in [
        (vec![], 40, 5),
        (vec!["/no-such-a2amx-program".into()], 40, 5),
        (vec!["cat".into()], 0, 5),
        (vec!["cat".into()], 40, 0),
    ] {
        assert!(matches!(
            client
                .request(Request::NewSession {
                    argv,
                    cols,
                    rows,
                    cwd: None,
                    env: vec![],
                    name: None,
                    harness: a2amx::harness::Harness::Generic,
                    deliver: None,
                })
                .await
                .unwrap(),
            Response::Error { .. }
        ));
        assert_eq!(list(&mut client).await, vec![]);
    }
    assert!(matches!(
        client
            .request(Request::Kill {
                session: "unknown".into()
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    let id = create(&mut client, &["cat"], vec![]).await;
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach(&id, false, 0, 5)
            .await
            .is_err()
    );
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach("unknown", false, 40, 5)
            .await
            .is_err()
    );
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
        client.request(Request::Kill { session: id }).await.unwrap(),
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
async fn kill_removes_running_session_and_shutdown_disconnects_attachments() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "sleep 30"], vec![]).await;
    assert_eq!(
        client.request(Request::Kill { session: id }).await.unwrap(),
        Response::Ok
    );
    assert_eq!(list(&mut client).await, vec![]);
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    next(&mut attachment).await;
    daemon.shutdown().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), attachment.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(!dir.path().join("addr").exists());
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
async fn missing_or_refused_daemon_has_the_exact_start_hint() {
    let temp = tempfile::tempdir().unwrap();
    let expected = "cannot reach the a2amx daemon: start it with `a2amx daemon`";
    assert_eq!(
        Client::connect(temp.path())
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert_eq!(
        Client::connect_addr(address, "unused")
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
}

#[tokio::test]
async fn partial_bind_failure_releases_addresses_and_lock_and_parents_are_private() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("parent/state");
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    assert!(
        Daemon::start(DaemonConfig {
            state_dir: home.clone(),
            listen: vec!["127.0.0.1:0".parse().unwrap(), address],
            host_name: None,
            limits: a2amx::messaging::Limits::default(),
        })
        .await
        .is_err()
    );
    assert!(!home.join("addr").exists());
    assert!(!home.join("admin.token").exists());
    for path in [home.clone(), temp.path().join("parent")] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let daemon = Daemon::start(DaemonConfig {
        state_dir: home,
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
    })
    .await
    .unwrap();
    assert_ne!(daemon.addrs()[0].port(), 0);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn ids_are_numerically_ordered_and_not_reused_after_kill() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for _ in 0..11 {
        create(&mut client, &["cat"], vec![]).await;
    }
    let ids = list(&mut client)
        .await
        .into_iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11"
        ]
    );
    assert_eq!(
        client
            .request(Request::Kill {
                session: "s1".into()
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(create(&mut client, &["cat"], vec![]).await, "s12");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn kill_escalates_ignored_hangup_and_reports_the_signal_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &["sh", "-c", "trap '' HUP; printf ready; exec sleep 30"],
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
    wait_text(&mut attachment, &mut emu, "ready").await;
    assert_eq!(
        client.request(Request::Kill { session: id }).await.unwrap(),
        Response::Ok
    );
    loop {
        match next(&mut attachment).await {
            ServerFrame::Exit(code) => {
                assert_eq!(code, 137);
                break;
            }
            ServerFrame::Data(_) => {}
            frame => panic!("{frame:?}"),
        }
    }
    assert_eq!(list(&mut client).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn cwd_environment_and_argument_boundaries_reach_the_child() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let result = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s|%s|%s|%s|%s' \"$VALUE\" \"$1\" \"$TERM\" \"$COLORTERM\" \"$PWD\""
                    .into(),
                "argv0".into(),
                "literal ; $text".into(),
            ],
            cols: 100,
            rows: 5,
            cwd: Some(dir.path().to_str().unwrap().into()),
            env: vec![
                ("VALUE".into(), "supplied".into()),
                ("TERM".into(), "wrong".into()),
                ("COLORTERM".into(), "wrong".into()),
            ],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
        })
        .await
        .unwrap();
    let Response::Created { session: id } = result else {
        panic!("{result:?}");
    };
    eventually_sessions(&mut client, |s| s[0].exit_code == Some(0)).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 100, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 100, rows: 5 });
    match next(&mut attachment).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("supplied|literal ; $text|xterm-256color|truecolor|"));
    assert!(text(&emu).contains(dir.path().to_str().unwrap()));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_stream_frame_disconnects_without_killing_the_session() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(
            &encode_frame(
                &serde_json::to_vec(&Request::Attach {
                    session: id,
                    force: false,
                    cols: 40,
                    rows: 5,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Attached);
    let length = socket.read_u32().await.unwrap();
    let mut initial = vec![0; length as usize];
    socket.read_exact(&mut initial).await.unwrap();
    socket
        .write_all(&encode_frame(&[0xff]).unwrap())
        .await
        .unwrap();
    assert_eq!(
        socket.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    eventually_sessions(&mut client, |s| !s[0].attached && s[0].exit_code.is_none()).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_attach_leaves_the_control_connection_usable() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let _attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    assert!(matches!(
        client
            .request(Request::Attach {
                session: id,
                force: false,
                cols: 40,
                rows: 5
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(list(&mut client).await[0].argv, ["cat"]);
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

#[test]
fn newly_created_parents_are_private_even_with_a_restrictive_umask() {
    use std::os::unix::process::CommandExt;

    if let Some(path) = std::env::var_os("A2AMX_TEST_PRIVATE_STATE") {
        let path = std::path::PathBuf::from(path);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let daemon = runtime
            .block_on(Daemon::start(DaemonConfig {
                state_dir: path.clone(),
                listen: vec!["127.0.0.1:0".parse().unwrap()],
                host_name: None,
                limits: a2amx::messaging::Limits::default(),
            }))
            .unwrap();
        for directory in [path.clone(), path.parent().unwrap().to_owned()] {
            assert_eq!(
                std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for file in ["admin.token", "addr"] {
            assert_eq!(
                std::fs::metadata(path.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        runtime.block_on(daemon.shutdown()).unwrap();
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "newly_created_parents_are_private_even_with_a_restrictive_umask",
        ])
        .env("A2AMX_TEST_PRIVATE_STATE", root.path().join("parent/state"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // SAFETY: after fork the child invokes only the non-allocating umask syscall
    // before exec; the parallel parent tests' process-global mask is never changed.
    unsafe {
        command.pre_exec(|| {
            rustix::process::umask(rustix::fs::Mode::all());
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let result = receiver.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
    let output = result
        .expect("isolated daemon check within deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
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
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
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
