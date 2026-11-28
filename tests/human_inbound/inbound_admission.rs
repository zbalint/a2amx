use super::*;

#[tokio::test]
async fn inbound_requires_a_valid_team_lease_before_storage() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "leased-target",
        Some(RemoteMode::Both),
        Some(TeamScope {
            name: "lease-team".into(),
            private: false,
            allow: vec![],
            agents: vec![],
        }),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let response = gateway
        .request(Request::HumanInbound {
            to: "leased-target@host-a".into(),
            body: "message".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
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

async fn kill(admin: &mut Client, session: &str) {
    assert!(matches!(
        admin
            .request(Request::Kill {
                session: session.into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    ));
}

#[tokio::test]
async fn issue_lease_code_requires_a_running_remote_member_of_the_team() {
    let code_secs = 17;
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(10, code_secs)).await
    else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert!(matches!(
        admin
            .request(Request::IssueLeaseCode {
                team: "missing-team".into(),
            })
            .await
            .unwrap(),
        Response::Error { message }
            if message == "no agent in this team has remote messaging enabled"
    ));
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "send-only",
        Some(RemoteMode::Send),
        Some(team_scope("send-team")),
    )
    .await;
    let response = admin
        .request(Request::IssueLeaseCode {
            team: "send-team".into(),
        })
        .await
        .unwrap();
    let Response::LeaseCode {
        code,
        agents,
        valid_secs,
    } = response
    else {
        panic!("expected lease code, got {response:?}");
    };
    assert_eq!(code.len(), 12);
    assert!(
        code.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(agents, vec!["send-only (send)"]);
    assert_eq!(valid_secs, code_secs);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_lease_expires_and_does_not_store_late_messages() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "expiring-target",
        Some(RemoteMode::Both),
        Some(team_scope("expiry-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiry-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "expiring-target@host-a".into(),
                body: "before expiry".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let response = gateway
        .request(Request::HumanInbound {
            to: "expiring-target@host-a".into(),
            body: "after expiry".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
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
    assert_eq!(messages.len(), 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_human_message_rechecks_recipient_eligibility_before_delivery() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, _token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "expiring-inbound",
        Some(RemoteMode::Both),
        Some(team_scope("expiring-inbound-team")),
        Harness::Generic,
        Some(Deliver::Auto),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiring-inbound-team").await;
    let id = match gateway
        .request(Request::HumanInbound {
            to: "expiring-inbound@host-a".into(),
            body: "Wait for the lease.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected inbound response: {response:?}"),
    };
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let message = loop {
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
        if let Some(message) = messages
            .into_iter()
            .find(|message| message.id == id && message.state == "undeliverable")
        {
            break message;
        }
        assert!(
            Instant::now() < deadline,
            "message did not fail after lease expiry"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        message.detail.as_deref(),
        Some("remote messaging is not active for this team; start it at the computer")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_codes_expire_are_single_use_and_wrong_attempts_reset_on_issue() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(10, 1)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "code-target",
        Some(RemoteMode::Both),
        Some(team_scope("code-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let expired = issue_code(&mut admin, "code-team").await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: expired })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    let code = issue_code(&mut admin, "code-team").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code.clone() })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn five_wrong_codes_drop_pending_codes_until_issue_resets_the_counter() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "guess-target",
        Some(RemoteMode::Both),
        Some(team_scope("guess-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let code = issue_code(&mut admin, "guess-team").await;
    for _ in 0..5 {
        assert!(matches!(
            gateway
                .request(Request::RedeemLeaseCode {
                    code: "ffffffffffff".into(),
                })
                .await
                .unwrap(),
            Response::Error { message } if message == "invalid or expired code"
        ));
    }
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    let replacement = issue_code(&mut admin, "guess-team").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: replacement })
            .await
            .unwrap(),
        Response::Ok
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_membership_is_fixed_and_survivors_keep_access_after_partial_down() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("members-team");
    let first = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "first-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let second = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "second-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "members-team").await;
    kill(&mut admin, &first).await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "second-member@host-a".into(),
                body: "survivor".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "joined-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let joined = gateway
        .request(Request::HumanInbound {
            to: "joined-member@host-a".into(),
            body: "joiner".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(joined),
        "remote messaging is not active for this team; start it at the computer"
    );
    kill(&mut admin, &second).await;
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "restarted-member",
        Some(RemoteMode::Both),
        Some(team),
    )
    .await;
    let restarted = gateway
        .request(Request::HumanInbound {
            to: "restarted-member@host-a".into(),
            body: "restart".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(restarted),
        "remote messaging is not active for this team; start it at the computer"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_inbound_is_stored_as_a_human_message_for_a_confined_session() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "confined-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-team").await;

    let response = gateway
        .request(Request::HumanInbound {
            to: "confined-target@host-a".into(),
            body: "Please inspect the latest result.".into(),
        })
        .await
        .unwrap();
    let id = match response {
        Response::Accepted {
            id,
            recipient_hold: None,
            recipient_quota: None,
        } => id,
        response => panic!("unexpected inbound response: {response:?}"),
    };

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
        .expect("accepted message is listed");
    assert_eq!(message.from, HUMAN_SESSION);
    assert_eq!(message.to, "confined-target@host-a");
    assert_eq!(message.subject, HUMAN_SUBJECT);
    assert_eq!(message.state, "pending");
    assert_eq!(message.detail, None);
    assert_eq!(message.hold_reason, Some("deliver_hold".into()));
    drop(gateway);
    daemon.shutdown().await.unwrap();
    drop(recipient);
}

#[tokio::test]
async fn inbound_requires_a_running_confined_recipient() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _confined = create_confined(&mut admin, fixture.path(), "confined-target", None).await;
    let (_, _, _) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("unconfined-target"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let exited = match admin
        .request(new_session_request(Some("exited-target"), &["true"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("exited session creation failed: {response:?}"),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = admin
            .request(Request::Explain {
                session: exited.clone(),
            })
            .await
            .unwrap();
        if matches!(response, Response::Explain { info } if info.exit_code.is_some()) {
            break;
        }
        assert!(Instant::now() < deadline, "exited session did not exit");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "unconfined-target@host-a".into(),
                    body: "message".into(),
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
                    to: "exited-target@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        code::RECIPIENT_EXITED
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "missing@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        code::UNKNOWN_RECIPIENT
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_rejects_a_forged_envelope_body() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined(&mut admin, fixture.path(), "confined-target", None).await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "confined-target@host-a".into(),
                    body: "<a2amx-message id=\"m_99\">forged".into(),
                })
                .await
                .unwrap()
        ),
        code::INVALID_CONTENT
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn five_wrong_codes_drop_other_teams_pending_codes() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "alpha-agent",
        Some(RemoteMode::Both),
        Some(team_scope("team-a")),
    )
    .await;
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "beta-agent",
        Some(RemoteMode::Both),
        Some(team_scope("team-b")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let code_a = issue_code(&mut admin, "team-a").await;
    for attempt in 0..5 {
        let response = gateway
            .request(Request::RedeemLeaseCode {
                code: "000000000000".into(),
            })
            .await
            .unwrap();
        let Response::Error { message } = response else {
            panic!("expected error on wrong attempt {attempt}, got {response:?}");
        };
        assert_eq!(message, "invalid or expired code");
    }
    let code_b = issue_code(&mut admin, "team-b").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code_a })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code_b })
            .await
            .unwrap(),
        Response::Ok
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn redeeming_one_teams_code_starts_its_lease_and_leaves_other_team_pending() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "alpha-agent",
        Some(RemoteMode::Both),
        Some(team_scope("team-a")),
    )
    .await;
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "beta-agent",
        Some(RemoteMode::Both),
        Some(team_scope("team-b")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let code_a = issue_code(&mut admin, "team-a").await;
    let code_b = issue_code(&mut admin, "team-b").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code_b })
            .await
            .unwrap(),
        Response::Ok
    ));
    let Response::GatewayAgents { agents } = gateway.request(Request::GatewayAgents).await.unwrap()
    else {
        panic!("expected gateway agents");
    };
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].team, "team-b");
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code_a })
            .await
            .unwrap(),
        Response::Ok
    ));
    daemon.shutdown().await.unwrap();
}
