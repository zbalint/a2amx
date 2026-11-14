#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::messaging::RemoteMode;
use a2amx::relay::{Io, pair_only, run};
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
    let layout = a2amx::layout::Layout::new(home.to_owned());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "bot-token\n", 0o600);
    write_file(&layout.gateway_config(), "guild_id = \"123\"\n", 0o600);
}
async fn wait_for_member_checks(discord: &FakeDiscord, baseline: u64) {
    eventually(|| async { (discord.member_count_requests() >= baseline + 2).then_some(()) }).await;
}
fn announced_pairing_code(lines: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Option<String> {
    lines.lock().unwrap().iter().find_map(|line| {
        line.strip_prefix("post this code in the control channel: ")
            .map(str::to_owned)
    })
}

#[test]
fn load_does_not_create_missing_home() {
    let parent = tempdir().unwrap();
    let home = parent.path().join("missing");
    let error = relay_files::load(&home).expect_err("missing gateway files");
    assert!(!home.exists(), "load must not create the a2amx directory");
    assert!(!error.to_string().is_empty());
}

#[tokio::test]
async fn pairing_existing_record_warns_replaces_and_announces_after_write() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"111"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let prompt = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let persisted_on_success = std::sync::Arc::new(std::sync::Mutex::new(Vec::<bool>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let prompt_for_io = std::sync::Arc::clone(&prompt);
    let persisted_for_io = std::sync::Arc::clone(&persisted_on_success);
    let home_for_io = home.path().to_owned();
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    announcements_for_io.lock().unwrap().push(line.to_owned());
                    if line
                        == "paired with Discord account 789; starting the gateway (Ctrl-C stops it)"
                    {
                        let pairing =
                            fs::read_to_string(home_for_io.join("state/gateway/pairing.json"))
                                .ok()
                                .and_then(|record| {
                                    serde_json::from_str::<a2amx::relay_files::Pairing>(&record)
                                        .ok()
                                });
                        persisted_for_io
                            .lock()
                            .unwrap()
                            .push(pairing.is_some_and(|pairing| {
                                pairing.guild_id == "123" && pairing.user_id == "789"
                            }));
                    }
                }),
                confirm: Box::new(move |line| {
                    *prompt_for_io.lock().unwrap() = Some(line.to_owned());
                    true
                }),
            }),
        },
    ));
    let code = eventually(|| async { announced_pairing_code(&announcements) }).await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    eventually(|| async {
        (prompt.lock().unwrap().as_deref() == Some("replace Discord account 111 with 789? [y/N]"))
            .then_some(())
    })
    .await;
    let success = "paired with Discord account 789; starting the gateway (Ctrl-C stops it)";
    eventually(|| async {
        announcements
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == success)
            .then_some(())
    })
    .await;
    let lines = announcements.lock().unwrap().clone();
    let warning = "warning: this gateway is already paired with Discord account 111 in server 123; confirming a new pairing replaces it, and the old account can no longer message your agents";
    let warning_index = lines.iter().position(|line| line == warning).unwrap();
    let code_index = lines
        .iter()
        .position(|line| line.starts_with("post this code in the control channel: "))
        .unwrap();
    let success_index = lines.iter().position(|line| line == success).unwrap();
    assert!(warning_index < code_index);
    assert!(code_index < success_index);
    assert_eq!(*persisted_on_success.lock().unwrap(), vec![true]);
    let record = fs::read_to_string(home.path().join("state/gateway/pairing.json")).unwrap();
    let pairing: a2amx::relay_files::Pairing = serde_json::from_str(&record).unwrap();
    assert_eq!(pairing.guild_id, "123");
    assert_eq!(pairing.user_id, "789");
    assert_eq!(
        fs::metadata(home.path().join("state/gateway/pairing.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pairing_without_record_announces_after_write() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let prompt = std::sync::Arc::new(std::sync::Mutex::new(None::<String>));
    let persisted_on_success = std::sync::Arc::new(std::sync::Mutex::new(Vec::<bool>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let prompt_for_io = std::sync::Arc::clone(&prompt);
    let persisted_for_io = std::sync::Arc::clone(&persisted_on_success);
    let home_for_io = home.path().to_owned();
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    announcements_for_io.lock().unwrap().push(line.to_owned());
                    if line
                        == "paired with Discord account 789; starting the gateway (Ctrl-C stops it)"
                    {
                        let pairing =
                            fs::read_to_string(home_for_io.join("state/gateway/pairing.json"))
                                .ok()
                                .and_then(|record| {
                                    serde_json::from_str::<a2amx::relay_files::Pairing>(&record)
                                        .ok()
                                });
                        persisted_for_io
                            .lock()
                            .unwrap()
                            .push(pairing.is_some_and(|pairing| {
                                pairing.guild_id == "123" && pairing.user_id == "789"
                            }));
                    }
                }),
                confirm: Box::new(move |line| {
                    *prompt_for_io.lock().unwrap() = Some(line.to_owned());
                    true
                }),
            }),
        },
    ));
    let code = eventually(|| async { announced_pairing_code(&announcements) }).await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    eventually(|| async {
        (prompt.lock().unwrap().as_deref() == Some("bind Discord account 789? [y/N]")).then_some(())
    })
    .await;
    let success = "paired with Discord account 789; starting the gateway (Ctrl-C stops it)";
    eventually(|| async {
        announcements
            .lock()
            .unwrap()
            .iter()
            .any(|line| line == success)
            .then_some(())
    })
    .await;
    let lines = announcements.lock().unwrap().clone();
    assert!(!lines.iter().any(|line| line.starts_with("warning")));
    assert_eq!(*persisted_on_success.lock().unwrap(), vec![true]);
    task.abort();
    let _ = task.await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_only_pairs_without_sync_or_ready_line() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let mut task = tokio::spawn(pair_only(
        loaded,
        discord.discord_config(),
        a2amx::relay::PairIo {
            announce: Box::new(move |line| {
                announcements_for_io.lock().unwrap().push(line.to_owned());
            }),
            confirm: Box::new(|_| true),
        },
    ));
    let code = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if let Some(code) = announced_pairing_code(&announcements) {
                break code;
            }
            if task.is_finished() {
                let result = (&mut task).await;
                panic!("pair_only exited before announcement: {result:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("pairing announcement timeout");
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    assert!(settled(task).await.is_ok());
    let lines = announcements.lock().unwrap().clone();
    assert!(lines.iter().any(|line| {
        line == "paired with Discord account 789; starting the gateway in the background"
    }));
    assert!(!lines.iter().any(|line| line == "gateway ready"));
    assert!(!lines.iter().any(|line| {
        line == "paired with Discord account 789; starting the gateway (Ctrl-C stops it)"
    }));
    let record = fs::read_to_string(home.path().join("state/gateway/pairing.json")).unwrap();
    let pairing: a2amx::relay_files::Pairing = serde_json::from_str(&record).unwrap();
    assert_eq!(pairing.user_id, "789");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_only_decline_does_not_write_pairing() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let task = tokio::spawn(pair_only(
        loaded,
        discord.discord_config(),
        a2amx::relay::PairIo {
            announce: Box::new(move |line| {
                announcements_for_io.lock().unwrap().push(line.to_owned());
            }),
            confirm: Box::new(|_| false),
        },
    ));
    let code = eventually(|| async { announced_pairing_code(&announcements) }).await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    let error = settled(task).await.unwrap_err().to_string();
    assert_eq!(error, "pairing not confirmed");
    assert!(!home.path().join("state/gateway/pairing.json").exists());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_only_refuses_before_pairing_when_daemon_is_stopped() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    daemon.shutdown().await.unwrap();
    let addr = home.path().join("run/link/addr");
    if addr.exists() {
        fs::remove_file(&addr).unwrap();
    }
    assert!(!addr.exists());
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let error = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        pair_only(
            loaded,
            discord.discord_config(),
            a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    announcements_for_io.lock().unwrap().push(line.to_owned());
                }),
                confirm: Box::new(|_| true),
            },
        ),
    )
    .await
    .expect("pair-only daemon check must settle")
    .unwrap_err()
    .to_string();
    assert!(!error.is_empty());
    assert!(announcements.lock().unwrap().is_empty());
    assert!(!home.path().join("state/gateway/pairing.json").exists());
}

#[tokio::test]
async fn pairing_decline_preserves_existing_record_and_has_no_success_announcement() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"111"}"#,
        0o600,
    );
    let before = fs::read(home.path().join("state/gateway/pairing.json")).unwrap();
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    announcements_for_io.lock().unwrap().push(line.to_owned());
                }),
                confirm: Box::new(|_| false),
            }),
        },
    ));
    let code = eventually(|| async { announced_pairing_code(&announcements) }).await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    let error = settled(task).await.unwrap_err().to_string();
    assert!(error.contains("pairing not confirmed"));
    assert_eq!(
        fs::read(home.path().join("state/gateway/pairing.json")).unwrap(),
        before
    );
    assert!(!announcements.lock().unwrap().iter().any(|line| {
        line == "paired with Discord account 789; starting the gateway (Ctrl-C stops it)"
    }));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pairing_rejects_invalid_author_before_confirmation_or_write() {
    let (home, daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
    let loaded = relay_files::load(home.path()).unwrap();
    let announcements = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let confirm_called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let announcements_for_io = std::sync::Arc::clone(&announcements);
    let confirm_called_for_io = std::sync::Arc::clone(&confirm_called);
    let task = tokio::spawn(run(
        loaded,
        discord.discord_config(),
        Io {
            stop: Box::pin(std::future::pending()),
            pair: Some(a2amx::relay::PairIo {
                announce: Box::new(move |line| {
                    announcements_for_io.lock().unwrap().push(line.to_owned());
                }),
                confirm: Box::new(move |_| {
                    confirm_called_for_io.store(true, std::sync::atomic::Ordering::SeqCst);
                    true
                }),
            }),
        },
    ));
    let code = eventually(|| async { announced_pairing_code(&announcements) }).await;
    let control = eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    let author = "9\u{1b}[31mX";
    discord.push_message(&control, author, &code).await.unwrap();
    let error = settled(task).await.unwrap_err().to_string();
    assert_eq!(
        error,
        "pairing refused: the message author is not a Discord user ID"
    );
    assert!(!confirm_called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(!home.path().join("state/gateway/pairing.json").exists());
    daemon.shutdown().await.unwrap();
}

#[test]
fn load_reads_defaults_and_pairing() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("state/gateway/pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
        0o600,
    );
    let loaded = relay_files::load(home.path()).unwrap();
    assert_eq!(loaded.guild_id, "123");
    assert_eq!(loaded.config.control_category, "a2amx");
    assert_eq!(loaded.config.control_channel, "control");
    assert_eq!(loaded.config.max_members, 2);
    assert_eq!(loaded.pairing.as_ref().unwrap().user_id, "456");
    assert_eq!(loaded.token, "bot-token");
}

#[test]
fn load_rejects_missing_or_comments_only_gateway_guild() {
    for comments in [None, Some("# gateway settings only\n")] {
        let home = tempdir().unwrap();
        let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
        layout.ensure().unwrap();
        write_file(&layout.discord_token(), "bot-token\n", 0o600);
        if let Some(comments) = comments {
            write_file(&layout.gateway_config(), comments, 0o600);
        }
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        assert!(
            error.contains("gateway.toml")
                && error.contains("guild_id")
                && error.contains("not set"),
            "{error}"
        );
    }
}

#[test]
fn gateway_template_with_guild_id_loads_through_relay() {
    let home = tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "bot-token\n", 0o600);
    a2amx::config::ensure_gateway_file(&layout).unwrap();
    let template = fs::read_to_string(layout.gateway_config()).unwrap();
    let template = template
        .lines()
        .map(|line| {
            line.strip_prefix("# guild_id = ")
                .map(|value| format!("guild_id = {value}"))
                .unwrap_or_else(|| line.to_owned())
        })
        .collect::<Vec<_>>()
        .join("\n");
    write_file(&layout.gateway_config(), &format!("{template}\n"), 0o600);
    assert_eq!(
        relay_files::load(home.path()).unwrap().guild_id,
        "123456789012345678"
    );
}
#[test]
fn load_rejects_unknown_config_key() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    fs::OpenOptions::new()
        .append(true)
        .open(home.path().join("config/gateway.toml"))
        .unwrap()
        .write_all(b"unknown = true\n")
        .unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("unknown") || error.contains("gateway.toml"));
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
                &home.path().join("state/gateway/pairing.json"),
                r#"{"guild_id":"123","user_id":"456"}"#,
                0o644,
            );
        } else {
            let path = if file == "discord.token" {
                home.path().join("config/secrets/discord.token")
            } else {
                home.path().join("config/gateway.toml")
            };
            fs::set_permissions(path, fs::Permissions::from_mode(0o644)).unwrap();
        }
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        let expected = if file == "config.toml" {
            "gateway.toml"
        } else {
            file
        };
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn load_rejects_symlinked_secret_file() {
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    let target = home.path().join("token-target");
    write_file(&target, "bot-token\n", 0o600);
    fs::remove_file(home.path().join("config/secrets/discord.token")).unwrap();
    std::os::unix::fs::symlink(&target, home.path().join("config/secrets/discord.token")).unwrap();
    let error = relay_files::load(home.path()).unwrap_err().to_string();
    assert!(error.contains("discord.token"), "{error}");
}

#[test]
fn load_rejects_bad_values_and_oversized_config() {
    for config in [
        "guild_id = \"not-numeric\"\n",
        "guild_id = \"123\"\ncontrol_channel = \"a_BAD\"\n",
        "guild_id = \"123\"\nidle = \"0ms\"\n",
    ] {
        let home = tempdir().unwrap();
        valid_gateway(home.path());
        write_file(&home.path().join("config/gateway.toml"), config, 0o600);
        assert!(relay_files::load(home.path()).is_err(), "{config}");
    }
    let home = tempdir().unwrap();
    valid_gateway(home.path());
    write_file(
        &home.path().join("config/gateway.toml"),
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
        &home.path().join("state/gateway/pairing.json"),
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
        fs::read_to_string(home.path().join("config/secrets/discord.token")).unwrap(),
        "secret-token"
    );
    assert_eq!(
        fs::metadata(home.path().join("config/secrets/discord.token"))
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
    assert!(!home.path().join("config/secrets/discord.token").exists());
}

#[tokio::test]
async fn gateway_client_uses_gateway_token_file() {
    let (home, daemon) = start_daemon().await;
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    let admin_token = fs::read_to_string(layout.admin_token()).unwrap();
    let gateway_token = fs::read_to_string(layout.link_gateway_token()).unwrap();
    assert_ne!(admin_token, gateway_token);
    let mut gateway = a2amx::client::Client::connect_gateway(&layout)
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
    eventually(|| async {
        fs::read_to_string(home.path().join("state/gateway/pairing.json")).ok()
    })
    .await;
    let record = fs::read_to_string(home.path().join("state/gateway/pairing.json")).unwrap();
    let pairing: a2amx::relay_files::Pairing = serde_json::from_str(&record).unwrap();
    assert_eq!(pairing.guild_id, "123");
    assert_eq!(pairing.user_id, "789");
    assert!(prompt.lock().unwrap().as_deref() == Some("bind Discord account 789? [y/N]"));
    task.abort();
    let _ = task.await;
    assert_eq!(
        fs::metadata(home.path().join("state/gateway/pairing.json"))
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
    assert!(!home.path().join("state/gateway/pairing.json").exists());
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
async fn gateway_run_pair_binary_prompts_and_writes_record() {
    // Binary seam with piped input: the announcement lands on standard error, any
    // author may post the code into the control channel, only a terminal-style
    // confirm writes the pairing record, and the run later stops on a signal.
    let (home, _daemon) = start_daemon().await;
    let discord = FakeDiscord::new("123", 2).await;
    valid_gateway(home.path());
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
        let record = fs::read_to_string(home.path().join("state/gateway/pairing.json")).ok()?;
        serde_json::from_str::<a2amx::relay_files::Pairing>(&record)
            .ok()
            .filter(|pairing| pairing.guild_id == "123" && pairing.user_id == "999")
            .map(|_| ())
    })
    .await;
    eventually(|| async {
        stderr_text
            .lock()
            .unwrap()
            .contains("paired with Discord account 999; starting the gateway (Ctrl-C stops it)")
            .then_some(())
    })
    .await;
    assert_eq!(
        fs::metadata(home.path().join("state/gateway/pairing.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let _ = kill_and_wait(child).await;

    let before = fs::read(home.path().join("state/gateway/pairing.json")).unwrap();
    let home_path = home.path().to_owned();
    let mut second = tokio::task::spawn_blocking(move || {
        ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(home_path)
            .args(["gateway", "run", "--pair"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();
    let second_stderr = second.stderr.take().unwrap();
    let second_text = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let second_copy = std::sync::Arc::clone(&second_text);
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(second_stderr);
        let mut buffer = [0_u8; 1024];
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            second_copy
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buffer[..count]));
        }
    });
    let second_code = eventually(|| async {
        let text = second_text.lock().unwrap().clone();
        text.lines()
            .find(|line| line.contains("post this code in the control channel:"))
            .and_then(|line| line.rsplit_once(' ').map(|(_, code)| code.to_owned()))
    })
    .await;
    discord
        .push_message(&control, "999", &second_code)
        .await
        .unwrap();
    eventually(|| async {
        let text = second_text.lock().unwrap().clone();
        (text.contains(
            "warning: this gateway is already paired with Discord account 999 in server 123; confirming a new pairing replaces it, and the old account can no longer message your agents",
        ) && text.contains("replace Discord account 999 with 999? [y/N]"))
            .then_some(())
    })
    .await;
    second.stdin.as_mut().unwrap().write_all(b"n\n").unwrap();
    let status = tokio::task::spawn_blocking(move || second.wait().unwrap())
        .await
        .unwrap();
    assert!(!status.success());
    assert!(
        second_text
            .lock()
            .unwrap()
            .contains("pairing not confirmed")
    );
    assert_eq!(
        fs::read(home.path().join("state/gateway/pairing.json")).unwrap(),
        before
    );
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
        let path = home.path().join(if file == "config.toml" {
            "config/gateway.toml"
        } else {
            "state/gateway/pairing.json"
        });
        if path.exists() {
            fs::remove_file(&path).unwrap();
        }
        std::os::unix::fs::symlink(&target_path, &path).unwrap();
        let error = relay_files::load(home.path()).unwrap_err().to_string();
        let expected = if file == "config.toml" {
            "gateway.toml"
        } else {
            file
        };
        assert!(error.contains(expected), "{error}");
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
