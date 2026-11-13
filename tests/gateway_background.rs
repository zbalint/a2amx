#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::gateway_proc::{self, LockError, State};
use rustix::process::{Pid, getpgid};
use std::fs::{self, hard_link};
use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::Command as AsyncCommand;

fn run_gateway(home: &Path, action: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home"])
        .arg(home)
        .args(["gateway", action])
        .output()
        .expect("run gateway command")
}

#[test]
fn gateway_root_named_gateway_uses_its_own_run_lock() {
    let parent = tempdir().unwrap();
    let root = parent.path().join("gateway");
    fs::create_dir(&root).unwrap();
    let _lock = gateway_proc::acquire(&root).unwrap();
    assert!(root.join("run/gateway.lock").is_file());
    assert!(!parent.path().join("run/gateway.lock").exists());
}

#[test]
fn status_without_gateway_reports_not_running() {
    let home = tempdir().unwrap();
    let output = run_gateway(home.path(), "status");
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "not running\n");
}

#[test]
fn stop_without_gateway_reports_no_gateway_running() {
    let home = tempdir().unwrap();
    let output = run_gateway(home.path(), "stop");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "no gateway running\n"
    );
}

#[test]
fn lock_is_exclusive_and_state_reports_holder() {
    let home = tempdir().unwrap();
    let first = gateway_proc::acquire(home.path()).unwrap();
    assert!(matches!(
        gateway_proc::state(home.path()).unwrap(),
        State::Running { .. }
    ));
    assert!(matches!(
        gateway_proc::acquire(home.path()),
        Err(LockError::Held)
    ));
    drop(first);
    assert!(gateway_proc::acquire(home.path()).is_ok());
}

#[test]
fn stop_rejects_unrelated_pid_and_missing_or_dead_process() {
    let home = tempdir().unwrap();
    let lock = gateway_proc::acquire(home.path()).unwrap();
    let mut sleeper = Command::new("sleep").arg("30").spawn().unwrap();
    gateway_proc::write_pid(home.path(), sleeper.id()).unwrap();
    let error = gateway_proc::stop(home.path()).unwrap_err().to_string();
    assert!(error.contains("refusing to signal"));
    assert!(sleeper.try_wait().unwrap().is_none());
    sleeper.kill().unwrap();
    sleeper.wait().unwrap();

    gateway_proc::write_pid(home.path(), u32::MAX).unwrap();
    let error = gateway_proc::stop(home.path()).unwrap_err().to_string();
    assert!(error.contains("process 4294967295 is gone"));
    assert!(error.contains("kill -TERM -- -4294967295"));

    fs::remove_file(home.path().join("run/gateway.pid")).unwrap();
    let error = gateway_proc::stop(home.path()).unwrap_err().to_string();
    assert!(error.contains("missing or unreadable"));
    drop(lock);
}

#[test]
fn stop_signals_verified_nonleader_only_without_killing_runner_group() {
    let home = tempdir().unwrap();
    let lock = gateway_proc::acquire(home.path()).unwrap();
    let state = fs::canonicalize(home.path()).unwrap();
    let mut child = Command::new("sh")
        .args([
            "-c",
            "sleep 30; :",
            "sh",
            "--home",
            state.to_str().unwrap(),
            "gateway",
            "run",
        ])
        .spawn()
        .unwrap();
    let expected = format!("--home\0{}\0gateway\0run\0", state.to_str().unwrap());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let ready = fs::read(format!("/proc/{}/cmdline", child.id()))
            .map(|cmdline| {
                cmdline
                    .windows(expected.len())
                    .any(|window| window == expected.as_bytes())
            })
            .unwrap_or(false);
        if ready {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child command line never became verifiable");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let child_pid = Pid::from_raw(child.id() as i32).unwrap();
    let own_pid = Pid::from_raw(std::process::id() as i32).unwrap();
    assert_eq!(
        getpgid(Some(child_pid)).unwrap(),
        getpgid(Some(own_pid)).unwrap()
    );
    assert_ne!(getpgid(Some(child_pid)).unwrap(), child_pid);
    gateway_proc::write_pid(home.path(), child.id()).unwrap();
    let error = gateway_proc::stop(home.path()).unwrap_err().to_string();
    assert!(
        error.contains("did not stop within 5s"),
        "stop error: {error}"
    );
    assert!(child.try_wait().unwrap().is_some());
    drop(lock);
}

#[test]
fn log_open_is_hardened_and_progress_uses_retained_inode() {
    let home = tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    let token = layout.discord_token();
    fs::write(&token, b"secret\n").unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    let log_path = layout.gateway_log();

    symlink("discord.token", &log_path).unwrap();
    assert!(gateway_proc::open_log(home.path()).is_err());
    fs::remove_file(&log_path).unwrap();
    hard_link(&token, &log_path).unwrap();
    assert!(gateway_proc::open_log(home.path()).is_err());
    assert_eq!(fs::read(&token).unwrap(), b"secret\n");
    fs::remove_file(&log_path).unwrap();

    let (log, offset) = gateway_proc::open_log(home.path()).unwrap();
    assert_eq!(offset, 0);
    use std::io::Write;
    (&log).write_all(b"old\n").unwrap();
    fs::remove_file(&log_path).unwrap();
    hard_link(&token, &log_path).unwrap();
    let progress = gateway_proc::log_progress(&log, 0).unwrap();
    assert_eq!(progress.tail, "old");
    assert!(!progress.tail.contains("secret"));
    drop(log);
    fs::remove_file(&log_path).unwrap();

    let fifo = layout.gateway_log();
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    assert!(gateway_proc::open_log(home.path()).is_err());
    assert!(fs::symlink_metadata(fifo).unwrap().file_type().is_fifo());
}

#[test]
fn sanitize_tail_is_bounded_and_neutralizes_controls() {
    let mut bytes = Vec::new();
    for index in 0..22 {
        bytes.extend_from_slice(format!("line-{index}\n").as_bytes());
    }
    bytes.extend_from_slice(b"\x1b[31m\r\0\t");
    bytes.extend(std::iter::repeat_n(b'x', 300));
    bytes.push(b'\n');
    bytes.extend_from_slice(&[0xff, 0xfe]);
    let tail = gateway_proc::sanitize_tail(&bytes);
    assert!(!tail.contains("line-0"));
    assert!(tail.contains("line-21"));
    assert!(!tail.contains('\x1b'));
    assert!(!tail.contains('\r'));
    assert!(!tail.contains('\0'));
    assert!(!tail.contains('\t'));
    assert!(tail.lines().any(|line| line.len() == 200));
}

#[test]
fn progress_scans_only_new_exact_ready_lines() {
    let home = tempdir().unwrap();
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    fs::write(layout.gateway_log(), b"gateway ready\n").unwrap();
    let (log, offset) = gateway_proc::open_log(home.path()).unwrap();
    assert!(!gateway_proc::log_progress(&log, offset).unwrap().ready);
    use std::io::Write;
    (&log).write_all(b"not gateway ready\n").unwrap();
    assert!(!gateway_proc::log_progress(&log, offset).unwrap().ready);
    (&log).write_all(b"gateway ready\n").unwrap();
    assert!(gateway_proc::log_progress(&log, offset).unwrap().ready);
}

fn bwrap_for_test() -> Option<std::path::PathBuf> {
    match a2amx::confine::resolve_bwrap(None) {
        Ok(path) => {
            a2amx::confine::preflight(&path).expect("bubblewrap preflight");
            Some(path)
        }
        Err(error) if std::env::var_os("A2AMX_REQUIRE_BWRAP").is_some() => {
            panic!("bubblewrap is required for this test: {error}")
        }
        Err(error) => {
            println!("skipping real-bubblewrap test: {error}");
            None
        }
    }
}

fn write_file(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

async fn gateway_command(home: &Path, args: &[&str]) -> std::process::Output {
    let home = home.to_owned();
    let args = args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .args(["--home"])
            .arg(home)
            .args(args)
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

async fn prepare_real_gateway(home: &Path, discord: &common::fake_discord::FakeDiscord) {
    let layout = a2amx::layout::Layout::new(home.to_owned());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "fake-token\n");
    write_file(
        &layout.gateway_config(),
        &format!(
            "guild_id = \"123\"\napi_base = \"{}\"\ngateway_url = \"{}\"\n",
            discord.api_base, discord.gateway_url
        ),
    );
    write_file(&layout.pairing(), r#"{"guild_id":"123","user_id":"456"}"#);
    a2amx::relay_files::write_token(home, "fake-token").unwrap();
}

#[tokio::test]
async fn pair_handover_runs_background_gateway_after_confirmation() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    fs::remove_file(home.path().join("state/gateway/pairing.json")).unwrap();

    let mut command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    let code = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("pairing child exited before code");
            if let Some(code) = line.strip_prefix("post this code in the control channel: ") {
                break code.to_owned();
            }
        }
    })
    .await
    .expect("pairing code timeout");
    let control = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some((id, _, _, _)) = discord
                .channels()
                .into_iter()
                .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            {
                break id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("control channel timeout");
    discord.push_message(&control, "789", &code).await.unwrap();
    let pid: u32 = common::eventually(|| async {
        fs::read_to_string(home.path().join("run/gateway.pid"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
    })
    .await;
    let waiting_status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(waiting_status.status.success());
    assert!(
        String::from_utf8_lossy(&waiting_status.stdout).contains(&format!("running (pid {pid})"))
    );
    stdin.write_all(b"y\n").await.unwrap();
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("gateway up timeout")
        .unwrap();
    let foreground_stderr = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        let mut text = String::new();
        while let Some(line) = stderr.next_line().await.unwrap() {
            text.push_str(&line);
            text.push('\n');
        }
        text
    })
    .await
    .expect("pairing stderr drain timeout");
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}\nforeground stderr: {foreground_stderr}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("gateway running in the background"));
    assert!(!foreground_stderr.contains("gateway ready"));
    let background_pid: u32 = common::eventually(|| async {
        fs::read_to_string(home.path().join("run/gateway.pid"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
    })
    .await;
    assert_eq!(session_id(background_pid), background_pid as i32);
    assert_ne!(session_id(background_pid), std::process::id() as i32);
    discord
        .push_message(&control, "789", "handover connection check")
        .await
        .unwrap();
    assert!(
        discord
            .channels()
            .iter()
            .any(|(id, kind, name, _)| id == &control && *kind == 0 && name == "control")
    );
    let pairing = fs::read_to_string(home.path().join("state/gateway/pairing.json")).unwrap();
    assert!(pairing.contains("\"user_id\":\"789\""));
    let log = fs::read_to_string(home.path().join("log/gateway.log")).unwrap();
    assert!(log.lines().any(|line| line == "gateway ready"));
    assert!(!log.contains("post this code"));
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(status.status.success());
    let stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(stop.status.success());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_handover_decline_releases_lock_without_starting_gateway() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    fs::remove_file(home.path().join("state/gateway/pairing.json")).unwrap();

    let mut command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    let code = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("pairing child exited before code");
            if let Some(code) = line.strip_prefix("post this code in the control channel: ") {
                break code.to_owned();
            }
        }
    })
    .await
    .expect("pairing code timeout");
    let control = common::eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    stdin.write_all(b"n\n").await.unwrap();
    let mut stderr_text = String::new();
    while let Some(line) = stderr.next_line().await.unwrap() {
        stderr_text.push_str(&line);
        stderr_text.push('\n');
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("gateway up timeout")
        .unwrap();
    assert!(!output.status.success());
    assert!(stderr_text.contains("pairing did not complete"));
    assert!(!home.path().join("state/gateway/pairing.json").exists());
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert_eq!(status.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&status.stdout).contains("not running"));
    assert!(gateway_proc::acquire(home.path()).is_ok());
    let mut retry_command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    retry_command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    retry_command.as_std_mut().process_group(0);
    let mut retry = retry_command.spawn().unwrap();
    let mut retry_stdin = retry.stdin.take().unwrap();
    let mut retry_stderr = tokio::io::BufReader::new(retry.stderr.take().unwrap()).lines();
    let retry_code = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = retry_stderr
                .next_line()
                .await
                .unwrap()
                .expect("retry pairing exited before code");
            if let Some(code) = line.strip_prefix("post this code in the control channel: ") {
                break code.to_owned();
            }
        }
    })
    .await
    .expect("retry pairing did not start");
    discord
        .push_message(&control, "789", &retry_code)
        .await
        .unwrap();
    retry_stdin.write_all(b"y\n").await.unwrap();
    let retry_ready = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(line) = retry_stderr.next_line().await.unwrap() {
            if line.contains("starting the gateway in the background") {
                return true;
            }
        }
        false
    })
    .await
    .expect("retry stderr drain timeout");
    assert!(retry_ready);
    let retry_status = tokio::time::timeout(std::time::Duration::from_secs(20), retry.wait())
        .await
        .expect("retry gateway up timeout")
        .unwrap();
    assert!(retry_status.success());
    assert!(
        fs::read_to_string(home.path().join("state/gateway/pairing.json"))
            .unwrap()
            .contains("\"user_id\":\"789\"")
    );
    let retry_gateway_status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(retry_gateway_status.status.success());
    let retry_stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(retry_stop.status.success());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_step_can_be_stopped_and_refuses_second_pair_launch() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    fs::remove_file(home.path().join("state/gateway/pairing.json")).unwrap();

    let mut command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("pairing child exited before code");
            if line.starts_with("post this code in the control channel: ") {
                break;
            }
        }
    })
    .await
    .expect("pairing code timeout");
    let pid: u32 = common::eventually(|| async {
        fs::read_to_string(home.path().join("run/gateway.pid"))
            .ok()
            .and_then(|text| text.trim().parse().ok())
    })
    .await;
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains(&format!("running (pid {pid})")));
    let duplicate = gateway_command(home.path(), &["gateway", "up", "--pair"]).await;
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already running"));
    let stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(stop.status.success());
    let output = tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
        .await
        .expect("stopped pair launcher timeout")
        .unwrap();
    assert!(!output.status.success());
    assert!(!home.path().join("state/gateway/pairing.json").exists());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn pair_foreground_stays_running_until_stopped() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    fs::remove_file(home.path().join("state/gateway/pairing.json")).unwrap();

    let mut command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair", "--foreground"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    let code = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("pairing child exited before code");
            if let Some(code) = line.strip_prefix("post this code in the control channel: ") {
                break code.to_owned();
            }
        }
    })
    .await
    .expect("pairing code timeout");
    let control = common::eventually(|| async {
        discord
            .channels()
            .into_iter()
            .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            .map(|(id, _, _, _)| id)
    })
    .await;
    discord.push_message(&control, "789", &code).await.unwrap();
    stdin.write_all(b"y\n").await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(status.status.success());
    let stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(stop.status.success());
    let _output =
        tokio::time::timeout(std::time::Duration::from_secs(20), child.wait_with_output())
            .await
            .expect("foreground pair stop timeout")
            .unwrap();
    daemon.shutdown().await.unwrap();
}

fn pair_only_pids(home: &Path) -> Vec<u32> {
    let needle = home.to_string_lossy();
    fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
        .filter(|pid| {
            let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
                return false;
            };
            cmdline
                .windows(needle.len())
                .any(|window| window == needle.as_bytes())
                && cmdline
                    .windows("--pair-only".len())
                    .any(|window| window == b"--pair-only")
        })
        .collect()
}

#[tokio::test]
async fn killing_pair_launcher_does_not_leave_pair_step_running() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    fs::remove_file(home.path().join("state/gateway/pairing.json")).unwrap();

    let mut command = AsyncCommand::new(env!("CARGO_BIN_EXE_a2amx"));
    command
        .args(["--home"])
        .arg(home.path())
        .args(["gateway", "up", "--pair"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.as_std_mut().process_group(0);
    let mut child = command.spawn().unwrap();
    let mut stderr = tokio::io::BufReader::new(child.stderr.take().unwrap()).lines();
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        loop {
            let line = stderr
                .next_line()
                .await
                .unwrap()
                .expect("pairing child exited before code");
            if line.starts_with("post this code in the control channel: ") {
                break;
            }
        }
    })
    .await
    .expect("pairing code timeout");
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let survivors = loop {
        let pids = pair_only_pids(home.path());
        if pids.is_empty() || tokio::time::Instant::now() >= deadline {
            break pids;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    for pid in &survivors {
        let _ = rustix::process::kill_process(
            Pid::from_raw(*pid as i32).unwrap(),
            rustix::process::Signal::KILL,
        );
    }
    assert!(
        survivors.is_empty(),
        "pair-only processes survived: {survivors:?}"
    );
    daemon.shutdown().await.unwrap();
}

fn session_id(pid: u32) -> i32 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let (_, fields) = stat.split_once(") ").unwrap();
    fields.split_whitespace().nth(3).unwrap().parse().unwrap()
}

#[tokio::test]
async fn background_gateway_lifecycle_survives_launcher_and_stops_by_group() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        gateway_command(home.path(), &["gateway", "up"]),
    )
    .await
    .expect("gateway up timeout");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("gateway running in the background"));
    let pid: u32 = fs::read_to_string(home.path().join("run/gateway.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let test_pid = std::process::id() as i32;
    assert_eq!(session_id(pid), pid as i32);
    assert_ne!(session_id(pid), test_pid);
    let control = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some((id, _, _, _)) = discord
                .channels()
                .into_iter()
                .find(|(_, kind, name, _)| *kind == 0 && name == "control")
            {
                break id;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("control channel timeout");
    assert!(!control.is_empty());
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains(&format!("running (pid {pid})")));
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let status_after_launcher = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(status_after_launcher.status.success());
    let duplicate = gateway_command(home.path(), &["gateway", "up"]).await;
    assert!(!duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already running"));
    let pair = gateway_command(home.path(), &["gateway", "up", "--pair"]).await;
    assert!(!pair.status.success());
    assert!(String::from_utf8_lossy(&pair.stderr).contains("already running"));
    let stop = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        gateway_command(home.path(), &["gateway", "stop"]),
    )
    .await
    .expect("gateway stop timeout");
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert_eq!(status.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&status.stdout).contains("not running"));
    for _ in 0..100 {
        if !Path::new(&format!("/proc/{pid}")).exists() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    assert!(!home.path().join("run/gateway.pid").exists());
    assert_eq!(
        fs::metadata(home.path().join("log/gateway.log"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let restart = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        gateway_command(home.path(), &["gateway", "up"]),
    )
    .await
    .expect("gateway restart timeout");
    assert!(
        restart.status.success(),
        "{}",
        String::from_utf8_lossy(&restart.stderr)
    );
    let restart_stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(restart_stop.status.success());
    let _ = daemon.shutdown().await;
}

#[tokio::test]
async fn background_gateway_failed_start_is_bounded_and_releases_lock() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "fake-token\n");
    write_file(
        &layout.gateway_config(),
        "guild_id = \"123\"\napi_base = \"http://127.0.0.1:1\"\ngateway_url = \"ws://127.0.0.1:1\"\n",
    );
    write_file(&layout.pairing(), r#"{"guild_id":"123","user_id":"456"}"#);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        gateway_command(home.path(), &["gateway", "up"]),
    )
    .await
    .expect("failed gateway up timeout");
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("exited during startup") || error.contains("did not become ready"));
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(String::from_utf8_lossy(&status.stdout).contains("not running"));
    let _ = daemon.shutdown().await;
}

#[tokio::test]
async fn foreground_gateway_is_stoppable() {
    let Some(_bwrap) = bwrap_for_test() else {
        return;
    };
    let (home, daemon) = common::start_daemon().await;
    let discord = common::fake_discord::FakeDiscord::new("123", 2).await;
    prepare_real_gateway(home.path(), &discord).await;
    let home_path = home.path().to_owned();
    let mut child = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .args(["--home"])
            .arg(home_path)
            .args(["gateway", "up", "--foreground"])
            .process_group(0)
            .spawn()
            .unwrap()
    })
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let status = gateway_command(home.path(), &["gateway", "status"]).await;
    assert!(String::from_utf8_lossy(&status.stdout).contains("running"));
    let stop = gateway_command(home.path(), &["gateway", "stop"]).await;
    assert!(
        stop.status.success(),
        "{}",
        String::from_utf8_lossy(&stop.stderr)
    );
    let _ = tokio::task::spawn_blocking(move || child.wait()).await;
    let _ = daemon.shutdown().await;
}
