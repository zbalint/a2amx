use super::*;

#[test]
fn activity_age_preserves_same_state_and_resets_on_transition() {
    use a2amx::session::observe_activity;
    use a2amx::wire::Activity::{Idle, Working};
    use std::time::Instant;

    let t0 = Instant::now();
    let mut slot = None;
    assert_eq!(observe_activity(&mut slot, Idle, t0), Duration::ZERO);
    assert_eq!(slot, Some((Idle, t0)));
    assert_eq!(
        observe_activity(&mut slot, Idle, t0 + Duration::from_secs(5)),
        Duration::from_secs(5)
    );
    assert_eq!(slot, Some((Idle, t0)));
    let changed = t0 + Duration::from_secs(7);
    assert_eq!(
        observe_activity(&mut slot, Working, changed),
        Duration::ZERO
    );
    assert_eq!(slot, Some((Working, changed)));
    assert_eq!(
        observe_activity(&mut slot, Working, t0 + Duration::from_secs(9)),
        Duration::from_secs(2)
    );
    assert_eq!(observe_activity(&mut slot, Working, t0), Duration::ZERO);
    assert_eq!(slot, Some((Working, changed)));
}

#[test]
fn duration_display_floors_at_unit_boundaries() {
    for (seconds, expected) in [
        (0, "1s"),
        (1, "1s"),
        (59, "59s"),
        (60, "1m"),
        (3599, "59m"),
        (3600, "1h"),
        (90000, "25h"),
    ] {
        assert_eq!(
            a2amx::messaging::display_duration(Duration::from_secs(seconds)),
            expected
        );
    }
}

async fn create_claude(client: &mut Client, command: &str) -> String {
    match client
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), command.into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Claude,
            deliver: Some(a2amx::harness::Deliver::Auto),
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
        response => panic!("unexpected creation response: {response:?}"),
    }
}

async fn create_claude_team(client: &mut Client, command: &str, team: TeamScope) -> String {
    match client
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), command.into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Claude,
            deliver: Some(a2amx::harness::Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: Some(team),
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

#[tokio::test]
async fn list_activity_reports_working_then_idle_and_non_ready_working() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let prompt_id = create(
        &mut client,
        &["sh", "-i", "-c", "printf '\\033[?2004h'; read x"],
        vec![],
    )
    .await;
    let sleeping_id = create(&mut client, &["sleep", "30"], vec![]).await;

    let sessions = list(&mut client).await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == prompt_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Working)
    );
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == sleeping_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Working)
    );

    tokio::time::sleep(Duration::from_millis(10_200)).await;
    let sessions = list(&mut client).await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == prompt_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Idle)
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn attach_status_reports_starting_latch_and_generic_never_starting() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "activity".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let claude = create_claude_team(
        &mut admin,
        r#"sleep 2; printf '\033[2J\033[?2004h\033[2;1H'; i=0; while [ "$i" -lt 40 ]; do printf '─'; i=$((i+1)); done; printf '\033[3;1H❯ \033[4;1H'; i=0; while [ "$i" -lt 40 ]; do printf '─'; i=$((i+1)); done; printf '\033[3;3H\033[?25h'; sleep 3; printf '\033[2J\033[HEnter to confirm · Esc to cancel'; sleep 30"#,
        team,
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&claude, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(initial) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    assert!(initial.starting);

    let mut saw_ready = false;
    while !saw_ready {
        match next(&mut attachment).await {
            ServerFrame::Status(info) => {
                if !info.starting {
                    saw_ready = true;
                }
            }
            ServerFrame::Data(_) => {}
            frame => panic!("unexpected frame while observing status: {frame:?}"),
        }
    }
    tokio::time::sleep(Duration::from_millis(14_500)).await;
    let mut held_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&claude, true, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(
        next(&mut held_attachment).await,
        ServerFrame::Data(_)
    ));
    let ServerFrame::Status(held_status) = next(&mut held_attachment).await else {
        panic!("held status frame");
    };
    assert!(held_status.hold.is_some());
    assert!(!held_status.starting);
    assert!(
        held_status
            .members
            .iter()
            .any(|member| member.id == claude && member.held)
    );

    let generic = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let mut generic_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&generic, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(
        next(&mut generic_attachment).await,
        ServerFrame::Data(_)
    ));
    let ServerFrame::Status(generic_status) = next(&mut generic_attachment).await else {
        panic!("generic initial status frame");
    };
    assert!(!generic_status.starting);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn agy_never_reports_starting_before_ready() {
    use a2amx::harness::{Deliver, Harness};

    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "agy-starting".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let agy = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("agy"),
        Harness::Agy,
        Deliver::Auto,
        Some(team),
        &[],
        &[],
    )
    .await
    .0;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&agy, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(status) = next(&mut attachment).await else {
        panic!("initial Agy status frame");
    };
    assert!(!status.starting);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_never_ready_team_member_is_not_starting() {
    use a2amx::harness::{Deliver, Harness};

    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "exited-starting".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let exited = create_claude_team(&mut admin, "sleep 0.2", team.clone()).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == exited)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;
    let live = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("live"),
        Harness::Generic,
        Deliver::Auto,
        Some(team),
        &[],
        &[],
    )
    .await
    .0;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&live, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(status) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    let member = status
        .members
        .iter()
        .find(|member| member.id == exited)
        .expect("exited Claude member in team status");
    assert!(member.exited);
    assert!(!member.starting);
    assert!(!member.held);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_needs_input_label_reaches_list_and_attach_status() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = create_claude(
        &mut admin,
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; sleep 30"#,
    )
    .await;
    let initial = list(&mut admin).await;
    let summary = initial.iter().find(|session| session.id == id).unwrap();
    assert_eq!(summary.hold_reason, None);
    assert!(!summary.held);

    tokio::time::sleep(Duration::from_millis(10_200)).await;
    let sessions = eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?") && session.held)
    })
    .await;
    let address = sessions
        .iter()
        .find(|session| session.id == id)
        .unwrap()
        .address
        .clone();

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&id, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    assert_eq!(info.hold.as_deref(), Some("input?"));
    let rendered = String::from_utf8(a2amx::status::render(
        &a2amx::status::StatusBar::new(&id, Some(&info), 2),
        80,
        24,
    ))
    .unwrap();
    assert!(rendered.contains("HELD input?"));
    assert!(!rendered.contains("^B r"));

    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    let sessions = eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("human_draft"))
    })
    .await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == id)
            .unwrap()
            .hold_reason
            .as_deref(),
        Some("human_draft")
    );

    let (_, token, sender_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        a2amx::harness::Harness::Generic,
        a2amx::harness::Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &token).await.unwrap();
    let Response::Accepted { recipient_hold, .. } = sender
        .request(Request::SendMessage {
            to: address,
            subject: "Needs input status".into(),
            message: "hello".into(),
        })
        .await
        .unwrap()
    else {
        panic!("message was not accepted");
    };
    assert_eq!(recipient_hold.as_deref(), Some("human_draft"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_needs_input_clears_when_dialog_marker_disappears() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = create_claude(
        &mut admin,
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; sleep 12; printf '\033[2J\033[Hready'; sleep 30"#,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10_200)).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?"))
    })
    .await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.is_none() && !session.held)
    })
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_claude_session_never_reports_needs_input() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let release = dir.path().join("release");
    let command = format!(
        "printf '\\033[2J\\033[H Enter to confirm · Esc to cancel'; while [ ! -e {} ]; do sleep 0.05; done",
        release.display()
    );
    let id = create_claude(&mut admin, &command).await;
    tokio::time::sleep(Duration::from_millis(10_200)).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?"))
    })
    .await;
    std::fs::write(&release, []).unwrap();
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.exit_code.is_some() && session.hold_reason.is_none())
    })
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_reports_session_durations_until_natural_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "sleep 5"], vec![]).await;

    let sessions = list(&mut client).await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert!(summary.uptime_secs.is_some_and(|seconds| seconds <= 1));
    assert!(summary.activity_secs.is_some_and(|seconds| seconds <= 1));

    tokio::time::sleep(Duration::from_millis(2_200)).await;
    let sessions = list(&mut client).await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert!(
        summary
            .uptime_secs
            .is_some_and(|seconds| (2..=5).contains(&seconds))
    );
    assert!(
        summary
            .activity_secs
            .is_some_and(|seconds| (2..=5).contains(&seconds))
    );

    let sessions = eventually_sessions(&mut client, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert_eq!(summary.uptime_secs, None);
    assert_eq!(summary.activity_secs, None);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn opted_in_attachments_receive_initial_and_changed_status_only() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let Response::Created { session: id } = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "stty raw -echo; cat".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agent-review".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    else {
        panic!("created session");
    };
    let address = list(&mut admin)
        .await
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
        .address;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&id, false, 40, 10, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let info = next(&mut attachment).await;
    let ServerFrame::Status(info) = info else {
        panic!("initial status frame");
    };
    assert_eq!(info.address, address);
    assert_eq!(info.pending, 0);
    assert_eq!(info.hold, None);
    assert_eq!(info.harness, Some(Harness::Generic));
    assert!(info.activity.is_some());
    assert_eq!(info.quota, None);
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .any(|s| s.id == id && s.hold_reason.as_deref() == Some("human_draft"))
    })
    .await;
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(addr, &token).await.unwrap();
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: address.clone(),
                subject: "Status test".into(),
                message: "hello".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(ServerFrame::Status(info)) = attachment.recv().await.unwrap()
                && info.pending == 1
            {
                assert_eq!(info.address, address);
                assert_eq!(info.pending, 1);
                assert_eq!(info.hold, Some("human_draft".into()));
                assert_eq!(info.harness, Some(Harness::Generic));
                assert!(info.activity.is_some());
                assert_eq!(info.quota, None);
                break;
            }
        }
    })
    .await
    .expect("changed status within five seconds");
    // A stable attachment sends no repeated status, and legacy attachments never opt in.
    let other = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let mut legacy = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&other, false, 40, 10)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_millis(2500), async {
        loop {
            tokio::select! {
                frame = legacy.recv() => assert!(!matches!(frame.unwrap(), Some(ServerFrame::Status(_)))),
                frame = attachment.recv() => assert!(!matches!(frame.unwrap(), Some(ServerFrame::Status(_)))),
            }
        }
    }).await.expect_err("observation ends at deadline");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn team_attachment_status_lists_sorted_members_and_exited_state() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "demo".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let first = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("zeta"),
        Harness::Generic,
        Deliver::Auto,
        Some(team.clone()),
        &[],
        &[],
    )
    .await
    .0;
    let second = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("alpha"),
        Harness::Generic,
        Deliver::Auto,
        Some(team.clone()),
        &[],
        &[],
    )
    .await
    .0;
    let other_team = TeamScope {
        name: "other".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let other = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Auto,
        Some(other_team),
        &[],
        &[],
    )
    .await
    .0;
    let ungrouped = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let exited = match admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "exit 0".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            activity: None,
            heartbeat: None,
            name: Some("dead".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            team: Some(team),
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        other => panic!("created exited team session: {other:?}"),
    };
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == exited)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&first, false, 40, 8, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut attachment).await else {
        panic!("initial team status frame");
    };
    assert_eq!(
        info.members
            .iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "dead", "zeta"]
    );
    assert!(
        info.members
            .iter()
            .all(|member| member.activity.is_some() || member.exited)
    );
    assert!(
        info.members
            .iter()
            .any(|member| member.id == exited && member.exited)
    );
    assert!(
        info.members
            .iter()
            .any(|member| member.id == second && !member.exited)
    );

    assert!(
        !info
            .members
            .iter()
            .any(|member| member.id == other || member.id == ungrouped)
    );

    let mut plain = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&ungrouped, false, 40, 8, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut plain).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut plain).await else {
        panic!("initial ungrouped status frame");
    };
    assert!(info.members.is_empty());
    daemon.shutdown().await.unwrap();
}
