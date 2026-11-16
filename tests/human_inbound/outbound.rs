use super::*;

#[tokio::test]
async fn enabled_agent_can_send_to_human_and_gateway_reports_outcome() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-team").await;
    let target_fixture = tempfile::tempdir().unwrap();
    let _target = create_confined_with_team(
        &mut admin,
        target_fixture.path(),
        "agent-target",
        None,
        Some(team_scope("outbound-team")),
    )
    .await;
    let agent_id = match sender
        .request(Request::SendMessage {
            to: "agent-target@host-a".into(),
            subject: "Agent-only".into(),
            message: "The gateway must not claim this.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected agent send response: {response:?}"),
    };
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: agent_id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus {
                id: agent_id.clone()
            })
            .await
            .unwrap(),
        Response::Status { message } if message.state == "pending"
    ));

    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Need help".into(),
            message: "Please check the latest result.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted {
            id,
            recipient_hold: None,
            recipient_quota: None,
        } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    let outbound = gateway.request(Request::HumanOutboundNext).await.unwrap();
    assert_eq!(
        outbound,
        Response::HumanOutbound {
            message: Some(a2amx::wire::HumanOutboundMessage {
                id: id.clone(),
                from: "outbound-agent@host-a".into(),
                subject: "Need help".into(),
                body: "Please check the latest result.".into(),
            }),
        }
    );
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus { id: id.clone() })
            .await
            .unwrap(),
        Response::Status { message } if message.state == "submitted"
    ));
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );

    let failed_id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Another request".into(),
            message: "This one should report failure.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected second send response: {response:?}"),
    };
    assert!(matches!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound {
            message: Some(a2amx::wire::HumanOutboundMessage { ref id, .. })
        } if id == &failed_id
    ));
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: failed_id.clone(),
                sent: false,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus { id: failed_id })
            .await
            .unwrap(),
        Response::Status { message }
            if message.state == "undeliverable"
                && message.detail.as_deref() == Some("the gateway could not send it")
    ));
    daemon.shutdown().await.unwrap();
    drop(session);
}

#[tokio::test]
async fn send_to_human_requires_enablement_and_stores_nothing_when_refused() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-disabled",
        None,
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "No".into(),
            message: "This must be refused.".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
    assert_eq!(
        failed_message(response),
        "remote messages to this session are not enabled"
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_to_human_without_a_team_is_refused_for_send_and_both_modes() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let send_fixture = tempfile::tempdir().unwrap();
    let (_send_session, send_token) = create_confined_with_token(
        &mut admin,
        send_fixture.path(),
        "no-team-send",
        Some(RemoteMode::Send),
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let both_fixture = tempfile::tempdir().unwrap();
    let (_both_session, both_token) = create_confined_with_token(
        &mut admin,
        both_fixture.path(),
        "no-team-both",
        Some(RemoteMode::Both),
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut send_client = Client::connect_addr(daemon.addrs()[0], send_token.trim())
        .await
        .unwrap();
    let mut both_client = Client::connect_addr(daemon.addrs()[0], both_token.trim())
        .await
        .unwrap();
    for (client, subject) in [
        (&mut send_client, "Send without team"),
        (&mut both_client, "Both without team"),
    ] {
        let response = client
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: subject.into(),
                message: "A team is required.".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            failed_message(response),
            "remote messaging is not active for this team; start it at the computer"
        );
    }
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_to_human_send_mode_needs_a_lease_and_exact_address() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-send",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-send-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "No lease".into(),
            message: "This must wait for a lease.".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
    );
    for to in ["human:owner@host-a", "Human:owner"] {
        assert_eq!(
            failed_code(
                sender
                    .request(Request::SendMessage {
                        to: to.into(),
                        subject: "Unknown".into(),
                        message: "Not the human address.".into(),
                    })
                    .await
                    .unwrap()
            ),
            code::UNKNOWN_RECIPIENT
        );
    }
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-send-team").await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "With lease".into(),
                message: "Now this is accepted.".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn human_outbound_rechecks_sender_eligibility_before_gateway_fetch() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "expiring-outbound",
        Some(RemoteMode::Send),
        Some(team_scope("expiring-outbound-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiring-outbound-team").await;
    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Before expiry".into(),
            message: "This should not leave after expiry.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound { message: None }
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    let message = messages
        .into_iter()
        .find(|message| message.id == id)
        .expect("expired message is listed");
    assert_eq!(message.state, "undeliverable");
    assert_eq!(
        message.detail.as_deref(),
        Some("sender is no longer eligible for remote messaging")
    );
    daemon.shutdown().await.unwrap();
}
