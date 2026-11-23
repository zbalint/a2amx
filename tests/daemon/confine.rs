use super::*;

fn confine_new_session_request() -> Request {
    Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec!["true".into()],
            cols: 40,
            rows: 5,
            cwd: Some(
                std::env::current_dir()
                    .expect("current directory")
                    .to_string_lossy()
                    .into_owned(),
            ),
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
        }),
    }
}
fn confined_reset_request() -> Request {
    Request::Confine {
        confine: a2amx::confine::ConfineRequest {
            auto: true,
            reasons: vec![a2amx::wire::GrantReason {
                grant: "cache".into(),
                reason: "always added by auto".into(),
            }],
            ..a2amx::confine::ConfineRequest::default()
        },
        request: Box::new(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '\\033[?2004h'; cat >/dev/null".into(),
            ],
            cols: 40,
            rows: 5,
            cwd: Some(
                std::env::current_dir()
                    .expect("current directory")
                    .to_string_lossy()
                    .into_owned(),
            ),
            env: vec![],
            reset: vec!["/clear".into()],
            control_from: vec!["sender".into()],
            watch: vec![],
            name: Some("target".into()),
            harness: a2amx::harness::Harness::Generic,
            deliver: Some(a2amx::harness::Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        }),
    }
}

#[tokio::test]
async fn nested_confine_is_rejected_without_closing_daemon_connection() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let nested = Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(Request::Confine {
            confine: a2amx::confine::ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec!["true".into()],
                cols: 40,
                rows: 5,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: None,
                harness: a2amx::harness::Harness::Generic,
                deliver: None,
                team: None,
                role: None,
                remote: None,
            }),
        }),
    };
    let response = client.request(nested).await.unwrap();
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("nested confinement")
    ));
    let id = create(&mut client, &["true"], vec![]).await;
    assert!(!id.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_tokens_cannot_confine_and_admin_confine_rejects_invalid_targets() {
    use a2amx::harness::{Deliver, Harness};

    let dir = tempfile::tempdir().unwrap();
    let Ok(real_bwrap) = a2amx::confine::resolve_bwrap(None) else {
        return;
    };
    let bwrap_dir = dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).unwrap();
    symlink(real_bwrap, bwrap_dir.join("bwrap")).unwrap();
    let daemon = Daemon::start(DaemonConfig {
        root: dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await
    .unwrap();
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let before = list(&mut admin).await;
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(
            &encode_frame(&serde_json::to_vec(&confine_new_session_request()).unwrap()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "not permitted for a session token".into()
        }
    );
    assert_eq!(list(&mut admin).await, before);

    let created = admin.request(confine_new_session_request()).await.unwrap();
    let Response::Created { .. } = created else {
        panic!("admin confined creation failed: {created:?}");
    };
    let Response::Created { .. } = admin.request(confined_reset_request()).await.unwrap() else {
        panic!("admin confined reset session creation failed");
    };
    let Response::Agents {
        agents: before_agents,
        ..
    } = admin.request(Request::ListAgents).await.unwrap()
    else {
        panic!("admin list_agents failed");
    };
    let sender_before = before_agents
        .iter()
        .find(|agent| agent.address == "sender@host-a")
        .expect("sender in list_agents");
    let target_before = before_agents
        .iter()
        .find(|agent| agent.address == "target@host-a")
        .expect("target in list_agents");
    assert_eq!(sender_before.confine, None);
    let target_confine = target_before
        .confine
        .clone()
        .expect("target confinement in list_agents");
    socket
        .write_all(
            &encode_frame(
                &serde_json::to_vec(&Request::Reset {
                    session: "target@host-a".into(),
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    let reset = raw_response(&mut socket).await;
    assert!(
        matches!(reset, Response::Reset { steps: 1 }),
        "unexpected reset response: {reset:?}"
    );
    let Response::Agents {
        agents: after_agents,
        ..
    } = admin.request(Request::ListAgents).await.unwrap()
    else {
        panic!("admin list_agents after reset failed");
    };
    let target_after = after_agents
        .iter()
        .find(|agent| agent.address == "target@host-a")
        .expect("target in list_agents after reset");
    assert_eq!(target_after.confine, Some(target_confine));
    let confine = target_after.confine.as_ref().expect("reset confine");
    assert!(confine.auto);
    assert_eq!(
        confine.reasons,
        vec![a2amx::wire::GrantReason {
            grant: "cache".into(),
            reason: "always added by auto".into(),
        }]
    );
    let nested = Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(confine_new_session_request()),
    };
    assert_eq!(
        admin.request(nested).await.unwrap(),
        Response::Error {
            message: "nested confinement requests are not allowed".into()
        }
    );
    assert_eq!(
        admin
            .request(Request::Confine {
                confine: a2amx::confine::ConfineRequest::default(),
                request: Box::new(Request::List),
            })
            .await
            .unwrap(),
        Response::Error {
            message: "expected new_session".into()
        }
    );
    daemon.shutdown().await.unwrap();
}
