#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::TeamScope;
use a2amx::wire::{Request, Response};

fn scope(name: &str, private: bool, allow: &[&str]) -> TeamScope {
    TeamScope {
        name: name.into(),
        private,
        allow: allow.iter().map(|entry| (*entry).to_owned()).collect(),
    }
}

async fn session(
    admin: &mut Client,
    dir: &std::path::Path,
    name: &str,
    team: Option<TeamScope>,
    watch: &[&str],
    control_from: &[&str],
) -> (String, Client) {
    let (_, token, address) = common::new_team_agent(
        admin,
        dir,
        Some(name),
        Harness::Generic,
        Deliver::Hold,
        team,
        watch,
        control_from,
    )
    .await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    let id = loop {
        let response = admin.request(Request::List).await.unwrap();
        if let Response::Sessions { sessions } = response {
            if let Some(session) = sessions
                .into_iter()
                .find(|session| session.name.as_deref() == Some(name))
            {
                break session.id;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    (id, client)
}

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
            heartbeat: None,
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

    let Response::Agents { agents } = a.request(Request::ListAgents).await.unwrap() else {
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
    let Response::Agents { agents } = admin.request(Request::ListAgents).await.unwrap() else {
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
    let Response::Agents { agents } = b.request(Request::ListAgents).await.unwrap() else {
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
        if let Response::Messages { messages } = response {
            if let Some(message) = messages
                .into_iter()
                .find(|message| message.subject.contains("same-peer"))
            {
                break message;
            }
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
            heartbeat: None,
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
            heartbeat: None,
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
            heartbeat: None,
        })
        .await
        .unwrap();
    assert!(matches!(after_exit, Response::Created { .. }));
}
