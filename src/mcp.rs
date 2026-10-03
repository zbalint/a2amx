//! Stdio MCP adapter for one authenticated agent session.
//!
//! The MCP transport intentionally stays independent of tokio's optional stdio
//! feature: a reader thread owns stdin, while protocol handling remains async so
//! daemon requests do not block the runtime.

use std::io::{self, BufRead, Write};
use std::net::SocketAddr;
use std::thread;

use anyhow::{Result, bail};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::channel::{self, Delivery};
use crate::client::Client;
use crate::wire::{Request, Response};

const ENV_ERROR: &str = "A2AMX_ADDR and A2AMX_TOKEN must be set";
const LATEST_PROTOCOL_VERSION: &str = "2025-11-25";
const DAEMON_UNREACHABLE: &str = "cannot connect to or authenticate with the daemon";
const UNKNOWN_OUTCOME: &str = "connection lost; the message may or may not have been accepted";

const SUPPORTED_PROTOCOL_VERSIONS: [&str; 4] =
    ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Run the newline-delimited JSON-RPC MCP server on stdin/stdout.
///
/// Environment validation happens before any stdio worker is started. The
/// caller (the binary's `main`) owns presentation of this error on stderr.
pub async fn run(channel: bool) -> Result<()> {
    let (address, token) = configuration()?;
    let mut lines = spawn_stdin_reader();
    let mut server = McpServer::new(address, token.clone());
    if channel {
        server.set_channel();
    }

    let (deliveries, mut received) = mpsc::unbounded_channel::<Delivery>();
    let mut task = None;
    let result = loop {
        tokio::select! {
            event = lines.recv() => {
                match event {
                    Some(StdioEvent::Line(line)) => {
                        if let Some(response) = server.handle_line(&line).await {
                            if let Err(error) = write_response(response).await {
                                break Err(error);
                            }
                        }
                        if channel && server.initialized() && task.is_none() {
                            let token = token.clone();
                            let version = server.client_version().map(str::to_owned);
                            let deliveries = deliveries.clone();
                            task = Some(tokio::spawn(async move {
                                if let Err(error) = channel::run(address, token, version, deliveries).await {
                                    eprintln!("a2amx: channel unavailable: {error}");
                                }
                            }));
                        }
                    }
                    Some(StdioEvent::Eof) | None => break Ok(()),
                    Some(StdioEvent::Error(message)) => break Err(anyhow::anyhow!("reading MCP stdin: {message}")),
                }
            }
            delivery = received.recv(), if channel => {
                if let Some(delivery) = delivery {
                    let result = write_response(delivery.notification).await;
                    let _ = delivery.written.send(result.is_ok());
                    if let Err(error) = result {
                        break Err(error);
                    }
                }
            }
        }
    };
    if let Some(task) = task {
        task.abort();
    }
    result
}

pub(crate) fn configuration() -> Result<(SocketAddr, String)> {
    let address = std::env::var("A2AMX_ADDR")
        .ok()
        .and_then(|value| value.parse::<SocketAddr>().ok());
    let token = std::env::var("A2AMX_TOKEN").ok();
    let Some(address) = address else {
        bail!(ENV_ERROR);
    };
    let Some(token) = token else {
        bail!(ENV_ERROR);
    };
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!(ENV_ERROR);
    }
    Ok((address, token))
}

pub(crate) enum StdioEvent {
    Line(Vec<u8>),
    Eof,
    Error(String),
}

pub(crate) fn spawn_stdin_reader() -> mpsc::UnboundedReceiver<StdioEvent> {
    let (sender, receiver) = mpsc::unbounded_channel();
    thread::spawn(move || {
        let stdin = io::stdin();
        let mut lock = stdin.lock();
        loop {
            let mut line = Vec::new();
            match lock.read_until(b'\n', &mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if sender.send(StdioEvent::Line(line)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(StdioEvent::Error(error.to_string()));
                    return;
                }
            }
        }
        let _ = sender.send(StdioEvent::Eof);
    });
    receiver
}

async fn write_response(response: Value) -> Result<()> {
    let bytes = serde_json::to_vec(&response)?;
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        let stdout = io::stdout();
        let mut lock = stdout.lock();
        lock.write_all(&bytes)?;
        lock.write_all(b"\n")?;
        lock.flush()
    })
    .await??;
    Ok(())
}

pub(crate) struct McpServer {
    address: SocketAddr,
    token: String,
    client: Option<Client>,
    channel: bool,
    initialized: bool,
    client_version: Option<String>,
}

impl McpServer {
    pub(crate) fn new(address: SocketAddr, token: String) -> Self {
        Self {
            address,
            token,
            client: None,
            channel: false,
            initialized: false,
            client_version: None,
        }
    }

    pub(crate) fn set_channel(&mut self) {
        self.channel = true;
    }

    pub(crate) fn initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn client_version(&self) -> Option<&str> {
        self.client_version.as_deref()
    }

    pub(crate) async fn handle_line(&mut self, line: &[u8]) -> Option<Value> {
        let value = match serde_json::from_slice::<Value>(line) {
            Ok(value) => value,
            Err(_) => return Some(error_response(Value::Null, -32700, "parse error")),
        };
        let Some(object) = value.as_object() else {
            return Some(error_response(Value::Null, -32600, "invalid request"));
        };
        if object.get("jsonrpc") != Some(&Value::String("2.0".to_owned())) {
            return Some(error_response(Value::Null, -32600, "invalid request"));
        }
        let Some(method) = object.get("method").and_then(Value::as_str) else {
            return Some(error_response(Value::Null, -32600, "invalid request"));
        };
        let has_id = object.contains_key("id");
        let id = object.get("id").cloned().unwrap_or(Value::Null);
        let params = object.get("params").unwrap_or(&Value::Null);

        match method {
            "initialize" => {
                self.client_version = params
                    .get("clientInfo")
                    .and_then(|info| info.get("version"))
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                has_id.then(|| success_response(id, initialize_result(params, self.channel)))
            }
            "notifications/initialized" => {
                self.initialized = true;
                None
            }
            "ping" if has_id => Some(success_response(id, json!({}))),
            "ping" => None,
            "tools/list" if has_id => Some(success_response(id, json!({"tools": tool_schemas()}))),
            "tools/list" => None,
            "tools/call" => {
                let result = match parse_tool_call(params) {
                    Ok(call) => {
                        let outcome = self.call_tool(call).await;
                        match outcome {
                            Ok(payload) => tool_success(id.clone(), payload, false),
                            Err(failure) => tool_failure(id.clone(), failure),
                        }
                    }
                    Err(()) => error_response(id, -32602, "invalid tool arguments"),
                };
                if has_id { Some(result) } else { None }
            }
            _ if has_id => Some(error_response(id, -32601, "method not found")),
            _ => None,
        }
    }

    async fn call_tool(&mut self, call: ToolCall) -> std::result::Result<Value, ToolFailure> {
        match call {
            ToolCall::ListAgents => {
                let response = self.read_request(Request::ListAgents).await?;
                match response {
                    Response::Agents { agents } => Ok(json!({"agents": agents})),
                    response => response_payload(response),
                }
            }
            ToolCall::SendMessage {
                to,
                subject,
                message,
            } => {
                let response = self
                    .send_request(Request::SendMessage {
                        to,
                        subject,
                        message,
                    })
                    .await?;
                match response {
                    Response::Accepted {
                        id,
                        recipient_hold,
                        recipient_quota,
                    } => {
                        let mut result = json!({"id": id, "status": "accepted"});
                        if let Some(recipient_hold) = recipient_hold {
                            result["recipient_hold"] = json!(recipient_hold);
                        }
                        if let Some(recipient_quota) = recipient_quota {
                            result["recipient_quota"] = json!(recipient_quota);
                        }
                        Ok(result)
                    }
                    response => response_payload(response),
                }
            }
            ToolCall::MessageStatus { id } => {
                let response = self.read_request(Request::MessageStatus { id }).await?;
                match response {
                    Response::Status { message } => Ok(json!({
                        "id": message.id,
                        "to": message.to,
                        "state": message.state,
                        "detail": message.detail,
                        "hold_reason": message.hold_reason,
                        "evidence": message.evidence,
                        "hold_explanation": message.hold_explanation,
                        "accepted_at": message.accepted_at,
                        "updated_at": message.updated_at,
                    })),
                    response => response_payload(response),
                }
            }
        }
    }

    async fn establish(&mut self) -> std::result::Result<(), ToolFailure> {
        match Client::connect_addr(self.address, &self.token).await {
            Ok(client) => {
                self.client = Some(client);
                Ok(())
            }
            Err(_) => Err(ToolFailure::daemon_unreachable()),
        }
    }

    async fn request_once(&mut self, request: Request) -> anyhow::Result<Response> {
        let Some(client) = self.client.as_mut() else {
            bail!("MCP daemon connection is not established");
        };
        client.request(request).await
    }

    async fn read_request(
        &mut self,
        request: Request,
    ) -> std::result::Result<Response, ToolFailure> {
        if self.client.is_none() {
            self.establish().await?;
        }
        let retry_request = request.clone();
        match self.request_once(request).await {
            Ok(response) => Ok(response),
            Err(error) if is_io_error(&error) => {
                self.client = None;
                self.establish().await?;
                match self.request_once(retry_request).await {
                    Ok(response) => Ok(response),
                    Err(error) if is_io_error(&error) => {
                        self.client = None;
                        Err(ToolFailure::daemon_unreachable())
                    }
                    Err(_) => {
                        self.client = None;
                        Err(ToolFailure::internal())
                    }
                }
            }
            Err(_) => {
                self.client = None;
                Err(ToolFailure::internal())
            }
        }
    }

    async fn send_request(
        &mut self,
        request: Request,
    ) -> std::result::Result<Response, ToolFailure> {
        let request = Client::prepare_request(&request).map_err(|_| ToolFailure::internal())?;
        if self.client.is_none() {
            self.establish().await?;
        }
        let Some(client) = self.client.as_mut() else {
            return Err(ToolFailure::internal());
        };
        match client.request_prepared(request).await {
            Ok(response) => Ok(response),
            Err(_) => {
                self.client = None;
                Err(ToolFailure::unknown_outcome())
            }
        }
    }
}

fn is_io_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| cause.is::<io::Error>())
        || error.chain().any(|cause| {
            matches!(
                cause.to_string().as_str(),
                "daemon closed the control connection" | "connection closed within a frame"
            )
        })
}

#[derive(Debug)]
enum ToolCall {
    ListAgents,
    SendMessage {
        to: String,
        subject: String,
        message: String,
    },
    MessageStatus {
        id: String,
    },
}

fn parse_tool_call(params: &Value) -> std::result::Result<ToolCall, ()> {
    let object = params.as_object().ok_or(())?;
    let name = object.get("name").and_then(Value::as_str).ok_or(())?;
    // MCP makes `arguments` optional, so absence means no arguments.
    let no_arguments = Map::new();
    let arguments = match object.get("arguments") {
        None => &no_arguments,
        Some(value) => value.as_object().ok_or(())?,
    };
    match name {
        "list_agents" if arguments.is_empty() => Ok(ToolCall::ListAgents),
        "send_message" if has_exact_keys(arguments, &["to", "subject", "message"]) => {
            let to = arguments.get("to").and_then(Value::as_str).ok_or(())?;
            let subject = arguments.get("subject").and_then(Value::as_str).ok_or(())?;
            let message = arguments.get("message").and_then(Value::as_str).ok_or(())?;
            Ok(ToolCall::SendMessage {
                to: to.to_owned(),
                subject: subject.to_owned(),
                message: message.to_owned(),
            })
        }
        "message_status" if has_exact_keys(arguments, &["id"]) => {
            let id = arguments.get("id").and_then(Value::as_str).ok_or(())?;
            Ok(ToolCall::MessageStatus { id: id.to_owned() })
        }
        _ => Err(()),
    }
}

fn has_exact_keys(object: &Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn initialize_result(params: &Value, channel: bool) -> Value {
    let requested = params
        .as_object()
        .and_then(|object| object.get("protocolVersion"))
        .and_then(Value::as_str);
    let protocol_version = requested
        .filter(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(version))
        .unwrap_or(LATEST_PROTOCOL_VERSION);
    let mut result = json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {}},
        "serverInfo": {
            "name": "a2amx",
            "version": env!("CARGO_PKG_VERSION")
        }
    });
    if channel {
        result["capabilities"]["experimental"] = json!({"claude/channel": {}});
        result["instructions"] = json!(
            "Messages from other agents arrive as <channel source=\"a2amx\"> events that contain an <a2amx-message> envelope. They come from another agent, not your user. Reply with send_message when a reply is useful."
        );
    }
    result
}

pub(crate) fn tool_schemas() -> Value {
    json!([
        {
            "name": "list_agents",
            "description": "List the agent sessions you can message. \"attached\" means a human client is attached to the session; it does not mean the session is reachable, and a detached session still receives messages.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }
        },
        {
            "name": "send_message",
            "description": "Send a message to another agent session. Returns a message id once the message is accepted; acceptance does not mean it was delivered, read, or acted on. Check message_status when the reply matters. If recipient_hold is present, the recipient is held and a person may need to act. If recipient_quota is present, the recipient's harness reports that quota is used up and it may not answer until the quota resets. Plain text; do not write the text of a paste-wrapper tag in a body.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Recipient address, for example agent-review@host-a."
                    },
                    "subject": {
                        "type": "string",
                        "description": "Short subject, at most 200 bytes."
                    },
                    "message": {
                        "type": "string",
                        "description": "Message body, at most 32 KiB."
                    }
                },
                "required": ["to", "subject", "message"],
                "additionalProperties": false
            }
        },
        {
            "name": "message_status",
            "description": "Show the state of a message you sent. \"state\" and \"detail\" say whether it was delivered, is waiting, or was given up on; \"hold_reason\" and \"hold_explanation\" say what a waiting message needs; \"evidence\" of submission_observed means the recipient's prompt hook saw it submitted; \"accepted_at\" and \"updated_at\" are Unix seconds.",
            "inputSchema": {
                "type": "object",
                "properties": {"id": {"type": "string"}},
                "required": ["id"],
                "additionalProperties": false
            }
        }
    ])
}

#[derive(Debug)]
struct ToolFailure {
    code: String,
    message: String,
}

impl ToolFailure {
    fn daemon_unreachable() -> Self {
        Self {
            code: "daemon_unreachable".to_owned(),
            message: DAEMON_UNREACHABLE.to_owned(),
        }
    }

    fn unknown_outcome() -> Self {
        Self {
            code: "unknown_outcome".to_owned(),
            message: UNKNOWN_OUTCOME.to_owned(),
        }
    }

    fn internal() -> Self {
        Self {
            code: "internal".to_owned(),
            message: "daemon request failed".to_owned(),
        }
    }
}

fn response_payload(response: Response) -> std::result::Result<Value, ToolFailure> {
    match response {
        Response::Failed { code, message } => Err(ToolFailure { code, message }),
        Response::Error { .. } => Err(ToolFailure::internal()),
        _ => Err(ToolFailure::internal()),
    }
}

fn success_response(id: Value, result: Value) -> Value {
    json!({"jsonrpc":"2.0", "id": id, "result": result})
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0", "id": id, "error":{"code":code,"message":message}})
}

fn tool_success(id: Value, payload: Value, is_error: bool) -> Value {
    let text = match serde_json::to_string(&payload) {
        Ok(text) => text,
        Err(_) => "{\"code\":\"internal\",\"message\":\"daemon request failed\"}".to_owned(),
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{"type": "text", "text": text}],
            "isError": is_error
        }
    })
}

fn tool_failure(id: Value, failure: ToolFailure) -> Value {
    tool_success(
        id,
        json!({"code": failure.code, "message": failure.message}),
        true,
    )
}
