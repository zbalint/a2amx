use super::*;

#[tokio::test]
async fn inbound_rejects_a_body_over_the_human_chat_limit_before_lookup() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "size-target",
        Some(RemoteMode::Both),
        Some(team_scope("size-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "size-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "size-target@host-a".into(),
                body: "x".repeat(8000),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "unknown@host-a".into(),
            body: "x".repeat(8001),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::TOO_LARGE);
    assert_eq!(
        failed_message(response),
        "message exceeds the byte limit for a human message"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_window_refuses_the_third_accepted_message() {
    let limits = Limits {
        human_inbound_per_minute: 2,
        max_pending_per_recipient: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-rate-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-rate-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "unknown@host-a".into(),
                    body: "unknown does not count".into(),
                })
                .await
                .unwrap()
        ),
        code::UNKNOWN_RECIPIENT
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: "no lease does not count".into(),
                })
                .await
                .unwrap()
        ),
        code::NOT_PERMITTED
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: "x".repeat(8001),
                })
                .await
                .unwrap()
        ),
        code::TOO_LARGE
    );
    issue_and_redeem(&mut admin, &mut gateway, "inbound-rate-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-rate-target@host-a".into(),
            body: "third".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "human inbound rate limit reached");
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
    assert_eq!(messages.len(), 2);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_respects_the_recipient_pending_queue_limit() {
    let limits = Limits {
        max_pending_per_recipient: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-cap-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-cap-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-cap-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "inbound-cap-target@host-a".into(),
                body: "first".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-cap-target@host-a".into(),
            body: "over cap".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::QUEUE_FULL);
    assert_eq!(failed_message(response), "message queue is full");
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
    assert_eq!(messages.len(), 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_queue_full_does_not_consume_the_rate_window() {
    let limits = Limits {
        human_inbound_per_minute: 2,
        max_pending_per_recipient: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-queue-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-queue-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-queue-team").await;
    let first_id = match gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "first".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected first response: {response:?}"),
    };
    let second = gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "queue full".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(second.clone()), code::QUEUE_FULL);
    assert_eq!(failed_message(second), "message queue is full");
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
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        admin
            .request(Request::CancelMessage { id: first_id })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "inbound-queue-target@host-a".into(),
                body: "after cancel".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "rate limit".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "human inbound rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_agent_outbound_window_refuses_its_third_message() {
    let limits = Limits {
        rate_per_minute: 2,
        human_outbound_per_minute: 30,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-rate-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-rate-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-rate-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "third".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "sender rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_total_window_refuses_messages_across_agents() {
    let limits = Limits {
        rate_per_minute: 3,
        human_outbound_per_minute: 2,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = team_scope("outbound-total-team");
    let first_fixture = tempfile::tempdir().unwrap();
    let (_first_session, first_token) = create_confined_with_token(
        &mut admin,
        first_fixture.path(),
        "outbound-total-first",
        Some(RemoteMode::Send),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let second_fixture = tempfile::tempdir().unwrap();
    let (_second_session, second_token) = create_confined_with_token(
        &mut admin,
        second_fixture.path(),
        "outbound-total-second",
        Some(RemoteMode::Send),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut first = Client::connect_addr(daemon.addrs()[0], first_token.trim())
        .await
        .unwrap();
    let mut second = Client::connect_addr(daemon.addrs()[0], second_token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-total-team").await;
    for sender in [&mut first, &mut second] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: "accepted".into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = first
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "total limit".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(
        failed_message(response),
        "human outbound rate limit reached"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_refusal_prefers_sender_when_both_windows_are_full() {
    let limits = Limits {
        max_pending_per_recipient: 3,
        rate_per_minute: 2,
        human_outbound_per_minute: 2,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-precedence-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-precedence-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-precedence-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "both windows full".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "sender rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_rate_refusal_does_not_push_a_timestamp() {
    let limits = Limits {
        max_pending_per_recipient: 3,
        rate_per_minute: 3,
        human_outbound_per_minute: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-refusal-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-refusal-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-refusal-team").await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "Request".into(),
                message: "first".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::sleep(Duration::from_secs(55)).await;
    let refused = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "refused".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(refused.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(refused), "human outbound rate limit reached");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "Request".into(),
                message: "after expiry".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}
