#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::emulator::Scroll;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{self, Limits, RemoteMode, code};
use a2amx::wire::{ClientFrame, Request, Response, ServerFrame};
use common::{
    create_confined_with_token, gateway, issue_and_redeem, new_session_request,
    start_confined_daemon_with_limits, team_scope,
};
use std::time::{Duration, Instant};

fn instant(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

#[test]
fn presence_starts_away_and_grants_grace_after_last_interactive_ends() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(60);
    assert!(!presence.present(start));

    presence.attach_started(start, false);
    assert!(presence.present(start));
    presence.attach_ended(start + Duration::from_secs(1), false);
    assert!(presence.present(start + Duration::from_secs(60)));
    assert!(!presence.present(start + Duration::from_secs(61)));
}

#[test]
fn zero_away_seconds_is_immediate() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(0);
    presence.attach_started(start, false);
    assert!(presence.present(start));
    presence.attach_ended(start, false);
    assert!(!presence.present(start));
}

#[test]
fn viewer_attachments_do_not_count_as_presence() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(60);
    presence.attach_started(start, true);
    assert!(!presence.present(start));
    presence.mode_changed(start + Duration::from_secs(1), false);
    assert!(presence.present(start + Duration::from_secs(1)));
    presence.mode_changed(start + Duration::from_secs(2), true);
    assert!(presence.present(start + Duration::from_secs(2)));
    assert!(!presence.present(start + Duration::from_secs(62)));
}

#[test]
fn multiple_interactive_attachments_keep_presence_until_the_last_ends() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(10);
    presence.attach_started(start, false);
    presence.attach_started(start + Duration::from_secs(1), false);
    presence.attach_ended(start + Duration::from_secs(2), false);
    assert!(presence.present(start + Duration::from_secs(2)));
    presence.attach_ended(start + Duration::from_secs(3), false);
    assert!(presence.present(start + Duration::from_secs(12)));
    assert!(!presence.present(start + Duration::from_secs(13)));
}

#[test]
fn poll_announces_each_present_to_away_transition_once() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(3);
    assert!(!presence.poll(start));
    presence.attach_started(start, false);
    assert!(!presence.poll(start));
    presence.attach_ended(start + Duration::from_secs(1), false);
    assert!(!presence.poll(start + Duration::from_secs(2)));
    assert!(presence.poll(start + Duration::from_secs(5)));
    assert!(!presence.poll(start + Duration::from_secs(6)));
    presence.attach_started(start + Duration::from_secs(7), false);
    assert!(!presence.poll(start + Duration::from_secs(7)));
    presence.attach_ended(start + Duration::from_secs(8), false);
    assert!(!presence.poll(start + Duration::from_secs(9)));
    assert!(presence.poll(start + Duration::from_secs(12)));
}

#[test]
fn five_grace_cycles_have_one_away_poll_transition() {
    let start = instant(0);
    let mut presence = messaging::Presence::new(20);
    presence.attach_started(start, false);
    assert!(!presence.poll(start));
    for cycle in 0..5 {
        let ended = start + Duration::from_secs(2 + cycle * 2);
        presence.attach_ended(ended, false);
        assert!(!presence.poll(ended + Duration::from_secs(1)));
        let restarted = ended + Duration::from_secs(2);
        presence.attach_started(restarted, false);
        assert!(!presence.poll(restarted));
    }
    let final_ended = start + Duration::from_secs(12);
    presence.attach_ended(final_ended, false);
    assert!(!presence.poll(final_ended + Duration::from_secs(19)));
    assert!(presence.poll(final_ended + Duration::from_secs(21)));
    assert!(!presence.poll(final_ended + Duration::from_secs(22)));
}

async fn owner_state(client: &mut Client) -> Option<String> {
    match client.request(Request::ListAgents).await.unwrap() {
        Response::Agents { owner, .. } => owner,
        response => panic!("unexpected list_agents response: {response:?}"),
    }
}

async fn session_held(admin: &mut Client, session: &str) -> bool {
    match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions
            .into_iter()
            .find(|candidate| candidate.id == session)
            .map(|candidate| candidate.held)
            .unwrap(),
        response => panic!("unexpected list response: {response:?}"),
    }
}

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

async fn receive_data(attachment: &mut a2amx::client::Attachment) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match attachment.recv().await.unwrap() {
                Some(ServerFrame::Data(_)) => break,
                Some(_) => {}
                None => panic!("viewer attachment closed"),
            }
        }
    })
    .await
    .expect("viewer receives redraw data");
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
            message: "the owner is at the computer; write to them in the terminal instead".into(),
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
async fn viewer_attach_does_not_resize_or_accept_input_until_mode_changes() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(Limits::default()).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = match admin
        .request(new_session_request(None, &["cat"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&id, false, 80, 20, 0)
        .await
        .unwrap();
    let viewer_agent = match admin.request(Request::ListAgents).await.unwrap() {
        Response::Agents { agents, .. } => {
            assert_eq!(agents.len(), 1);
            agents.into_iter().next().unwrap()
        }
        response => panic!("unexpected list_agents response: {response:?}"),
    };
    assert!(viewer_agent.attached);
    viewer
        .send(ClientFrame::Resize { cols: 80, rows: 20 })
        .await
        .unwrap();
    viewer
        .send(ClientFrame::Scroll(Scroll::LineUp))
        .await
        .unwrap();
    receive_data(&mut viewer).await;
    viewer.send(ClientFrame::Redraw).await.unwrap();
    receive_data(&mut viewer).await;
    viewer.send(ClientFrame::Release).await.unwrap();
    viewer
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let before = match admin
        .request(Request::Screen {
            session: id.clone(),
        })
        .await
        .unwrap()
    {
        Response::Screen { lines } => lines.join("\n"),
        response => panic!("screen failed: {response:?}"),
    };
    assert!(!before.contains('x'));
    viewer
        .send(ClientFrame::Mode { viewer: false })
        .await
        .unwrap();
    viewer
        .send(ClientFrame::Input(b"y".to_vec()))
        .await
        .unwrap();
    let mut found_y = false;
    for _ in 0..100 {
        let response = admin
            .request(Request::Screen {
                session: id.clone(),
            })
            .await
            .unwrap();
        let Response::Screen { lines } = response else {
            panic!("screen failed");
        };
        if lines.join("\n").contains('y') {
            found_y = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(found_y, "interactive mode input was not rendered");
    let sessions = match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("list failed: {response:?}"),
    };
    let session = sessions
        .into_iter()
        .find(|session| session.id == id)
        .unwrap();
    assert_eq!((session.cols, session.rows), (40, 10));
    drop(viewer);
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
    assert_eq!(owner_state(&mut observer).await.as_deref(), Some("present"));
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
#[tokio::test]
async fn release_clears_interactive_hold_but_viewer_release_is_ignored() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(Limits::default()).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = match admin
        .request(new_session_request(None, &["cat"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("session creation failed: {response:?}"),
    };
    let mut interactive = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 10)
        .await
        .unwrap();
    interactive
        .send(ClientFrame::Input(b"draft".to_vec()))
        .await
        .unwrap();
    let mut held = false;
    for _ in 0..100 {
        if session_held(&mut admin, &id).await {
            held = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(held, "interactive input creates a human-draft hold");
    interactive.send(ClientFrame::Release).await.unwrap();
    let mut released = false;
    for _ in 0..100 {
        if !session_held(&mut admin, &id).await {
            released = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(released, "interactive Release clears the hold");
    interactive
        .send(ClientFrame::Input(b"draft again".to_vec()))
        .await
        .unwrap();
    let mut held_again = false;
    for _ in 0..100 {
        if session_held(&mut admin, &id).await {
            held_again = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(held_again, "interactive input recreates the hold");
    let mut viewer = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_viewer(&id, true, 40, 10, 0)
        .await
        .unwrap();
    viewer.send(ClientFrame::Release).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(session_held(&mut admin, &id).await);
    drop(viewer);
    drop(interactive);
    daemon.shutdown().await.unwrap();
}
