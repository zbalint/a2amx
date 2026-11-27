use super::*;

#[tokio::test]
async fn reset_authorizes_controller_and_rejects_unlisted_or_self() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (sender_id, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("controller"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_other_id, other_token, other_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_unnamed_id, unnamed_token, unnamed_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '\\033[?2004h'; cat".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec!["controller".into()],
            watch: vec![],
            name: Some("target".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected target response: {response:?}"),
    };
    let screen_before = match admin
        .request(Request::Screen {
            session: target.clone(),
        })
        .await
        .unwrap()
    {
        Response::Screen { lines } => lines,
        response => panic!("unexpected screen response: {response:?}"),
    };

    let mut other = Client::connect_addr(other_addr, &other_token)
        .await
        .unwrap();
    let denied = other
        .request(Request::Reset {
            session: target.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        denied,
        Response::Failed { code, .. } if code == "not_permitted"
    ));
    let screen_after_other = match admin
        .request(Request::Screen {
            session: target.clone(),
        })
        .await
        .unwrap()
    {
        Response::Screen { lines } => lines,
        response => panic!("unexpected screen response: {response:?}"),
    };
    assert_eq!(screen_after_other, screen_before);
    let mut unnamed = Client::connect_addr(unnamed_addr, &unnamed_token)
        .await
        .unwrap();
    let unnamed_denied = unnamed
        .request(Request::Reset {
            session: target.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        unnamed_denied,
        Response::Failed { code, .. } if code == "not_permitted"
    ));
    let screen_after_unnamed = match admin
        .request(Request::Screen {
            session: target.clone(),
        })
        .await
        .unwrap()
    {
        Response::Screen { lines } => lines,
        response => panic!("unexpected screen response: {response:?}"),
    };
    assert_eq!(screen_after_unnamed, screen_before);

    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let self_denied = sender
        .request(Request::Reset {
            session: sender_id.clone(),
        })
        .await
        .unwrap();
    assert!(matches!(
        self_denied,
        Response::Failed { code, .. } if code == "not_permitted"
    ));
    let allowed = sender
        .request(Request::Reset { session: target })
        .await
        .unwrap();
    assert!(matches!(allowed, Response::Reset { steps: 1 }));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_holds_message_delivery_until_sequence_finishes() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_sender_id, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("controller"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '\\033[?2004h'; cat".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec!["/clear".into(), "/second".into()],
            control_from: vec!["controller".into()],
            watch: vec![],
            name: Some("target".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected target response: {response:?}"),
    };
    let target_for_reset = target.clone();
    let mut resetter = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let reset_task = tokio::spawn(async move {
        resetter
            .request(Request::Reset {
                session: target_for_reset,
            })
            .await
            .unwrap()
    });
    let mut reset_started = false;
    for _ in 0..100 {
        let screen = match admin
            .request(Request::Screen {
                session: target.clone(),
            })
            .await
            .unwrap()
        {
            Response::Screen { lines } => lines,
            response => panic!("unexpected screen response: {response:?}"),
        };
        if screen.iter().any(|line| line.contains("/clear")) {
            reset_started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(reset_started, "reset did not type its first step");
    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let accepted = send(&mut sender, "target@host-a", "during-reset", "queued").await;
    assert!(matches!(
        accepted,
        Response::Accepted {
            recipient_hold: Some(reason),
            ..
        } if reason == "deliver_hold"
    ));
    assert!(matches!(
        reset_task.await.unwrap(),
        Response::Reset { steps: 2 }
    ));
    let mut screen = Vec::new();
    for _ in 0..100 {
        screen = match admin
            .request(Request::Screen {
                session: target.clone(),
            })
            .await
            .unwrap()
        {
            Response::Screen { lines } => lines,
            response => panic!("unexpected screen response: {response:?}"),
        };
        if screen.iter().any(|line| line.contains("queued")) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        screen.iter().any(|line| line.contains("queued")),
        "queued message did not arrive after reset: {screen:?}"
    );
    daemon.shutdown().await.unwrap();
}

async fn exited_reset_target(admin: &mut Client, dir: &Path) -> String {
    let Response::Created { session: target } = admin
        .request(common::new_session_request(
            Some("exited-target"),
            &["sh", "-c", "exit 0"],
        ))
        .await
        .unwrap()
    else {
        panic!("unexpected target response");
    };
    common::eventually(|| async {
        let mut client = Client::connect(dir).await.unwrap();
        let Response::Sessions { sessions } = client.request(Request::List).await.unwrap() else {
            return None;
        };
        sessions
            .into_iter()
            .find(|session| session.id == target && session.exit_code == Some(0))
    })
    .await;
    target
}

#[tokio::test]
async fn reset_unknown_target_reports_literal_unknown_session_error() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let unknown = admin
        .request(Request::Reset {
            session: "missing-target".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        unknown,
        Response::Failed {
            code: "unknown_session".into(),
            message: "unknown session".into(),
        }
    );
    daemon.shutdown().await.unwrap();
}

async fn assert_reset_not_permitted(client: &mut Client, target: String) {
    let denied = client
        .request(Request::Reset { session: target })
        .await
        .unwrap();
    assert_eq!(
        denied,
        Response::Failed {
            code: "not_permitted".into(),
            message: "sender is not permitted to reset this session".into(),
        }
    );
}

#[tokio::test]
async fn reset_checks_permission_before_exited_for_unlisted_sender() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_other_id, _other_token, mut other) = session(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = exited_reset_target(&mut admin, dir.path()).await;
    assert_reset_not_permitted(&mut other, target).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_exited_target_by_admin_reports_literal_exited_error() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = exited_reset_target(&mut admin, dir.path()).await;
    let exited = admin
        .request(Request::Reset { session: target })
        .await
        .unwrap();
    assert_eq!(
        exited,
        Response::Failed {
            code: "exited".into(),
            message: "session has exited".into(),
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_rejects_self_even_when_listed_in_control_from() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (self_id, self_token, self_addr) = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("self-controller"),
        Harness::Generic,
        Deliver::Hold,
        None,
        &[],
        &["self-controller"],
    )
    .await;
    let mut self_client = Client::connect_addr(self_addr, &self_token).await.unwrap();
    assert_reset_not_permitted(&mut self_client, self_id).await;
    daemon.shutdown().await.unwrap();
}
