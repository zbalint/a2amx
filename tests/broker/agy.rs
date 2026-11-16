use super::*;

async fn agy_reset_target(
    admin: &mut Client,
    harness: Harness,
    above: &str,
    composer: &str,
    deliver: Option<Deliver>,
    post_input: Option<&str>,
) -> String {
    let rules = "─".repeat(40);
    let redraw = post_input.unwrap_or("");
    let command = format!(
        "stty raw -echo; printf '\\033[2J\\033[?2004h\\033[1;1H{above}\\033[2;1H{rules}\\033[3;1H> {composer}\\033[4;1H{rules}\\033[3;3H\\033[?25h'; {} exec cat >/dev/null",
        if redraw.is_empty() {
            String::new()
        } else {
            format!("head -c 1 >/dev/null; printf '{redraw}';")
        }
    );
    match admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), command],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agy-target".into()),
            harness,
            deliver,
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
    }
}

async fn wait_screen_contains(home: &Path, target: &str, marker: &str) {
    let target = target.to_owned();
    let marker = marker.to_owned();
    common::eventually(|| {
        let target = target.clone();
        let marker = marker.clone();
        async move {
            let mut client = Client::connect(home).await.ok()?;
            let Response::Screen { lines } = client
                .request(Request::Screen { session: target })
                .await
                .ok()?
            else {
                return None;
            };
            lines
                .iter()
                .any(|line| line.contains(&marker))
                .then_some(())
        }
    })
    .await;
}

#[tokio::test]
async fn agy_default_delivery_is_auto() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = agy_reset_target(&mut admin, Harness::Agy, "", "", None, None).await;
    wait_claude_ready(dir.path(), &target).await;
    let Response::Explain { info } = admin
        .request(Request::Explain {
            session: target.clone(),
        })
        .await
        .unwrap()
    else {
        panic!("expected explain response");
    };
    assert_ne!(
        info.hold.as_ref().map(|hold| hold.reason.as_str()),
        Some("deliver_hold")
    );
    let id = match send(&mut sender, "agy-target@host-a", "subject", "body").await {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    let message = wait_status(&mut sender, &id, |message| message.state == "submitted").await;
    assert_eq!(message.state, "submitted");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_nonempty_agy_composer_without_hold_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "draft",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    reset_rejection_admin(dir.path(), &target, "draft_present", "composer not empty").await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn agy_delivery_nonempty_composer_without_hold_stays_pending() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "draft",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    let id = match send(&mut sender, "agy-target@host-a", "subject", "body").await {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let message = match status(&mut sender, &id).await {
        Response::Status { message } => message,
        response => panic!("unexpected status response: {response:?}"),
    };
    assert_eq!(message.state, "pending");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_stale_agy_draft_hold_clears_after_margin() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target =
        agy_reset_target(&mut admin, Harness::Agy, "", "", Some(Deliver::Auto), None).await;
    wait_claude_ready(dir.path(), &target).await;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    wait_claude_hold(dir.path(), &target).await;
    reset_rejection_admin(dir.path(), &target, "draft_present", "hold human_draft").await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(reset_success_admin(dir.path(), &target).await, 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_agy_hold_releases_before_delivery_without_manual_release() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target =
        agy_reset_target(&mut admin, Harness::Agy, "", "", Some(Deliver::Auto), None).await;
    wait_claude_ready(dir.path(), &target).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    wait_claude_hold(dir.path(), &target).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let id = match send(&mut sender, "agy-target@host-a", "subject", "body").await {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    let message = wait_status(&mut sender, &id, |message| message.state == "submitted").await;
    assert_eq!(message.state, "submitted");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn stale_agy_hold_keeps_nonempty_composer_held_for_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "draft",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    wait_claude_hold(dir.path(), &target).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let id = match send(&mut sender, "agy-target@host-a", "subject", "body").await {
        Response::Accepted {
            id,
            recipient_hold: Some(hold),
            ..
        } => {
            assert_eq!(hold, "human_draft");
            id
        }
        response => panic!("unexpected send response: {response:?}"),
    };
    let message = wait_status(&mut sender, &id, |message| {
        message.state == "pending" && message.hold_reason.as_deref() == Some("human_draft")
    })
    .await;
    assert_eq!(message.hold_reason.as_deref(), Some("human_draft"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_nonempty_agy_composer_stays_held_after_margin() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "draft",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    wait_claude_hold(dir.path(), &target).await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    reset_rejection_admin(dir.path(), &target, "draft_present", "hold human_draft").await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn agy_delivery_recheck_with_pasted_composer_is_submitted() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let redraw = "\\033[3;3Hpasted envelope\\033[3;18H";
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "",
        Some(Deliver::Auto),
        Some(redraw),
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    accepted(
        send(&mut sender, "agy-target@host-a", "subject", "body").await,
        "m_1",
        None,
    );
    wait_screen_contains(dir.path(), &target, "pasted envelope").await;
    let message = wait_status(&mut sender, "m_1", |message| message.state == "submitted").await;
    assert_eq!(message.state, "submitted");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn agy_delivery_recheck_without_composer_is_unsubmitted() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let redraw = "\\033[2J\\033[?25l\\033[2;1HMCP\\033[3;1H> 1. Yes, allow tool call";
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "",
        "",
        Some(Deliver::Auto),
        Some(redraw),
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    accepted(
        send(&mut sender, "agy-target@host-a", "subject", "body").await,
        "m_1",
        None,
    );
    wait_screen_contains(dir.path(), &target, "MCP").await;
    let message = wait_status(&mut sender, "m_1", |message| message.state == "unsubmitted").await;
    assert_eq!(message.detail.as_deref(), Some("screen_not_ready"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn agy_delivery_while_turn_in_flight_is_submitted() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, _, mut sender) = session(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "⣷  Working...",
        "",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    accepted(
        send(&mut sender, "agy-target@host-a", "subject", "body").await,
        "m_1",
        None,
    );
    let message = wait_status(&mut sender, "m_1", |message| message.state == "submitted").await;
    assert_eq!(message.state, "submitted");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_agy_turn_in_flight_spinner_is_rejected_busy() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "⣷  Working...",
        "",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    reset_rejection_admin(dir.path(), &target, "busy", "turn in flight").await;
    let Response::Explain { info } = admin
        .request(Request::Explain { session: target })
        .await
        .unwrap()
    else {
        panic!("expected explain response");
    };
    assert!(!info.reset.allowed);
    assert_eq!(info.reset.code.as_deref(), Some("busy"));
    assert_eq!(info.reset.reason.as_deref(), Some("turn in flight"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_agy_turn_in_flight_queued_row_is_rejected_busy() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "▸ queued (alt+m to send immediately)",
        "",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    reset_rejection_admin(dir.path(), &target, "busy", "turn in flight").await;
    let Response::Explain { info } = admin
        .request(Request::Explain { session: target })
        .await
        .unwrap()
    else {
        panic!("expected explain response");
    };
    assert!(!info.reset.allowed);
    assert_eq!(info.reset.code.as_deref(), Some("busy"));
    assert_eq!(info.reset.reason.as_deref(), Some("turn in flight"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_agy_turn_in_flight_with_draft_is_rejected_draft_present() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Agy,
        "⣷  Working...",
        "draft",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    reset_rejection_admin(dir.path(), &target, "draft_present", "composer not empty").await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_non_agy_with_spinner_above_composer_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = agy_reset_target(
        &mut admin,
        Harness::Generic,
        "⣷  Working...",
        "",
        Some(Deliver::Auto),
        None,
    )
    .await;
    wait_claude_ready(dir.path(), &target).await;
    assert_eq!(reset_success_admin(dir.path(), &target).await, 1);
    daemon.shutdown().await.unwrap();
}
