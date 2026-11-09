#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::messaging::RemoteMode;
use a2amx::relay::{Io, run};
use a2amx::relay_files::{self, Loaded};
use a2amx::wire::{Request, Response};
use common::eventually;
use common::fake_discord::FakeDiscord;
use common::start_daemon;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tempfile::tempdir;

// A relay mutation can leave the run task in a state where it never settles; awaiting
// it directly would hang instead of failing. Every direct await of the relay task is
// wrapped in this helper so the suite reports a failure in bounded time.
async fn settled(task: tokio::task::JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), task)
        .await
        .expect("relay task must settle");
    result.expect("relay task finished without panic")
}
async fn kill_and_wait(child: std::process::Child) -> std::process::Output {
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        child.kill().unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn write_file(path: &std::path::Path, text: &str, mode: u32) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn valid_gateway(home: &std::path::Path) {
    let gateway = home.join("gateway");
    fs::create_dir_all(&gateway).unwrap();
    fs::set_permissions(&gateway, fs::Permissions::from_mode(0o700)).unwrap();
    write_file(&gateway.join("discord.token"), "bot-token\n", 0o600);
    write_file(&gateway.join("config.toml"), "guild_id = \"123\"\n", 0o600);
}
async fn wait_for_member_checks(discord: &FakeDiscord, baseline: u64) {
    eventually(|| async { (discord.member_count_requests() >= baseline + 2).then_some(()) }).await;
}

#[test]
fn load_reads_defaults_and_pairing() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    assert_eq!(loaded.config.guild_id, "123");
    assert_eq!(loaded.config.control_category, "a2amx");
    assert_eq!(loaded.config.control_channel, "control");
    assert_eq!(loaded.config.max_members, 2);
    assert_eq!(loaded.pairing.as_ref().unwrap().user_id, "456");
    assert_eq!(loaded.token, "bot-token");
}

#[test]
fn load_rejects_unknown_config_key() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    fs::OpenOptions::new()
        .append(true)
        .open(home.path().join("gateway/config.toml"))
        .unwrap()
        .write_all(b"unknown = true\n")
        .unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("config.toml"));
    assert!(error.contains("unknown"));
}

#[test]
fn loaded_type_is_public_contract() {
    fn assert_loaded(_: Loaded) {}
    let _ = assert_loaded;
}

#[test]
fn load_rejects_loose_modes_for_each_secret_file() {
    for file in ["discord.token", "config.toml", "pairing.json"] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        if file == "pairing.json" {
            write_file(
                &home.path().join("gateway/pairing.json"),
                r#"{"guild_id":"123","user_id":"456"}"#,
                0o644,
            );
        } else {
            fs::set_permissions(
                home.path().join("gateway").join(file),
                fs::Permissions::from_mode(0o644),
            )
            .unwrap();
        }
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        assert!(error.contains(file), "{error}");
    }
}

#[test]
fn load_rejects_symlinked_secret_file() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    let target = home.path().join("token-target");
    write_file(&target, "bot-token\n", 0o600);
    fs::remove_file(home.path().join("gateway/discord.token")).unwrap();
    std::os::unix::fs::symlink(&target, home.path().join("gateway/discord.token")).unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("discord.token"), "{error}");
}

#[test]
fn load_rejects_bad_values_and_oversized_config() {
    for config in [
        "guild_id = \"not-numeric\"\n",
        "guild_id = \"123\"\ncontrol_channel = \"a_BAD\"\n",
        "guild_id = \"123\"\nidle_ms = 0\n",
    ] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        write_file(&home.path().join("gateway/config.toml"), config, 0o600);
        assert!(relay_files::load(home.path()).is_err(), "{config}");
    }
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/config.toml"),
        &format!("guild_id = \"123\"\n#{}\n", "x".repeat(70 * 1024)),
        0o600,
    );
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("64 KiB"), "{error}");
}

#[test]
fn load_rejects_pairing_for_another_guild() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/pairing.json"),
        r#"{"guild_id":"999","user_id":"456"}"#,
        0o600,
    );
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("pairing"), "{error}");
}

#[tokio::test]
async fn relay_starts_syncs_control_channel_and_stops() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/pairing.json"),
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

#[test]
fn gateway_token_cli_reads_stdin_and_writes_trimmed_secret() {
    let home = tempdir().unwrap();
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home", home.path().to_str().unwrap(), "gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"  secret-token  \n")
        .unwrap();
    let output = command.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert_eq!(
        fs::read_to_string(home.path().join("gateway/discord.token")).unwrap(),
        "secret-token"
    );
    assert_eq!(
        fs::metadata(home.path().join("gateway/discord.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn gateway_token_cli_rejects_oversized_stdin_without_writing() {
    let home = tempdir().unwrap();
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home", home.path().to_str().unwrap(), "gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&vec![b'x'; 64 * 1024 + 1])
        .unwrap();
    let output = command.wait_with_output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("gateway token exceeds 64 KiB"));
    assert!(!home.path().join("gateway/discord.token").exists());
}

#[tokio::test]
async fn gateway_client_uses_gateway_token_file() {
    let (home, daemon) = start_daemon().await;
    let admin_token = fs::read_to_string(home.path().join("admin.token")).unwrap();
    let gateway_token = fs::read_to_string(home.path().join("gateway.token")).unwrap();
    assert_ne!(admin_token, gateway_token);
    let mut gateway = a2amx::client::Client::connect_gateway(home.path())
        .await
        .unwrap();
    assert!(matches!(
        gateway.request(Request::GatewayAgents).await.unwrap(),
        Response::GatewayAgents { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pairing_true_confirmation_writes_record_with_ids() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let config = discord.discord_config();
    let announcement = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let prompt = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let announcement_for_io = std::sync::Arc::clone(&announcement);
    let prompt_for_io = std::sync::Arc::clone(&prompt);
    let task = tokio::spawn(run(
        loaded,
        config,
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    *announcement_for_io.lock().unwrap() = Some(line.to_owned());
                }),
                confirm: Box::new(move |line| {
                    *prompt_for_io.lock().unwrap() = Some(line.to_owned());
                    true
                }),
            }),
        },
    ));
    let code = eventually(|| async {
        announcement.lock().unwrap().clone().and_then(|line| {
            line.rsplit_once(':')
                .map(|(_, code)| code.trim().to_owned())
        })
    })
    .await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord
        .push_message(&control, "789", "wrong")
        .await
        .unwrap();
    discord.push_message(&control, "789", &code).await.unwrap();
    // Wrong texts and the control category/channel are established by run(); pairing.json
    // records the confirmed account once the code matched and the prompt returned true.
    eventually(|| async { fs::read_to_string(home.path().join("gateway/pairing.json")).ok() })
        .await;
    let record = fs::read_to_string(home.path().join("gateway/pairing.json")).unwrap();
    let pairing: a2amx::relay_files::Pairing = serde_json::from_str(&record).unwrap();
    assert_eq!(pairing.guild_id, "123");
    assert_eq!(pairing.user_id, "789");
    assert!(prompt.lock().unwrap().as_deref() == Some("bind Discord account 789? [y/N]"));
    task.abort();
    let _ = task.await;
    assert_eq!(
        fs::metadata(home.path().join("gateway/pairing.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pairing_false_confirmation_writes_no_record() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let config = discord.discord_config();
    let announcement = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let prompt = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let announcement_for_io = std::sync::Arc::clone(&announcement);
    let prompt_for_io = std::sync::Arc::clone(&prompt);
    let task = tokio::spawn(run(
        loaded,
        config,
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    *announcement_for_io.lock().unwrap() = Some(line.to_owned());
                }),
                confirm: Box::new(move |line| {
                    *prompt_for_io.lock().unwrap() = Some(line.to_owned());
                    false
                }),
            }),
        },
    ));
    eventually(|| async {
        announcement.lock().unwrap().clone().and_then(|line| {
            line.rsplit_once(':')
                .map(|(_, code)| code.trim().to_owned())
        })
    })
    .await;
    let code = announcement
        .lock()
        .unwrap()
        .clone()
        .unwrap()
        .rsplit_once(':')
        .unwrap()
        .1
        .trim()
        .to_owned();
    let control = common::eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord
        .push_message(&control, "789", "wrong")
        .await
        .unwrap();
    discord.push_message(&control, "789", &code).await.unwrap();
    let result = settled(task).await;
    assert!(result.unwrap_err().to_string().contains("not confirmed"));
    assert!(prompt.lock().unwrap().as_deref() == Some("bind Discord account 789? [y/N]"));
    assert!(!home.path().join("gateway/pairing.json").exists());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn member_limit_above_max_fails_the_strict_start() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 3).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nmember_check_ms = 10\nsync_ms = 10\nidle_ms = 10\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 20\nreconnect_ms = 50\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 50\nreconnect_ms = 50\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 60000\nidle_ms = 10\nmember_check_ms = 50\nreconnect_ms = 50\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 50\nreconnect_ms = 50\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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

#[tokio::test]
async fn daemon_down_answers_only_the_paired_author_and_reconnects_after_restart() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 50\nreconnect_ms = 50\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 50\nreconnect_ms = 400\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
        &home.path().join("link/addr"),
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
        &home.path().join("gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    a2amx::relay_files::write_token(home.path(), "fake-token").unwrap();
    write_file(
        &home.path().join("gateway/config.toml"),
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
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gateway_run_pair_binary_prompts_and_writes_record() {
    // Binary seam with piped input: the announcement lands on standard error, any
    // author may post the code into the control channel, only a terminal-style
    // confirm writes the pairing record, and the run later stops on a signal.
    let (home, _daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    a2amx::relay_files::write_token(home.path(), "fake-token").unwrap();
    write_file(
        &home.path().join("gateway/config.toml"),
        &format!(
            "guild_id = \"123\"\napi_base = \"{}\"\ngateway_url = \"{}\"\n",
            discord.api_base, discord.gateway_url
        ),
        0o600,
    );
    let home_path = home.path().to_owned();
    let mut child = tokio::task::spawn_blocking(move || {
        ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(home_path)
            .args(["gateway", "run", "--pair"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();
    let stderr = child.stderr.take().unwrap();
    let stderr_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let copy = std::sync::Arc::clone(&stderr_text);
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(stderr);
        let mut buffer = [0_u8; 1024];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            copy.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buffer[..count]));
        }
    });
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    let code = eventually(|| async {
        let text = stderr_text.lock().unwrap().clone();
        let line = text
            .lines()
            .find(|line| line.contains("post this code in the control channel:"))?;
        line.rsplit_once(' ').map(|(_, code)| code.to_owned())
    })
    .await;
    assert_eq!(code.len(), 12);
    assert!(
        code.as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)),
    );
    // Wrong and cross-channel texts are ignored; the matched code confirms from any author.
    discord
        .push_message(&control, "999", "not the code")
        .await
        .unwrap();
    discord.push_message(&control, "999", &code).await.unwrap();
    eventually(|| async {
        let text = stderr_text.lock().unwrap().clone();
        text.contains("bind Discord account 999? [y/N]")
            .then_some(())
    })
    .await;
    use std::io::Write as _;
    child.stdin.as_mut().unwrap().write_all(b"y\n").unwrap();
    eventually(|| async {
        let record = fs::read_to_string(home.path().join("gateway/pairing.json")).ok()?;
        serde_json::from_str::<a2amx::relay_files::Pairing>(&record)
            .ok()
            .filter(|pairing| pairing.guild_id == "123" && pairing.user_id == "999")
            .map(|_| ())
    })
    .await;
    assert_eq!(
        fs::metadata(home.path().join("gateway/pairing.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let _ = kill_and_wait(child).await;
}

#[test]
fn load_rejects_symlinked_config_and_pairing() {
    for (file, target, contents) in [
        ("config.toml", "config-target", "guild_id = \"123\"\n"),
        (
            "pairing.json",
            "pairing-target",
            r#"{"guild_id":"123","user_id":"456"}"#,
        ),
    ] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        let target_path = home.path().join(target);
        write_file(&target_path, contents, 0o600);
        let path = home.path().join("gateway").join(file);
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        std::os::unix::fs::symlink(&target_path, &path).unwrap();
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        assert!(error.contains(file), "{error}");
    }
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
        &home.path().join("gateway/config.toml"),
        "guild_id = \"123\"\nsync_ms = 10\nidle_ms = 10\nmember_check_ms = 20\n",
        0o600,
    );
    write_file(
        &home.path().join("gateway/pairing.json"),
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
