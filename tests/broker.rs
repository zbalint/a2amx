#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::Limits;
use a2amx::wire::{Request, Response};

struct DaemonChild(Option<Child>);

impl DaemonChild {
    fn start(dir: &Path, clock: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command
            .arg("--home")
            .arg(dir)
            .args(["daemon", "--host-name", "host-a", "--listen", "127.0.0.1:0"])
            .stdout(std::process::Stdio::null());
        if let Some(clock) = clock {
            command.env("LD_PRELOAD", clock);
        }
        Self(Some(command.spawn().expect("daemon child")))
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn session(
    admin: &mut Client,
    dir: &Path,
    name: Option<&str>,
    harness: Harness,
    deliver: Deliver,
) -> (String, String, Client) {
    let (id, token, address) = common::new_agent(admin, dir, name, harness, deliver).await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    (id, token, client)
}

fn error_message(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        other => panic!("expected error, got {other:?}"),
    }
}

fn failed_code(response: Response) -> String {
    match response {
        Response::Failed { code, .. } => code,
        other => panic!("expected failed response, got {other:?}"),
    }
}

fn accepted(response: Response, id: &str) {
    assert_eq!(response, Response::Accepted { id: id.to_owned() });
}

fn assert_secure_database(dir: &Path) {
    assert_eq!(
        std::fs::metadata(dir.join("messages.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for suffix in ["-wal", "-shm"] {
        let path = dir.join(format!("messages.db{suffix}"));
        if path.exists() {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

async fn send(client: &mut Client, to: &str, subject: &str, message: &str) -> Response {
    client
        .request(Request::SendMessage {
            to: to.to_owned(),
            subject: subject.to_owned(),
            message: message.to_owned(),
        })
        .await
        .unwrap()
}

async fn status(client: &mut Client, id: &str) -> Response {
    client
        .request(Request::MessageStatus { id: id.to_owned() })
        .await
        .unwrap()
}

async fn wait_agents(
    client: &mut Client,
    predicate: impl Fn(&[a2amx::wire::AgentSummary]) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match client.request(Request::ListAgents).await.unwrap() {
                Response::Agents { agents } if predicate(&agents) => return,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("agent state becomes observable");
}

async fn wait_status(
    client: &mut Client,
    id: &str,
    predicate: impl Fn(&a2amx::wire::MessageInfo) -> bool,
) -> a2amx::wire::MessageInfo {
    let id = id.to_owned();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match status(client, &id).await {
                Response::Status { message } if predicate(&message) => return message,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("message state becomes observable")
}

#[tokio::test]
async fn session_tokens_roles_permissions_and_child_identity() {
    let (dir, daemon) = common::start_daemon().await;
    let admin_token = std::fs::read_to_string(dir.path().join("admin.token"))
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
            name: Some("override".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
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
        Request::List,
        Request::NewSession {
            argv: vec!["sh".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
        },
        Request::Attach {
            session: sender_id.clone(),
            force: false,
            cols: 40,
            rows: 10,
        },
        Request::Kill {
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
        Response::Agents { agents } => agents,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(agents.iter().any(|agent| {
        agent.address == "agent-plan@host-a" && agent.state == "running" && !agent.attached
    }));
    assert!(agents.iter().any(|agent| {
        agent.address == "s2@host-a" && agent.state == "running" && !agent.attached
    }));

    for name in ["s12", "Agent", &"a".repeat(64)] {
        let response = admin
            .request(Request::NewSession {
                argv: vec!["sh".into()],
                cols: 40,
                rows: 10,
                cwd: None,
                env: vec![],
                name: Some(name.to_owned()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
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
            name: Some("agent-plan".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
        })
        .await
        .unwrap();
    assert!(error_message(duplicate).contains("already in use"));

    assert_eq!(
        admin
            .request(Request::Kill { session: named_id })
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
            name: Some("natural".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
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
        Response::Agents { agents } => agents,
        response => panic!("unexpected response: {response:?}"),
    };
    assert!(role_agents.iter().any(|agent| {
        agent.address == "natural@host-a" && agent.state == "exited" && !agent.attached
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
            name: Some("natural".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
        })
        .await
        .unwrap();
    assert!(error_message(duplicate_after_natural).contains("already in use"));
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
#[tokio::test]
async fn pending_limit_and_content_errors() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            max_pending_per_recipient: 2,
            rate_per_minute: 100,
            max_stored_messages: 100,
            retention_secs: 7 * 24 * 3600,
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
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "queue_full"
    );
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
            max_stored_messages: 2,
            retention_secs: 7 * 24 * 3600,
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
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
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
            max_stored_messages: 100,
            retention_secs: 7 * 24 * 3600,
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
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
    );
    assert_eq!(
        failed_code(send(&mut sender, "recipient@host-a", "three", "body").await),
        "rate_limited"
    );
    accepted(
        send(&mut other, "recipient@host-a", "other", "body").await,
        "m_3",
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
            name: Some("recipient".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
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
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
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
    );
    accepted(
        send(&mut sender, "recipient@host-a", "two", "body").await,
        "m_2",
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

#[tokio::test]
async fn retention_zero_and_database_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            retention_secs: 0,
            ..Limits::default()
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
    accepted(
        send(&mut sender, "recipient@host-a", "one", "body").await,
        "m_1",
    );
    assert_secure_database(dir.path());
    daemon.shutdown().await.unwrap();

    let daemon = common::start_daemon_in(
        dir.path(),
        Some("host-a"),
        Limits {
            retention_secs: 0,
            ..Limits::default()
        },
    )
    .await;
    let mut new_admin = Client::connect(dir.path()).await.unwrap();
    match new_admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => assert!(messages.is_empty()),
        response => panic!("unexpected response: {response:?}"),
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn abrupt_binary_restart_recovers_pending_as_daemon_restarted() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = DaemonChild::start(dir.path(), None);
    let address = common::eventually(|| async {
        std::fs::read_to_string(dir.path().join("addr"))
            .ok()
            .and_then(|text| text.lines().next()?.parse::<SocketAddr>().ok())
    })
    .await;
    let admin_token = std::fs::read_to_string(dir.path().join("admin.token"))
        .unwrap()
        .trim()
        .to_owned();
    let mut admin = Client::connect_addr(address, &admin_token).await.unwrap();
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
    );
    child.kill();

    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut new_admin = Client::connect(dir.path()).await.unwrap();
    let message = match status(&mut new_admin, "m_1").await {
        Response::Status { message } => message,
        response => panic!("unexpected response: {response:?}"),
    };
    assert_eq!(message.state, "undeliverable");
    assert_eq!(message.detail.as_deref(), Some("daemon_restarted"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn storage_failure_returns_internal_without_acceptance_or_content_leak() {
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
    let _ = session(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let wal = dir.path().join("messages.db-wal");
    let original = dir.path().join("messages.db-wal.original");
    std::fs::rename(&wal, &original).unwrap();
    std::os::unix::fs::symlink("messages.db-wal", &wal).unwrap();
    let response = send(
        &mut sender,
        "recipient@host-a",
        "private subject",
        "private body",
    )
    .await;
    std::fs::remove_file(&wal).unwrap();
    std::fs::rename(&original, &wal).unwrap();
    let Response::Failed { code, message } = response else {
        panic!("storage failure was accepted")
    };
    assert_eq!(code, "internal");
    assert!(!message.contains("private subject"));
    assert!(!message.contains("private body"));
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("message list")
    };
    assert!(
        messages.is_empty(),
        "a failed acceptance must store nothing"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn migrates_v1_database_and_preserves_submitted_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let database = rusqlite::Connection::open(dir.path().join("messages.db")).unwrap();
    database
        .execute_batch(
            "CREATE TABLE messages (
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               boot TEXT NOT NULL,
               sender_session TEXT NOT NULL,
               sender_address TEXT NOT NULL,
               recipient_session TEXT NOT NULL,
               recipient_address TEXT NOT NULL,
               subject TEXT NOT NULL,
               body TEXT NOT NULL,
               state TEXT NOT NULL,
               detail TEXT,
               accepted_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE INDEX messages_recipient
               ON messages (boot, recipient_session, state, seq);
             CREATE TABLE attempts (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               message_seq INTEGER NOT NULL REFERENCES messages(seq),
               started_at INTEGER NOT NULL,
               outcome TEXT NOT NULL,
               detail TEXT,
               finished_at INTEGER
             );
             PRAGMA user_version = 1;
             INSERT INTO messages
               (seq, boot, sender_session, sender_address, recipient_session,
                recipient_address, subject, body, state, detail, accepted_at, updated_at)
             VALUES
               (1, 'boot-v1', 's1', 'sender@host-a', 's2', 'recipient@host-a',
                'subject', 'body', 'submitted', NULL, 2000000000, 2000000000);
             INSERT INTO attempts
               (message_seq, started_at, outcome, detail, finished_at)
             VALUES (1, 2000000000, 'submitted', NULL, 2000000000);",
        )
        .unwrap();
    drop(database);

    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let messages = match admin
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
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].id, "m_1");
    assert_eq!(messages[0].state, "submitted");
    assert_eq!(messages[0].evidence.as_deref(), Some("write_complete"));

    daemon.shutdown().await.unwrap();
    let database = rusqlite::Connection::open(dir.path().join("messages.db")).unwrap();
    let version: i64 = database
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 2);
}

#[tokio::test]
async fn daemon_refuses_a_future_database_schema() {
    let dir = tempfile::tempdir().unwrap();
    // The database is a startup input fixture, not an assertion side channel.
    let database = rusqlite::Connection::open(dir.path().join("messages.db")).unwrap();
    database.pragma_update(None, "user_version", 3).unwrap();
    drop(database);
    let error = a2amx::daemon::Daemon::start(a2amx::daemon::DaemonConfig {
        state_dir: dir.path().to_owned(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
    })
    .await
    .err()
    .expect("future schema must fail startup");
    assert_eq!(
        error.to_string(),
        "unsupported messages.db schema version 3"
    );
    assert!(
        !dir.path().join("admin.token").exists(),
        "a failed start must not leave a token behind"
    );
}

#[tokio::test]
async fn retention_includes_the_exact_nonzero_expiry_boundary() {
    let (dir, bootstrap) = common::start_daemon().await;
    bootstrap.shutdown().await.unwrap();
    let path = dir.path().to_owned();
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/frozen_realtime.c");
    let mut process = tokio::task::spawn_blocking(move || {
        let library = path.join("frozen_realtime.so");
        let compiled = Command::new("cc")
            .args(["-shared", "-fPIC", "-Wall", "-Wextra", "-Werror", "-o"])
            .arg(&library).arg(fixture).arg("-ldl").output().unwrap();
        assert!(compiled.status.success(), "clock fixture compiler: {}", String::from_utf8_lossy(&compiled.stderr));
        let database = rusqlite::Connection::open(path.join("messages.db")).unwrap();
        // Persisted startup input only. At fixed now=2000000000 and the default
        // 604800-second TTL: m_1 is exactly expired, m_2 has one second left,
        // m_4 is strictly expired, and recovery gives m_3 a fresh updated_at.
        database.execute_batch(
            "INSERT INTO messages
             (seq,boot,sender_session,sender_address,recipient_session,recipient_address,subject,body,state,accepted_at,updated_at)
             VALUES
             (1,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','At expiry','Fixture','submitted',1999395200,1999395200),
             (2,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Before expiry','Fixture','cancelled',1999395201,1999395201),
             (3,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Interrupted','Fixture','delivering',1999395199,1999395199),
             (4,'0000000000000000','s1','agent-plan@host-a','s2','agent-review@host-a','Expired','Fixture','submitted',1999395199,1999395199);
             INSERT INTO attempts (message_seq,started_at,finished_at,outcome) VALUES
             (1,1999395200,1999395200,'submitted'),
             (3,1999395199,NULL,'started'),
             (4,1999395199,1999395199,'submitted');"
        ).unwrap();
        drop(database);
        DaemonChild::start(&path, Some(&library))
    }).await.unwrap();
    let mut admin = common::eventually(|| async { Client::connect(dir.path()).await.ok() }).await;
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("message list")
    };
    // This witness catches a missing preload rather than misclassifying fixture
    // setup failure as a retention-boundary reproduction.
    assert!(
        messages.iter().all(|message| message.id != "m_4"),
        "fixed clock must expire a strictly old row"
    );
    assert_eq!(
        messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>(),
        ["m_2", "m_3"]
    );
    assert_eq!(messages[0].state, "cancelled");
    assert_eq!(messages[1].state, "undeliverable");
    assert_eq!(messages[1].detail.as_deref(), Some("daemon_restarted"));
    let mut child = process.0.take().unwrap();
    let status = tokio::task::spawn_blocking(move || {
        let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
        rustix::process::kill_process(pid, rustix::process::Signal::TERM).unwrap();
        child.wait().unwrap()
    })
    .await
    .unwrap();
    assert!(status.success());
}
