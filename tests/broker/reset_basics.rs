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
