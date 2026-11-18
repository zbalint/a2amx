use super::*;

async fn exit_session(admin: &mut Client, name: &str, team: TeamScope) -> String {
    let response = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "exit 0".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some(name.into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(team),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("expected created session: {response:?}");
    };
    session
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_team_visibility_hides_other_private_teams_but_not_ungrouped() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_plain_id, _plain) = session(&mut admin, dir.path(), "plain", None, &[], &[]).await;

    let Response::Agents { agents, .. } = a.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a@") && agent.you)
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("plain@"))
    );
    assert!(!agents.iter().any(|agent| agent.address.starts_with("b@")));

    let denied = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "hidden".into(),
            message: "hidden".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        denied,
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into()
        }
    );
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("expected admin agent list");
    };
    assert_eq!(agents.len(), 3);
    assert!(agents.iter().all(|agent| !agent.you));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_allow_is_required_for_private_team_message_visibility() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &["b"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, mut b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &["a"])),
        &[],
        &[],
    )
    .await;
    let sent = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "visible".into(),
            message: "visible".into(),
        })
        .await
        .unwrap();
    assert!(matches!(sent, Response::Accepted { .. }), "{sent:?}");
    let Response::Agents { agents, .. } = b.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(agents.iter().any(|agent| agent.address.starts_with("a@")));

    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &["b"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let denied = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "hidden".into(),
            message: "hidden".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        denied,
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_visibility_fails_before_control_from() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &["a"],
    )
    .await;
    let response = a
        .request(Request::Reset {
            session: "b".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        response,
        Response::Failed {
            code: "unknown_session".into(),
            message: "unknown session".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invisible_exit_notifications_are_not_queued() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (same_id, _same) = session(
        &mut admin,
        dir.path(),
        "same",
        Some(scope("a", true, &[])),
        &["same-peer"],
        &[],
    )
    .await;
    let (other_id, _other) = session(
        &mut admin,
        dir.path(),
        "other",
        Some(scope("a", true, &[])),
        &["other-peer"],
        &[],
    )
    .await;
    let same_peer = exit_session(&mut admin, "same-peer", scope("a", true, &[])).await;
    let other_peer = exit_session(&mut admin, "other-peer", scope("b", true, &[])).await;
    let same_message = loop {
        let response = admin
            .request(Request::ListMessages {
                session: Some(same_id.clone()),
                state: Some("pending".into()),
            })
            .await
            .unwrap();
        if let Response::Messages { messages } = response
            && let Some(message) = messages
                .into_iter()
                .find(|message| message.subject.contains("same-peer"))
        {
            break message;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(same_message.subject.contains("same-peer"));
    let other_messages = admin
        .request(Request::ListMessages {
            session: Some(other_id.clone()),
            state: Some("pending".into()),
        })
        .await
        .unwrap();
    let Response::Messages {
        messages: other_messages,
    } = other_messages
    else {
        panic!("expected messages response");
    };
    assert!(
        !other_messages
            .iter()
            .any(|message| message.subject.contains("other-peer"))
    );
    let _ = (same_peer, other_peer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_agent_allow_limits_private_team_visibility_and_messaging() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_arch_id, mut a_arch) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &[],
        &[],
    )
    .await;
    let (_a_dev_id, mut a_dev) = session(
        &mut admin,
        dir.path(),
        "a-developer",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_arch_id, mut b_arch) = session(
        &mut admin,
        dir.path(),
        "b-architect",
        Some(scope_with_agents("b", true, &[], &["a-architect"])),
        &[],
        &[],
    )
    .await;
    let (_b_dev_id, _b_dev) = session(
        &mut admin,
        dir.path(),
        "b-developer",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;

    let Response::Agents { agents, .. } = a_arch.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a-architect@"))
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a-developer@"))
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("b-architect@"))
    );
    assert!(
        !agents
            .iter()
            .any(|agent| agent.address.starts_with("b-developer@"))
    );

    let sent = a_arch
        .request(Request::SendMessage {
            to: "b-architect".into(),
            subject: "hello".into(),
            message: "hello".into(),
        })
        .await
        .unwrap();
    assert!(matches!(sent, Response::Accepted { .. }), "{sent:?}");
    let reply = b_arch
        .request(Request::SendMessage {
            to: "a-architect".into(),
            subject: "reply".into(),
            message: "reply".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reply, Response::Accepted { .. }), "{reply:?}");
    assert_eq!(
        a_arch
            .request(Request::SendMessage {
                to: "b-developer".into(),
                subject: "hidden".into(),
                message: "hidden".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );
    assert_eq!(
        a_dev
            .request(Request::SendMessage {
                to: "b-architect".into(),
                subject: "hidden".into(),
                message: "hidden".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );

    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b-architect",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let Response::Agents { agents, .. } = a.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        !agents
            .iter()
            .any(|agent| agent.address.starts_with("b-architect@"))
    );
    assert_eq!(
        a.request(Request::SendMessage {
            to: "b-architect".into(),
            subject: "one-sided".into(),
            message: "one-sided".into(),
        })
        .await
        .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_agent_allow_controls_exit_notifications() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (architect_id, _architect) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &["b-architect"],
        &[],
    )
    .await;
    let (developer_id, _developer) = session(
        &mut admin,
        dir.path(),
        "a-developer",
        Some(scope("a", true, &[])),
        &["b-architect"],
        &[],
    )
    .await;
    let _peer_id = exit_session(
        &mut admin,
        "b-architect",
        scope_with_agents("b", true, &[], &["a-architect"]),
    )
    .await;
    let architect_message = loop {
        let response = admin
            .request(Request::ListMessages {
                session: Some(architect_id.clone()),
                state: Some("pending".into()),
            })
            .await
            .unwrap();
        if let Response::Messages { messages } = response
            && let Some(message) = messages
                .into_iter()
                .find(|message| message.subject.contains("b-architect"))
        {
            break message;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(architect_message.subject.contains("peer exited"));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: Some(developer_id),
            state: Some("pending".into()),
        })
        .await
        .unwrap()
    else {
        panic!("expected messages response");
    };
    assert!(
        !messages
            .iter()
            .any(|message| message.subject.contains("b-architect"))
    );
}
