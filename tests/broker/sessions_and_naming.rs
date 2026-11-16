use super::*;

#[tokio::test]
async fn session_tokens_roles_permissions_and_child_identity() {
    let (dir, daemon) = common::start_daemon().await;
    let admin_token = std::fs::read_to_string(dir.path().join("run/admin.token"))
        .unwrap()
        .trim()
        .to_owned();
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (sender_id, sender_token, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient_id, recipient_token, recipient) = session(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    assert!(sender_token != recipient_token);
    assert!(sender_token != admin_token);
    assert!(recipient_token != admin_token);
    assert_eq!(sender_token.len(), 64);
    assert_eq!(recipient_token.len(), 64);
    assert_eq!(sender_id, "s1");
    assert_eq!(recipient_id, "s2");

    let override_output = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let supplied_token = "client-supplied-token";
    let override_response = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; sleep 30".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![
                (
                    "OUT".into(),
                    override_output.path().to_string_lossy().into_owned(),
                ),
                ("A2AMX_TOKEN".into(), supplied_token.into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("override".into()),
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
    assert!(matches!(override_response, Response::Created { .. }));
    let child_token = common::eventually(|| async {
        let text = std::fs::read_to_string(override_output.path()).ok()?;
        text.lines()
            .next()
            .filter(|token| token.len() == 64)
            .map(str::to_owned)
    })
    .await;
    assert!(child_token != supplied_token);
    assert!(child_token != sender_token);
    assert!(child_token != recipient_token);

    let unknown = Client::connect_addr(daemon.addrs()[0], &"0".repeat(64)).await;
    assert!(unknown.is_err());

    for request in [
        Request::Version,
        Request::Hello {
            token: sender_token.clone(),
        },
        Request::Shutdown { now: true },
        Request::List,
        Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        },
        Request::Attach {
            session: sender_id.clone(),
            force: false,
            cols: 40,
            rows: 10,
            status: false,
            top_rows: 0,
            viewer: false,
        },
        Request::Kill {
            session: sender_id.clone(),
            now: true,
        },
        Request::Screen {
            session: sender_id.clone(),
        },
        Request::ListMessages {
            session: None,
            state: None,
        },
        Request::CancelMessage { id: "m_1".into() },
    ] {
        assert_eq!(
            error_message(sender.request(request).await.unwrap()),
            "not permitted for a session token"
        );
    }
    assert_eq!(
        error_message(
            admin
                .request(Request::SendMessage {
                    to: "agent-review@host-a".into(),
                    subject: "subject".into(),
                    message: "message".into(),
                })
                .await
                .unwrap()
        ),
        "send_message needs a session token"
    );

    accepted(
        send(&mut sender, "agent-review@host-a", "subject", "message").await,
        "m_1",
        Some("deliver_hold"),
    );
    let response_text = serde_json::to_string(&status(&mut sender, "m_1").await).unwrap();
    assert!(!response_text.contains(&admin_token));
    assert!(!response_text.contains(&sender_token));
    assert!(!response_text.contains(&recipient_token));
    drop(recipient);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn naming_addresses_and_role_expires_after_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (named_id, named_token, mut named) = session(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (unnamed_id, _unnamed_token, mut unnamed) = session(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    assert_eq!(named_id, "s1");
    assert_eq!(unnamed_id, "s2");
    let agents = match admin.request(Request::ListAgents).await.unwrap() {
        Response::Agents { agents, .. } => agents,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(agents.iter().any(|agent| {
        agent.address == "agent-plan@host-a"
            && agent.state == "running"
            && !agent.attached
            && agent.harness == Harness::Generic
            && agent.cwd.is_none()
    }));
    assert!(agents.iter().any(|agent| {
        agent.address == "s2@host-a"
            && agent.state == "running"
            && !agent.attached
            && agent.harness == Harness::Generic
            && agent.cwd.is_none()
    }));

    for name in ["s12", "Agent", &"a".repeat(64)] {
        let response = admin
            .request(Request::NewSession {
                argv: vec!["sh".into()],
                cols: 40,
                rows: 10,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some(name.to_owned()),
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
        assert!(matches!(response, Response::Error { .. }));
    }
    let duplicate = admin
        .request(Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agent-plan".into()),
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
    assert!(error_message(duplicate).contains("already in use"));

    assert_eq!(
        admin
            .request(Request::Kill {
                session: named_id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    wait_agents(&mut admin, |agents| {
        !agents
            .iter()
            .any(|agent| agent.address == "agent-plan@host-a")
    })
    .await;
    assert!(
        Client::connect_addr(daemon.addrs()[0], &named_token)
            .await
            .is_err()
    );
    assert!(matches!(
        named.request(Request::ListAgents).await,
        Err(_) | Ok(Response::Error { .. })
    ));
    let natural = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "exit 0".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("natural".into()),
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
    assert!(matches!(natural, Response::Created { .. }));
    wait_agents(&mut admin, |agents| {
        agents
            .iter()
            .any(|agent| agent.address == "natural@host-a" && agent.state == "exited")
    })
    .await;
    let role_agents = match unnamed.request(Request::ListAgents).await.unwrap() {
        Response::Agents { agents, .. } => agents,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(role_agents.iter().any(|agent| {
        agent.address == "natural@host-a"
            && agent.state == "exited"
            && !agent.attached
            && agent.activity.is_none()
    }));
    assert!(role_agents.iter().any(|agent| {
        agent.address == "s2@host-a" && agent.state == "running" && !agent.attached
    }));
    let duplicate_after_natural = admin
        .request(Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("natural".into()),
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
    assert!(error_message(duplicate_after_natural).contains("already in use"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_agents_reports_harness_and_spawn_directory() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let response = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "cd / && sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: Some("/tmp".into()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agent-omp".into()),
            harness: Harness::Omp,
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
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|agent| agent.address == "agent-omp@host-a")
        .expect("OMP agent listed");
    assert_eq!(agent.harness, Harness::Omp);
    assert_eq!(agent.cwd.as_deref(), Some("/tmp"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn message_status_visibility_and_hold_reason() {
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
        send(&mut sender, "recipient@host-a", "subject", "body").await,
        "m_1",
        Some("deliver_hold"),
    );
    let message = match status(&mut sender, "m_1").await {
        Response::Status { message } => message,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(message.id, "m_1");
    assert_eq!(message.from, "sender@host-a");
    assert_eq!(message.to, "recipient@host-a");
    assert_eq!(message.state, "pending");
    assert_eq!(message.hold_reason.as_deref(), Some("deliver_hold"));
    assert_eq!(
        message.hold_explanation.as_deref(),
        Some(
            "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery."
        )
    );
    let accepted_at = message.accepted_at.expect("accepted_at is present");
    let updated_at = message.updated_at.expect("updated_at is present");
    assert!(accepted_at > 0);
    assert!(updated_at > 0);
    assert!(accepted_at <= updated_at);
    assert_eq!(
        failed_code(status(&mut other, "m_1").await),
        "unknown_message"
    );
    assert!(matches!(
        status(&mut admin, "m_1").await,
        Response::Status { .. }
    ));
    assert_eq!(
        failed_code(status(&mut sender, "malformed").await),
        "unknown_message"
    );
    assert_eq!(
        failed_code(status(&mut sender, "m_99").await),
        "unknown_message"
    );
    assert!(matches!(
        admin
            .request(Request::ListMessages {
                session: None,
                state: Some("not_a_state".into()),
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    daemon.shutdown().await.unwrap();
}
