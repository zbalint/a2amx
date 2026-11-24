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

async fn spawn_screen_session(
    admin: &mut Client,
    name: &str,
    harness: Harness,
    below: &str,
) -> String {
    let rules = "─".repeat(40);
    let command = format!(
        "stty raw -echo; printf '\\033[2J\\033[?2004h\\033[2;1H{rules}\\033[3;1H> \\033[4;1H{rules}{below}\\033[3;3H\\033[?25h'; exec cat >/dev/null"
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
            name: Some(name.into()),
            harness,
            deliver: None,
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
        response => panic!("unexpected response: {response:?}"),
    }
}

#[tokio::test]
async fn agy_background_job_activity_is_waiting_after_quiet_and_working_on_change() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let rules = "─".repeat(40);
    let job_below = format!("\\033[5;1H  ● [21:48:54] sleep 60 running\\033[6;1H{rules}");

    let waiting_id = spawn_screen_session(&mut admin, "agy-job", Harness::Agy, &job_below).await;
    let idle_id = spawn_screen_session(&mut admin, "agy-idle", Harness::Agy, "").await;
    let generic_id =
        spawn_screen_session(&mut admin, "generic-job", Harness::Generic, &job_below).await;
    wait_claude_ready(dir.path(), &waiting_id).await;
    wait_claude_ready(dir.path(), &idle_id).await;
    wait_claude_ready(dir.path(), &generic_id).await;

    tokio::time::sleep(Duration::from_millis(10_500)).await;
    let working_id = spawn_screen_session(&mut admin, "agy-fresh", Harness::Agy, &job_below).await;
    wait_claude_ready(dir.path(), &working_id).await;

    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected sessions response");
    };
    let activity_of = |id: &str| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .and_then(|session| session.activity)
    };
    assert_eq!(
        activity_of(&waiting_id),
        Some(a2amx::wire::Activity::Waiting)
    );
    assert_eq!(activity_of(&idle_id), Some(a2amx::wire::Activity::Idle));
    assert_eq!(activity_of(&generic_id), Some(a2amx::wire::Activity::Idle));
    assert_eq!(
        activity_of(&working_id),
        Some(a2amx::wire::Activity::Working)
    );

    daemon.shutdown().await.unwrap();
}

async fn spawn_activity_watcher(admin: &mut Client, watch: Vec<String>) -> String {
    match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "stty -echo; printf '\\033[?2004h'; exec cat".into(),
            ],
            cols: 80,
            rows: 24,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch,
            name: Some("w".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: Some(a2amx::wire::ActivityWatch {
                working_after: Some(1),
                idle_after: Some(1),
            }),
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected watcher response: {response:?}"),
    }
}

#[tokio::test]
async fn agy_background_job_suppresses_idle_activity_notice_while_row_stays() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let rules = "─".repeat(40);
    let job_below = format!("\\033[5;1H  ● [21:48:54] sleep 60 running\\033[6;1H{rules}");

    let watcher = spawn_activity_watcher(&mut admin, vec!["p".into(), "q".into()]).await;
    let _p = spawn_screen_session(&mut admin, "p", Harness::Agy, &job_below).await;
    let q_command = format!(
        "stty raw -echo; printf '\\033[2J\\033[?2004h\\033[2;1H{rules}\\033[3;1H> \\033[4;1H{rules}{job_below}\\033[3;3H\\033[?25h'; sleep 2; printf '\\033[2J\\033[?2004h\\033[2;1H{rules}\\033[3;1H> \\033[4;1H{rules}\\033[3;3H\\033[?25h'; exec cat >/dev/null"
    );
    let Response::Created { .. } = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), q_command],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("q".into()),
            harness: Harness::Agy,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    else {
        panic!("expected q created");
    };

    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let text = loop {
        let Response::Screen { lines } = admin
            .request(Request::Screen {
                session: watcher.clone(),
            })
            .await
            .unwrap()
        else {
            panic!("expected screen response");
        };
        let joined = lines.join("\n");
        if joined.contains("- q: idle (was working") {
            break joined;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for q: idle notice; screen:\n{joined}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    assert!(
        !text.contains("- p: idle"),
        "unexpected p: idle notice while job row present:\n{text}"
    );
    daemon.shutdown().await.unwrap();
}
