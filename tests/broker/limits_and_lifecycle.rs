use super::*;

#[tokio::test]
async fn pending_limit_and_content_errors() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            max_pending_per_recipient: 2,
            rate_per_minute: 100,
            human_inbound_per_minute: 20,
            human_outbound_per_minute: 30,
            max_stored_messages: 100,
            retention_secs: 7 * 24 * 3600,
            lease_secs: 24 * 60 * 60,
            code_secs: 10 * 60,
            away_secs: 60,
        },
    )
    .await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    for to in ["missing@host-a", "recipient@host-b", "sender@host-a"] {
        assert_eq!(
            failed_code(send(&mut sender, to, "subject", "body").await),
            "unknown_recipient"
        );
    }
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "", "body").await),
        "invalid_content"
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "subject", "\x1b").await),
        "invalid_content"
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", &"s".repeat(201), "body").await),
        "too_large"
    );
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
        Some("deliver_hold"),
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "queue_full"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn limit_edge_exact_body_is_delivered_and_submitted() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let output = dir.path().join("recipient.input");
    let response = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                // Raw input avoids the terminal's canonical line-length limit.
                "stty raw -echo; printf '\\033[?2004h'; cat > \"$OUT\"".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("recipient".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    assert!(matches!(response, Response::Created { .. }));
    let body = format!("0123456789ABCDEF{}FEDCBA9876543210", "x".repeat(32768 - 32));
    assert!(matches!(
        send(&mut sender, "recipient@host-a", "size boundary", &body).await,
        Response::Accepted { id, .. } if id == "m_1"
    ));
    let message = wait_status(&mut sender, "m_1", |message| message.state == "submitted").await;
    assert_eq!(message.state, "submitted");
    let received = common::eventually(|| async {
        let bytes = std::fs::read(&output).ok()?;
        bytes.ends_with(b"\r").then_some(bytes)
    })
    .await;
    let received = std::str::from_utf8(&received).unwrap();
    let (_, received_body) = received.split_once("\n\n").expect("envelope body");
    let received_body = received_body
        .strip_suffix("\n</a2amx-message>\x1b[201~\r")
        .expect("envelope terminator, paste end and submit");
    assert_eq!(received_body.len(), 32768);
    assert_eq!(&received_body[..16], "0123456789ABCDEF");
    assert_eq!(
        &received_body[received_body.len() - 16..],
        "FEDCBA9876543210"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn limit_edge_oversized_body_is_refused_without_queueing() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient_id, _, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    assert_eq!(
        failed_code(
            send(
                &mut sender,
                "recipient@host-a",
                "oversized body",
                &"m".repeat(32769),
            )
            .await,
        ),
        "too_large"
    );
    let messages = match admin
        .request(Request::ListMessages {
            session: Some(recipient_id),
            state: Some("pending".into()),
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn limit_edge_default_queue_accepts_fifty_open_messages() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender_a) = session(
        &mut admin,
        dir.path(),
        Some("sender-a"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _, mut sender_b) = session(
        &mut admin,
        dir.path(),
        Some("sender-b"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _, mut sender_c) = session(
        &mut admin,
        dir.path(),
        Some("sender-c"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient_id, _, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    // Three senders keep each per-sender burst below 20, so queue_full is the first limit.
    for index in 0..50 {
        let sender = match index % 3 {
            0 => &mut sender_a,
            1 => &mut sender_b,
            _ => &mut sender_c,
        };
        assert!(matches!(
            send(sender, "recipient@host-a", "queued", "body").await,
            Response::Accepted { .. }
        ));
    }
    assert_eq!(
        failed_code(send(&mut sender_a, "recipient@host-a", "overflow", "body").await),
        "queue_full"
    );
    let messages = match admin
        .request(Request::ListMessages {
            session: Some(recipient_id),
            state: Some("pending".into()),
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(messages.len(), 50);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn limit_edge_default_rate_accepts_twenty_sends() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient_a_id, _, _recipient_a) = session(
        &mut admin,
        dir.path(),
        Some("recipient-a"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient_b_id, _, _recipient_b) = session(
        &mut admin,
        dir.path(),
        Some("recipient-b"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    // Two recipients keep each queue at 10, so queue_full cannot fire before rate_limited.
    for index in 0..20 {
        let recipient = if index % 2 == 0 {
            "recipient-a@host-a"
        } else {
            "recipient-b@host-a"
        };
        assert!(matches!(
            send(&mut sender, recipient, "burst", "body").await,
            Response::Accepted { .. }
        ));
    }
    assert_eq!(
        failed_code(send(&mut sender, "recipient-a@host-a", "overflow", "body").await),
        "rate_limited"
    );
    for recipient_id in [recipient_a_id, recipient_b_id] {
        let messages = match admin
            .request(Request::ListMessages {
                session: Some(recipient_id),
                state: Some("pending".into()),
            })
            .await
            .unwrap()
        {
            Response::Messages { messages } => messages,
            response => panic!("unexpected response: {response:?}"),
        };
        assert_eq!(messages.len(), 10);
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn stored_limit_and_rate_limit_are_independent() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            max_pending_per_recipient: 100,
            rate_per_minute: 100,
            human_inbound_per_minute: 20,
            human_outbound_per_minute: 30,
            max_stored_messages: 2,
            retention_secs: 7 * 24 * 3600,
            lease_secs: 24 * 60 * 60,
            code_secs: 10 * 60,
            away_secs: 60,
        },
    )
    .await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_other_id, _other_token, mut other) = session(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
        Some("deliver_hold"),
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "queue_full"
    );
    // The other sender has a separate rate window, but the shared stored limit applies.
    assert_eq!(
        failed_code(send(&mut other, "recipient@host-a", "three", "body").await),
        "queue_full"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn rate_limit_does_not_cross_sender_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            max_pending_per_recipient: 100,
            rate_per_minute: 2,
            human_inbound_per_minute: 20,
            human_outbound_per_minute: 30,
            max_stored_messages: 100,
            retention_secs: 7 * 24 * 3600,
            lease_secs: 24 * 60 * 60,
            code_secs: 10 * 60,
            away_secs: 60,
        },
    )
    .await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_other_id, _other_token, mut other) = session(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
        Some("deliver_hold"),
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "rate_limited"
    );
    accepted(
        send(&mut other, "recipient@host-a", "other", "body").await,
        "m_3",
        Some("deliver_hold"),
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn recipient_exit_cancelling_and_filtering() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let exit_marker = dir.path().join("recipient.exit");
    let recipient_response = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "while [ ! -f \"$EXIT\" ]; do sleep 0.01; done".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("EXIT".into(), exit_marker.to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("recipient".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    let recipient_id = match recipient_response {
        Response::Created { session } => session,
        response => panic!("unexpected response: {response:?}"),
    };
    wait_agents(&mut admin, |agents| {
        agents
            .iter()
            .any(|agent| agent.address == "recipient@host-a" && agent.state == "running")
    })
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
        Some("deliver_hold"),
    );
    assert_eq!(
        admin
            .request(Request::CancelMessage { id: "m_1".into() })
            .await
            .unwrap(),
        Response::Ok
    );
    assert!(
        error_message(
            admin
                .request(Request::CancelMessage { id: "m_1".into() })
                .await
                .unwrap()
        )
        .contains("cancelled")
    );
    let filtered = match admin
        .request(Request::ListMessages {
            session: Some(recipient_id.clone()),
            state: Some("pending".into()),
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].id, "m_2");
    assert_eq!(filtered[0].to, "recipient@host-a");

    std::fs::write(&exit_marker, b"").unwrap();
    let message = wait_status(&mut admin, "m_2", |message| {
        message.state == "undeliverable" && message.detail.as_deref() == Some("recipient_exited")
    })
    .await;
    assert_eq!(message.detail.as_deref(), Some("recipient_exited"));
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "recipient_exited"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn not_ready_and_queued_hold_reasons() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Claude,
        Deliver::Auto,
    )
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        None,
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
        None,
    );
    let first = wait_status(&mut sender, "m_1", |message| {
        message.hold_reason.as_deref() == Some("not_ready")
    })
    .await;
    assert_eq!(first.state, "pending");
    let second = match status(&mut sender, "m_2").await {
        Response::Status { message } => message,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(second.hold_reason.as_deref(), Some("queued"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn graceful_restart_marks_exited_and_continues_ids() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, _sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    let (_recipient_id, _recipient_token, _recipient) = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    daemon.shutdown().await.unwrap();

    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut new_admin = Client::connect(dir.path()).await.unwrap();
    let stopped = match status(&mut new_admin, "m_1").await {
        Response::Status { message } => message,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(stopped.state, "undeliverable");
    assert_eq!(stopped.detail.as_deref(), Some("daemon_stopped"));
    let historical = match new_admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(historical.iter().any(|message| message.id == "m_1"));
    let (_new_id, _new_token, mut new_sender) = session(
        &mut new_admin,
        dir.path(),
        Some("new-sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    assert_eq!(
        failed_code(status(&mut new_sender, "m_1").await),
        "unknown_message"
    );
    let (new_recipient_id, _new_recipient_token, _new_recipient) = session(
        &mut new_admin,
        dir.path(),
        Some("new-recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    accepted(
        send(&mut new_sender, "new-recipient@host-a", "next", "body").await,
        "m_2",
        Some("deliver_hold"),
    );
    let current_boot = match new_admin
        .request(Request::ListMessages {
            session: Some(new_recipient_id),
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(current_boot.len(), 1);
    assert_eq!(current_boot[0].id, "m_2");
    daemon.shutdown().await.unwrap();
}
