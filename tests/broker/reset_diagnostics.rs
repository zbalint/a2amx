use super::*;

async fn claude_reset_target(admin: &mut Client, composer: &str) -> String {
    let rules = "─".repeat(40);
    let command = format!(
        "stty raw -echo; printf '\\033[2J\\033[?2004h\\033[2;1H{rules}\\033[3;1H❯ {composer}\\033[4;1H{rules}\\033[3;3H\\033[?25h'; exec cat >/dev/null"
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
            name: Some("target".into()),
            harness: Harness::Claude,
            deliver: Some(Deliver::Auto),
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
fn timing_seconds(message: &str, field: &str) -> u64 {
    message
        .split_whitespace()
        .find_map(|part| part.strip_prefix(field))
        .and_then(|value| value.strip_suffix('s'))
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("missing timing field {field} in {message}"))
}

#[tokio::test]
async fn reset_diagnostic_busy_names_native_flags() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, _target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    bridge
        .send(BridgeUp::State {
            idle: false,
            pending: false,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "reset gate rejected step 1 of 1: native busy: idle=false in_flight=false waiting=false",
    )
    .await;
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: true,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "native busy: idle=true in_flight=true waiting=false",
    )
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_diagnostic_extension_queue_tracks_state_and_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, _target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    for (text, flags, suffix) in [
        (
            r#"{"type":"state","idle":true,"pending":true,"draft":false}"#,
            "idle=true in_flight=true waiting=false",
            None,
        ),
        (
            r#"{"type":"state","idle":true,"pending":true,"draft":false,"queue":2,"host_pending":true}"#,
            "idle=true in_flight=true waiting=false",
            Some("ext_queue=2 ext_pending=true"),
        ),
        (
            r#"{"type":"state","idle":true,"pending":true,"draft":false,"queue":0,"host_pending":false}"#,
            "idle=true in_flight=true waiting=false",
            Some("ext_queue=0 ext_pending=false"),
        ),
        (
            r#"{"type":"state","idle":false,"pending":true,"draft":false}"#,
            "idle=false in_flight=true waiting=false",
            None,
        ),
        (
            r#"{"type":"state","idle":true,"pending":true,"draft":false,"queue":2,"host_pending":true}"#,
            "idle=true in_flight=true waiting=false",
            Some("ext_queue=2 ext_pending=true"),
        ),
        (
            r#"{"type":"state","idle":false,"pending":true,"draft":false,"queue":2}"#,
            "idle=false in_flight=true waiting=false",
            None,
        ),
        (
            r#"{"type":"state","idle":true,"pending":true,"draft":false,"queue":2,"host_pending":true}"#,
            "idle=true in_flight=true waiting=false",
            Some("ext_queue=2 ext_pending=true"),
        ),
        (
            r#"{"type":"state","idle":false,"pending":true,"draft":false,"host_pending":true}"#,
            "idle=false in_flight=true waiting=false",
            None,
        ),
    ] {
        bridge
            .send(serde_json::from_str(text).unwrap())
            .await
            .unwrap();
        let source = format!("reset gate rejected step 1 of 1: native busy: {flags}");
        let message =
            reset_rejection(address, &token, &target, "busy", suffix.unwrap_or(&source)).await;
        assert!(message.starts_with(&source), "{message}");
        if let Some(suffix) = suffix {
            assert!(message.ends_with(suffix), "{message}");
        } else {
            assert!(!message.contains("ext_"), "{message}");
        }
    }
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn reset_diagnostic_native_timing_survives_reconnect() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    bridge
        .send(BridgeUp::State {
            idle: false,
            pending: false,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let first = reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "native busy: idle=false in_flight=false waiting=false",
    )
    .await;
    assert!(first.contains("idle_for="), "{first}");
    assert!(first.contains("reported="), "{first}");
    assert!(timing_seconds(&first, "idle_for=") >= 1, "{first}");

    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: true,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    let second = reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "native busy: idle=true in_flight=true waiting=false",
    )
    .await;
    assert!(timing_seconds(&second, "idle_for=") <= 1, "{second}");
    bridge
        .send(BridgeUp::State {
            idle: false,
            pending: true,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "native busy: idle=false in_flight=true waiting=false",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1100)).await;

    drop(bridge);
    reset_rejection(
        address,
        &token,
        &target,
        "not_ready",
        "native bridge not connected",
    )
    .await;
    let mut reconnected = common::eventually(|| async {
        Client::connect_addr(daemon.addrs()[0], &target_token)
            .await
            .ok()?
            .bridge()
            .await
            .ok()
    })
    .await;
    reconnected
        .send(BridgeUp::Hello {
            protocol: 1,
            omp_version: "18.4.5".into(),
            missing: vec![],
        })
        .await
        .unwrap();
    assert!(matches!(
        reconnected.recv().await.unwrap(),
        Some(BridgeDown::Ready)
    ));
    reconnected
        .send(BridgeUp::State {
            idle: false,
            pending: true,
            draft: true,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    let third = reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "native draft=true",
    )
    .await;
    assert!(timing_seconds(&third, "idle_for=") >= 1, "{third}");
    assert!(timing_seconds(&third, "reported=") <= 1, "{third}");
    reconnected
        .send(BridgeUp::State {
            idle: true,
            pending: true,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    let after_reconnect = reset_rejection(
        address,
        &token,
        &target,
        "busy",
        "native busy: idle=true in_flight=true waiting=false",
    )
    .await;
    assert!(!after_reconnect.contains("ext_"), "{after_reconnect}");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_diagnostic_draft_names_native_flag() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, _target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: true,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "native draft=true",
    )
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_diagnostic_hold_names_human_draft() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, _target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: true,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "native draft=true",
    )
    .await;
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
    let message = reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "hold human_draft",
    )
    .await;
    assert!(!message.contains("idle_for="), "{message}");
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn reset_stale_omp_draft_hold_clears_after_margin() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (target, mut bridge, address, token, _target_token) =
        omp_reset_target(&mut admin, dir.path(), daemon.addrs()[0]).await;
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: true,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "native draft=true",
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(vec![0x1b]))
        .await
        .unwrap();
    reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "hold human_draft",
    )
    .await;
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(reset_success(address, &token, &target).await, 1);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn reset_stale_claude_draft_hold_clears_after_margin() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = claude_reset_target(&mut admin, "").await;
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
async fn reset_nonempty_claude_composer_stays_held_after_margin() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let target = claude_reset_target(&mut admin, "draft").await;
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
    reset_rejection_admin(dir.path(), &target, "draft_present", "hold human_draft").await;
    daemon.shutdown().await.unwrap();
}
