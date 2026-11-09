#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::gateway::{Event, Gateway, MAX_CHAT_CHARS, Next, UNREACHABLE_LINE, split_message};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{HUMAN_SESSION, HUMAN_SUBJECT, RemoteMode};
use a2amx::wire::{Request, Response};
use common::{
    create_confined_with_team, create_confined_with_token, eventually, gateway as gateway_client,
    issue_and_redeem, issue_code, new_session_request_with_remote, start_confined_daemon,
    start_confined_daemon_with_limits, team_scope,
};
use proptest::prelude::*;
use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

fn event(channel: &str, message_id: u64, author: &str, text: &str) -> Event {
    Event {
        channel: channel.into(),
        message_id,
        author: author.into(),
        text: text.into(),
    }
}

struct FakeChat {
    events: Vec<Event>,
    posted: Vec<String>,
}

impl FakeChat {
    fn new(events: Vec<Event>) -> Self {
        Self {
            events,
            posted: Vec::new(),
        }
    }

    async fn pump(&mut self, gateway: &mut Gateway, daemon: &mut Client) -> anyhow::Result<()> {
        let events = std::mem::take(&mut self.events);
        for event in events {
            if let Some(line) = gateway.inbound(daemon, event).await? {
                self.posted.push(line);
            }
        }
        Ok(())
    }
}

fn gateway_state(dir: &Path) -> std::path::PathBuf {
    dir.join("gateway-seen.json")
}

async fn list_messages(admin: &mut Client) -> Vec<a2amx::wire::MessageInfo> {
    match admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages,
        response => panic!("expected messages, got {response:?}"),
    }
}

#[test]
fn split_message_uses_newline_then_space_and_never_splits_scalars() {
    assert_eq!(split_message("one\ntwo three", 7), ["one", "two", "three"]);
    assert_eq!(split_message("abcdefghij", 5), ["abcde", "fghij"]);
    let text = "é".repeat(MAX_CHAT_CHARS + 1);
    let pieces = split_message(&text, MAX_CHAT_CHARS);
    assert_eq!(pieces.len(), 2);
    assert!(
        pieces
            .iter()
            .all(|piece| piece.chars().count() <= MAX_CHAT_CHARS)
    );
    assert_eq!(pieces.concat(), text);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn split_message_preserves_non_whitespace_chars(
        text in "[\\u{0}-\\u{10ffff}]{0,300}",
        max_chars in 1usize..=80,
    ) {
        let pieces = split_message(&text, max_chars);
        prop_assert!(pieces.iter().all(|piece| !piece.is_empty()));
        prop_assert!(pieces.iter().all(|piece| piece.chars().count() <= max_chars));
        let input_non_whitespace: String = text.chars().filter(|character| !character.is_whitespace()).collect();
        let output_non_whitespace: String = pieces.iter().flat_map(|piece| piece.chars()).filter(|character| !character.is_whitespace()).collect();
        prop_assert_eq!(output_non_whitespace, input_non_whitespace);
    }
}

#[tokio::test]
async fn gateway_agents_lists_only_running_confined_leased_remote_agents() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _eligible_both = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "eligible-both",
        Some(RemoteMode::Both),
        Some(team_scope("eligible-team")),
    )
    .await;
    let _eligible_send = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "eligible-send",
        Some(RemoteMode::Send),
        Some(team_scope("eligible-team")),
    )
    .await;
    let _no_lease = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "no-lease",
        Some(RemoteMode::Both),
        Some(team_scope("unleased-team")),
    )
    .await;
    let _no_remote = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "no-remote",
        None,
        Some(team_scope("eligible-team")),
    )
    .await;
    let mut unconfined_request = new_session_request_with_remote(
        Some("unconfined-remote"),
        &["sleep", "30"],
        Some(RemoteMode::Both),
    );
    if let Request::NewSession { team, .. } = &mut unconfined_request {
        team.replace(team_scope("eligible-team"));
    }
    let _unconfined = admin.request(unconfined_request).await.unwrap();
    let exited = match admin
        .request(Request::Confine {
            confine: a2amx::confine::ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec!["sleep".into(), "1".into()],
                cols: 40,
                rows: 10,
                cwd: Some(fixture.path().to_string_lossy().into_owned()),
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some("exited-remote".into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                team: Some(team_scope("eligible-team")),
                role: None,
                remote: Some(RemoteMode::Both),
            }),
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("expected exited session creation, got {response:?}"),
    };

    let mut gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "eligible-team").await;
    let state_dir = dir.path().to_path_buf();
    eventually(|| {
        let state_dir = state_dir.clone();
        let session = exited.clone();
        async move {
            let mut explain_client = Client::connect(&state_dir).await.unwrap();
            match explain_client
                .request(Request::Explain { session })
                .await
                .unwrap()
            {
                Response::Explain { info } if info.exit_code.is_some() => Some(()),
                _ => None,
            }
        }
    })
    .await;
    let response = gateway.request(Request::GatewayAgents).await.unwrap();
    let Response::GatewayAgents { agents } = response else {
        panic!("expected gateway agents");
    };
    let addresses: HashSet<_> = agents.iter().map(|agent| agent.address.as_str()).collect();
    assert_eq!(
        addresses,
        HashSet::from(["eligible-both@host-a", "eligible-send@host-a"])
    );
    assert_eq!(
        agents
            .iter()
            .find(|agent| agent.name == "eligible-both")
            .map(|agent| agent.mode.as_str()),
        Some("both")
    );
    assert_eq!(
        agents
            .iter()
            .find(|agent| agent.name == "eligible-send")
            .map(|agent| agent.mode.as_str()),
        Some("send")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_filters_author_and_deduplicates_before_delivery_across_restart() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "dedup-target",
        Some(RemoteMode::Both),
        Some(team_scope("dedup-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "dedup-team").await;
    let state_path = gateway_state(dir.path());
    let mut gateway =
        Gateway::new(state_path.clone(), "paired-owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("agent".into(), "dedup-target@host-a".into())
        .unwrap();

    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("agent", 1, "someone-else", "ignored"),
            )
            .await
            .unwrap(),
        None
    );
    assert!(!state_path.exists());
    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("agent", 1, "paired-owner", "first"),
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("agent", 1, "paired-owner", "duplicate"),
            )
            .await
            .unwrap(),
        None
    );
    let messages = list_messages(&mut admin).await;
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.to == "dedup-target@host-a")
            .count(),
        1
    );

    let mut restarted = Gateway::new(state_path, "paired-owner".into(), "control".into()).unwrap();
    restarted
        .bind_channel("agent".into(), "dedup-target@host-a".into())
        .unwrap();
    assert_eq!(
        restarted
            .inbound(
                &mut daemon_gateway,
                event("agent", 1, "paired-owner", "replayed"),
            )
            .await
            .unwrap(),
        None
    );
    let messages = list_messages(&mut admin).await;
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.to == "dedup-target@host-a")
            .count(),
        1
    );
    daemon.shutdown().await.unwrap();
}

#[test]
fn gateway_rejects_corrupt_or_oversized_dedup_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = gateway_state(dir.path());
    fs::write(&path, b"not json").unwrap();
    assert!(Gateway::new(path.clone(), "owner".into(), "control".into()).is_err());
    fs::write(&path, vec![b'x'; 1024 * 1024 + 1]).unwrap();
    assert!(Gateway::new(path, "owner".into(), "control".into()).is_err());
}

#[tokio::test]
async fn gateway_routes_each_bound_channel_to_its_own_agent() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _first = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "channel-a-agent",
        Some(RemoteMode::Both),
        Some(team_scope("channel-team")),
    )
    .await;
    let _second = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "channel-b-agent",
        Some(RemoteMode::Both),
        Some(team_scope("channel-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "channel-team").await;
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("A".into(), "channel-a-agent@host-a".into())
        .unwrap();
    gateway
        .bind_channel("B".into(), "channel-b-agent@host-a".into())
        .unwrap();
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("A", 1, "owner", "to A"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("B", 1, "owner", "to B"))
            .await
            .unwrap(),
        None
    );
    let messages = list_messages(&mut admin).await;
    assert!(
        messages
            .iter()
            .any(|message| message.to == "channel-a-agent@host-a"
                && message.subject == HUMAN_SUBJECT)
    );
    assert!(
        messages
            .iter()
            .any(|message| message.to == "channel-b-agent@host-a"
                && message.subject == HUMAN_SUBJECT)
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_posts_fixed_lines_for_unbound_and_control_channels() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "control-target",
        Some(RemoteMode::Both),
        Some(team_scope("control-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    let code = issue_code(&mut admin, "control-team").await;
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    assert!(
        gateway
            .bind_channel("control".into(), "control-target@host-a".into())
            .is_err()
    );
    let mut chat = FakeChat::new(vec![
        event("unbound", 1, "owner", "hello"),
        event("control", 1, "owner", "not a code"),
        event("control", 2, "owner", "ABCDEF012345"),
    ]);
    chat.pump(&mut gateway, &mut daemon_gateway).await.unwrap();
    assert_eq!(
        chat.posted,
        [
            "that channel is not linked to a running agent",
            "send the code that a2amx team up printed",
            "send the code that a2amx team up printed",
        ]
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("control", 3, "owner", &code))
            .await
            .unwrap()
            .as_deref(),
        Some("remote messaging is on")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_maps_daemon_refusals_without_copying_daemon_text() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _send_only = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "send-only-target",
        Some(RemoteMode::Send),
        Some(team_scope("refusal-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "refusal-team").await;
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("not-permitted".into(), "send-only-target@host-a".into())
        .unwrap();
    gateway
        .bind_channel("unknown".into(), "missing@host-a".into())
        .unwrap();
    gateway
        .bind_channel("invalid".into(), "send-only-target@host-a".into())
        .unwrap();
    gateway
        .bind_channel("too-large".into(), "send-only-target@host-a".into())
        .unwrap();

    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("not-permitted", 1, "owner", "text")
            )
            .await
            .unwrap()
            .as_deref(),
        Some("that agent is not taking messages from you")
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("unknown", 1, "owner", "text"))
            .await
            .unwrap()
            .as_deref(),
        Some("that agent is not running")
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("invalid", 1, "owner", ""),)
            .await
            .unwrap()
            .as_deref(),
        Some("the message could not be delivered")
    );
    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("too-large", 1, "owner", &"x".repeat(8_001)),
            )
            .await
            .unwrap()
            .as_deref(),
        Some("that message is too long")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_maps_rate_queue_and_exited_refusals() {
    let limits = a2amx::messaging::Limits {
        human_inbound_per_minute: 1,
        ..Default::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _rate_target = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "rate-target",
        Some(RemoteMode::Both),
        Some(team_scope("rate-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "rate-team").await;
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("rate".into(), "rate-target@host-a".into())
        .unwrap();
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("rate", 1, "owner", "first"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("rate", 2, "owner", "second"))
            .await
            .unwrap()
            .as_deref(),
        Some("too many messages, wait a minute")
    );
    daemon.shutdown().await.unwrap();

    let limits = a2amx::messaging::Limits {
        max_pending_per_recipient: 1,
        ..Default::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _queue_target = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "queue-target",
        Some(RemoteMode::Both),
        Some(team_scope("queue-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "queue-team").await;
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("queue".into(), "queue-target@host-a".into())
        .unwrap();
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("queue", 1, "owner", "first"))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("queue", 2, "owner", "second"))
            .await
            .unwrap()
            .as_deref(),
        Some("that agent has too many unread messages")
    );
    daemon.shutdown().await.unwrap();

    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let exited = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "exited-target",
        Some(RemoteMode::Both),
        Some(team_scope("exited-team")),
    )
    .await;
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "exited-team").await;
    assert!(matches!(
        admin
            .request(Request::Kill {
                session: exited,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    ));
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    gateway
        .bind_channel("exited".into(), "exited-target@host-a".into())
        .unwrap();
    assert_eq!(
        gateway
            .inbound(&mut daemon_gateway, event("exited", 1, "owner", "text"))
            .await
            .unwrap()
            .as_deref(),
        Some("that agent is not running")
    );
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn gateway_persists_before_reporting_daemon_unreachable() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let address = daemon.addrs()[0];
    let mut daemon_gateway = gateway_client(dir.path(), address).await;
    let state_path = gateway_state(dir.path());
    let mut gateway = Gateway::new(state_path.clone(), "owner".into(), "control".into()).unwrap();
    daemon.shutdown().await.unwrap();
    assert!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("control", 1, "owner", "000000000000"),
            )
            .await
            .is_err()
    );
    let mut restarted = Gateway::new(state_path, "owner".into(), "control".into()).unwrap();
    assert_eq!(
        restarted
            .inbound(
                &mut daemon_gateway,
                event("control", 1, "owner", "000000000000")
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(UNREACHABLE_LINE, "a2amx is not reachable right now");
}

#[tokio::test]
async fn gateway_state_file_is_owner_only_after_write() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    let path = gateway_state(dir.path());
    let mut gateway = Gateway::new(path.clone(), "owner".into(), "control".into()).unwrap();
    assert_eq!(
        gateway
            .inbound(
                &mut daemon_gateway,
                event("control", 1, "owner", "000000000000")
            )
            .await
            .unwrap()
            .as_deref(),
        Some("that code is not valid")
    );
    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_fetches_outbound_and_holds_until_channel_binding() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "outbound-team").await;
    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Question".into(),
            message: "first line\nsecond line".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("expected accepted, got {response:?}"),
    };
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    let Next::NeedChannel { id: held_id, agent } =
        gateway.outbound(&mut daemon_gateway).await.unwrap()
    else {
        panic!("expected channel request");
    };
    assert_eq!(held_id, id);
    gateway
        .bind_channel("outbound".into(), agent.address.clone())
        .unwrap();
    let Next::Send(outbound) = gateway.outbound(&mut daemon_gateway).await.unwrap() else {
        panic!("expected outbound send");
    };
    assert_eq!(outbound.id, id);
    assert_eq!(outbound.channel, "outbound");
    assert_eq!(
        outbound.pieces.join(""),
        "**Question**\nfirst line\nsecond line"
    );
    gateway
        .outbound_done(&mut daemon_gateway, &outbound.id, true)
        .await
        .unwrap();
    assert!(matches!(
        gateway.outbound(&mut daemon_gateway).await.unwrap(),
        Next::Idle
    ));
    let _ = session;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_give_up_drops_held_message_and_next_fetch_is_idle() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "give-up-agent",
        Some(RemoteMode::Send),
        Some(team_scope("give-up-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "give-up-team").await;
    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: HUMAN_SUBJECT.into(),
            message: "give up".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("expected accepted, got {response:?}"),
    };
    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    let Next::NeedChannel { id: held_id, agent } =
        gateway.outbound(&mut daemon_gateway).await.unwrap()
    else {
        panic!("expected a held message");
    };
    assert_eq!(held_id, id);
    assert_eq!(agent.address, "give-up-agent@host-a");
    gateway
        .outbound_done(&mut daemon_gateway, &held_id, false)
        .await
        .unwrap();
    let messages = list_messages(&mut admin).await;
    let message = messages
        .into_iter()
        .find(|message| message.id == id)
        .expect("given-up message is listed");
    assert_eq!(message.state, "undeliverable");
    assert_eq!(
        message.detail.as_deref(),
        Some("the gateway could not send it")
    );
    assert!(matches!(
        gateway.outbound(&mut daemon_gateway).await.unwrap(),
        Next::Idle
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_reports_unlisted_messages_as_undeliverable() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "unlisted-agent",
        Some(RemoteMode::Send),
        Some(team_scope("unlisted-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut daemon_gateway = gateway_client(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut daemon_gateway, "unlisted-team").await;
    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: HUMAN_SUBJECT.into(),
            message: "unlisted".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("expected accepted, got {response:?}"),
    };

    let mut gateway =
        Gateway::new(gateway_state(dir.path()), "owner".into(), "control".into()).unwrap();
    let Next::NeedChannel { id: held_id, agent } =
        gateway.outbound(&mut daemon_gateway).await.unwrap()
    else {
        panic!("expected a held message");
    };
    assert_eq!(held_id, id);
    assert_eq!(agent.address, "unlisted-agent@host-a");
    assert!(matches!(
        admin
            .request(Request::Kill { session, now: true })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway.outbound(&mut daemon_gateway).await.unwrap(),
        Next::Idle
    ));
    let messages = list_messages(&mut admin).await;
    let message = messages
        .into_iter()
        .find(|message| message.id == held_id)
        .expect("held message is listed");
    assert_eq!(message.state, "undeliverable");
    assert_eq!(
        message.detail.as_deref(),
        Some("the gateway could not send it")
    );
    daemon.shutdown().await.unwrap();
}

#[test]
fn gateway_rejects_control_binding() {
    let dir = tempfile::tempdir().unwrap();
    let path = gateway_state(dir.path());
    let mut gateway = Gateway::new(path, "owner".into(), "control".into()).unwrap();
    assert!(gateway.channel_for("none").is_none());
    assert!(
        gateway
            .bind_channel("control".into(), "agent@host-a".into())
            .is_err()
    );
}
