#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::gateway_proc::{self, LockError, State};
use rustix::process::{Pid, getpgid};
use std::fs::{self, hard_link};
use std::os::unix::fs::{FileTypeExt, PermissionsExt, symlink};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

fn run_gateway(home: &Path, action: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home"])
        .arg(home)
        .args(["gateway", action])
        .output()
        .expect("run gateway command")
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

    fs::remove_file(home.path().join("gateway.pid")).unwrap();
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
    let gateway = home.path().join("gateway");
    fs::create_dir(&gateway).unwrap();
    let token = gateway.join("discord.token");
    fs::write(&token, b"secret\n").unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    let log_path = gateway.join("gateway.log");

    symlink("discord.token", &log_path).unwrap();
    assert!(gateway_proc::open_log(&gateway).is_err());
    fs::remove_file(&log_path).unwrap();
    hard_link(&token, &log_path).unwrap();
    assert!(gateway_proc::open_log(&gateway).is_err());
    assert_eq!(fs::read(&token).unwrap(), b"secret\n");
    fs::remove_file(&log_path).unwrap();

    let (log, offset) = gateway_proc::open_log(&gateway).unwrap();
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

    let fifo = gateway.join("gateway.log");
    let status = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(status.success());
    assert!(gateway_proc::open_log(&gateway).is_err());
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
    let gateway = home.path().join("gateway");
    fs::create_dir(&gateway).unwrap();
    fs::write(gateway.join("gateway.log"), b"gateway ready\n").unwrap();
    let (log, offset) = gateway_proc::open_log(&gateway).unwrap();
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
    let gateway = home.join("gateway");
    fs::create_dir_all(&gateway).unwrap();
    fs::set_permissions(&gateway, fs::Permissions::from_mode(0o700)).unwrap();
    write_file(&gateway.join("discord.token"), "fake-token\n");
    write_file(
        &gateway.join("config.toml"),
        &format!(
            "guild_id = \"123\"\napi_base = \"{}\"\ngateway_url = \"{}\"\n",
            discord.api_base, discord.gateway_url
        ),
    );
    write_file(
        &gateway.join("pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
    );
    a2amx::relay_files::write_token(home, "fake-token").unwrap();
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
    let pid: u32 = fs::read_to_string(home.path().join("gateway.pid"))
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
    assert!(!home.path().join("gateway.pid").exists());
    assert_eq!(
        fs::metadata(home.path().join("gateway/gateway.log"))
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
    let gateway = home.path().join("gateway");
    fs::create_dir_all(&gateway).unwrap();
    fs::set_permissions(&gateway, fs::Permissions::from_mode(0o700)).unwrap();
    write_file(&gateway.join("discord.token"), "fake-token\n");
    write_file(
        &gateway.join("config.toml"),
        "guild_id = \"123\"\napi_base = \"http://127.0.0.1:1\"\ngateway_url = \"ws://127.0.0.1:1\"\n",
    );
    write_file(
        &gateway.join("pairing.json"),
        r#"{"guild_id":"123","user_id":"456"}"#,
    );
    a2amx::relay_files::write_token(home.path(), "fake-token").unwrap();
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
