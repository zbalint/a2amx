#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{HUMAN_SESSION, HUMAN_SUBJECT, Limits, RemoteMode, TeamScope, code};
use a2amx::wire::{Request, Response};
use common::{
    create_confined, create_confined_with_team, create_confined_with_token, error_message,
    failed_code, failed_message, gateway, issue_and_redeem, issue_code, limits_with,
    new_session_request, new_session_request_with_remote, start_confined_daemon,
    start_confined_daemon_with_limits, team_scope,
};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use std::path::Path;
use std::process::{Command, Output};
pub fn run_binary(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime: &Path,
    args: &[&str],
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime)
        .output()
        .expect("run a2amx")
}
#[tokio::test]
async fn inbound_requires_a_valid_team_lease_before_storage() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "leased-target",
        Some(RemoteMode::Both),
        Some(TeamScope {
            name: "lease-team".into(),
            private: false,
            allow: vec![],
            agents: vec![],
        }),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let response = gateway
        .request(Request::HumanInbound {
            to: "leased-target@host-a".into(),
            body: "message".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

async fn kill(admin: &mut Client, session: &str) {
    assert!(matches!(
        admin
            .request(Request::Kill {
                session: session.into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    ));
}

#[tokio::test]
async fn issue_lease_code_requires_a_running_remote_member_of_the_team() {
    let code_secs = 17;
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(10, code_secs)).await
    else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert!(matches!(
        admin
            .request(Request::IssueLeaseCode {
                team: "missing-team".into(),
            })
            .await
            .unwrap(),
        Response::Error { message }
            if message == "no agent in this team has remote messaging enabled"
    ));
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "send-only",
        Some(RemoteMode::Send),
        Some(team_scope("send-team")),
    )
    .await;
    let response = admin
        .request(Request::IssueLeaseCode {
            team: "send-team".into(),
        })
        .await
        .unwrap();
    let Response::LeaseCode {
        code,
        agents,
        valid_secs,
    } = response
    else {
        panic!("expected lease code, got {response:?}");
    };
    assert_eq!(code.len(), 12);
    assert!(
        code.bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    );
    assert_eq!(agents, vec!["send-only (send)"]);
    assert_eq!(valid_secs, code_secs);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_lease_expires_and_does_not_store_late_messages() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "expiring-target",
        Some(RemoteMode::Both),
        Some(team_scope("expiry-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiry-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "expiring-target@host-a".into(),
                body: "before expiry".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let response = gateway
        .request(Request::HumanInbound {
            to: "expiring-target@host-a".into(),
            body: "after expiry".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert_eq!(messages.len(), 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn queued_human_message_rechecks_recipient_eligibility_before_delivery() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, _token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "expiring-inbound",
        Some(RemoteMode::Both),
        Some(team_scope("expiring-inbound-team")),
        Harness::Omp,
        None,
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiring-inbound-team").await;
    let id = match gateway
        .request(Request::HumanInbound {
            to: "expiring-inbound@host-a".into(),
            body: "Wait for the lease.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected inbound response: {response:?}"),
    };
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let deadline = Instant::now() + Duration::from_secs(10);
    let message = loop {
        let Response::Messages { messages } = admin
            .request(Request::ListMessages {
                session: None,
                state: None,
            })
            .await
            .unwrap()
        else {
            panic!("expected message listing");
        };
        if let Some(message) = messages
            .into_iter()
            .find(|message| message.id == id && message.state == "undeliverable")
        {
            break message;
        }
        assert!(
            Instant::now() < deadline,
            "message did not fail after lease expiry"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        message.detail.as_deref(),
        Some("remote messaging is not active for this team; start it at the computer")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_codes_expire_are_single_use_and_wrong_attempts_reset_on_issue() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(10, 1)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "code-target",
        Some(RemoteMode::Both),
        Some(team_scope("code-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let expired = issue_code(&mut admin, "code-team").await;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: expired })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    let code = issue_code(&mut admin, "code-team").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: code.clone() })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn five_wrong_codes_drop_pending_codes_until_issue_resets_the_counter() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "guess-target",
        Some(RemoteMode::Both),
        Some(team_scope("guess-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    let code = issue_code(&mut admin, "guess-team").await;
    for _ in 0..5 {
        assert!(matches!(
            gateway
                .request(Request::RedeemLeaseCode {
                    code: "ffffffffffff".into(),
                })
                .await
                .unwrap(),
            Response::Error { message } if message == "invalid or expired code"
        ));
    }
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Error { message } if message == "invalid or expired code"
    ));
    let replacement = issue_code(&mut admin, "guess-team").await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code: replacement })
            .await
            .unwrap(),
        Response::Ok
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn lease_membership_is_fixed_and_survivors_keep_access_after_partial_down() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let team = team_scope("members-team");
    let first = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "first-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let second = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "second-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "members-team").await;
    kill(&mut admin, &first).await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "second-member@host-a".into(),
                body: "survivor".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "joined-member",
        Some(RemoteMode::Both),
        Some(team.clone()),
    )
    .await;
    let joined = gateway
        .request(Request::HumanInbound {
            to: "joined-member@host-a".into(),
            body: "joiner".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(joined),
        "remote messaging is not active for this team; start it at the computer"
    );
    kill(&mut admin, &second).await;
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "restarted-member",
        Some(RemoteMode::Both),
        Some(team),
    )
    .await;
    let restarted = gateway
        .request(Request::HumanInbound {
            to: "restarted-member@host-a".into(),
            body: "restart".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(restarted),
        "remote messaging is not active for this team; start it at the computer"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_inbound_is_stored_as_a_human_message_for_a_confined_session() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "confined-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-team").await;

    let response = gateway
        .request(Request::HumanInbound {
            to: "confined-target@host-a".into(),
            body: "Please inspect the latest result.".into(),
        })
        .await
        .unwrap();
    let id = match response {
        Response::Accepted {
            id,
            recipient_hold: None,
            recipient_quota: None,
        } => id,
        response => panic!("unexpected inbound response: {response:?}"),
    };

    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    let message = messages
        .into_iter()
        .find(|message| message.id == id)
        .expect("accepted message is listed");
    assert_eq!(message.from, HUMAN_SESSION);
    assert_eq!(message.to, "confined-target@host-a");
    assert_eq!(message.subject, HUMAN_SUBJECT);
    assert_eq!(message.state, "pending");
    assert_eq!(message.detail, None);
    assert_eq!(message.hold_reason, Some("deliver_hold".into()));
    drop(gateway);
    daemon.shutdown().await.unwrap();
    drop(recipient);
}

#[tokio::test]
async fn enabled_agent_can_send_to_human_and_gateway_reports_outcome() {
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
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-team").await;
    let target_fixture = tempfile::tempdir().unwrap();
    let _target = create_confined_with_team(
        &mut admin,
        target_fixture.path(),
        "agent-target",
        None,
        Some(team_scope("outbound-team")),
    )
    .await;
    let agent_id = match sender
        .request(Request::SendMessage {
            to: "agent-target@host-a".into(),
            subject: "Agent-only".into(),
            message: "The gateway must not claim this.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected agent send response: {response:?}"),
    };
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: agent_id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus {
                id: agent_id.clone()
            })
            .await
            .unwrap(),
        Response::Status { message } if message.state == "pending"
    ));

    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Need help".into(),
            message: "Please check the latest result.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted {
            id,
            recipient_hold: None,
            recipient_quota: None,
        } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    let outbound = gateway.request(Request::HumanOutboundNext).await.unwrap();
    assert_eq!(
        outbound,
        Response::HumanOutbound {
            message: Some(a2amx::wire::HumanOutboundMessage {
                id: id.clone(),
                from: "outbound-agent@host-a".into(),
                subject: "Need help".into(),
                body: "Please check the latest result.".into(),
            }),
        }
    );
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus { id: id.clone() })
            .await
            .unwrap(),
        Response::Status { message } if message.state == "submitted"
    ));
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: id.clone(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );

    let failed_id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Another request".into(),
            message: "This one should report failure.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected second send response: {response:?}"),
    };
    assert!(matches!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound {
            message: Some(a2amx::wire::HumanOutboundMessage { ref id, .. })
        } if id == &failed_id
    ));
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: failed_id.clone(),
                sent: false,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert!(matches!(
        sender
            .request(Request::MessageStatus { id: failed_id })
            .await
            .unwrap(),
        Response::Status { message }
            if message.state == "undeliverable"
                && message.detail.as_deref() == Some("the gateway could not send it")
    ));
    daemon.shutdown().await.unwrap();
    drop(session);
}

#[tokio::test]
async fn send_to_human_requires_enablement_and_stores_nothing_when_refused() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-disabled",
        None,
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "No".into(),
            message: "This must be refused.".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
    assert_eq!(
        failed_message(response),
        "remote messages to this session are not enabled"
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_to_human_without_a_team_is_refused_for_send_and_both_modes() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let send_fixture = tempfile::tempdir().unwrap();
    let (_send_session, send_token) = create_confined_with_token(
        &mut admin,
        send_fixture.path(),
        "no-team-send",
        Some(RemoteMode::Send),
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let both_fixture = tempfile::tempdir().unwrap();
    let (_both_session, both_token) = create_confined_with_token(
        &mut admin,
        both_fixture.path(),
        "no-team-both",
        Some(RemoteMode::Both),
        None,
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut send_client = Client::connect_addr(daemon.addrs()[0], send_token.trim())
        .await
        .unwrap();
    let mut both_client = Client::connect_addr(daemon.addrs()[0], both_token.trim())
        .await
        .unwrap();
    for (client, subject) in [
        (&mut send_client, "Send without team"),
        (&mut both_client, "Both without team"),
    ] {
        let response = client
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: subject.into(),
                message: "A team is required.".into(),
            })
            .await
            .unwrap();
        assert_eq!(
            failed_message(response),
            "remote messaging is not active for this team; start it at the computer"
        );
    }
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_to_human_send_mode_needs_a_lease_and_exact_address() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-send",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-send-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "No lease".into(),
            message: "This must wait for a lease.".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        failed_message(response),
        "remote messaging is not active for this team; start it at the computer"
    );
    for to in ["human:owner@host-a", "Human:owner"] {
        assert_eq!(
            failed_code(
                sender
                    .request(Request::SendMessage {
                        to: to.into(),
                        subject: "Unknown".into(),
                        message: "Not the human address.".into(),
                    })
                    .await
                    .unwrap()
            ),
            code::UNKNOWN_RECIPIENT
        );
    }
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-send-team").await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "With lease".into(),
                message: "Now this is accepted.".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn human_outbound_rechecks_sender_eligibility_before_gateway_fetch() {
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits_with(1, 10)).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "expiring-outbound",
        Some(RemoteMode::Send),
        Some(team_scope("expiring-outbound-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "expiring-outbound-team").await;
    let id = match sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Before expiry".into(),
            message: "This should not leave after expiry.".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert_eq!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound { message: None }
    );
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    let message = messages
        .into_iter()
        .find(|message| message.id == id)
        .expect("expired message is listed");
    assert_eq!(message.state, "undeliverable");
    assert_eq!(
        message.detail.as_deref(),
        Some("sender is no longer eligible for remote messaging")
    );
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn inbound_requires_explicit_both_mode_and_stores_nothing_when_refused() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _send = create_confined(
        &mut admin,
        fixture.path(),
        "send-target",
        Some(RemoteMode::Send),
    )
    .await;
    let _unset = create_confined(&mut admin, fixture.path(), "unset-target", None).await;
    let refused = new_session_request_with_remote(
        Some("unconfined-remote"),
        &["sleep", "30"],
        Some(RemoteMode::Both),
    );
    assert_eq!(
        error_message(admin.request(refused).await.unwrap()),
        "remote messaging needs a confined session"
    );
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected session listing");
    };
    assert!(
        sessions
            .iter()
            .all(|session| session.name.as_deref() != Some("unconfined-remote"))
    );
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    for target in ["send-target@host-a", "unset-target@host-a"] {
        let response = gateway
            .request(Request::HumanInbound {
                to: target.into(),
                body: "message".into(),
            })
            .await
            .unwrap();
        assert_eq!(failed_code(response.clone()), code::NOT_PERMITTED);
        assert_eq!(
            failed_message(response),
            "remote messages to this session are not enabled"
        );
    }
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert!(messages.iter().all(|message| {
        !["send-target@host-a", "unset-target@host-a"].contains(&message.to.as_str())
    }));
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_remote_requires_confinement_and_creates_nothing() {
    let (dir, daemon) = common::start_daemon().await;
    let cwd = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for mode in ["maybe", ""] {
        let invalid = run_binary(
            dir.path(),
            cwd.path(),
            user_home.path(),
            runtime.path(),
            &["new", "--remote", mode, "--", "true"],
        );
        let stderr = String::from_utf8_lossy(&invalid.stderr);
        assert!(!invalid.status.success());
        assert!(stderr.contains("send"), "{stderr}");
        assert!(stderr.contains("both"), "{stderr}");
    }
    let output = run_binary(
        dir.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &[
            "new",
            "--detach",
            "--name",
            "unconfined-remote",
            "--remote",
            "both",
            "--",
            "sleep",
            "30",
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("remote messaging needs a confined session")
    );
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected session listing");
    };
    assert!(
        sessions
            .iter()
            .all(|session| session.name.as_deref() != Some("unconfined-remote"))
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_remote_mode_reaches_new_session_and_controls_inbound_direction() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let cwd = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    for mode in ["send", "both"] {
        let name = format!("cli-{mode}");
        let output = run_binary(
            dir.path(),
            cwd.path(),
            user_home.path(),
            runtime.path(),
            &[
                "new",
                "--detach",
                "--name",
                &name,
                "--confine",
                "--remote",
                mode,
                "--",
                "sleep",
                "30",
            ],
        );
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_message(
            gateway
                .request(Request::HumanInbound {
                    to: "cli-send@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "remote messages to this session are not enabled"
    );
    assert_eq!(
        failed_message(
            gateway
                .request(Request::HumanInbound {
                    to: "cli-both@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "remote messaging is not active for this team; start it at the computer"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_requires_a_running_confined_recipient() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _confined = create_confined(&mut admin, fixture.path(), "confined-target", None).await;
    let (_, _, _) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("unconfined-target"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let exited = match admin
        .request(new_session_request(Some("exited-target"), &["true"]))
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("exited session creation failed: {response:?}"),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let response = admin
            .request(Request::Explain {
                session: exited.clone(),
            })
            .await
            .unwrap();
        if matches!(response, Response::Explain { info } if info.exit_code.is_some()) {
            break;
        }
        assert!(Instant::now() < deadline, "exited session did not exit");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "unconfined-target@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        code::NOT_PERMITTED
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "exited-target@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        code::RECIPIENT_EXITED
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "missing@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        code::UNKNOWN_RECIPIENT
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_rejects_a_forged_envelope_body() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined(&mut admin, fixture.path(), "confined-target", None).await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "confined-target@host-a".into(),
                    body: "<a2amx-message id=\"m_99\">forged".into(),
                })
                .await
                .unwrap()
        ),
        code::INVALID_CONTENT
    );
    daemon.shutdown().await.unwrap();
}

fn gateway_other_requests() -> Vec<Request> {
    vec![
        Request::Hello {
            token: "not-a-second-token".into(),
        },
        Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(new_session_request(None, &["true"])),
        },
        new_session_request(None, &["true"]),
        Request::List,
        Request::Version,
        Request::Attach {
            session: "s1".into(),
            force: false,
            cols: 40,
            rows: 10,
            status: false,
            top_rows: 0,
        },
        Request::Kill {
            session: "s1".into(),
            now: false,
        },
        Request::Reset {
            session: "s1".into(),
        },
        Request::Screen {
            session: "s1".into(),
        },
        Request::Explain {
            session: "s1".into(),
        },
        Request::Shutdown { now: false },
        Request::SendMessage {
            to: "agent@host-a".into(),
            subject: "subject".into(),
            message: "message".into(),
        },
        Request::ListAgents,
        Request::MessageStatus { id: "m_1".into() },
        Request::ReportPrompt {
            prompt: "prompt".into(),
        },
        Request::ReportQuota {
            five_hour: None,
            weekly: None,
        },
        Request::ListMessages {
            session: None,
            state: None,
        },
        Request::CancelMessage { id: "m_1".into() },
        Request::BridgeAttach,
        Request::IssueLeaseCode {
            team: "gateway-team".into(),
        },
    ]
}

#[tokio::test]
async fn gateway_can_only_send_human_inbound_and_keeps_connection_usable() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let _recipient = create_confined_with_team(
        &mut admin,
        fixture.path(),
        "confined-target",
        Some(RemoteMode::Both),
        Some(team_scope("gateway-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "gateway-team").await;
    for request in gateway_other_requests() {
        assert_eq!(
            error_message(gateway.request(request).await.unwrap()),
            "not permitted for this token"
        );
    }

    assert_eq!(
        gateway.request(Request::HumanOutboundNext).await.unwrap(),
        Response::HumanOutbound { message: None }
    );
    assert_eq!(
        gateway
            .request(Request::HumanOutboundDone {
                id: "m_999".into(),
                sent: true,
            })
            .await
            .unwrap(),
        Response::Error {
            message: "unknown message".into()
        }
    );
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "confined-target@host-a".into(),
                body: "connection remains usable".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn admin_and_session_tokens_cannot_speak_as_the_human() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert_eq!(
        error_message(
            admin
                .request(Request::HumanInbound {
                    to: "missing@host-a".into(),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for this token"
    );
    assert_eq!(
        error_message(
            admin
                .request(Request::RedeemLeaseCode {
                    code: "0123456789ab".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for this token"
    );
    for request in [
        Request::GatewayAgents,
        Request::HumanOutboundNext,
        Request::HumanOutboundDone {
            id: "m_1".into(),
            sent: true,
        },
    ] {
        assert_eq!(
            error_message(admin.request(request).await.unwrap()),
            "not permitted for this token"
        );
    }
    let (session, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-session"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut session_client = Client::connect_addr(address, &token).await.unwrap();
    assert_eq!(
        error_message(
            session_client
                .request(Request::HumanInbound {
                    to: format!("{session}@host-a"),
                    body: "message".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    assert_eq!(
        error_message(
            session_client
                .request(Request::IssueLeaseCode {
                    team: "agent-team".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    assert_eq!(
        error_message(
            session_client
                .request(Request::RedeemLeaseCode {
                    code: "0123456789ab".into(),
                })
                .await
                .unwrap()
        ),
        "not permitted for a session token"
    );
    for request in [
        Request::GatewayAgents,
        Request::HumanOutboundNext,
        Request::HumanOutboundDone {
            id: "m_1".into(),
            sent: true,
        },
    ] {
        assert_eq!(
            error_message(session_client.request(request).await.unwrap()),
            "not permitted for a session token"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn human_session_name_is_reserved() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let response = admin
        .request(new_session_request(Some("human:owner"), &["sleep", "30"]))
        .await
        .unwrap();
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("invalid name")
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_token_and_link_files_are_owner_only_and_follow_daemon_lifecycle() {
    let (dir, daemon) = common::start_daemon().await;
    let gateway_token = std::fs::read_to_string(dir.path().join("gateway.token")).unwrap();
    let link_dir = dir.path().join("link");
    assert_eq!(
        std::fs::metadata(&link_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let link_token = link_dir.join("gateway.token");
    assert_eq!(
        std::fs::metadata(&link_token).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_to_string(&link_token).unwrap(), gateway_token);
    let addr = std::fs::read_to_string(dir.path().join("addr")).unwrap();
    assert_eq!(
        std::fs::read_to_string(link_dir.join("addr")).unwrap(),
        addr
    );
    assert_eq!(
        std::fs::metadata(dir.path().join("gateway.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    daemon.shutdown().await.unwrap();
    assert!(!dir.path().join("addr").exists());
    assert!(!link_dir.join("addr").exists());
    assert!(dir.path().join("gateway.token").exists());
    assert!(link_token.exists());
}

#[tokio::test]
async fn bind_failure_removes_both_capability_tokens() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("parent/state");
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let result = Daemon::start(DaemonConfig {
        state_dir: home.clone(),
        listen: vec!["127.0.0.1:0".parse().unwrap(), address],
        host_name: None,
        limits: Limits::default(),
        bwrap_path: Some(home.join("bwrap-path").into_os_string()),
        env_path: None,
    })
    .await;
    assert!(result.is_err());
    assert!(!home.join("admin.token").exists());
    assert!(!home.join("gateway.token").exists());
    assert!(!home.join("link/gateway.token").exists());
    assert!(!home.join("addr").exists());
}

#[test]
fn remote_mode_wire_shape_is_stable_and_backward_compatible() {
    let without = new_session_request(None, &["sh"]);
    let without_json = serde_json::to_value(&without).unwrap();
    assert!(!without_json.as_object().unwrap().contains_key("remote"));

    let mut with = new_session_request_with_remote(None, &["sh"], Some(RemoteMode::Both));
    if let Request::NewSession { role, .. } = &mut with {
        *role = Some("worker".into());
    }
    let encoded = serde_json::to_string(&with).unwrap();
    assert_eq!(
        serde_json::to_value(&with).unwrap()["remote"],
        serde_json::json!("both")
    );
    assert!(
        encoded.find("\"role\":\"worker\"").unwrap() < encoded.find("\"remote\":\"both\"").unwrap()
    );
    let mut old = serde_json::to_value(&with).unwrap();
    old.as_object_mut().unwrap().remove("remote");
    let Request::NewSession { remote, .. } = serde_json::from_value(old).unwrap() else {
        panic!("expected new_session request");
    };
    assert_eq!(remote, None);
}

#[test]
fn human_inbound_wire_shape_is_stable() {
    let request = Request::HumanInbound {
        to: "agent-review@host-a".into(),
        body: "hello".into(),
    };
    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "type": "human_inbound",
            "to": "agent-review@host-a",
            "body": "hello"
        })
    );
    assert_eq!(serde_json::from_value::<Request>(json).unwrap(), request);
}
#[tokio::test]
async fn inbound_rejects_a_body_over_the_human_chat_limit_before_lookup() {
    let Some((dir, daemon)) = start_confined_daemon().await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "size-target",
        Some(RemoteMode::Both),
        Some(team_scope("size-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "size-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "size-target@host-a".into(),
                body: "x".repeat(8000),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "unknown@host-a".into(),
            body: "x".repeat(8001),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::TOO_LARGE);
    assert_eq!(
        failed_message(response),
        "message exceeds the byte limit for a human message"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_window_refuses_the_third_accepted_message() {
    let limits = Limits {
        human_inbound_per_minute: 2,
        max_pending_per_recipient: 3,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-rate-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-rate-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "unknown@host-a".into(),
                    body: "unknown does not count".into(),
                })
                .await
                .unwrap()
        ),
        code::UNKNOWN_RECIPIENT
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: "no lease does not count".into(),
                })
                .await
                .unwrap()
        ),
        code::NOT_PERMITTED
    );
    assert_eq!(
        failed_code(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: "x".repeat(8001),
                })
                .await
                .unwrap()
        ),
        code::TOO_LARGE
    );
    issue_and_redeem(&mut admin, &mut gateway, "inbound-rate-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            gateway
                .request(Request::HumanInbound {
                    to: "inbound-rate-target@host-a".into(),
                    body: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-rate-target@host-a".into(),
            body: "third".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "human inbound rate limit reached");
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert_eq!(messages.len(), 2);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_respects_the_recipient_pending_queue_limit() {
    let limits = Limits {
        max_pending_per_recipient: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-cap-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-cap-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-cap-team").await;
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "inbound-cap-target@host-a".into(),
                body: "first".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-cap-target@host-a".into(),
            body: "over cap".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::QUEUE_FULL);
    assert_eq!(failed_message(response), "message queue is full");
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert_eq!(messages.len(), 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn inbound_queue_full_does_not_consume_the_rate_window() {
    let limits = Limits {
        human_inbound_per_minute: 2,
        max_pending_per_recipient: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    create_confined_with_team(
        &mut admin,
        fixture.path(),
        "inbound-queue-target",
        Some(RemoteMode::Both),
        Some(team_scope("inbound-queue-team")),
    )
    .await;
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "inbound-queue-team").await;
    let first_id = match gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "first".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected first response: {response:?}"),
    };
    let second = gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "queue full".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(second.clone()), code::QUEUE_FULL);
    assert_eq!(failed_message(second), "message queue is full");
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: None,
            state: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected message listing");
    };
    assert_eq!(messages.len(), 1);
    assert!(matches!(
        admin
            .request(Request::CancelMessage { id: first_id })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "inbound-queue-target@host-a".into(),
                body: "after cancel".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    let response = gateway
        .request(Request::HumanInbound {
            to: "inbound-queue-target@host-a".into(),
            body: "rate limit".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "human inbound rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn one_agent_outbound_window_refuses_its_third_message() {
    let limits = Limits {
        rate_per_minute: 2,
        human_outbound_per_minute: 30,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-rate-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-rate-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-rate-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "third".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "sender rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_total_window_refuses_messages_across_agents() {
    let limits = Limits {
        rate_per_minute: 3,
        human_outbound_per_minute: 2,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = team_scope("outbound-total-team");
    let first_fixture = tempfile::tempdir().unwrap();
    let (_first_session, first_token) = create_confined_with_token(
        &mut admin,
        first_fixture.path(),
        "outbound-total-first",
        Some(RemoteMode::Send),
        Some(team.clone()),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let second_fixture = tempfile::tempdir().unwrap();
    let (_second_session, second_token) = create_confined_with_token(
        &mut admin,
        second_fixture.path(),
        "outbound-total-second",
        Some(RemoteMode::Send),
        Some(team),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut first = Client::connect_addr(daemon.addrs()[0], first_token.trim())
        .await
        .unwrap();
    let mut second = Client::connect_addr(daemon.addrs()[0], second_token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-total-team").await;
    for sender in [&mut first, &mut second] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: "accepted".into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = first
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "total limit".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(
        failed_message(response),
        "human outbound rate limit reached"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_refusal_prefers_sender_when_both_windows_are_full() {
    let limits = Limits {
        max_pending_per_recipient: 3,
        rate_per_minute: 2,
        human_outbound_per_minute: 2,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-precedence-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-precedence-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-precedence-team").await;
    for body in ["first", "second"] {
        assert!(matches!(
            sender
                .request(Request::SendMessage {
                    to: HUMAN_SESSION.into(),
                    subject: "Request".into(),
                    message: body.into(),
                })
                .await
                .unwrap(),
            Response::Accepted { .. }
        ));
    }
    let response = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "both windows full".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(response.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(response), "sender rate limit reached");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn outbound_rate_refusal_does_not_push_a_timestamp() {
    let limits = Limits {
        max_pending_per_recipient: 3,
        rate_per_minute: 3,
        human_outbound_per_minute: 1,
        ..Limits::default()
    };
    let Some((dir, daemon)) = start_confined_daemon_with_limits(limits).await else {
        return;
    };
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let (_session, token) = create_confined_with_token(
        &mut admin,
        fixture.path(),
        "outbound-refusal-agent",
        Some(RemoteMode::Send),
        Some(team_scope("outbound-refusal-team")),
        Harness::Generic,
        Some(Deliver::Hold),
    )
    .await;
    let mut sender = Client::connect_addr(daemon.addrs()[0], token.trim())
        .await
        .unwrap();
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    issue_and_redeem(&mut admin, &mut gateway, "outbound-refusal-team").await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "Request".into(),
                message: "first".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::sleep(Duration::from_secs(55)).await;
    let refused = sender
        .request(Request::SendMessage {
            to: HUMAN_SESSION.into(),
            subject: "Request".into(),
            message: "refused".into(),
        })
        .await
        .unwrap();
    assert_eq!(failed_code(refused.clone()), code::RATE_LIMITED);
    assert_eq!(failed_message(refused), "human outbound rate limit reached");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: HUMAN_SESSION.into(),
                subject: "Request".into(),
                message: "after expiry".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    daemon.shutdown().await.unwrap();
}
