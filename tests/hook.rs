#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use a2amx::client::{Attachment, Client};
use a2amx::daemon::Daemon;
use a2amx::emulator::{Emulator, Size};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::CORRUPTED_SUBMISSION_REASON;
use a2amx::wire::{MessageInfo, Request, Response, ServerFrame};
use serde_json::{Value, json};
use tempfile::TempDir;

const BODY: &str = "I found the regression in parser.py.";
const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";
const WAIT: Duration = Duration::from_secs(10);

fn wrapped(text: &str) -> String {
    format!("\n\n<pasted_content id=\"458d\">\n{text}\n</pasted_content id=\"458d\">\n")
}

struct HookCase {
    dir: TempDir,
    _daemon: Daemon,
    admin: Client,
    sender: Client,
    recipient: String,
    token: String,
    addr: SocketAddr,
}

impl HookCase {
    async fn start() -> Self {
        let (dir, daemon) = common::start_daemon().await;
        let mut admin = Client::connect(dir.path()).await.unwrap();
        let (_, sender_token, sender_addr) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-plan"),
            Harness::Generic,
            Deliver::Hold,
        )
        .await;
        let sender = Client::connect_addr(sender_addr, &sender_token)
            .await
            .unwrap();
        let out = dir.path().join("received");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake_claude.sh");
        let response = admin
            .request(Request::NewSession {
                argv: vec!["sh".into(), fixture.to_string_lossy().into_owned()],
                cols: 40,
                rows: 10,
                cwd: None,
                env: vec![
                    ("OUT".into(), out.to_string_lossy().into_owned()),
                    ("PASTE_LEN".into(), "222".into()),
                    ("MODE".into(), "ready".into()),
                ],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("agent-review".into()),
                harness: Harness::Claude,
                deliver: Some(Deliver::Auto),
            })
            .await
            .unwrap();
        let Response::Created { session: recipient } = response else {
            panic!("fake session not created")
        };
        let (token, addr) = common::eventually(|| async {
            let text = std::fs::read_to_string(out.with_extension("creds")).ok()?;
            let mut lines = text.lines();
            let token = lines.next()?.to_owned();
            let addr = lines.next()?.parse::<SocketAddr>().ok()?;
            (token.len() == 64).then_some((token, addr))
        })
        .await;
        Self {
            dir,
            _daemon: daemon,
            admin,
            sender,
            recipient,
            token,
            addr,
        }
    }

    async fn attach_ready(&self) -> Attachment {
        let mut attachment = Client::connect(self.dir.path())
            .await
            .unwrap()
            .attach(&self.recipient, false, 40, 10)
            .await
            .unwrap();
        let mut mirror = Emulator::new(Size { cols: 40, rows: 10 });
        tokio::time::timeout(WAIT, async {
            loop {
                let Some(ServerFrame::Data(bytes)) = attachment.recv().await.unwrap() else {
                    panic!("fake exited")
                };
                mirror.feed(&bytes);
                let screen = mirror.screen();
                if screen.modes.bracketed_paste
                    && screen.cursor.visible
                    && screen.cell(2, 0).ch == '❯'
                    && screen.cell(3, 0).ch == '─'
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        attachment
    }

    async fn send(&mut self) -> String {
        let response = self
            .sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Parser issue".into(),
                message: BODY.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id, .. } = response else {
            panic!("message not accepted")
        };
        id
    }

    async fn status(&mut self, id: &str) -> MessageInfo {
        let Response::Status { message } = self
            .admin
            .request(Request::MessageStatus { id: id.into() })
            .await
            .unwrap()
        else {
            panic!("missing message")
        };
        message
    }

    async fn wait_state(&mut self, id: &str, state: &str) -> MessageInfo {
        tokio::time::timeout(WAIT, async {
            loop {
                let message = self.status(id).await;
                if message.state == state {
                    return message;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }

    async fn run_hook(&self, payload: &str) -> Output {
        run_hook(payload, Some(self.addr), Some(self.token.clone())).await
    }
}

async fn run_hook(payload: &str, address: Option<SocketAddr>, token: Option<String>) -> Output {
    let payload = payload.as_bytes().to_vec();
    tokio::task::spawn_blocking(move || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command
            .env_clear()
            .arg("hook")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(address) = address {
            command.env("A2AMX_ADDR", address.to_string());
        }
        if let Some(token) = token {
            command.env("A2AMX_TOKEN", token);
        }
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(&payload).unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn hook_payload(prompt: &str) -> String {
    hook_payload_with_event("UserPromptSubmit", prompt)
}

fn hook_payload_with_event(event: &str, prompt: &str) -> String {
    json!({
        "session_id": "s-example",
        "transcript_path": "/tmp/example.jsonl",
        "cwd": "/tmp/example",
        "prompt_id": "p-1",
        "permission_mode": "auto",
        "hook_event_name": event,
        "prompt": prompt,
    })
    .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_exact_wrapped_envelope_records_receipt_and_cli_detail() {
    let mut case = HookCase::start().await;
    let _attachment = case.attach_ready().await;
    let id = case.send().await;
    let before = case.wait_state(&id, "submitted").await;
    assert_eq!(before.evidence.as_deref(), Some("write_complete"));

    let output = case.run_hook(&hook_payload(&wrapped(ENVELOPE_TEXT))).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let after = case.wait_state(&id, "submitted").await;
    assert_eq!(after.evidence.as_deref(), Some("submission_observed"));

    let cli = tokio::task::spawn_blocking({
        let home = case.dir.path().to_owned();
        move || {
            Command::new(env!("CARGO_BIN_EXE_a2amx"))
                .env_clear()
                .args(["--home", home.to_str().unwrap(), "messages"])
                .output()
                .unwrap()
        }
    })
    .await
    .unwrap();
    assert!(cli.status.success());
    let stdout = String::from_utf8(cli.stdout).unwrap();
    let row = stdout
        .lines()
        .find(|line| line.starts_with("m_1 "))
        .unwrap();
    assert_eq!(row.split_whitespace().nth(4), Some("submission_observed"));
    assert!(!stdout.contains(case.token.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_corrupted_submission_blocks_with_fixed_reason() {
    let mut case = HookCase::start().await;
    let _attachment = case.attach_ready().await;
    let id = case.send().await;
    case.wait_state(&id, "submitted").await;

    let output = case
        .run_hook(&hook_payload(&format!(
            "my draft {}",
            wrapped(ENVELOPE_TEXT)
        )))
        .await;
    assert!(output.status.success());
    assert!(output.stdout.ends_with(b"\n"));
    assert_eq!(
        output.stdout.iter().filter(|&&byte| byte == b'\n').count(),
        1
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap(),
        json!({"decision":"block", "reason":CORRUPTED_SUBMISSION_REASON})
    );
    assert!(output.stderr.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_missing_credentials_fails_open_without_secrets() {
    let payload = hook_payload("hello");
    let output = run_hook(&payload, None, None).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.starts_with("a2amx hook:"));
    assert!(!stderr.contains("hello"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_closed_port_fails_open() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let output = run_hook(&hook_payload("hello"), Some(address), Some("a".repeat(64))).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("a2amx hook:")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_invalid_json_fails_open() {
    let output = run_hook("not json", None, None).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("a2amx hook:")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_other_event_is_silent_and_does_not_report_receipt() {
    let mut case = HookCase::start().await;
    let _attachment = case.attach_ready().await;
    let id = case.send().await;
    let before = case.wait_state(&id, "submitted").await;
    let payload = hook_payload_with_event("SessionStart", &wrapped(ENVELOPE_TEXT));
    let output = case.run_hook(&payload).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    let after = case.status(&id).await;
    assert_eq!(after.state, before.state);
    assert_eq!(after.evidence, before.evidence);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_oversized_prompt_fails_open() {
    let case = HookCase::start().await;
    let prompt = "x".repeat(512 * 1024 + 1);
    let output = case.run_hook(&hook_payload(&prompt)).await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("a2amx hook:")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_unresponsive_daemon_finishes_within_eight_seconds() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        done_rx.recv().unwrap();
    });
    let started = std::time::Instant::now();
    let output = run_hook(&hook_payload("hello"), Some(address), Some("a".repeat(64))).await;
    done_tx.send(()).unwrap();
    server.join().unwrap();
    assert!(output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("a2amx hook:")
    );
    assert!(started.elapsed() < Duration::from_secs(8));
}
