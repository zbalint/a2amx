#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::emulator::{Emulator, Size};
use a2amx::harness::{Deliver, Harness};
use a2amx::quota;
use a2amx::wire::{QuotaInfo, Request, Response, SessionSummary};

fn screen_with(rows: u16, lines: &[(u16, &str)]) -> a2amx::emulator::Screen {
    let mut emulator = Emulator::new(Size { cols: 169, rows });
    for (row, text) in lines {
        emulator.feed(format!("\x1b[{row};1H{text}").as_bytes());
    }
    emulator.screen()
}

fn read_with(harness: Harness, rows: u16, lines: &[(u16, &str)]) -> Option<QuotaInfo> {
    quota::read(&screen_with(rows, lines), harness)
}

fn read(rows: u16, lines: &[(u16, &str)]) -> Option<QuotaInfo> {
    read_with(Harness::Codex, rows, lines)
}

fn info(five_hour: Option<u8>, weekly: Option<u8>) -> Option<QuotaInfo> {
    Some(QuotaInfo {
        five_hour,
        weekly,
        ..Default::default()
    })
}

#[test]
fn reads_omp_usage_limit_error() {
    let screen = screen_with(
        51,
        &[(
            41,
            "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)",
        )],
    );
    assert_eq!(
        quota::read(&screen, Harness::Omp).and_then(|quota| quota.exhausted()),
        Some("usage")
    );
}

#[test]
fn reads_omp_wrapped_usage_limit_error() {
    let lines = [
        (
            43,
            "  Error: Retry failed after 1 attempts: ... Original error: Codex error event: The usage limit has",
        ),
        (44, "  been reached (code=usage_limit_reached)"),
    ];
    assert_eq!(
        read_with(Harness::Omp, 51, &lines),
        Some(QuotaInfo {
            limit_reached: true,
            ..Default::default()
        })
    );
}

#[test]
fn omp_error_outside_last_twelve_rows_is_ignored() {
    let text =
        "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Omp, 51, &[(19, text)]), None);
}

#[test]
fn omp_advisor_warning_is_not_a_limit() {
    let text = "  Warning: advisor: Advisor \"Advisor\" quota exhausted — pausing until reset.";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_other_error_is_not_a_limit() {
    let text = "  Error: Codex error event: overloaded (code=server_error)";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_quoted_error_is_not_a_limit() {
    let text = "  the log said Error: Codex error event: usage (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Omp, 51, &[(41, text)]), None);
}

#[test]
fn omp_nonadjacent_code_is_not_a_limit() {
    assert_eq!(
        read_with(
            Harness::Omp,
            51,
            &[(41, "Error: x"), (43, "(code=usage_limit_reached)")],
        ),
        None
    );
}

#[test]
fn generic_does_not_read_omp_error() {
    let text =
        "  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)";
    assert_eq!(read_with(Harness::Generic, 51, &[(41, text)]), None);
}

#[test]
fn empty_omp_screen_has_no_limit() {
    assert_eq!(read_with(Harness::Omp, 51, &[]), None);
}

#[test]
fn reads_both_windows_from_a_codex_status_line() {
    let text = "gpt-5 · repo · Ready · 40% context · weekly 12% left · 1.2M used · 5h 0% left";
    assert_eq!(read(24, &[(24, text)]), info(Some(0), Some(12)));
}

#[test]
fn reads_both_windows_from_a_claude_status_line() {
    let text = "[Sonnet] repo | main | $1.20 | 45k/200k (22%) | 5h 7% left 7d 63% left";
    assert_eq!(read(24, &[(24, text)]), info(Some(7), Some(63)));
}

#[test]
fn accepts_one_hundred_and_rejects_more() {
    assert_eq!(read(24, &[(24, "5h 100% left")]), info(Some(100), None));
    assert_eq!(read(24, &[(24, "5h 101% left")]), None);
}

#[test]
fn rejects_malformed_tokens() {
    for text in [
        "5h left",
        "5h 3%left",
        "5h 3% leftover",
        "x5h 3% left",
        "5h  3% left",
    ] {
        assert_eq!(read(24, &[(24, text)]), None, "{text}");
    }
}

#[test]
fn looks_only_at_the_last_three_rows() {
    assert_eq!(read(24, &[(19, "5h 0% left")]), None);
    assert_eq!(read(24, &[(22, "5h 0% left")]), info(Some(0), None));
}

#[test]
fn the_bottom_row_wins() {
    assert_eq!(
        read(24, &[(24, "5h 40% left"), (23, "5h 0% left")]),
        info(Some(40), None)
    );
}

#[test]
fn short_and_empty_screens_are_handled() {
    assert_eq!(read(2, &[(1, "5h 9% left")]), info(Some(9), None));
    assert_eq!(read(24, &[]), None);
}

#[test]
fn exhausted_names_the_windows_at_zero() {
    let exhausted = |five_hour, weekly| {
        QuotaInfo {
            five_hour,
            weekly,
            ..Default::default()
        }
        .exhausted()
    };
    assert_eq!(exhausted(Some(0), Some(0)), Some("5h and weekly"));
    assert_eq!(exhausted(Some(0), Some(5)), Some("5h"));
    assert_eq!(exhausted(Some(5), Some(0)), Some("weekly"));
    assert_eq!(exhausted(Some(5), None), None);
    assert_eq!(
        QuotaInfo {
            limit_reached: true,
            ..Default::default()
        }
        .exhausted(),
        Some("usage")
    );
    assert_eq!(
        QuotaInfo {
            limit_reached: true,
            five_hour: Some(0),
            ..Default::default()
        }
        .exhausted(),
        Some("5h")
    );
    assert_eq!(
        QuotaInfo {
            limit_reached: false,
            ..Default::default()
        }
        .exhausted(),
        None
    );
}

const EXHAUSTED_SCRIPT: &str = "printf '\\033[24;1H5h 0%% left 7d 80%% left'; exec sleep 30";
const HEALTHY_SCRIPT: &str = "printf '\\033[24;1H5h 30%% left'; exec sleep 30";
const OMP_EXHAUSTED_SCRIPT: &str =
    "printf '\\033[41;1H  Error: Codex error event: x (code=usage_limit_reached)'; exec sleep 30";
const OMP_HEALTHY_SCRIPT: &str = "printf '\\033[41;1H ready'; exec sleep 30";

async fn status_line_session(
    admin: &mut Client,
    name: &str,
    harness: Harness,
    script: &str,
) -> String {
    status_line_session_sized(admin, name, harness, script, 80, 24).await
}

async fn status_line_session_sized(
    admin: &mut Client,
    name: &str,
    harness: Harness,
    script: &str,
    cols: u16,
    rows: u16,
) -> String {
    let response = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), script.into()],
            cols,
            rows,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some(name.to_owned()),
            harness,
            deliver: Some(Deliver::Hold),
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("session creation failed: {response:?}");
    };
    session
}

async fn summary(admin: &mut Client, id: &str) -> SessionSummary {
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("list failed");
    };
    sessions.into_iter().find(|s| s.id == id).unwrap()
}

/// Waits until the session's status line has been drawn and read.
async fn quota_of(admin: &mut Client, id: &str) -> QuotaInfo {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(quota) = summary(admin, id).await.quota {
                return quota;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("status line is read")
}

#[tokio::test]
async fn list_and_list_agents_report_a_claude_session_quota() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = status_line_session(
        &mut admin,
        "agent-claude",
        Harness::Claude,
        EXHAUSTED_SCRIPT,
    )
    .await;

    let quota = quota_of(&mut admin, &id).await;

    assert_eq!(
        quota,
        QuotaInfo {
            five_hour: Some(0),
            weekly: Some(80),
            ..Default::default()
        }
    );
    let Response::Agents { agents } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|a| a.address.starts_with("agent-claude@"))
        .unwrap();
    assert_eq!(agent.quota, Some(quota));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_and_list_agents_report_an_omp_limit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let omp = status_line_session_sized(
        &mut admin,
        "agent-omp",
        Harness::Omp,
        OMP_EXHAUSTED_SCRIPT,
        169,
        51,
    )
    .await;
    let generic = status_line_session_sized(
        &mut admin,
        "agent-plain-omp",
        Harness::Generic,
        OMP_EXHAUSTED_SCRIPT,
        169,
        51,
    )
    .await;

    let quota = quota_of(&mut admin, &omp).await;
    assert_eq!(
        quota,
        QuotaInfo {
            limit_reached: true,
            ..Default::default()
        }
    );
    let Response::Agents { agents } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|a| a.address.starts_with("agent-omp@"))
        .unwrap();
    assert_eq!(agent.quota, Some(quota));
    assert_eq!(summary(&mut admin, &generic).await.quota, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn generic_sessions_report_no_quota() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let claude = status_line_session(
        &mut admin,
        "agent-claude",
        Harness::Claude,
        EXHAUSTED_SCRIPT,
    )
    .await;
    let generic = status_line_session(
        &mut admin,
        "agent-plain",
        Harness::Generic,
        EXHAUSTED_SCRIPT,
    )
    .await;
    // The Claude session drawing its line proves the generic one had time to draw too.
    quota_of(&mut admin, &claude).await;

    assert_eq!(summary(&mut admin, &generic).await.quota, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_message_reports_an_exhausted_recipient_and_delivery_is_unchanged() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let exhausted =
        status_line_session(&mut admin, "agent-out", Harness::Claude, EXHAUSTED_SCRIPT).await;
    let healthy =
        status_line_session(&mut admin, "agent-ok", Harness::Claude, HEALTHY_SCRIPT).await;
    quota_of(&mut admin, &exhausted).await;
    quota_of(&mut admin, &healthy).await;
    let (_, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(address, &token).await.unwrap();

    let mut accepted = Vec::new();
    for to in ["agent-out@host-a", "agent-ok@host-a"] {
        let response = sender
            .request(Request::SendMessage {
                to: to.into(),
                subject: "s".into(),
                message: "m".into(),
            })
            .await
            .unwrap();
        let Response::Accepted {
            recipient_hold,
            recipient_quota,
            ..
        } = response
        else {
            panic!("expected accepted, got {response:?}");
        };
        accepted.push((recipient_hold, recipient_quota));
    }

    assert_eq!(accepted[0].1.as_deref(), Some("5h quota exhausted"));
    assert_eq!(accepted[1].1, None);
    assert_eq!(accepted[0].0, accepted[1].0);
    assert_eq!(
        summary(&mut admin, &exhausted).await.hold_reason,
        summary(&mut admin, &healthy).await.hold_reason
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn send_message_reports_omp_limit_without_changing_delivery() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let exhausted = status_line_session_sized(
        &mut admin,
        "agent-omp-out",
        Harness::Omp,
        OMP_EXHAUSTED_SCRIPT,
        169,
        51,
    )
    .await;
    let healthy = status_line_session_sized(
        &mut admin,
        "agent-omp-ok",
        Harness::Omp,
        OMP_HEALTHY_SCRIPT,
        169,
        51,
    )
    .await;
    quota_of(&mut admin, &exhausted).await;

    let (_, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-omp-sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(address, &token).await.unwrap();

    let mut accepted = Vec::new();
    for to in ["agent-omp-out@host-a", "agent-omp-ok@host-a"] {
        let response = sender
            .request(Request::SendMessage {
                to: to.into(),
                subject: "s".into(),
                message: "m".into(),
            })
            .await
            .unwrap();
        let Response::Accepted {
            recipient_hold,
            recipient_quota,
            ..
        } = response
        else {
            panic!("expected accepted, got {response:?}");
        };
        accepted.push((recipient_hold, recipient_quota));
    }

    assert_eq!(accepted[0].1.as_deref(), Some("usage quota exhausted"));
    assert_eq!(accepted[1].1, None);
    assert_eq!(accepted[0].0, accepted[1].0);
    assert_eq!(
        summary(&mut admin, &exhausted).await.hold_reason,
        summary(&mut admin, &healthy).await.hold_reason
    );
    daemon.shutdown().await.unwrap();
}
