use super::*;

#[tokio::test]
async fn serve_authentication_errors_preserve_connection_lifecycle() {
    let (dir, daemon) = common::start_daemon().await;
    for body in [
        br#"{"type":"list"}"#.as_slice(),
        br#"{"type":"hello","token":"wrong-token"}"#.as_slice(),
        b"invalid",
    ] {
        let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
        socket
            .write_all(&encode_frame(body).unwrap())
            .await
            .unwrap();
        assert_eq!(
            raw_response(&mut socket).await,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read_u8())
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert_eq!(
        admin
            .request(Request::Hello {
                token: "ignored".into()
            })
            .await
            .unwrap(),
        Response::Error {
            message: "already authenticated".into()
        }
    );
    assert_eq!(
        admin.request(Request::Version).await.unwrap(),
        Response::Version {
            version: a2amx::VERSION.into(),
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_control_validation_messages_and_attachment_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    for (request, expected) in [
        (
            Request::ListMessages {
                session: None,
                state: Some("bad-state".into()),
            },
            "unknown message state",
        ),
        (
            Request::CancelMessage { id: "abc".into() },
            "unknown message abc",
        ),
        (
            Request::Screen {
                session: "s999".into(),
            },
            "unknown session s999",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 40,
                rows: 5,
                status: false,
                top_rows: 0,
                viewer: false,
            },
            "unknown session s999",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 0,
                rows: 5,
                status: false,
                top_rows: 0,
                viewer: false,
            },
            "attachment dimensions must be nonzero",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 40,
                rows: 0,
                status: false,
                top_rows: 0,
                viewer: false,
            },
            "attachment dimensions must be nonzero",
        ),
    ] {
        assert_eq!(
            admin.request(request).await.unwrap(),
            Response::Error {
                message: expected.into()
            }
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_cancelled_message_reports_full_state_error() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    common::new_agent(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(addr, &token).await.unwrap();
    let Response::Accepted { id, .. } = sender
        .request(Request::SendMessage {
            to: "recipient@host-a".into(),
            subject: "Cancel".into(),
            message: "body".into(),
        })
        .await
        .unwrap()
    else {
        panic!("message not accepted");
    };
    assert_eq!(id, "m_1");
    assert_eq!(
        admin
            .request(Request::CancelMessage { id: id.clone() })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(
        admin.request(Request::CancelMessage { id }).await.unwrap(),
        Response::Error {
            message: "message m_1 is cancelled and cannot be cancelled".into()
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_session_token_parse_precedes_permission_check() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(&encode_frame(b"invalid").unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "invalid control request".into()
        }
    );
    socket
        .write_all(&encode_frame(br#"{"type":"list"}"#).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "not permitted for a session token".into()
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_exited_and_removed_sessions_close_before_parsing_requests() {
    for removed in [false, true] {
        for body in [
            b"invalid".as_slice(),
            br#"{"type":"list_agents"}"#.as_slice(),
        ] {
            let (dir, daemon) = common::start_daemon().await;
            let mut admin = Client::connect(dir.path()).await.unwrap();
            let credentials = dir.path().join("credentials");
            let exit = dir.path().join("exit");
            let id = create(&mut admin, &["sh", "-c",
                "printf '%s' \"$A2AMX_TOKEN\" > \"$OUT\"; while [ ! -f \"$EXIT\" ]; do sleep 0.01; done"],
                vec![("OUT".into(), credentials.to_string_lossy().into_owned()),
                     ("EXIT".into(), exit.to_string_lossy().into_owned())]).await;
            let token = common::eventually(|| async {
                let token = std::fs::read_to_string(&credentials).ok()?;
                (token.len() == 64).then_some(token)
            })
            .await;
            let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
            socket
                .write_all(
                    &encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(raw_response(&mut socket).await, Response::Ok);
            if removed {
                assert_eq!(
                    admin
                        .request(Request::Kill {
                            session: id.clone(),
                            now: true
                        })
                        .await
                        .unwrap(),
                    Response::Ok
                );
                assert!(list(&mut admin).await.is_empty());
            } else {
                std::fs::write(&exit, b"exit").unwrap();
                eventually_sessions(&mut admin, |sessions| {
                    sessions
                        .iter()
                        .any(|session| session.id == id && session.exit_code == Some(0))
                })
                .await;
            }
            socket
                .write_all(&encode_frame(body).unwrap())
                .await
                .unwrap();
            assert_eq!(
                raw_response(&mut socket).await,
                Response::Error {
                    message: "session has exited".into()
                }
            );
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), socket.read_u8())
                    .await
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::UnexpectedEof
            );
            daemon.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn admin_version_request_reports_the_build_version() {
    let (dir, daemon) = common::start_daemon().await;
    let token = std::fs::read_to_string(dir.path().join("run/admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);

    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Version).unwrap()).unwrap())
        .await
        .unwrap();
    match raw_response(&mut socket).await {
        Response::Version { version } => assert!(!version.is_empty()),
        response => panic!("unexpected version response: {response:?}"),
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_control_request_is_recoverable_on_an_authenticated_socket() {
    let (dir, daemon) = common::start_daemon().await;
    let token = std::fs::read_to_string(dir.path().join("run/admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);

    socket
        .write_all(&encode_frame(br#"{"type":"no_such_request"}"#).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "invalid control request".into(),
        }
    );

    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::List).unwrap()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        raw_response(&mut socket).await,
        Response::Sessions { .. }
    ));
    daemon.shutdown().await.unwrap();
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
async fn malformed_stream_frame_disconnects_without_killing_the_session() {
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
    socket
        .write_all(
            &encode_frame(
                &serde_json::to_vec(&Request::Attach {
                    session: id,
                    force: false,
                    cols: 40,
                    rows: 5,
                    status: false,
                    top_rows: 0,
                    viewer: false,
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
                rows: 5,
                status: false,
                top_rows: 0,
                viewer: false,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(list(&mut client).await[0].argv, ["cat"]);
    daemon.shutdown().await.unwrap();
}
