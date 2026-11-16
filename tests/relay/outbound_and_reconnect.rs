use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_outbound_post_reports_undeliverable() {
    let Some((home, daemon)) = common::start_confined_daemon().await else {
        return;
    };
    let discord = FakeDiscord::new("123", 2).await;
    let mut admin = a2amx::client::Client::connect(home.path()).await.unwrap();
    let fixture = tempdir().unwrap();
    let (_session, token) = common::create_confined_with_token(
        &mut admin,
        fixture.path(),
        "fail-agent",
        Some(RemoteMode::Both),
        Some(common::team_scope("fail-team")),
        a2amx::harness::Harness::Generic,
        Some(a2amx::harness::Deliver::Hold),
    )
    .await;
    let mut gw = common::gateway(home.path(), daemon.addrs()[0]).await;
    common::issue_and_redeem(&mut admin, &mut gw, "fail-team").await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"10ms\"\nidle = \"10ms\"\nmember_check = \"50ms\"\nreconnect = \"50ms\"\n",
        0o600,
    );
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: None,
        },
    ));
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    let _ = control;
    let agent_channel = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "fail-agent")
            .map(|(id, _, _, _)| id)
    })
    .await;
    // Discord posts fail for the whole outbound; set before the send so the 10ms pump
    // can never win the race and post successfully first.
    discord.set_fail_posts(true).await;
    let mut agent = a2amx::client::Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    let outbound_id = match agent
        .request(Request::SendMessage {
            to: "human:owner".into(),
            subject: "Will fail".into(),
            message: "the gateway cannot post this".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                agent
                    .request(Request::MessageStatus {
                        id: outbound_id.clone(),
                    })
                    .await
                    .unwrap(),
                Response::Status { message }
                    if message.state == "undeliverable"
                        && message.detail.as_deref() == Some("the gateway could not send it")
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        discord.posts(&agent_channel).is_empty(),
        "failed posts must not be captured"
    );
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_channel_creation_reports_undeliverable() {
    let Some((home, daemon)) = common::start_confined_daemon().await else {
        return;
    };
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"60s\"\nidle = \"10ms\"\nmember_check = \"50ms\"\nreconnect = \"50ms\"\n",
        0o600,
    );
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: None,
        },
    ));
    let _control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.set_fail_channel_creates(true).await;

    let mut admin = a2amx::client::Client::connect(home.path()).await.unwrap();
    let fixture = tempdir().unwrap();
    let (_session, token) = common::create_confined_with_token(
        &mut admin,
        fixture.path(),
        "need-channel",
        Some(RemoteMode::Both),
        Some(common::team_scope("need-team")),
        a2amx::harness::Harness::Generic,
        Some(a2amx::harness::Deliver::Hold),
    )
    .await;
    let mut gateway = common::gateway(home.path(), daemon.addrs()[0]).await;
    common::issue_and_redeem(&mut admin, &mut gateway, "need-team").await;

    let mut agent = a2amx::client::Client::connect_addr(daemon.addrs()[0], &token)
        .await
        .unwrap();
    let outbound_id = match agent
        .request(Request::SendMessage {
            to: "human:owner".into(),
            subject: "Missing channel".into(),
            message: "the gateway cannot create its channel".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                agent
                    .request(Request::MessageStatus {
                        id: outbound_id.clone(),
                    })
                    .await
                    .unwrap(),
                Response::Status { message }
                    if message.state == "undeliverable"
                        && message.detail.as_deref() == Some("the gateway could not send it")
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        discord
            .channels()
            .iter()
            .all(|(_, _, name, _)| name != "need-channel")
    );
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn daemon_down_answers_only_the_paired_author_and_reconnects_after_restart() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"10ms\"\nidle = \"10ms\"\nmember_check = \"50ms\"\nreconnect = \"50ms\"\n",
        0o600,
    );
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let config = discord.discord_config();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run(
        loaded,
        config,
        Io {
            stop: Box::pin(async move {
                let _ = stop_rx.await;
            }),
            pair: None,
        },
    ));
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    daemon.shutdown().await.unwrap();
    // The fake gateway preserves frame order. Queue ignored authors before the
    // paired event, so observing the paired reply proves all three were handled.
    discord.push_message(&control, "bot", "bot").await.unwrap();
    discord
        .push_message(&control, "999", "stranger")
        .await
        .unwrap();
    discord
        .push_message(&control, "456", "paired")
        .await
        .unwrap();
    // A paired event on a second channel is a FIFO sentinel: its reply cannot
    // be posted until the three queued control events have been handled.
    let sentinel_channel = "999999";
    discord
        .push_message(sentinel_channel, "456", "sentinel")
        .await
        .unwrap();
    eventually(|| async { (discord.posts(sentinel_channel).len() == 1).then_some(()) }).await;
    assert_eq!(discord.posts(&control).len(), 1);
    assert_eq!(
        discord.posts(&control)[0]["content"],
        a2amx::gateway::UNREACHABLE_LINE
    );

    // Restart the daemon in the same state directory: the relay must rebuild with
    // connect_gateway, which rereads addr and gateway.token on every attempt.
    let restarted = common::start_daemon_in(home.path(), Some("host-a"), Default::default()).await;
    // A well-formed unknown 12-hex code in the control channel is answered with the
    // fixed line once the daemon connection is rebuilt; pushes retried until then get
    // the unreachable line and do not advance the counter beyond the paired event.
    let code = "00000000aaaa";
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let count = discord.posts(&control).len();
            let connected = count >= 2
                && discord.posts(&control)[count - 1]["content"] == "that code is not valid";
            if connected {
                break;
            }
            discord.push_message(&control, "456", code).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    restarted.shutdown().await.unwrap();
    let _ = stop_tx.send(());
    let result = settled(task).await;
    assert!(
        result.is_ok(),
        "relay stopped with Err instead of Ok: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_waits_for_configured_interval() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"10ms\"\nidle = \"10ms\"\nmember_check = \"50ms\"\nreconnect = \"400ms\"\n",
        0o600,
    );
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(async move {
                let _ = stop_rx.await;
            }),
            pair: None,
        },
    ));
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;

    daemon.shutdown().await.unwrap();
    discord
        .push_message(&control, "456", "trigger failure")
        .await
        .unwrap();
    eventually(|| async {
        (discord.posts(&control).len() == 1
            && discord.posts(&control)[0]["content"] == a2amx::gateway::UNREACHABLE_LINE)
            .then_some(())
    })
    .await;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_task = Arc::clone(&attempts);
    let listener_task = tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            attempts_for_task.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    write_file(
        &home.path().join("run/link/addr"),
        &format!("{address}\n"),
        0o600,
    );

    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    tokio::time::sleep(std::time::Duration::from_millis(450)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);

    listener_task.abort();
    let _ = listener_task.await;
    let _ = stop_tx.send(());
    assert!(settled(task).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_run_with_trace_logs_leak_no_token() {
    // Binary seam: RUST_LOG=trace must not reveal the token or Authorization even
    // though tungstenite and ureq would print them without the unoverridable layer
    // filter (the red-first run without the filter shows exactly that). The run is
    // SIGKILLed after one control-channel exchange; its output is scanned afterwards.
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    a2amx::relay_files::write_token(home.path(), "fake-token").unwrap();
    write_file(
        &home.path().join("config/gateway.toml"),
        &format!(
            "guild_id = \"123\"\napi_base = \"{}\"\ngateway_url = \"{}\"\n",
            discord.api_base, discord.gateway_url
        ),
        0o600,
    );
    let home_path = home.path().to_owned();
    let child = tokio::task::spawn_blocking(move || {
        ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(home_path)
            .args(["gateway", "run"])
            .env("RUST_LOG", "trace")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();
    let control = match tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let found = discord
                .channels()
                .into_iter()
                .find(|(_, kind, name, _)| *kind == 0 && name == "control")
                .map(|(id, _, _, _)| id);
            if let Some(id) = found {
                return id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    {
        Ok(id) => id,
        Err(_) => {
            let output = kill_and_wait(child).await;
            eprintln!("child stdout: {}", String::from_utf8_lossy(&output.stdout));
            eprintln!("child stderr: {}", String::from_utf8_lossy(&output.stderr));
            panic!("control channel never appeared");
        }
    };
    discord
        .push_message(&control, "456", "not-a-code")
        .await
        .unwrap();
    eventually(|| async { (!discord.posts(&control).is_empty()).then_some(()) }).await;
    let output = kill_and_wait(child).await;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let token = "fake-token".as_bytes();
    let hex_lower: String = token.iter().map(|byte| format!("{byte:02x}")).collect();
    let hex_upper: String = token.iter().map(|byte| format!("{byte:02X}")).collect();
    assert!(!text.contains("fake-token"));
    assert!(!text.contains("Authorization"));
    assert!(!text.contains(&hex_lower));
    assert!(!text.contains(&hex_upper), "hex token found in output");
    assert!(text.lines().any(|line| line == "gateway ready"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relay_end_to_end_confined_both_and_send_agents() {
    let Some((home, daemon)) = common::start_confined_daemon().await else {
        return;
    };
    let discord = FakeDiscord::new("123", 2).await;
    let mut admin = a2amx::client::Client::connect(home.path()).await.unwrap();
    let both_fixture = tempdir().unwrap();
    let (both_session, _both_token) = common::create_confined_with_token(
        &mut admin,
        both_fixture.path(),
        "both-agent",
        Some(RemoteMode::Both),
        Some(common::team_scope("both-team")),
        a2amx::harness::Harness::Generic,
        Some(a2amx::harness::Deliver::Hold),
    )
    .await;
    let send_fixture = tempdir().unwrap();
    let (_send_session, send_token) = common::create_confined_with_token(
        &mut admin,
        send_fixture.path(),
        "send-agent",
        Some(RemoteMode::Send),
        Some(common::team_scope("send-team")),
        a2amx::harness::Harness::Generic,
        Some(a2amx::harness::Deliver::Hold),
    )
    .await;
    let no_lease_fixture = tempdir().unwrap();
    let _no_lease = common::create_confined_with_team(
        &mut admin,
        no_lease_fixture.path(),
        "unleased-agent",
        Some(RemoteMode::Both),
        Some(common::team_scope("unleased-team")),
    )
    .await;
    let gateway = common::gateway(home.path(), daemon.addrs()[0]).await;
    let both_code = common::issue_code(&mut admin, "both-team").await;
    let send_code = common::issue_code(&mut admin, "send-team").await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"10ms\"\nidle = \"10ms\"\nmember_check = \"20ms\"\n",
        0o600,
    );
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let mut discord_config = discord.discord_config();
    discord_config.token = loaded.token.clone();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run(
        loaded,
        discord_config,
        Io {
            stop: Box::pin(async move {
                let _ = stop_rx.await;
            }),
            pair: None,
        },
    ));
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord
        .push_message(&control, "456", &both_code)
        .await
        .unwrap();
    discord
        .push_message(&control, "456", &send_code)
        .await
        .unwrap();
    eventually(|| async { (discord.posts(&control).len() >= 2).then_some(()) }).await;
    let both_channel = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "both-agent")
            .map(|(id, _, _, _)| id)
    })
    .await;
    let send_channel = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "send-agent")
            .map(|(id, _, _, _)| id)
    })
    .await;
    assert!(
        !discord
            .channels()
            .iter()
            .any(|(_, _, name, _)| name == "unleased-agent")
    );

    let before_inbound = match admin
        .request(Request::ListMessages {
            session: Some(both_session.clone()),
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages.len(),
        response => panic!("unexpected messages response: {response:?}"),
    };
    discord
        .push_message(&both_channel, "999", "ignored other")
        .await
        .unwrap();
    discord
        .push_message(&both_channel, "456", "inbound to both")
        .await
        .unwrap();
    let inbound = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let Response::Messages { messages } = admin
                .request(Request::ListMessages {
                    session: Some(both_session.clone()),
                    state: None,
                })
                .await
                .unwrap()
            else {
                panic!("unexpected messages response");
            };
            if let Some(message) = messages.into_iter().find(|message| {
                message.to == "both-agent@host-a"
                    && message.state == "pending"
                    && message.subject == a2amx::messaging::HUMAN_SUBJECT
            }) {
                break message;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(inbound.state, "pending");
    assert_eq!(inbound.to, "both-agent@host-a");

    let after_inbound = match admin
        .request(Request::ListMessages {
            session: Some(both_session.clone()),
            state: None,
        })
        .await
        .unwrap()
    {
        Response::Messages { messages } => messages.len(),
        response => panic!("unexpected messages response: {response:?}"),
    };
    assert_eq!(after_inbound, before_inbound + 1);
    let mut sender = a2amx::client::Client::connect_addr(daemon.addrs()[0], &send_token)
        .await
        .unwrap();
    let long_body = "x".repeat(4500);
    let outbound_id = match sender
        .request(Request::SendMessage {
            to: "human:owner".into(),
            subject: "Long".into(),
            message: long_body,
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected send response: {response:?}"),
    };
    eventually(|| async { (discord.posts(&send_channel).len() >= 3).then_some(()) }).await;
    let posts = discord.posts(&send_channel);
    assert!(posts.iter().all(|post| {
        post["content"]
            .as_str()
            .is_some_and(|body| body.chars().count() <= 2000)
            && post["allowed_mentions"] == serde_json::json!({"parse": []})
    }));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                sender
                    .request(Request::MessageStatus {
                        id: outbound_id.clone(),
                    })
                    .await
                    .unwrap(),
                Response::Status { message } if message.state == "submitted"
            ) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let prior_send_posts = discord.posts(&send_channel).len();
    discord
        .push_message(&send_channel, "456", "cannot send")
        .await
        .unwrap();
    eventually(|| async {
        (discord.posts(&send_channel).len() == prior_send_posts + 1).then_some(())
    })
    .await;
    assert_eq!(
        discord.posts(&send_channel).last().unwrap()["content"],
        "that agent is not taking messages from you"
    );

    let _ = stop_tx.send(());
    assert!(settled(task).await.is_ok());
    drop(gateway);
    daemon.shutdown().await.unwrap();
}
