use super::*;

async fn message_infos(admin: &mut Client, session: &str) -> Vec<a2amx::wire::MessageInfo> {
    match admin
        .request(Request::ListMessages {
            session: Some(session.to_owned()),
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("unexpected messages response: {response:?}"),
    }
}

fn envelope_attr<'a>(envelope: &'a str, name: &str) -> &'a str {
    let marker = format!("{name}=\"");
    let start = envelope.find(&marker).unwrap() + marker.len();
    let end = start + envelope[start..].find('"').unwrap();
    &envelope[start..end]
}

fn envelope_body(envelope: &str) -> String {
    let normalized = envelope.replace('\r', "");
    let start = normalized.find("<a2amx-message").unwrap();
    let closing = "</a2amx-message>";
    let end = normalized.rfind(closing).unwrap() + closing.len();
    let framed = &normalized[start..end];
    framed
        .split_once("\n\n")
        .and_then(|(_, body)| body.strip_suffix("\n</a2amx-message>"))
        .unwrap()
        .to_owned()
}

async fn message_subjects(admin: &mut Client, session: &str) -> Vec<String> {
    match admin
        .request(Request::ListMessages {
            session: Some(session.to_owned()),
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages
            .into_iter()
            .map(|message| message.subject)
            .collect(),
        response => panic!("unexpected messages response: {response:?}"),
    }
}

#[tokio::test]
async fn away_notice_is_sent_only_to_the_transitioning_session() {
    let limits = Limits {
        away_secs: 0,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture_a = tempfile::tempdir().unwrap();
    let fixture_b = tempfile::tempdir().unwrap();
    let team = team_scope("presence-notice-sessions");
    let (session_a, _token_a) = create_confined_with_token(
        &mut admin,
        fixture_a.path(),
        "notice-session-a",
        Some(RemoteMode::Both),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let (session_b, _token_b) = create_confined_with_token(
        &mut admin,
        fixture_b.path(),
        "notice-session-b",
        Some(RemoteMode::Both),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-notice-sessions").await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&session_a, false, 40, 10)
        .await
        .unwrap();
    let second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&session_b, false, 40, 10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    first
        .send(ClientFrame::Mode { viewer: true })
        .await
        .unwrap();
    first.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut first).await;
    let mut a_notice = false;
    for _ in 0..100 {
        if message_subjects(&mut admin, &session_a)
            .await
            .iter()
            .any(|subject| subject == messaging::PRESENCE_SUBJECT)
        {
            a_notice = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(a_notice, "transitioning session receives owner-away notice");
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert!(
        !message_subjects(&mut admin, &session_b)
            .await
            .iter()
            .any(|subject| subject == messaging::PRESENCE_SUBJECT)
    );
    drop(second);
    drop(first);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn away_notice_is_fixed_and_sent_once_to_remote_sessions_only() {
    let limits = Limits {
        away_secs: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let sender_team = team_scope("presence-notice");
    let (sender_id, _token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "notice-sender",
        Some(RemoteMode::Both),
        Some(sender_team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-notice").await;
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(owner_state(&mut admin).await, None);
    let initial_subjects = message_subjects(&mut admin, &sender_id).await;
    assert!(
        !initial_subjects
            .iter()
            .any(|subject| subject == messaging::PRESENCE_SUBJECT)
    );
    let local_fixture = tempfile::tempdir().unwrap();
    let (non_remote, non_remote_token) = create_confined_with_token(
        &mut admin,
        local_fixture.path(),
        "notice-local",
        None,
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut non_remote_client = Client::connect_addr(daemon.addrs()[0], &non_remote_token)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut non_remote_client).await, None);
    let local_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&non_remote, false, 40, 10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    drop(local_attachment);
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 40, 10)
        .await
        .unwrap();
    drop(attachment);
    let mut subjects = None;
    for _ in 0..100 {
        let current = message_subjects(&mut admin, &sender_id).await;
        if current
            .iter()
            .any(|subject| subject == messaging::PRESENCE_SUBJECT)
        {
            subjects = Some(current);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let subjects = subjects.expect("owner-away notice arrives");
    assert_eq!(
        subjects
            .iter()
            .filter(|subject| subject.as_str() == messaging::PRESENCE_SUBJECT)
            .count(),
        1
    );
    let local_subjects = message_subjects(&mut admin, &non_remote).await;
    assert!(
        !local_subjects
            .iter()
            .any(|subject| subject == messaging::PRESENCE_SUBJECT)
    );
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 40, 10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let returned_subjects = message_subjects(&mut admin, &sender_id).await;
    assert_eq!(
        returned_subjects
            .iter()
            .filter(|subject| subject.as_str() == messaging::PRESENCE_SUBJECT)
            .count(),
        1
    );
    drop(attachment);
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let subjects_after = message_subjects(&mut admin, &sender_id).await;
    assert_eq!(
        subjects_after
            .iter()
            .filter(|subject| subject.as_str() == messaging::PRESENCE_SUBJECT)
            .count(),
        1
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn away_notice_body_matches_the_fixed_operator_message() {
    let limits = Limits {
        away_secs: 0,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let output = tempfile::NamedTempFile::new_in(fixture.path()).unwrap();
    let sender_id = match admin
        .request(Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf '\\033[?2004h'; exec cat > \"$OUT\"".into(),
                ],
                cols: 120,
                rows: 10,
                cwd: Some(fixture.path().to_string_lossy().into_owned()),
                env: vec![("OUT".into(), output.path().to_string_lossy().into_owned())],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("notice-body".into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Auto),
                team: Some(team_scope("presence-body")),
                role: None,
                remote: Some(RemoteMode::Both),
                activity: None,
                heartbeat: None,
            }),
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "presence-body").await;
    let observer_fixture = tempfile::tempdir().unwrap();
    let (_observer_id, observer_token) = create_confined_with_token(
        &mut admin,
        observer_fixture.path(),
        "presence-observer",
        Some(RemoteMode::Both),
        Some(team_scope("presence-body")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut observer = Client::connect_addr(daemon.addrs()[0], &observer_token)
        .await
        .unwrap();
    assert_eq!(owner_state(&mut observer).await.as_deref(), Some("away"));
    assert!(
        message_infos(&mut admin, &sender_id)
            .await
            .iter()
            .all(|message| message.subject != messaging::PRESENCE_SUBJECT)
    );
    let attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, false, 120, 10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(owner_state(&mut observer).await.as_deref(), Some("away"));
    assert!(
        message_infos(&mut admin, &sender_id)
            .await
            .iter()
            .all(|message| message.subject != messaging::PRESENCE_SUBJECT)
    );
    drop(attachment);
    let viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&sender_id, false, 120, 10, 0)
        .await
        .unwrap();
    let mut body = None;
    for _ in 0..100 {
        let text = std::fs::read_to_string(output.path()).unwrap_or_default();
        if text.contains("The owner is away from the computer. You may message them with send_message(to=\"human:owner\").")
            && text.contains("</a2amx-message>")
        {
            body = Some(text);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let body = body.expect("owner-away body arrives");
    const EXPECTED_BODY: &str = "The owner is away from the computer. You may message them with send_message(to=\"human:owner\").";
    assert_eq!(envelope_attr(&body, "from"), "a2amx-daemon@host-a");
    assert_eq!(envelope_attr(&body, "subject"), messaging::PRESENCE_SUBJECT);
    assert_eq!(envelope_body(&body), EXPECTED_BODY);
    assert_eq!(messaging::OWNER_AWAY_BODY, EXPECTED_BODY);
    let notices = message_infos(&mut admin, &sender_id)
        .await
        .into_iter()
        .filter(|message| message.subject == messaging::PRESENCE_SUBJECT)
        .collect::<Vec<_>>();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].from, "a2amx-daemon@host-a");
    let returned = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&sender_id, true, 120, 10)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    let returned_infos = message_infos(&mut admin, &sender_id).await;
    assert_eq!(
        returned_infos
            .iter()
            .filter(|message| message.subject == messaging::PRESENCE_SUBJECT)
            .count(),
        1
    );
    drop(returned);
    drop(viewer);
    daemon.shutdown().await.unwrap();
}
