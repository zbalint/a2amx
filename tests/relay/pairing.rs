use super::*;

fn announced_pairing_code(lines: &std::sync::Arc<std::sync::Mutex<Vec<String>>>) -> Option<String> {
    lines.lock().unwrap().iter().find_map(|line| {
        line.strip_prefix("post this code in the control channel: ")
            .map(str::to_owned)
    })
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
