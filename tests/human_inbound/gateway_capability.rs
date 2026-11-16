use super::*;

fn gateway_other_requests() -> Vec<Request> {
    vec![
        Request::Hello {
            token: "not-a-second-token".into(),
        },
        Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(new_session_request(None, &["true"])),
        },
        new_session_request(None, &["true"]),
        Request::List,
        Request::Version,
        Request::Attach {
            session: "s1".into(),
            force: false,
            cols: 40,
            rows: 10,
            status: false,
            top_rows: 0,
            viewer: false,
        },
        Request::Kill {
            session: "s1".into(),
            now: false,
        },
        Request::Reset {
            session: "s1".into(),
        },
        Request::Screen {
            session: "s1".into(),
        },
        Request::Explain {
            session: "s1".into(),
        },
        Request::Shutdown { now: false },
        Request::SendMessage {
            to: "agent@host-a".into(),
            subject: "subject".into(),
            message: "message".into(),
        },
        Request::ListAgents,
        Request::MessageStatus { id: "m_1".into() },
        Request::ReportPrompt {
            prompt: "prompt".into(),
        },
        Request::ReportQuota {
            five_hour: None,
            weekly: None,
        },
        Request::ListMessages {
            session: None,
            state: None,
        },
        Request::CancelMessage { id: "m_1".into() },
        Request::BridgeAttach,
        Request::IssueLeaseCode {
            team: "gateway-team".into(),
        },
    ]
}

#[tokio::test]
async fn gateway_can_only_send_human_inbound_and_keeps_connection_usable() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "confined-target",
        Some(RemoteMode::Both),
        Some(team_scope("gateway-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "gateway-team").await;
    for request in gateway_other_requests() {
        assert_eq!(
            error_message(gateway.request(request).await.unwrap()),
            "not permitted for this token"
        );
    }

    assert_eq!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound { message: None }
    );
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: "m_999".into(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "confined-target@host-a".into(),
                body: "connection remains usable".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn admin_and_session_tokens_cannot_speak_as_the_human() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert_eq!(
        error_message(
            admin
                .request(Request::HumanInbound {
                    to: "missing@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for this token"
    );
    assert_eq!(
        error_message(
            admin
                .request(Request::RedeemLeaseCode {
                    code: "0123456789ab".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for this token"
    );
    for request in [
        Request::GatewayAgents,
        Request::HumanOutboundNext,
        Request::HumanOutboundDone {
            id: "m_1".into(),
            sent: true,
        },
    ] {
        assert_eq!(
            error_message(admin.request(request).await.unwrap()),
            "not permitted for this token"
        );
    }
    let (session, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-session"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut session_client = Client::connect_addr(address, &token).await.unwrap();
    assert_eq!(
        error_message(
            session_client
                .request(Request::HumanInbound {
                    to: format!("{session}@host-a"),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    assert_eq!(
        error_message(
            session_client
                .request(Request::IssueLeaseCode {
                    team: "agent-team".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    assert_eq!(
        error_message(
            session_client
                .request(Request::RedeemLeaseCode {
                    code: "0123456789ab".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    for request in [
        Request::GatewayAgents,
        Request::HumanOutboundNext,
        Request::HumanOutboundDone {
            id: "m_1".into(),
            sent: true,
        },
    ] {
        assert_eq!(
            error_message(session_client.request(request).await.unwrap()),
            "not permitted for a session token"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn human_session_name_is_reserved() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let response = admin
        .request(new_session_request(Some("human:owner"), &["sleep", "30"]))
        .await
        .unwrap();
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("invalid name")
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_token_and_link_files_are_owner_only_and_follow_daemon_lifecycle() {
    let (dir, daemon) = common::start_daemon().await;
    let gateway_token = std::fs::read_to_string(dir.path().join("run/gateway.token")).unwrap();
    let link_dir = dir.path().join("run/link");
    assert_eq!(
        std::fs::metadata(&link_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let link_token = link_dir.join("gateway.token");
    assert_eq!(
        std::fs::metadata(&link_token).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&link_token).unwrap(), gateway_token);
    let addr = std::fs::read_to_string(dir.path().join("run/addr")).unwrap();
    assert_eq!(
        std::fs::read_to_string(link_dir.join("addr")).unwrap(),
        addr
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("run/gateway.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    daemon.shutdown().await.unwrap();
    assert!(!dir.path().join("run/addr").exists());
    assert!(dir.path().join("run/gateway.token").exists());
    assert!(link_token.exists());
}

#[tokio::test]
async fn bind_failure_removes_both_capability_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("parent/state");
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let result = Daemon::start(DaemonConfig {
        root: home.clone(),
        listen: vec!["127.0.0.1:0".parse().unwrap(), address],
        host_name: None,
        limits: Limits::default(),
        bwrap_path: Some(home.join("bwrap-path").into_os_string()),
        env_path: None,
    })
    .await;
    assert!(result.is_err());
    assert!(!home.join("run/admin.token").exists());
    assert!(!home.join("run/gateway.token").exists());
    assert!(!home.join("run/link/gateway.token").exists());
    assert!(!home.join("addr").exists());
}

#[test]
fn human_inbound_wire_shape_is_stable() {
    let request = Request::HumanInbound {
        to: "agent-review@host-a".into(),
        body: "hello".into(),
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "type": "human_inbound",
            "to": "agent-review@host-a",
            "body": "hello"
        })
    );
    assert_eq!(serde_json::from_value::<Request>(json).unwrap(), request);
}
