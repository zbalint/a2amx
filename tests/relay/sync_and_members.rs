use super::*;

#[tokio::test]
async fn relay_starts_syncs_control_channel_and_stops() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let mut loaded = relay_files::load(home.path()).unwrap();
    loaded.token = "fake-token".into();
    let mut config = discord.discord_config();
    config.token = loaded.token.clone();
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
    eventually(|| async { (discord.channels().len() >= 2).then_some(()) }).await;
    let _ = stop_tx.send(());
    assert!(settled(task).await.is_ok());
    assert!(
        discord
            .channels()
            .iter()
            .any(|(_, kind, name, _)| *kind == 4 && name == "a2amx")
    );
    assert!(
        discord
            .channels()
            .iter()
            .any(|(_, kind, name, _)| *kind == 0 && name == "control")
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn member_limit_above_max_fails_the_strict_start() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 3).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        run(
            loaded,
            discord.discord_config(),
            Io {
                stop: Box::pin(std::future::pending()),
                pair: None,
            },
        ),
    )
    .await
    .expect("the strict start check must fail quickly")
    .unwrap_err()
    .to_string();
    assert!(error.contains("server has 3 members, the limit is 2"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn unpaired_relay_refuses_without_pairing_record() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let error = run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: None,
        },
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(error.contains("not paired; run with --pair"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn member_limit_pauses_inbound_and_resumes_at_limit() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nmember_check = \"10ms\"\nsync = \"10ms\"\nidle = \"10ms\"\n",
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
    let pause_checks = discord.set_member_count(3).await;
    wait_for_member_checks(&discord, pause_checks).await;
    let event_checks = discord.member_count_requests();
    discord
        .push_message(&control, "456", "ignored while paused")
        .await
        .unwrap();
    wait_for_member_checks(&discord, event_checks).await;
    assert!(discord.posts(&control).is_empty());
    let resume_checks = discord.set_member_count(2).await;
    wait_for_member_checks(&discord, resume_checks).await;
    discord
        .push_message(&control, "456", "not-a-code")
        .await
        .unwrap();
    eventually(|| async { (discord.posts(&control).len() == 1).then_some(()) }).await;
    assert_eq!(
        discord.posts(&control)[0]["content"],
        "send the code that a2amx team up printed"
    );
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn member_pause_holds_agent_delivery_and_channels() {
    let Some((home, daemon)) = common::start_confined_daemon().await else {
        return;
    };
    let discord = FakeDiscord::new("123", 2).await;
    let mut admin = a2amx::client::Client::connect(home.path()).await.unwrap();
    let q_fixture = tempdir().unwrap(); // pre-pause agent: leased and known to the relay's first sync
    let (_q_session, q_token) = common::create_confined_with_token(
        &mut admin,
        q_fixture.path(),
        "q-pause",
        Some(RemoteMode::Both),
        Some(common::team_scope("q-team")),
        a2amx::harness::Harness::Generic,
        Some(a2amx::harness::Deliver::Hold),
    )
    .await;
    let mut gw = common::gateway(home.path(), daemon.addrs()[0]).await;
    common::issue_and_redeem(&mut admin, &mut gw, "q-team").await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
        "guild_id = \"123\"\nsync = \"10ms\"\nidle = \"10ms\"\nmember_check = \"20ms\"\nreconnect = \"50ms\"\n",
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
    let _q_channel = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "q-pause")
            .map(|(id, _, _, _)| id)
    })
    .await;

    // Values at paused depth frame:
    let pause_checks = discord.set_member_count(3).await;
    wait_for_member_checks(&discord, pause_checks).await;
    if !discord.posts(&control).is_empty() {
        panic!(
            "unexpected control posts before the pause push: {:?}",
            discord.posts(&control)
        );
    }
    let event_checks = discord.member_count_requests();
    discord
        .push_message(&control, "456", "held while paused")
        .await
        .unwrap();
    wait_for_member_checks(&discord, event_checks).await;
    assert!(
        discord.posts(&control).is_empty(),
        "inbound dropped while paused, posts: {:?}",
        discord.posts(&control)
    );
    // A new agent marked remote during the pause: the paused sync must not ensure its
    // channel; the fact that a bound channel exists proves paused still creating a channel.
    let u_fixture = tempdir().unwrap(); // post-pause agent
    let _u_session = common::create_confined_with_team(
        &mut admin,
        u_fixture.path(),
        "u-pause",
        Some(RemoteMode::Both),
        Some(common::team_scope("u-team")),
    )
    .await;
    common::issue_and_redeem(&mut admin, &mut gw, "u-team").await;
    let sync_checks = discord.member_count_requests();
    wait_for_member_checks(&discord, sync_checks).await;
    assert!(
        !discord
            .channels()
            .iter()
            .any(|(_, _, name, _)| name == "u-pause"),
        "paused sync must not create channels"
    );

    // A pending agent message while paused: the paused pump must not post it; resuming
    // count returns to a sync that ensures u-pause and a pump that delivers the body.
    let mut q_client = a2amx::client::Client::connect_addr(daemon.addrs()[0], &q_token)
        .await
        .unwrap();
    let q_id = match q_client
        .request(Request::SendMessage {
            to: "human:owner".into(),
            subject: "Paused note".into(),
            message: "held during pause".into(),
        })
        .await
        .unwrap()
    {
        Response::Accepted { id, .. } => id,
        response => panic!("unexpected human send response: {response:?}"),
    };
    let pump_checks = discord.member_count_requests();
    wait_for_member_checks(&discord, pump_checks).await;
    let q_channel = discord
        .channels()
        .into_iter()
        .find(|(_, kind, name, _)| *kind == 0 && name == "q-pause")
        .map(|(id, _, _, _)| id)
        .unwrap();
    assert!(
        discord.posts(&q_channel).is_empty(),
        "paused pump must not post the pending human message"
    );

    let resume_checks = discord.set_member_count(2).await;
    wait_for_member_checks(&discord, resume_checks).await;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let has_u = discord
                .channels()
                .iter()
                .any(|(_, _, name, _)| name == "u-pause");
            let q_channel = discord
                .channels()
                .into_iter()
                .find(|(_, kind, name, _)| *kind == 0 && name == "q-pause")
                .map(|(id, _, _, _)| id);
            let posted = q_channel
                .as_deref()
                .is_some_and(|channel| !discord.posts(channel).is_empty());
            if has_u && posted {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // The delivered message is recorded submitted by the daemon.
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if matches!(
                q_client
                    .request(Request::MessageStatus { id: q_id.clone() })
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
    drop(q_client);
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unlisted_agent_channel_is_unbound() {
    let Some((home, daemon)) = common::start_confined_daemon().await else {
        return;
    };
    let discord = FakeDiscord::new("123", 2).await;
    let mut admin = a2amx::client::Client::connect(home.path()).await.unwrap();
    let fixture = tempdir().unwrap();
    let credentials = tempfile::NamedTempFile::new_in(fixture.path()).unwrap();
    let response = admin
        .request(Request::Confine {
            confine: a2amx::confine::ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec![
                    "sh".into(),
                    "-c".into(),
                    "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; sleep 1"
                        .into(),
                ],
                cols: 40,
                rows: 10,
                cwd: Some(fixture.path().to_string_lossy().into_owned()),
                env: vec![(
                    "OUT".into(),
                    credentials.path().to_string_lossy().into_owned(),
                )],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some("leaving-agent".into()),
                harness: a2amx::harness::Harness::Generic,
                deliver: Some(a2amx::harness::Deliver::Hold),
                team: Some(common::team_scope("unbind-team")),
                role: None,
                remote: Some(RemoteMode::Both),
            }),
        })
        .await
        .unwrap_or_else(|error| panic!("confined session creation errored: {error:?}"));
    let Response::Created { .. } = response else {
        panic!("confined session creation failed: {response:?}");
    };
    let (_token, _addr) = eventually(|| async {
        let text = std::fs::read_to_string(credentials.path()).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let addr = lines.next()?.parse::<std::net::SocketAddr>().ok()?;
        (token.len() == 64).then_some((token, addr))
    })
    .await;
    let mut gw = common::gateway(home.path(), daemon.addrs()[0]).await;
    common::issue_and_redeem(&mut admin, &mut gw, "unbind-team").await;
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
    let _control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    // Correct behavior: while the agent is listed its channel is bound; once the
    // session exits the next sync tick binds no more. The paired-author message in
    // that channel then always becomes the not-linked fixed line, and the mutation
    // dropping the unbind answers "that agent is not running" instead.
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let Some(channel) = discord
                .channels()
                .into_iter()
                .find(|(_, kind, name, _)| *kind == 0 && name == "leaving-agent")
                .map(|(id, _, _, _)| id)
            else {
                tokio::task::yield_now().await;
                continue;
            };
            let checks = discord.member_count_requests();
            discord
                .push_message(&channel, "456", "any text")
                .await
                .unwrap();
            wait_for_member_checks(&discord, checks).await;
            let posts = discord.posts(&channel);
            if posts.last().is_some_and(|post| {
                post["content"] == "that channel is not linked to a running agent"
            }) {
                break;
            }
        }
    })
    .await
    .unwrap();
    // The Discord channel itself stays (only the relay's binding is cleared).
    assert!(
        discord
            .channels()
            .iter()
            .any(|(_, kind, name, _)| *kind == 0 && name == "leaving-agent")
    );
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}
