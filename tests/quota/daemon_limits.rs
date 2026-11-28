use super::*;

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
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("session creation failed: {response:?}");
    };
    session
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
            five_hour_left: Some(0),
            weekly_left: Some(80),
            ..Default::default()
        }
    );
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
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
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("list_agents failed");
    };
    let agent = agents
        .iter()
        .find(|a| a.address.starts_with("agent-omp@"))
        .unwrap();
    assert_eq!(agent.quota, Some(quota));
    assert_eq!(agent.harness, Harness::Omp);
    assert_eq!(agent.cwd, None);
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

// The quota text stays on the screen after the exit, so only the exit guard in
// `session_quota` keeps an exited session at no quota.
#[tokio::test]
async fn exited_claude_session_reports_no_quota() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = status_line_session(
        &mut admin,
        "agent-claude-exited",
        Harness::Claude,
        "printf '\\033[24;1H5h 0%% left 7d 80%% left'; exit 0",
    )
    .await;

    let exited = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let latest = summary(&mut admin, &id).await;
            if latest.exit_code.is_some() {
                return latest;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(exited.exit_code, Some(0));
    assert_eq!(exited.quota, None);
    daemon.shutdown().await.unwrap();
}
