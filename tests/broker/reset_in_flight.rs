use super::*;

#[tokio::test]
async fn reset_real_omp_draft_hold_survives_margin_then_clears() {
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
    reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "hold human_draft",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(2200)).await;
    let message = reset_rejection(
        address,
        &token,
        &target,
        "draft_present",
        "hold human_draft",
    )
    .await;
    assert!(!message.contains("native draft=true"), "{message}");
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
    assert_eq!(reset_success(address, &token, &target).await, 1);
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
            queue: None,
            host_pending: None,
            provider: None,
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
