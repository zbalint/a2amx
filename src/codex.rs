//! Codex hosting: one private `codex app-server` per session, the TUI attached to it
//! with `--remote`, and the daemon's native delivery channel built on its protocol.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::process::{Child, Command};

use crate::harness::PEER_AUTHORIZATION_PROMPT;
use crate::session::Session;
use crate::store::Store;

/// Request-environment marker for `--no-authorize-peers`, consumed by the daemon.
pub const NO_AUTHORIZE_ENV: &str = "A2AMX_NO_AUTHORIZE_PEERS";
const CALL_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(500);
// shortcut: a message that never shows up in the thread's items stops blocking the
// queue after this long; a smarter check would compare against turn completion.
const RECEIPT_GIVE_UP: Duration = Duration::from_secs(120);
const MAX_FRAME: usize = 16 * 1024 * 1024;
const ITEMS_PAGE: u32 = 50;
/// The app-server does not validate the key, and the socket is owner-only.
const WS_KEY: &str = "YTJhbXgtY29kZXgtcHJvYmU=";
// shortcut: a fixed mask; masking only matters to intermediaries on a local socket.
const WS_MASK: [u8; 4] = [0x5a, 0xa5, 0x3c, 0xc3];

/// A failed protocol call, split by whether the server could have acted on it.
#[derive(Debug)]
pub(crate) enum CallError {
    /// The server answered with a JSON-RPC error.
    Rejected(String),
    /// The exchange broke; the server may or may not have acted.
    Transport(anyhow::Error),
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(message) => write!(f, "{message}"),
            Self::Transport(error) => write!(f, "{error:#}"),
        }
    }
}

impl std::error::Error for CallError {}

/// Minimal JSON-RPC client for the app-server's WebSocket-over-unix-socket transport.
pub(crate) struct Rpc {
    stream: UnixStream,
    buffer: Vec<u8>,
    next_id: u64,
}

impl Rpc {
    pub(crate) async fn connect(path: &Path) -> anyhow::Result<Self> {
        let attempt = async {
            // A path beyond SUN_LEN cannot be connected to directly; Codex answers on a
            // short socket and links the requested path to it, so follow the link.
            let linked = path.to_path_buf();
            let target = tokio::task::spawn_blocking(move || std::fs::canonicalize(&linked))
                .await?
                .unwrap_or_else(|_| path.to_path_buf());
            let mut stream = UnixStream::connect(target).await?;
            let request = format!(
                "GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {WS_KEY}\r\nSec-WebSocket-Version: 13\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).await?;
            let mut head = Vec::new();
            let mut byte = [0u8; 1];
            while !head.ends_with(b"\r\n\r\n") {
                if head.len() > 8192 {
                    bail!("oversized WebSocket handshake response");
                }
                if stream.read(&mut byte).await? == 0 {
                    bail!("closed during the WebSocket handshake");
                }
                head.push(byte[0]);
            }
            let status = head.split(|b| *b == b'\r').next().unwrap_or_default();
            if !String::from_utf8_lossy(status).contains(" 101 ") {
                bail!("WebSocket upgrade refused");
            }
            let mut rpc = Self {
                stream,
                buffer: Vec::new(),
                next_id: 0,
            };
            let info = json!({
                "clientInfo": {
                    "name": "a2amx",
                    "title": "a2amx",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            });
            rpc.call("initialize", info)
                .await
                .map_err(|error| anyhow!("{error}"))?;
            rpc.send(&json!({"method": "initialized"})).await?;
            Ok(rpc)
        };
        tokio::time::timeout(CALL_TIMEOUT, attempt)
            .await
            .context("connecting to the Codex app-server timed out")?
    }

    pub(crate) async fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        self.next_id += 1;
        let id = self.next_id;
        let exchange = async {
            self.send(&json!({"id": id, "method": method, "params": params}))
                .await?;
            loop {
                let message = self.receive().await?;
                // Notifications and server requests are not ours to answer: this client
                // never subscribes, so it never needs to approve anything.
                if message.get("id").and_then(Value::as_u64) != Some(id)
                    || message.get("method").is_some()
                {
                    continue;
                }
                return Ok(message);
            }
        };
        let message = tokio::time::timeout(CALL_TIMEOUT, exchange)
            .await
            .map_err(|_| CallError::Transport(anyhow!("{method} timed out")))?
            .map_err(CallError::Transport)?;
        if let Some(error) = message.get("error") {
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("request rejected");
            return Err(CallError::Rejected(text.to_owned()));
        }
        Ok(message.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn send(&mut self, value: &Value) -> anyhow::Result<()> {
        let payload = serde_json::to_vec(value)?;
        self.write_frame(0x1, &payload).await
    }

    async fn write_frame(&mut self, opcode: u8, payload: &[u8]) -> anyhow::Result<()> {
        let mut frame = vec![0x80 | opcode];
        match payload.len() {
            len if len < 126 => frame.push(0x80 | len as u8),
            len if len <= usize::from(u16::MAX) => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(len as u16).to_be_bytes());
            }
            len => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(len as u64).to_be_bytes());
            }
        }
        frame.extend_from_slice(&WS_MASK);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ WS_MASK[index % 4]),
        );
        self.stream.write_all(&frame).await?;
        Ok(())
    }

    async fn fill(&mut self, wanted: usize) -> anyhow::Result<()> {
        let mut chunk = [0u8; 8192];
        while self.buffer.len() < wanted {
            let count = self.stream.read(&mut chunk).await?;
            if count == 0 {
                bail!("the Codex app-server closed the connection");
            }
            self.buffer.extend_from_slice(&chunk[..count]);
        }
        Ok(())
    }

    /// Next complete JSON text message; answers pings and ignores other frames.
    async fn receive(&mut self) -> anyhow::Result<Value> {
        let mut message = Vec::new();
        loop {
            self.fill(2).await?;
            let (first, second) = (self.buffer[0], self.buffer[1]);
            let (fin, opcode) = (first & 0x80 != 0, first & 0x0f);
            let mut offset = 2;
            let length = match second & 0x7f {
                126 => {
                    self.fill(4).await?;
                    offset = 4;
                    usize::from(u16::from_be_bytes([self.buffer[2], self.buffer[3]]))
                }
                127 => {
                    self.fill(10).await?;
                    offset = 10;
                    let mut bytes = [0u8; 8];
                    bytes.copy_from_slice(&self.buffer[2..10]);
                    usize::try_from(u64::from_be_bytes(bytes)).unwrap_or(usize::MAX)
                }
                short => usize::from(short),
            };
            if length > MAX_FRAME {
                bail!("oversized WebSocket frame");
            }
            self.fill(offset + length).await?;
            let payload: Vec<u8> = self.buffer[offset..offset + length].to_vec();
            self.buffer.drain(..offset + length);
            match opcode {
                0x0 | 0x1 => {
                    message.extend_from_slice(&payload);
                    if message.len() > MAX_FRAME {
                        bail!("oversized WebSocket message");
                    }
                    if fin {
                        return serde_json::from_slice(&message)
                            .context("the Codex app-server sent invalid JSON");
                    }
                }
                0x8 => bail!("the Codex app-server closed the connection"),
                0x9 => self.write_frame(0xa, &payload).await?,
                _ => {}
            }
        }
    }
}

/// What the poller last saw, shared with the delivery channel and status handlers.
#[derive(Default)]
struct LinkState {
    reason: Option<&'static str>,
    thread: Option<String>,
    awaiting: Option<Awaiting>,
}

struct Awaiting {
    id: String,
    envelope: String,
    since: Instant,
}

/// The private app-server of one session and the channel state derived from it.
pub(crate) struct Link {
    socket: PathBuf,
    child: Mutex<Option<Child>>,
    state: Mutex<LinkState>,
}

impl Link {
    fn lock(&self) -> std::sync::MutexGuard<'_, LinkState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    /// Why a message cannot be sent right now; `None` when one can.
    pub(crate) fn reason(&self) -> Option<&'static str> {
        self.lock().reason
    }

    /// Thread the next message goes to.
    pub(crate) fn thread(&self) -> Option<String> {
        self.lock().thread.clone()
    }

    /// Records a message `turn/start` accepted, until the thread shows it.
    pub(crate) fn expect(&self, id: &str, envelope: &str) {
        let mut state = self.lock();
        state.awaiting = Some(Awaiting {
            id: id.to_owned(),
            envelope: envelope.to_owned(),
            since: Instant::now(),
        });
        state.reason = Some("in_flight");
    }

    /// The app-server stopped answering; the poller restores the state when it is back.
    pub(crate) fn down(&self) {
        self.lock().reason = Some("app_server_down");
    }

    fn kill(&self) {
        let child = self.child.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(mut child) = child {
            // start_kill only signals; kill_on_drop reaps the zombie with the handle.
            let _ = child.start_kill();
        }
    }
}

/// Config the app-server needs so its threads can message peers. The token reaches the
/// MCP server through the inherited environment, never argv.
pub(crate) fn server_config(exe: &Path, authorize_peers: bool) -> Vec<String> {
    let mut config = vec![
        format!("mcp_servers.a2amx.command={}", json!(exe.to_string_lossy())),
        "mcp_servers.a2amx.args=[\"mcp\"]".to_owned(),
        "mcp_servers.a2amx.env_vars=[\"A2AMX_TOKEN\",\"A2AMX_ADDR\"]".to_owned(),
    ];
    for tool in ["list_agents", "send_message", "message_status"] {
        config.push(format!(
            "mcp_servers.a2amx.tools.{tool}.approval_mode=\"approve\""
        ));
    }
    if authorize_peers {
        config.push(format!(
            "developer_instructions={}",
            json!(PEER_AUTHORIZATION_PROMPT)
        ));
    }
    config
        .into_iter()
        .flat_map(|value| ["-c".to_owned(), value])
        .collect()
}

/// Points the TUI at the session's app-server.
pub(crate) fn wire_argv(argv: Vec<String>, socket: &Path) -> Vec<String> {
    let extras = vec![
        "--remote".to_owned(),
        format!("unix://{}", socket.to_string_lossy()),
    ];
    crate::harness::insert_extras(argv, extras)
}

/// Starts the session's app-server and waits until it accepts connections.
pub(crate) async fn start(
    program: &str,
    state_dir: &Path,
    session_id: &str,
    env: &[(String, String)],
    config: Vec<String>,
) -> anyhow::Result<Arc<Link>> {
    let dir = state_dir.join("codex");
    let socket = dir.join(format!("{session_id}.sock"));
    let prepare = {
        let (dir, socket) = (dir.clone(), socket.clone());
        move || -> anyhow::Result<()> {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&dir)?;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
            match std::fs::remove_file(&socket) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
                _ => Ok(()),
            }
        }
    };
    tokio::task::spawn_blocking(prepare).await??;
    let mut command = Command::new(program);
    command
        .arg("app-server")
        .arg("--listen")
        .arg(format!("unix://{}", socket.to_string_lossy()))
        .args(config)
        .env_clear()
        .envs(env.iter().map(|(key, value)| (key, value)))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {program} app-server"))?;
    // The first start after a host reboot is slow (cold caches, Codex's own /tmp state).
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut last = anyhow!("not started");
    loop {
        if let Some(status) = child.try_wait()? {
            bail!("the Codex app-server exited at startup ({status})");
        }
        match Rpc::connect(&socket).await {
            Ok(_) => break,
            Err(error) => last = error,
        }
        if Instant::now() >= deadline {
            bail!("the Codex app-server did not become ready: {last:#}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(Arc::new(Link {
        socket,
        child: Mutex::new(Some(child)),
        state: Mutex::new(LinkState {
            reason: Some("app_server_down"),
            ..LinkState::default()
        }),
    }))
}

struct Observed {
    thread: Option<String>,
    reason: Option<&'static str>,
}

/// Picks the session's thread among those the app-server has loaded and names what
/// stops delivery to it.
async fn observe(rpc: &mut Rpc) -> Result<Observed, CallError> {
    let loaded = rpc.call("thread/loaded/list", json!({})).await?;
    let ids: Vec<String> = loaded
        .get("data")
        .and_then(Value::as_array)
        .map(|ids| {
            ids.iter()
                .filter_map(|id| id.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    // shortcut: the most recently updated loaded thread wins; the TUI's /new leaves the
    // old one loaded. Match on something the TUI reports if two threads ever alternate.
    let mut best: Option<(i64, String, Value)> = None;
    for id in ids {
        let read = rpc.call("thread/read", json!({"threadId": id})).await?;
        let Some(thread) = read.get("thread") else {
            continue;
        };
        let updated = thread.get("updatedAt").and_then(Value::as_i64).unwrap_or(0);
        if best.as_ref().is_none_or(|(seen, _, _)| updated >= *seen) {
            best = Some((
                updated,
                id,
                thread.get("status").cloned().unwrap_or(Value::Null),
            ));
        }
    }
    let Some((_, thread, status)) = best else {
        return Ok(Observed {
            thread: None,
            reason: Some("no_thread"),
        });
    };
    let flags = status
        .get("activeFlags")
        .and_then(Value::as_array)
        .is_some_and(|flags| !flags.is_empty());
    let reason = match status.get("type").and_then(Value::as_str) {
        Some("idle") => None,
        Some("active") if flags => Some("waiting_on_approval"),
        Some("active") => None,
        Some("systemError") => Some("thread_error"),
        _ => Some("no_thread"),
    };
    Ok(Observed {
        thread: Some(thread),
        reason,
    })
}

/// Text of the user message carrying `client_id`, if the thread has it.
async fn find_message(
    rpc: &mut Rpc,
    thread: &str,
    client_id: &str,
) -> Result<Option<String>, CallError> {
    let items = rpc
        .call(
            "thread/items/list",
            json!({"threadId": thread, "limit": ITEMS_PAGE, "sortDirection": "desc"}),
        )
        .await?;
    let found = items
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("item"))
        .find(|item| {
            item.get("type").and_then(Value::as_str) == Some("userMessage")
                && item.get("clientId").and_then(Value::as_str) == Some(client_id)
        })
        .map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>()
        });
    Ok(found)
}

/// Keeps the link's state current and turns thread items into receipts, until the
/// session exits.
pub(crate) async fn run(session: Arc<Session>, link: Arc<Link>, store: Store, boot: String) {
    let mut rpc: Option<Rpc> = None;
    let mut interval = tokio::time::interval(POLL);
    loop {
        interval.tick().await;
        if session.exit_code().is_some() {
            link.kill();
            return;
        }
        let step = async {
            if rpc.is_none() {
                rpc = Some(Rpc::connect(&link.socket).await?);
            }
            let Some(connection) = rpc.as_mut() else {
                return Ok(());
            };
            let observed = observe(connection).await?;
            let awaiting = {
                let state = link.lock();
                state
                    .awaiting
                    .as_ref()
                    .map(|a| (a.id.clone(), a.envelope.clone(), a.since))
            };
            let mut satisfied = false;
            if let (Some((id, envelope, since)), Some(thread)) = (&awaiting, &observed.thread) {
                match find_message(connection, thread, id).await? {
                    Some(text) => {
                        if text == *envelope {
                            crate::daemon::record_receipt(&store, &boot, &session, id).await?;
                        }
                        satisfied = true;
                    }
                    None => {
                        satisfied = since.elapsed() >= RECEIPT_GIVE_UP && observed.reason.is_none();
                    }
                }
            }
            let mut state = link.lock();
            if satisfied {
                state.awaiting = None;
            }
            let reason = observed
                .reason
                .or_else(|| state.awaiting.is_some().then_some("in_flight"));
            let changed = state.reason != reason || state.thread != observed.thread;
            state.reason = reason;
            state.thread = observed.thread;
            if changed || satisfied {
                session.message_notify().notify_one();
            }
            Ok::<(), anyhow::Error>(())
        }
        .await;
        match step {
            Ok(()) => {}
            Err(error) => {
                tracing::debug!(%error, "Codex app-server poll failed");
                rpc = None;
                let changed =
                    link.lock().reason.replace("app_server_down") != Some("app_server_down");
                if changed {
                    session.message_notify().notify_one();
                }
            }
        }
    }
}
