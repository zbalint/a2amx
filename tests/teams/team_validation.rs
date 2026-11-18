use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_team_settings_are_consistent_until_the_first_member_exits() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (first_id, _first) = session(
        &mut admin,
        dir.path(),
        "first",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let different = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("second".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", false, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    assert_eq!(
        different,
        Response::Error {
            message: "team a is already running with different settings".into()
        }
    );
    let same = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("second".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", true, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    let same_id = match same {
        Response::Created { session } => session,
        other => panic!("expected same-settings session, got {other:?}"),
    };
    assert_eq!(
        admin
            .request(Request::Kill {
                session: first_id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(
        admin
            .request(Request::Kill {
                session: same_id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let after_exit = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("third".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", false, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    assert!(matches!(after_exit, Response::Created { .. }));
}

fn new_session_request(name: &str, team: TeamScope) -> Request {
    Request::NewSession {
        argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
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
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_validates_agent_allow_without_affecting_team_scope_consistency() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_first_id, _first) = session(
        &mut admin,
        dir.path(),
        "a-one",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    for (name, team, expected) in [
        (
            "self",
            scope_with_agents("a", true, &[], &["self"]),
            "must not name itself",
        ),
        (
            "repeat",
            scope_with_agents("a", true, &[], &["other", "other"]),
            "is repeated",
        ),
        (
            "public",
            scope_with_agents("a", false, &[], &["other"]),
            "allow needs a private team",
        ),
    ] {
        let response = admin
            .request(new_session_request(name, team))
            .await
            .unwrap();
        let Response::Error { message } = response else {
            panic!("expected validation error");
        };
        assert!(message.contains(expected), "{message}");
    }
    let too_many = (0..17)
        .map(|index| format!("other-{index}"))
        .collect::<Vec<_>>();
    let response = admin
        .request(new_session_request(
            "too-many",
            TeamScope {
                name: "a".into(),
                private: true,
                allow: Vec::new(),
                agents: too_many,
            },
        ))
        .await
        .unwrap();
    let Response::Error { message } = response else {
        panic!("expected validation error");
    };
    assert!(message.contains("at most 16"), "{message}");

    let accepted = admin
        .request(new_session_request(
            "a-two",
            scope_with_agents("a", true, &[], &["other"]),
        ))
        .await
        .unwrap();
    assert!(matches!(accepted, Response::Created { .. }), "{accepted:?}");
}

#[tokio::test]
async fn new_session_team_validation_messages_and_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    for (team, role, expected) in [
        (
            scope("Bad_Name", true, &[]),
            None,
            "team: invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (scope("a-", true, &[]), None, "team must not end in '-'"),
        (
            scope("a", false, &["Bad_Name"]),
            None,
            "allow needs a private team",
        ),
        (
            scope("a", true, &["Bad_Name"]),
            None,
            "team allow entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            scope("a", true, &["a"]),
            None,
            "team allow entry must not be the team itself",
        ),
        (
            scope("a", true, &["b", "b"]),
            None,
            "team allow entry \"b\" is repeated",
        ),
        (
            scope_with_agents("a", true, &[], &["Bad_Name"]),
            None,
            "session allow entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            scope("Bad_Name", true, &[]),
            Some(""),
            "team: invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
    ] {
        let mut request = new_session_request("worker", team);
        if let Request::NewSession { role: value, .. } = &mut request {
            *value = role.map(str::to_owned);
        }
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
async fn new_session_team_conflict_precedes_in_use_name() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let created = admin
        .request(new_session_request("worker", scope("a", true, &[])))
        .await
        .unwrap();
    assert!(matches!(created, Response::Created { .. }), "{created:?}");
    assert_eq!(
        admin
            .request(new_session_request("worker", scope("a", false, &[])))
            .await
            .unwrap(),
        Response::Error {
            message: "team a is already running with different settings".into()
        }
    );
    daemon.shutdown().await.unwrap();
}
