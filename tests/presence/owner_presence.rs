use super::*;

#[tokio::test]
async fn interactive_presence_refuses_human_send_then_allows_it_after_grace() {
    let limits = Limits {
        away_secs: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("presence-send");
    let (sender_id, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "presence-sender",
        Some(RemoteMode::Both),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-send").await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();

    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 80, 24)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("present"));
    assert_eq!(
        sender
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "while present".into(),
                message: "refused".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: code::OWNER_PRESENT.into(),
            message: "the owner is attached to this session in the terminal (viewer mode does not count); write to them there instead".into(),
        }
    );
    drop(attachment);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    let accepted_id = match sender
        .request(Request::SendMessage {
            to: messaging::HUMAN_SESSION.into(),
            subject: "while away".into(),
            message: "accepted".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected away send response: {response:?}"),
    };
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 80, 24)
        .await
        .unwrap();
    let outbound = gateway.request(Request::HumanOutboundNext).await.unwrap();
    assert_eq!(
        outbound,
        Response::HumanOutbound {
            message: Some(a2amx::wire::HumanOutboundMessage {
                id: accepted_id.clone(),
                from: "presence-sender@host-a".into(),
                subject: "while away".into(),
                body: "accepted".into(),
            }),
        }
    );
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: accepted_id,
                sent: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    drop(attachment);
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn attached_owner_presence_is_scoped_to_the_sending_session() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(Limits::default()).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture_a = tempfile::tempdir().unwrap();
    let fixture_b = tempfile::tempdir().unwrap();
    let team = team_scope("presence-sender-session");
    let (session_a, token_a) = create_confined_with_token(
        &mut admin,
        fixture_a.path(),
        "presence-session-a",
        Some(RemoteMode::Both),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let (_session_b, token_b) = create_confined_with_token(
        &mut admin,
        fixture_b.path(),
        "presence-session-b",
        Some(RemoteMode::Both),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-sender-session").await;
    let mut sender_a = Client::connect_addr(daemon.addrs()[0], &token_a)
        .await
        .unwrap();
    let mut sender_b = Client::connect_addr(daemon.addrs()[0], &token_b)
        .await
        .unwrap();
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&session_a, false, 80, 24)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender_a).await.as_deref(), Some("present"));
    assert_eq!(owner_state(&mut sender_b).await.as_deref(), Some("away"));
    assert!(matches!(
        sender_b
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "detached sender".into(),
                message: "accepted".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    assert_eq!(
        sender_a
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "attached sender".into(),
                message: "refused".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: code::OWNER_PRESENT.into(),
            message: "the owner is attached to this session in the terminal (viewer mode does not count); write to them there instead".into(),
        }
    );
    drop(attachment);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn grace_presence_is_scoped_and_reports_its_cause() {
    let limits = Limits {
        away_secs: 30,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture_a = tempfile::tempdir().unwrap();
    let fixture_b = tempfile::tempdir().unwrap();
    let team = team_scope("presence-grace");
    let (session_a, token_a) = create_confined_with_token(
        &mut admin,
        fixture_a.path(),
        "presence-grace-a",
        Some(RemoteMode::Both),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let (_session_b, token_b) = create_confined_with_token(
        &mut admin,
        fixture_b.path(),
        "presence-grace-b",
        Some(RemoteMode::Both),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-grace").await;
    let mut sender_a = Client::connect_addr(daemon.addrs()[0], &token_a)
        .await
        .unwrap();
    let mut sender_b = Client::connect_addr(daemon.addrs()[0], &token_b)
        .await
        .unwrap();
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&session_a, false, 80, 24)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Mode { viewer: true })
        .await
        .unwrap();
    attachment.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut attachment).await;
    assert_eq!(owner_state(&mut sender_a).await.as_deref(), Some("present"));
    assert_eq!(owner_state(&mut sender_b).await.as_deref(), Some("away"));
    assert_eq!(
        sender_a
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "grace".into(),
                message: "refused".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: code::OWNER_PRESENT.into(),
            message: "the owner left this session less than 30 seconds ago; try again later or write to them in the terminal".into(),
        }
    );
    assert!(matches!(
        sender_b
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "grace other session".into(),
                message: "accepted".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    drop(attachment);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn viewer_only_remote_session_can_message_owner() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "presence-viewer-only",
        Some(RemoteMode::Both),
        Some(team_scope("presence-viewer-only")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-viewer-only").await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&session, false, 80, 24, 0)
        .await
        .unwrap();
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: messaging::HUMAN_SESSION.into(),
                subject: "viewer".into(),
                message: "accepted".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    drop(attachment);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn force_takeover_replaces_presence_without_double_counting() {
    let limits = Limits {
        away_secs: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("presence-force");
    let (sender_id, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "force-sender",
        Some(RemoteMode::Both),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-force").await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    let first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 40, 10)
        .await
        .unwrap();
    let second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, true, 40, 10)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("present"));
    drop(second);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    drop(first);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_session_attach_does_not_make_owner_present() {
    let limits = Limits {
        away_secs: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_sender_id, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "exited-attach-owner",
        Some(RemoteMode::Both),
        Some(team_scope("exited-attach")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "exited-attach").await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    let id = match admin
        .request(new_session_request(None, &["sh", "-c", "exit 7"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut exited = false;
    for _ in 0..100 {
        let response = admin.request(Request::List).await.unwrap();
        let Response::Sessions { sessions } = response else {
            panic!("list failed");
        };
        if sessions
            .into_iter()
            .any(|session| session.id == id && session.exit_code == Some(7))
        {
            exited = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(exited, "session exits with code 7");
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 10)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    let exited = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match attachment.recv().await.unwrap() {
                Some(ServerFrame::Exit(7)) => break true,
                Some(_) => {}
                None => break false,
            }
        }
    })
    .await
    .unwrap();
    assert!(exited);
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_attach_does_not_drop_another_interactive_count() {
    let limits = Limits {
        away_secs: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (sender_id, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "count-survivor",
        Some(RemoteMode::Both),
        Some(team_scope("count-survivor")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "count-survivor").await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    let survivor = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 40, 10)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("present"));
    let exited_id = match admin
        .request(new_session_request(None, &["sh", "-c", "exit 7"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut exited_ready = false;
    for _ in 0..100 {
        let response = admin.request(Request::List).await.unwrap();
        let Response::Sessions { sessions } = response else {
            panic!("list failed");
        };
        if sessions
            .into_iter()
            .any(|session| session.id == exited_id && session.exit_code == Some(7))
        {
            exited_ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(exited_ready, "session exits with code 7");
    let mut exited = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&exited_id, false, 40, 10)
        .await
        .unwrap();
    let received_exit = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match exited.recv().await.unwrap() {
                Some(ServerFrame::Exit(7)) => break true,
                Some(_) => {}
                None => break false,
            }
        }
    })
    .await
    .unwrap();
    assert!(received_exit);
    drop(exited);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("present"));
    drop(survivor);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(owner_state(&mut sender).await.as_deref(), Some("away"));
    daemon.shutdown().await.unwrap();
}
