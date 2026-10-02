#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{MessageInfo, Request, Response};
use anyhow::{Result, bail, ensure};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, oneshot};
use tokio::task::{JoinHandle, JoinSet};

const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";
const WAIT: Duration = Duration::from_secs(60);
const MODEL_ENTRY: &str = "      - id: fake-1\n        name: fake-1\n        contextWindow: 1000000\n        maxTokens: 4096\n";

#[derive(Clone, Debug)]
struct RequestRecord {
    first_user: Option<String>,
    last_user: Option<String>,
    last_role: Option<String>,
    tools: Vec<String>,
    tool_messages: Vec<String>,
    request: Arc<Value>,
}

struct FakeServer {
    addr: SocketAddr,
    records: Arc<Mutex<Vec<RequestRecord>>>,
    errors: Arc<Mutex<Vec<String>>>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl FakeServer {
    async fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let records = Arc::new(Mutex::new(Vec::new()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let (stop, stop_signal) = oneshot::channel();
        let task_records = Arc::clone(&records);
        let task_errors = Arc::clone(&errors);
        let task = tokio::spawn(async move {
            run_server(listener, task_records, task_errors, stop_signal).await;
        });
        Ok(Self {
            addr,
            records,
            errors,
            stop: Some(stop),
            task: Some(task),
        })
    }

    fn port(&self) -> u16 {
        self.addr.port()
    }

    async fn snapshot(&self) -> Vec<RequestRecord> {
        self.records.lock().await.clone()
    }

    async fn with_request_dump(&self, outcome: Result<()>) -> Result<()> {
        match outcome {
            Ok(()) => Ok(()),
            Err(error) => {
                let records = self.records.lock().await;
                let dump = records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| format!("request {index}:\n{:#}", record.request))
                    .collect::<Vec<_>>()
                    .join("\n");
                bail!(
                    "{error:#}\nmodel entry written:\n{MODEL_ENTRY}\nfake-server full request dump:\n{dump}"
                )
            }
        }
    }

    async fn ensure_healthy(&self) -> Result<()> {
        let errors = self.errors.lock().await.clone();
        if errors.is_empty() {
            return Ok(());
        }
        bail!("fake server errors: {}", errors.join("; "));
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(stop) = self.stop.take() {
            drop(stop);
        }
        if let Some(task) = self.task.take() {
            task.await?;
        }
        self.ensure_healthy().await
    }
}

async fn record_server_error(errors: &Arc<Mutex<Vec<String>>>, error: impl ToString) {
    errors.lock().await.push(error.to_string());
}

async fn run_server(
    listener: TcpListener,
    records: Arc<Mutex<Vec<RequestRecord>>>,
    errors: Arc<Mutex<Vec<String>>>,
    mut stop: oneshot::Receiver<()>,
) {
    let mut clients = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stop => break,
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _)) => {
                        let records = Arc::clone(&records);
                        clients.spawn(async move {
                            handle_connection(stream, records).await
                        });
                    }
                    Err(error) => {
                        record_server_error(&errors, format!("accept: {error}")).await;
                        break;
                    }
                }
            }
            Some(result) = clients.join_next(), if !clients.is_empty() => {
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        record_server_error(&errors, format!("connection: {error}")).await;
                    }
                    Err(error) => {
                        record_server_error(&errors, format!("connection task: {error}")).await;
                    }
                }
            }
        }
    }
    clients.abort_all();
    while let Some(result) = clients.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                record_server_error(&errors, format!("connection: {error}")).await;
            }
            Err(error) if !error.is_cancelled() => {
                record_server_error(&errors, format!("connection task: {error}")).await;
            }
            Err(_) => {}
        }
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    records: Arc<Mutex<Vec<RequestRecord>>>,
) -> Result<()> {
    let Some((method, path, body)) = read_http_request(&mut stream).await? else {
        return Ok(());
    };
    if method != "POST" || !path.ends_with("/chat/completions") {
        stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await?;
        return Ok(());
    }
    let request = match serde_json::from_slice::<Value>(&body) {
        Ok(request) => request,
        Err(error) => {
            stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await?;
            bail!("invalid completion JSON: {error}");
        }
    };
    let record = record_request(request);
    records.lock().await.push(record.clone());
    write_completion(&mut stream, &record).await?;
    Ok(())
}

async fn read_http_request(
    stream: &mut TcpStream,
) -> std::io::Result<Option<(String, String, Vec<u8>)>> {
    const MAX_REQUEST: usize = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let count = stream.read(&mut chunk).await?;
        if count == 0 {
            return Ok(None);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > MAX_REQUEST {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP request exceeds fake-server limit",
            ));
        }
        let Some(header_end) = find_bytes(&bytes, b"\r\n\r\n") else {
            continue;
        };
        let body_start = header_end + 4;
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
            .unwrap_or(0);
        let Some(end) = body_start.checked_add(content_length) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP content length overflows fake-server limit",
            ));
        };
        if bytes.len() < end {
            continue;
        }
        let request_line = headers.lines().next().unwrap_or_default();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap_or_default().to_owned();
        let path = request_parts.next().unwrap_or_default().to_owned();
        return Ok(Some((method, path, bytes[body_start..end].to_vec())));
    }
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn record_request(request: Value) -> RequestRecord {
    let messages = request
        .get("messages")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let first_user = messages
        .iter()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| content_text(message.get("content").unwrap_or(&Value::Null)));
    let last_role = messages
        .last()
        .and_then(|message| message.get("role"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let last_user = messages
        .iter()
        .rev()
        .find(|message| message.get("role").and_then(Value::as_str) == Some("user"))
        .map(|message| content_text(message.get("content").unwrap_or(&Value::Null)));
    let tool_messages = messages
        .iter()
        .filter(|message| message.get("role").and_then(Value::as_str) == Some("tool"))
        .map(|message| content_text(message.get("content").unwrap_or(&Value::Null)))
        .collect();
    let tools = request
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            tool.get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .or_else(|| tool.get("name").and_then(Value::as_str))
                .map(str::to_owned)
        })
        .collect();
    RequestRecord {
        first_user,
        last_user,
        last_role,
        tools,
        tool_messages,
        request: Arc::new(request),
    }
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part {
                Value::String(text) => text.clone(),
                Value::Object(_) => part
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                _ => String::new(),
            })
            .collect(),
        Value::Object(object) => object
            .get("text")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| content.to_string()),
        Value::Null => String::new(),
        _ => content.to_string(),
    }
}
fn requested_call(record: &RequestRecord) -> Option<(String, Value)> {
    if record.last_role.as_deref() != Some("user") {
        return None;
    }
    let text = record.last_user.as_deref()?;
    let (_, raw_call) = text.split_once("CALL ")?;
    let call = serde_json::from_str::<Value>(raw_call.trim()).ok()?;
    let name = call.get("name").and_then(Value::as_str)?.to_owned();
    let args = call.get("args").cloned().unwrap_or_else(|| json!({}));
    Some((name, args))
}

async fn write_completion(stream: &mut TcpStream, record: &RequestRecord) -> std::io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await?;
    if record
        .last_user
        .as_deref()
        .is_some_and(|text| text.contains("SLOW"))
        || (record
            .first_user
            .as_deref()
            .is_some_and(|text| text.contains("SPAWN"))
            && record.last_role.as_deref() == Some("tool")
            && !record.tool_messages.is_empty())
    {
        for index in 0..8 {
            if index != 0 {
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
            send_chunk(
                stream,
                json!({
                    "id": "fake-slow",
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": "fake-1",
                    "choices": [{
                        "index": 0,
                        "delta": {"role": "assistant", "content": format!("chunk-{index}")},
                        "finish_reason": Value::Null
                    }]
                }),
            )
            .await?;
        }
        send_finish(stream, "stop").await?;
        return send_done(stream).await;
    }
    if let Some((name, args)) = requested_call(record) {
        return send_tool_call(stream, &name, &args, "fake-call").await;
    }
    if record.last_role.as_deref() == Some("user")
        && record
            .last_user
            .as_deref()
            .is_some_and(|text| text.contains("SPAWN"))
    {
        let args = json!({
            "i": "Spawning probe",
            "context": "probe context",
            "tasks": [{"task": "Reply with the word ok", "solutionSpace": "none"}]
        });
        return send_tool_call(stream, "task", &args, "fake-task").await;
    }
    send_chunk(
        stream,
        json!({
            "id": "fake-ok",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "fake-1",
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant", "content": "ok"},
                "finish_reason": Value::Null
            }]
        }),
    )
    .await?;
    send_finish(stream, "stop").await?;
    send_done(stream).await
}

async fn send_tool_call(
    stream: &mut TcpStream,
    name: &str,
    args: &Value,
    id: &str,
) -> std::io::Result<()> {
    let arguments = serde_json::to_string(args).map_err(std::io::Error::other)?;
    send_chunk(
        stream,
        json!({
            "id": id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "fake-1",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [{
                        "index": 0,
                        "id": id,
                        "type": "function",
                        "function": {"name": name, "arguments": arguments}
                    }]
                },
                "finish_reason": Value::Null
            }]
        }),
    )
    .await?;
    send_finish(stream, "tool_calls").await?;
    send_done(stream).await
}

async fn send_finish(stream: &mut TcpStream, reason: &str) -> std::io::Result<()> {
    send_chunk(
        stream,
        json!({
            "id": "fake-finish",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "fake-1",
            "choices": [{"index": 0, "delta": {}, "finish_reason": reason}]
        }),
    )
    .await
}

async fn send_chunk(stream: &mut TcpStream, chunk: Value) -> std::io::Result<()> {
    let text = serde_json::to_string(&chunk).map_err(std::io::Error::other)?;
    stream.write_all(b"data: ").await?;
    stream.write_all(text.as_bytes()).await?;
    stream.write_all(b"\n\n").await?;
    stream.flush().await
}

async fn send_done(stream: &mut TcpStream) -> std::io::Result<()> {
    stream.write_all(b"data: [DONE]\n\n").await?;
    stream.flush().await
}

async fn omp_available() -> Result<bool> {
    let status = tokio::task::spawn_blocking(|| {
        Command::new("omp")
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
    })
    .await?;
    match status {
        Ok(status) => {
            ensure!(
                status.success(),
                "omp --version failed with status {status}"
            );
            Ok(true)
        }
        Err(error) => {
            eprintln!("omp smoke: skipped (omp --version could not be launched: {error})");
            Ok(false)
        }
    }
}

fn write_models(scratch: &Path, port: u16) -> Result<()> {
    let models = format!(
        "providers:\n  fake:\n    api: openai-completions\n    baseUrl: http://127.0.0.1:{port}/v1\n    apiKey: fake-key\n    models:\n{MODEL_ENTRY}"
    );
    std::fs::write(scratch.join("models.yml"), models)?;
    Ok(())
}

async fn launch_omp(home: &Path, scratch: &Path, prompt: &str) -> Result<String> {
    let script = r#"exec omp -p --no-prewalk --no-session --no-title --no-lsp --model fake/fake-1 "$@" "$0" < /dev/null"#;
    let home = home.to_path_buf();
    let scratch = scratch.to_path_buf();
    let prompt = prompt.to_owned();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("--home")
            .arg(home)
            .args([
                "new",
                "--detach",
                "--name",
                "agent-review",
                "--harness",
                "omp",
                "--",
                "sh",
                "-c",
                script,
                &prompt,
            ])
            .env("PI_CODING_AGENT_DIR", scratch)
            .stdin(Stdio::null())
            .output()
    })
    .await??;
    ensure!(
        output.status.success(),
        "a2amx new --harness omp failed with status {}",
        output.status
    );
    let session = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    ensure!(!session.is_empty(), "a2amx new returned no session id");
    Ok(session)
}

async fn wait_for_record<F>(
    server: &FakeServer,
    label: &str,
    timeout: Duration,
    predicate: F,
) -> Result<RequestRecord>
where
    F: Fn(&RequestRecord) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        server.ensure_healthy().await?;
        let records = server.snapshot().await;
        if let Some(record) = records.iter().find(|record| predicate(record)) {
            return Ok(record.clone());
        }
        if Instant::now() >= deadline {
            bail!(
                "timed out waiting for {label}; fake-server records: {}",
                records_summary(&records)
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn query_status(home: &Path, id: &str) -> Option<MessageInfo> {
    let mut client = Client::connect(home).await.ok()?;
    match client
        .request(Request::MessageStatus { id: id.to_owned() })
        .await
        .ok()?
    {
        Response::Status { message } => Some(message),
        _ => None,
    }
}

async fn wait_native_receipt(home: &Path, id: &str, server: &FakeServer) -> Result<MessageInfo> {
    let deadline = Instant::now() + WAIT;
    let mut last = String::from("no status response");
    loop {
        server.ensure_healthy().await?;
        if let Some(message) = query_status(home, id).await {
            last = status_summary(&message);
            if message.state == "submitted" && message.evidence.as_deref() == Some("native_receipt")
            {
                return Ok(message);
            }
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for native_receipt for {id}; last status: {last}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn status_summary(message: &MessageInfo) -> String {
    format!(
        "state={}, evidence={}, hold_reason={}",
        message.state,
        message.evidence.as_deref().unwrap_or("-"),
        message.hold_reason.as_deref().unwrap_or("-")
    )
}

fn records_summary(records: &[RequestRecord]) -> String {
    records
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let user = record
                .last_user
                .as_deref()
                .map(summary_text)
                .unwrap_or_else(|| "-".to_owned());
            let text = record.last_user.as_deref().unwrap_or("");
            let tail = &text[text.char_indices().rev().nth(159).map_or(0, |(index, _)| index)..];
            let tool_text: Vec<String> = record.tool_messages.iter().map(|text| text.chars().take(512).collect()).collect();
            format!(
                "r{index}: user={user:?}, tail={tail:?}, exact_envelope={}, contains_envelope={}, role={:?}, tools={:?}, tool_messages={tool_text:?}",
                record.last_user.as_deref() == Some(ENVELOPE_TEXT),
                record.last_user.as_deref().is_some_and(|text| text.contains(ENVELOPE_TEXT)),
                record.last_role,
                record.tools,
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn summary_text(text: &str) -> String {
    let mut summary = text
        .chars()
        .map(|character| {
            if matches!(character, '\n' | '\r') {
                ' '
            } else {
                character
            }
        })
        .take(96)
        .collect::<String>();
    if text.chars().count() > 96 {
        summary.push_str("...");
    }
    summary
}

async fn native_message_case(home: &Path, server: &FakeServer, scratch: &Path) -> Result<()> {
    write_models(scratch, server.port())?;
    let mut admin = Client::connect(home).await?;
    let (_, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        home,
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Auto,
    )
    .await;
    launch_omp(home, scratch, "SLOW work").await?;
    wait_for_record(
        server,
        "the initial SLOW request",
        Duration::from_secs(30),
        |record| {
            record
                .last_user
                .as_deref()
                .is_some_and(|text| text.contains("SLOW"))
        },
    )
    .await?;
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    let response = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".to_owned(),
            subject: "Parser issue".to_owned(),
            message: "I found the regression in parser.py.".to_owned(),
        })
        .await?;
    let Response::Accepted { id, .. } = response else {
        bail!("generic sender did not accept the message");
    };

    let initial = Client::connect(home)
        .await?
        .request(Request::MessageStatus { id: id.clone() })
        .await?;
    let Response::Status { message } = initial else {
        bail!("message status was unavailable immediately after acceptance");
    };
    ensure!(
        message.evidence.as_deref() != Some("native_receipt")
            && matches!(
                message.state.as_str(),
                "pending" | "delivering" | "submitted"
            ),
        "message was not queued mid-turn before the slow turn completed: {}",
        status_summary(&message)
    );

    let delivered = wait_native_receipt(home, &id, server).await?;
    ensure!(
        delivered.evidence.as_deref() == Some("native_receipt"),
        "message did not receive native receipt: {}",
        status_summary(&delivered)
    );
    wait_for_record(server, "the exact native envelope", WAIT, |record| {
        record.last_user.as_deref() == Some(ENVELOPE_TEXT)
    })
    .await?;
    let records = server.snapshot().await;
    ensure!(
        records.iter().all(|record| {
            record
                .last_user
                .as_deref()
                .is_none_or(|text| !text.contains("User interjection"))
        }),
        "fake-server request contained an unexpected User interjection: {}",
        records_summary(&records)
    );
    Ok(())
}

async fn native_tools_case(home: &Path, server: &FakeServer, scratch: &Path) -> Result<()> {
    write_models(scratch, server.port())?;
    launch_omp(
        home,
        scratch,
        r#"CALL {"name":"list_agents","args":{"i":"Listing agents"}}"#,
    )
    .await?;
    let first = wait_for_record(server, "the messaging-tool request", WAIT, |record| {
        record
            .last_user
            .as_deref()
            .is_some_and(|text| text.contains("CALL "))
    })
    .await?;
    for name in ["list_agents", "send_message", "message_status"] {
        ensure!(
            first.tools.iter().any(|tool| tool == name),
            "messaging-tool request omitted {name}; request tools: {:?}",
            first.tools
        );
    }
    let tool_result = wait_for_record(server, "the list_agents tool result", WAIT, |record| {
        record
            .tool_messages
            .iter()
            .any(|message| message.contains("agent-review@host-a"))
    })
    .await?;
    ensure!(
        tool_result
            .tool_messages
            .iter()
            .any(|message| message.contains("agent-review@host-a")),
        "list_agents result did not contain agent-review@host-a"
    );
    Ok(())
}

async fn subagent_tools_case(home: &Path, server: &FakeServer, scratch: &Path) -> Result<()> {
    write_models(scratch, server.port())?;
    launch_omp(home, scratch, "SPAWN a subagent").await?;
    let first = wait_for_record(server, "the main SPAWN request", WAIT, |record| {
        record
            .last_user
            .as_deref()
            .is_some_and(|text| text.contains("SPAWN"))
    })
    .await?;
    ensure!(
        first.tools.iter().any(|tool| tool == "send_message"),
        "main OMP request did not expose send_message; request tools: {:?}",
        first.tools
    );
    let subagent = wait_for_record(
        server,
        "the subagent request",
        Duration::from_secs(30),
        |record| {
            record
                .last_user
                .as_deref()
                .is_some_and(|text| text.contains("Reply with the word ok"))
        },
    )
    .await?;
    ensure!(
        !subagent.tools.iter().any(|tool| tool == "send_message"),
        "subagent OMP request exposed send_message; request tools: {:?}",
        subagent.tools
    );
    Ok(())
}
fn merge_cleanup_results(
    outcome: Result<()>,
    server_outcome: Result<()>,
    daemon_outcome: Result<()>,
) -> Result<()> {
    let mut errors = Vec::new();
    if let Err(error) = outcome {
        errors.push(error.to_string());
    }
    if let Err(error) = server_outcome {
        errors.push(format!("fake-server cleanup: {error}"));
    }
    if let Err(error) = daemon_outcome {
        errors.push(format!("daemon cleanup: {error}"));
    }
    if errors.is_empty() {
        Ok(())
    } else {
        bail!("{}", errors.join("; "))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the omp binary; run with cargo test -- --ignored"]
async fn a_native_message_reaches_real_omp_and_is_receipted() -> Result<()> {
    if !omp_available().await? {
        return Ok(());
    }
    let (home, daemon) = common::start_daemon().await;
    let server = FakeServer::start().await?;
    let scratch = tempfile::tempdir()?;
    let outcome = native_message_case(home.path(), &server, scratch.path()).await;
    let outcome = server.with_request_dump(outcome).await;
    let server_outcome = server.shutdown().await;
    let daemon_outcome = daemon.shutdown().await;
    merge_cleanup_results(outcome, server_outcome, daemon_outcome)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the omp binary; run with cargo test -- --ignored"]
async fn real_omp_calls_the_messaging_tools_natively() -> Result<()> {
    if !omp_available().await? {
        return Ok(());
    }
    let (home, daemon) = common::start_daemon().await;
    let server = FakeServer::start().await?;
    let scratch = tempfile::tempdir()?;
    let outcome = native_tools_case(home.path(), &server, scratch.path()).await;
    let outcome = server.with_request_dump(outcome).await;
    let server_outcome = server.shutdown().await;
    let daemon_outcome = daemon.shutdown().await;
    merge_cleanup_results(outcome, server_outcome, daemon_outcome)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the omp binary; run with cargo test -- --ignored"]
async fn a_subagent_gets_no_messaging_tools() -> Result<()> {
    if !omp_available().await? {
        return Ok(());
    }
    let (home, daemon) = common::start_daemon().await;
    let server = FakeServer::start().await?;
    let scratch = tempfile::tempdir()?;
    let outcome = subagent_tools_case(home.path(), &server, scratch.path()).await;
    let outcome = server.with_request_dump(outcome).await;
    let server_outcome = server.shutdown().await;
    let daemon_outcome = daemon.shutdown().await;
    merge_cleanup_results(outcome, server_outcome, daemon_outcome)
}
