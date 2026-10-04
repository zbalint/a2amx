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
use a2amx::wire::{BridgeDown, BridgeUp, Request, Response};

struct DaemonChild(Option<Child>);

impl DaemonChild {
    fn start(dir: &Path, clock: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command
            .arg("--home")
            .arg(dir)
            .args([
                "daemon",
                "start",
                "--foreground",
                "--host-name",
                "host-a",
                "--listen",
                "127.0.0.1:0",
            ])
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

fn accepted(response: Response, id: &str, recipient_hold: Option<&str>) {
    assert_eq!(
        response,
        Response::Accepted {
            id: id.to_owned(),
            recipient_hold: recipient_hold.map(str::to_owned),
            recipient_quota: None,
        }
    );
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
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("override".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
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
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
        },
        Request::Attach {
            session: sender_id.clone(),
            force: false,
            cols: 40,
            rows: 10,
            status: false,
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
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some(name.to_owned()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                heartbeat: None,
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
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("natural".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            heartbeat: None,
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

#[tokio::test]
async fn reset_rejects_an_in_flight_native_delivery_as_busy() {
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
    let output = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let target = match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; printf '\\033[?2004h'; sleep 30".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.path().to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec!["controller".into()],
            watch: vec![],
            name: Some("target".into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected target response: {response:?}"),
    };
    let credentials = common::eventually(|| async {
        let text = std::fs::read_to_string(output.path()).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let address = lines.next()?.parse::<std::net::SocketAddr>().ok()?;
        (token.len() == 64).then_some((token, address))
    })
    .await;
    let mut bridge = Client::connect_addr(daemon.addrs()[0], &credentials.0)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    bridge
        .send(BridgeUp::Hello {
            protocol: 1,
            omp_version: "18.4.5".into(),
            missing: vec![],
        })
        .await
        .unwrap();
    let ready = bridge.recv().await.unwrap();
    assert!(matches!(ready, Some(BridgeDown::Ready)), "{ready:?}");
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: false,
        })
        .await
        .unwrap();
    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let message = send(&mut sender, "target@host-a", "native", "queued").await;
    let message_id = match message {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    assert!(matches!(
        bridge.recv().await.unwrap(),
        Some(BridgeDown::Deliver { id, .. }) if id == message_id
    ));
    let mut resetter = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let response = resetter
        .request(Request::Reset { session: target })
        .await
        .unwrap();
    assert!(matches!(
        response,
        Response::Failed { code, .. } if code == "busy"
    ));
    bridge
        .send(BridgeUp::Nack {
            id: message_id,
            reason: "test".into(),
        })
        .await
        .unwrap();
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
        Some("deliver_hold"),
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
        Some("deliver_hold"),
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
