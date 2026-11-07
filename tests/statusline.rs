#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use a2amx::client::Client;
use a2amx::daemon::Daemon;
use a2amx::harness::{Deliver, Harness, wire_claude_argv};
use a2amx::wire::{QuotaInfo, Request, Response};
use serde_json::{Value, json};
use tempfile::TempDir;

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn recv_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0; 4];
    stream.read_exact(&mut header).unwrap();
    let length = u32::from_be_bytes(header) as usize;
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).unwrap();
    payload
}

fn send_frame(stream: &mut TcpStream, payload: &[u8]) {
    let frame = a2amx::wire::encode_frame(payload).unwrap();
    stream.write_all(&frame).unwrap();
}

fn payload(reset: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "rate_limits": {
            "five_hour": {"used_percentage": 23, "resets_at": reset},
            "seven_day": {"used_percentage": 41, "resets_at": reset}
        }
    }))
    .unwrap()
}

fn sized_payload(size: usize, reset: u64) -> Vec<u8> {
    let rate_limits = serde_json::to_vec(&json!({
        "rate_limits": {
            "five_hour": {"used_percentage": 23, "resets_at": reset},
            "seven_day": {"used_percentage": 41, "resets_at": reset}
        }
    }))
    .unwrap();
    let prefix_len = rate_limits.len() - 1 + b",\"padding\":\"".len();
    let suffix_len = b"\"}".len();
    assert!(size >= prefix_len + suffix_len);
    let mut input = rate_limits[..rate_limits.len() - 1].to_vec();
    input.extend_from_slice(b",\"padding\":\"");
    input.extend(std::iter::repeat_n(b'x', size - prefix_len - suffix_len));
    input.extend_from_slice(b"\"}");
    input
}

fn run_statusline(
    address: SocketAddr,
    token: &str,
    input: &[u8],
    report: bool,
    clear_environment: bool,
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
    command.arg("statusline");
    if report {
        command.arg("--report");
    }
    if clear_environment {
        command.env_clear();
    }
    command
        .env("A2AMX_ADDR", address.to_string())
        .env("A2AMX_TOKEN", token)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    child.wait_with_output().unwrap()
}

async fn run_statusline_async(
    address: SocketAddr,
    token: String,
    input: Vec<u8>,
    report: bool,
    clear_environment: bool,
) -> Output {
    tokio::task::spawn_blocking(move || {
        run_statusline(address, &token, &input, report, clear_environment)
    })
    .await
    .unwrap()
}

async fn list_quota(admin: &mut Client, id: &str) -> Option<QuotaInfo> {
    match admin.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions
            .into_iter()
            .find(|session| session.id == id)
            .and_then(|session| session.quota),
        other => panic!("unexpected list response: {other:?}"),
    }
}

async fn wait_quota(admin: &mut Client, id: &str) -> QuotaInfo {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(quota) = list_quota(admin, id).await {
                return quota;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("quota becomes visible")
}

async fn claude_case() -> (TempDir, Daemon, Client, String, String, SocketAddr) {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (id, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-claude"),
        Harness::Claude,
        Deliver::Hold,
    )
    .await;
    (dir, daemon, admin, id, token, address)
}

#[tokio::test]
async fn statusline_renders_and_reports_quota() {
    let (_dir, daemon, mut admin, id, token, address) = claude_case().await;
    let reset = now_unix() + 3600;
    let output = run_statusline_async(address, token.clone(), payload(reset), false, false).await;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"5h 77% left 7d 59% left\n");
    assert!(
        !output
            .stderr
            .windows(token.len())
            .any(|part| part == token.as_bytes())
    );
    let quota = wait_quota(&mut admin, &id).await;
    assert_eq!(quota.five_hour_left, Some(77));
    assert_eq!(quota.weekly_left, Some(59));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn statusline_report_is_silent_and_works_with_only_session_environment() {
    let (_dir, daemon, mut admin, id, token, address) = claude_case().await;
    let output = run_statusline_async(
        address,
        token.clone(),
        payload(now_unix() + 3600),
        true,
        true,
    )
    .await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        !output
            .stderr
            .windows(token.len())
            .any(|part| part == token.as_bytes())
    );
    let quota = wait_quota(&mut admin, &id).await;
    assert_eq!(quota.five_hour_left, Some(77));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn statusline_bounds_input_and_fails_open() {
    let (_dir, daemon, mut admin, accepted_id, accepted_token, accepted_address) =
        claude_case().await;
    let accepted_input = sized_payload(256 * 1024, now_unix() + 3600);
    assert_eq!(accepted_input.len(), 256 * 1024);
    assert!(serde_json::from_slice::<Value>(&accepted_input).is_ok());
    let accepted = run_statusline_async(
        accepted_address,
        accepted_token.clone(),
        accepted_input,
        true,
        false,
    )
    .await;
    assert!(accepted.status.success());
    assert!(accepted.stdout.is_empty());
    let accepted_quota = wait_quota(&mut admin, &accepted_id).await;
    assert_eq!(accepted_quota.weekly_left, Some(59));

    let (rejected_id, rejected_token, rejected_address) = common::new_agent(
        &mut admin,
        _dir.path(),
        Some("agent-too-large"),
        Harness::Claude,
        Deliver::Hold,
    )
    .await;
    let too_large = run_statusline_async(
        rejected_address,
        rejected_token.clone(),
        sized_payload(256 * 1024 + 1, now_unix() + 3600),
        false,
        false,
    )
    .await;
    assert!(too_large.status.success());
    assert_eq!(too_large.stdout, b"\n");
    assert_eq!(list_quota(&mut admin, &rejected_id).await, None);

    let invalid = run_statusline_async(
        "127.0.0.1:9".parse().unwrap(),
        "a".repeat(64),
        b"not json".to_vec(),
        false,
        false,
    )
    .await;
    assert!(invalid.status.success());
    assert_eq!(invalid.stdout, b"\n");
    assert!(!invalid.stderr.windows(64).any(|part| part == b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    daemon.shutdown().await.unwrap();
}

#[test]
fn composed_statusline_preserves_input_and_user_shell_behavior() {
    let cwd = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let exe = Path::new(env!("CARGO_BIN_EXE_a2amx"));
    let settings =
        json!({"statusLine":{"type":"command","command":"cat","padding":2,"refreshInterval":4}})
            .to_string();
    let argv = wire_claude_argv(
        vec!["claude".into(), "--settings".into(), settings],
        exe,
        cwd.path(),
        config_dir.path(),
        false,
        None,
    )
    .unwrap();
    let settings_index = argv.iter().position(|arg| arg == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&argv[settings_index + 1]).unwrap();
    let command = settings["statusLine"]["command"].as_str().unwrap();
    assert_eq!(settings["statusLine"]["padding"], 2);
    assert_eq!(settings["statusLine"]["refreshInterval"], 4);

    let input = b"status payload\n\n";
    let mut child = Command::new("sh")
        .args(["-c", command])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, input);

    for (user_command, expected_status, expected_stdout) in [
        ("exit 3", 3, Vec::new()),
        ("echo ok # trailing comment", 0, b"ok\n".to_vec()),
        ("printf '%s\\n' 'quoted'", 0, b"quoted\n".to_vec()),
    ] {
        let settings = json!({"statusLine":{"type":"command","command":user_command}}).to_string();
        let argv = wire_claude_argv(
            vec!["claude".into(), "--settings".into(), settings],
            exe,
            cwd.path(),
            config_dir.path(),
            false,
            None,
        )
        .unwrap();
        let settings_index = argv.iter().position(|arg| arg == "--settings").unwrap();
        let settings: Value = serde_json::from_str(&argv[settings_index + 1]).unwrap();
        let command = settings["statusLine"]["command"].as_str().unwrap();
        let output = Command::new("sh").args(["-c", command]).output().unwrap();
        assert_eq!(output.status.code(), Some(expected_status));
        assert_eq!(output.stdout, expected_stdout);
    }
}

#[test]
fn composed_statusline_reports_before_user_output_after_handshake() {
    let cwd = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (report_tx, report_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let hello: Request = serde_json::from_slice(&recv_frame(&mut stream)).unwrap();
        assert!(matches!(hello, Request::Hello { .. }));
        send_frame(&mut stream, &serde_json::to_vec(&Response::Ok).unwrap());
        let report: Request = serde_json::from_slice(&recv_frame(&mut stream)).unwrap();
        assert!(matches!(report, Request::ReportQuota { .. }));
        report_tx.send(Instant::now()).unwrap();
        send_frame(&mut stream, &serde_json::to_vec(&Response::Ok).unwrap());
    });
    let settings =
        json!({"statusLine":{"type":"command","command":"sleep 1; echo done"}}).to_string();
    let argv = wire_claude_argv(
        vec!["claude".into(), "--settings".into(), settings],
        Path::new(env!("CARGO_BIN_EXE_a2amx")),
        cwd.path(),
        config_dir.path(),
        false,
        None,
    )
    .unwrap();
    let settings_index = argv.iter().position(|arg| arg == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&argv[settings_index + 1]).unwrap();
    let command = settings["statusLine"]["command"].as_str().unwrap();
    let mut child = Command::new("sh")
        .args(["-c", command])
        .env("A2AMX_ADDR", address.to_string())
        .env("A2AMX_TOKEN", "c".repeat(64))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&payload(now_unix() + 3600))
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let stdout_thread = std::thread::spawn(move || {
        let mut first = [0; 1];
        stdout.read_exact(&mut first).unwrap();
        let first_byte = Instant::now();
        let mut output = first.to_vec();
        stdout.read_to_end(&mut output).unwrap();
        (first_byte, output)
    });
    let status = child.wait().unwrap();
    let (first_output, output) = stdout_thread.join().unwrap();
    let report_arrival = report_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    server.join().unwrap();
    assert!(status.success());
    assert_eq!(output, b"done\n");
    assert!(report_arrival < first_output);
}

#[test]
fn composed_statusline_reports_before_slow_user_command_even_when_daemon_hangs() {
    let cwd = TempDir::new().unwrap();
    let config_dir = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let thread = std::thread::spawn(move || {
        let _ = listener.accept();
        std::thread::sleep(Duration::from_secs(2));
    });
    let settings =
        json!({"statusLine":{"type":"command","command":"sleep 1; echo done"}}).to_string();
    let argv = wire_claude_argv(
        vec!["claude".into(), "--settings".into(), settings],
        Path::new(env!("CARGO_BIN_EXE_a2amx")),
        cwd.path(),
        config_dir.path(),
        false,
        None,
    )
    .unwrap();
    let settings_index = argv.iter().position(|arg| arg == "--settings").unwrap();
    let settings: Value = serde_json::from_str(&argv[settings_index + 1]).unwrap();
    let command = settings["statusLine"]["command"].as_str().unwrap();
    let started = Instant::now();
    let mut child = Command::new("sh")
        .args(["-c", command])
        .env("A2AMX_ADDR", address.to_string())
        .env("A2AMX_TOKEN", "b".repeat(64))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&payload(now_unix() + 3600))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"done\n");
    assert!(started.elapsed() < Duration::from_secs(2));
    thread.join().unwrap();
}
