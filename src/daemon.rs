//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::client::Framed;
use crate::emulator::Size;
use crate::harness::{self, Deliver};
use crate::messaging::{self, COOLDOWN, Limits, MessageState, code};
use crate::session::{AttachmentSlot, Hold, Session, SessionId, SessionSpec};
use crate::store::{self, CancelResult, InsertError, Message, NewMessage, Store};
use crate::wire::{
    AgentSummary, ClientFrame, MessageInfo, Request, Response, ServerFrame, SessionSummary,
};

pub struct DaemonConfig {
    /// State directory (`A2AMX_HOME` or `--home`).
    pub state_dir: PathBuf,
    /// Listen addresses; loopback by default. Port 0 lets the OS choose.
    pub listen: Vec<SocketAddr>,
    pub host_name: Option<String>,
    pub limits: Limits,
}

pub struct Daemon {
    addrs: Vec<SocketAddr>,
    runtime: Arc<Runtime>,
    shutdown: watch::Sender<bool>,
    listeners: Vec<JoinHandle<()>>,
    purge: Option<JoinHandle<()>>,
    state_dir: PathBuf,
    lock: Option<File>,
}

struct Runtime {
    sessions: Mutex<BTreeMap<u64, Arc<Session>>>,
    next_id: AtomicU64,
    token: String,
    store: Store,
    host_name: String,
    boot: String,
    limits: Limits,
    address: String,
    rate: tokio::sync::Mutex<HashMap<String, VecDeque<Instant>>>,
    new_session_gate: tokio::sync::Mutex<()>,
    deliveries: Mutex<Vec<JoinHandle<()>>>,
}

enum Role {
    Admin,
    Session(String),
}

fn constant_time_eq(actual: &str, expected: &str) -> bool {
    let mut difference = actual.len() ^ expected.len();
    for (index, byte) in expected.bytes().enumerate() {
        difference |= usize::from(byte ^ actual.as_bytes().get(index).copied().unwrap_or(0));
    }
    difference == 0
}

fn parse_message_id(id: &str) -> Option<i64> {
    let digits = id.strip_prefix("m_")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok().filter(|seq| *seq > 0)
}

fn failed(code: &str, message: &str) -> Response {
    Response::Failed {
        code: code.into(),
        message: message.into(),
    }
}

impl Runtime {
    fn authenticate(&self, token: &str) -> Option<Role> {
        let mut role = constant_time_eq(token, &self.token).then_some(Role::Admin);
        for session in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
        {
            if session.exit_code().is_none() {
                let matches = constant_time_eq(token, session.token());
                if matches && role.is_none() {
                    role = Some(Role::Session(session.id().0.clone()));
                }
            }
        }
        role
    }

    async fn create_session(&self, request: Request) -> anyhow::Result<Response> {
        let Request::NewSession {
            argv,
            cols,
            rows,
            cwd,
            mut env,
            name,
            harness,
            deliver,
        } = request
        else {
            anyhow::bail!("expected new_session");
        };
        if argv.is_empty() || cols == 0 || rows == 0 {
            return Ok(Response::Error {
                message: "command and nonzero dimensions are required".into(),
            });
        }
        let _gate = self.new_session_gate.lock().await;
        if let Some(name) = &name {
            if let Err(error) = messaging::validate_name(name) {
                return Ok(Response::Error {
                    message: error.to_string(),
                });
            }
            if self
                .sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .any(|session| session.name() == Some(name.as_str()))
            {
                return Ok(Response::Error {
                    message: format!("session name {name} is already in use"),
                });
            }
        }
        let number = self
            .next_id
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| anyhow::anyhow!("session id space exhausted"))?;
        let id = SessionId(format!("s{number}"));
        let address = self.address.clone();
        let session = tokio::task::spawn_blocking(move || {
            let token = random_hex::<32>()?;
            env.push(("A2AMX_TOKEN".into(), token.clone()));
            env.push(("A2AMX_ADDR".into(), address));
            Session::spawn(
                id,
                SessionSpec {
                    argv,
                    cwd: cwd.map(PathBuf::from),
                    size: Size { cols, rows },
                    env,
                    name,
                    harness,
                    deliver: deliver.unwrap_or_else(|| harness.default_deliver()),
                    token,
                },
            )
        })
        .await?;
        match session {
            Ok(session) => {
                let session = Arc::new(session);
                let id = session.id().0.clone();
                self.sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(number, session.clone());
                let store = self.store.clone();
                let boot = self.boot.clone();
                let task = tokio::spawn(crate::delivery::run(session, store, boot));
                self.deliveries
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(task);
                Ok(Response::Created { session: id })
            }
            Err(error) => Ok(Response::Error {
                message: error.to_string(),
            }),
        }
    }

    async fn send_message(
        &self,
        sender_id: &str,
        to: String,
        subject: String,
        body: String,
    ) -> Response {
        if let Err(error) = messaging::validate_message(&subject, &body) {
            return failed(error.code, &error.message);
        }
        let recipient = messaging::local_part(&to, &self.host_name).and_then(|local| {
            self.sessions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .find(|session| session.name().unwrap_or(&session.id().0) == local)
                .cloned()
        });
        let Some(recipient) = recipient else {
            return failed(code::UNKNOWN_RECIPIENT, "unknown recipient");
        };
        if recipient.id().0 == sender_id {
            return failed(code::UNKNOWN_RECIPIENT, "a session cannot message itself");
        }
        if recipient.exit_code().is_some() {
            return failed(code::RECIPIENT_EXITED, "recipient has exited");
        }
        let Some((_, sender)) = lookup(self, sender_id) else {
            return failed(code::INTERNAL, "sender session is unavailable");
        };
        let mut rate = self.rate.lock().await;
        let timestamps = rate.entry(sender_id.into()).or_default();
        while timestamps
            .front()
            .is_some_and(|time| time.elapsed() > Duration::from_secs(60))
        {
            timestamps.pop_front();
        }
        if timestamps.len() >= self.limits.rate_per_minute as usize {
            return failed(code::RATE_LIMITED, "sender rate limit reached");
        }
        let new = NewMessage {
            boot: self.boot.clone(),
            sender_session: sender_id.into(),
            sender_address: messaging::address(sender.name(), sender_id, &self.host_name),
            recipient_session: recipient.id().0.clone(),
            recipient_address: messaging::address(
                recipient.name(),
                &recipient.id().0,
                &self.host_name,
            ),
            subject,
            body,
        };
        match self.store.insert_message(new).await {
            Ok(seq) => {
                timestamps.push_back(Instant::now());
                recipient.message_notify().notify_one();
                Response::Accepted {
                    id: format!("m_{seq}"),
                }
            }
            Err(InsertError::QueueFull) => failed(code::QUEUE_FULL, "message queue is full"),
            Err(InsertError::Internal(error)) => {
                tracing::error!(%error, "message acceptance failed");
                failed(code::INTERNAL, "message storage failed")
            }
        }
    }

    async fn report_prompt(&self, session_id: &str, prompt: String) -> anyhow::Result<Response> {
        let (_, session) =
            lookup(self, session_id).ok_or_else(|| anyhow::anyhow!("session is unavailable"))?;
        let text = messaging::unwrap_pastes(&prompt);
        let mut known = Vec::new();
        // shortcut: a session-token holder can forge prompts for that session;
        // stronger attestation requires a different same-user trust contract.
        for seq in messaging::envelope_ids(&text) {
            if let Some(message) = self.store.get(seq).await? {
                if message.boot == self.boot
                    && message.recipient_session == session_id
                    && matches!(
                        message.state,
                        MessageState::Delivering
                            | MessageState::Submitted
                            | MessageState::Unsubmitted
                    )
                    && !message.observed
                {
                    let envelope = messaging::render_envelope(
                        &format!("m_{seq}"),
                        &message.sender_address,
                        &message.subject,
                        &message.body,
                    );
                    known.push((seq, envelope));
                }
            }
        }
        if known.is_empty() {
            session.note_submit(false);
        } else if known.len() == 1 && text == known[0].1 {
            self.store.record_receipt(known[0].0).await?;
            session.note_submit(true);
        } else {
            let mut remainder = text;
            let mut complete = true;
            for (_, envelope) in &known {
                if let Some(start) = remainder.find(envelope) {
                    remainder.replace_range(start..start + envelope.len(), "");
                } else {
                    complete = false;
                    break;
                }
            }
            // shortcut: interleaved envelopes cannot be separated safely; their
            // human text stays in the transcript until a harness can rewrite it.
            let draft = complete.then(|| messaging::sanitize_draft(remainder.trim()));
            session.note_corrupted(draft);
            for (seq, _) in known {
                self.store.reject_attempt(seq).await?;
            }
            session.message_notify().notify_one();
            return Ok(Response::PromptVerdict {
                verdict: "block".into(),
                reason: Some(messaging::CORRUPTED_SUBMISSION_REASON.into()),
            });
        }
        Ok(Response::PromptVerdict {
            verdict: "allow".into(),
            reason: None,
        })
    }

    async fn message_status(&self, role: &Role, id: &str) -> anyhow::Result<Response> {
        let Some(seq) = parse_message_id(id) else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        let Some(message) = self.store.get(seq).await? else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        if let Role::Session(sender) = role {
            if message.boot != self.boot || message.sender_session != *sender {
                return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
            }
        }
        Ok(Response::Status {
            message: self.message_info(message).await?,
        })
    }

    async fn message_info(&self, message: Message) -> anyhow::Result<MessageInfo> {
        let recipient = if message.boot == self.boot && message.state == MessageState::Pending {
            lookup(self, &message.recipient_session).map(|(_, session)| session)
        } else {
            None
        };
        let hold_reason = if let Some(session) = recipient {
            let queued = self
                .store
                .has_earlier_open(&self.boot, &message.recipient_session, message.seq)
                .await?;
            let state = session.lock();
            if state.exit_code.is_some() {
                None
            } else if session.deliver() == Deliver::Hold {
                Some("deliver_hold")
            } else if state.hold == Some(Hold::UnsubmittedEnvelope) {
                Some("unsubmitted_envelope")
            } else if state.hold == Some(Hold::CorruptedSubmissions) {
                Some("corrupted_submissions")
            } else if state.hold == Some(Hold::HumanDraft) {
                Some("human_draft")
            } else if queued {
                Some("queued")
            } else if !harness::ready(
                session.harness(),
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            ) {
                Some("not_ready")
            } else if state
                .last_submit
                .is_some_and(|time| time.elapsed() < COOLDOWN)
            {
                Some("cooldown")
            } else {
                None
            }
        } else {
            None
        };
        Ok(MessageInfo {
            id: format!("m_{}", message.seq),
            from: message.sender_address,
            to: message.recipient_address,
            subject: message.subject,
            state: message.state.as_str().into(),
            detail: message.detail,
            hold_reason: hold_reason.map(str::to_owned),
            evidence: if message.state == MessageState::Submitted {
                Some(
                    if message.observed {
                        "submission_observed"
                    } else {
                        "write_complete"
                    }
                    .into(),
                )
            } else {
                None
            },
        })
    }
}

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// state dir, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        let path = config.state_dir.clone();
        let lock = tokio::task::spawn_blocking(move || -> anyhow::Result<File> {
            let mut missing = Vec::new();
            let mut ancestor = path.as_path();
            loop {
                match std::fs::metadata(ancestor) {
                    Ok(metadata) if metadata.is_dir() => break,
                    Ok(_) => return Err(anyhow::anyhow!("state path is not a directory")),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
                missing.push(ancestor.to_owned());
                ancestor = ancestor
                    .parent()
                    .filter(|path| !path.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."));
            }
            for directory in missing.iter().rev() {
                match std::fs::DirBuilder::new().mode(0o700).create(directory) {
                    Ok(()) => {}
                    Err(error)
                        if error.kind() == std::io::ErrorKind::AlreadyExists
                            && directory.is_dir() => {}
                    Err(error) => return Err(error.into()),
                }
                // A restrictive umask must not prevent creation of the next child.
                std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            }
            if missing.is_empty() {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
            }
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path.join("daemon.lock"))?;
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| {
                    anyhow::anyhow!("another a2amx daemon is running for this state dir")
                })?;
            Ok(lock)
        })
        .await??;
        let host_name = config.host_name;
        let (token, host_name, boot) = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let token = random_hex::<32>()?;
            let host = match host_name {
                Some(host) => {
                    messaging::validate_host(&host).context("invalid host name")?;
                    host
                }
                None => messaging::default_host_name(),
            };
            Ok((token, host, random_hex::<8>()?))
        })
        .await??;
        let store = Store::open(config.state_dir.clone(), config.limits).await?;
        // Written only after the steps that can fail without binding, so an early
        // startup error never leaves a token for a daemon that is not running.
        let token_path = config.state_dir.join("admin.token");
        let token_text = token.clone();
        tokio::task::spawn_blocking(move || write_state_file(&token_path, token_text.as_bytes()))
            .await??;
        let mut bound = Vec::new();
        let mut addrs = Vec::new();
        let listen = if config.listen.is_empty() {
            vec![SocketAddr::from(([127, 0, 0, 1], 0))]
        } else {
            config.listen
        };
        for address in listen {
            let listener = match TcpListener::bind(address).await {
                Ok(listener) => listener,
                Err(error) => {
                    let path = config.state_dir.join("admin.token");
                    tokio::task::spawn_blocking(move || std::fs::remove_file(path)).await??;
                    return Err(error.into());
                }
            };
            addrs.push(listener.local_addr()?);
            bound.push(listener);
        }
        let path = config.state_dir.clone();
        let addresses = addrs
            .iter()
            .map(|addr| format!("{addr}\n"))
            .collect::<String>();
        tokio::task::spawn_blocking(move || {
            write_state_file(&path.join("addr"), addresses.as_bytes())
        })
        .await??;
        // shortcut: sessions live only in this daemon; add persistence only with
        // a later lifecycle specification.
        let (shutdown, _) = watch::channel(false);
        let runtime = Arc::new(Runtime {
            sessions: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            token,
            store,
            host_name,
            boot,
            limits: config.limits,
            address: addrs[0].to_string(),
            // shortcut: accepted-send timestamps are memory-only; persist them if
            // rate limits must survive daemon restarts.
            rate: tokio::sync::Mutex::new(HashMap::new()),
            new_session_gate: tokio::sync::Mutex::new(()),
            deliveries: Mutex::new(Vec::new()),
        });
        let purge = {
            let store = runtime.store.clone();
            let mut stopping = shutdown.subscribe();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(3600));
                interval.tick().await;
                loop {
                    tokio::select! {
                        _ = stopping.changed() => break,
                        _ = interval.tick() => {
                            if let Err(error) = store.purge(store::now()).await {
                                tracing::error!(%error, "message purge failed");
                            }
                        }
                    }
                }
            })
        };
        let listeners = bound.into_iter().map(|listener| {
            let runtime = runtime.clone();
            let mut stopping = shutdown.subscribe();
            tokio::spawn(async move {
                let mut connections = JoinSet::new();
                loop {
                    tokio::select! {
                        _ = stopping.changed() => break,
                        accepted = listener.accept() => {
                            match accepted {
                                Ok((socket, _)) => {
                                    let runtime = runtime.clone();
                                    let mut stopping = stopping.clone();
                                    connections.spawn(async move {
                                        tokio::select! {
                                            _ = stopping.changed() => {},
                                            result = serve(socket, runtime) => {
                                                if let Err(error) = result {
                                                    tracing::debug!(%error, "client connection closed");
                                                }
                                            }
                                        }
                                    });
                                }
                                Err(error) => tracing::warn!(%error, "accept failed"),
                            }
                        }
                        Some(result) = connections.join_next(), if !connections.is_empty() => {
                            if let Err(error) = result {
                                tracing::warn!(%error, "connection task failed");
                            }
                        }
                    }
                }
                connections.abort_all();
                while connections.join_next().await.is_some() {}
            })
        }).collect();
        Ok(Self {
            addrs,
            runtime,
            shutdown,
            listeners,
            purge: Some(purge),
            state_dir: config.state_dir,
            lock: Some(lock),
        })
    }

    /// Addresses actually bound (resolves port 0).
    pub fn addrs(&self) -> &[SocketAddr] {
        &self.addrs
    }

    pub fn host_name(&self) -> &str {
        &self.runtime.host_name
    }

    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        let _ = self.shutdown.send(true);
        for listener in self.listeners.drain(..) {
            listener.await?;
        }
        if let Some(purge) = self.purge.take() {
            purge.await?;
        }
        let mut failure = None;
        let deliveries = std::mem::take(
            &mut *self
                .runtime
                .deliveries
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
        );
        // Stop injection before killing children so graceful shutdown, rather
        // than a racing recipient-exit observer, owns the remaining open rows.
        for delivery in &deliveries {
            delivery.abort();
        }
        for delivery in deliveries {
            if let Err(error) = delivery.await
                && !error.is_cancelled()
            {
                failure.get_or_insert(anyhow::Error::new(error).context("message delivery task"));
            }
        }
        let sessions = self
            .runtime
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut kills = JoinSet::new();
        for session in sessions {
            kills.spawn(async move { session.kill().await });
        }
        while let Some(result) = kills.join_next().await {
            if let Err(error) = result
                .context("session shutdown task")
                .and_then(|value| value)
            {
                failure.get_or_insert(error);
            }
        }
        self.runtime
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        if let Err(error) = self
            .runtime
            .store
            .fail_open(&self.runtime.boot, None, "daemon_stopped")
            .await
        {
            failure.get_or_insert(error);
        }
        if let Err(error) = self.runtime.store.close().await {
            failure.get_or_insert(error);
        }
        let addr = self.state_dir.join("addr");
        let lock = self.lock.take();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            match std::fs::remove_file(addr) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            drop(lock);
            Ok(())
        })
        .await??;
        match failure {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for delivery in self
            .runtime
            .deliveries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
        {
            delivery.abort();
        }
    }
}

fn random_hex<const N: usize>() -> anyhow::Result<String> {
    let mut random = [0; N];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let mut text = String::with_capacity(N * 2);
    use std::fmt::Write as _;
    for byte in random {
        write!(&mut text, "{byte:02x}")?;
    }
    Ok(text)
}

fn write_state_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    Ok(())
}

async fn response(connection: &mut Framed, response: Response) -> anyhow::Result<()> {
    connection.send(&serde_json::to_vec(&response)?).await
}

async fn serve(socket: TcpStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let mut connection = Framed::new(socket);
    let hello = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let role = match hello {
        Ok(Ok(Some(bytes))) => match serde_json::from_slice::<Request>(&bytes) {
            Ok(Request::Hello { token }) => runtime.authenticate(&token),
            _ => None,
        },
        _ => None,
    };
    let Some(role) = role else {
        response(
            &mut connection,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            },
        )
        .await?;
        connection.stream.shutdown().await?;
        return Ok(());
    };
    response(&mut connection, Response::Ok).await?;
    while let Some(bytes) = connection.recv().await? {
        if let Role::Session(id) = &role {
            if !lookup(&runtime, id).is_some_and(|(_, session)| session.exit_code().is_none()) {
                response(
                    &mut connection,
                    Response::Error {
                        message: "session has exited".into(),
                    },
                )
                .await?;
                connection.stream.shutdown().await?;
                return Ok(());
            }
        }
        let request = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                response(
                    &mut connection,
                    Response::Error {
                        message: "invalid control request".into(),
                    },
                )
                .await?;
                continue;
            }
        };
        if matches!(&role, Role::Session(_))
            && !matches!(
                &request,
                Request::SendMessage { .. }
                    | Request::ListAgents
                    | Request::MessageStatus { .. }
                    | Request::ReportPrompt { .. },
            )
        {
            response(
                &mut connection,
                Response::Error {
                    message: "not permitted for a session token".into(),
                },
            )
            .await?;
            continue;
        }
        let result = match request {
            Request::Hello { .. } => Response::Error {
                message: "already authenticated".into(),
            },
            Request::List => {
                let counts = runtime.store.open_counts(&runtime.boot).await?;
                let sessions = runtime
                    .sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .map(|session| {
                        let state = session.lock();
                        SessionSummary {
                            id: session.id().0.clone(),
                            argv: session.argv.clone(),
                            cols: state.size.cols,
                            rows: state.size.rows,
                            attached: state.attachment.is_some(),
                            exit_code: state.exit_code,
                            name: session.name().map(str::to_owned),
                            address: messaging::address(
                                session.name(),
                                &session.id().0,
                                &runtime.host_name,
                            ),
                            pending: counts.get(&session.id().0).copied().unwrap_or(0),
                            held: state.hold.is_some(),
                        }
                    })
                    .collect();
                Response::Sessions { sessions }
            }
            request @ Request::NewSession { .. } => runtime.create_session(request).await?,
            Request::Kill { session: id } => match lookup(&runtime, &id) {
                Some((number, session)) => match session.kill().await {
                    Ok(()) => {
                        runtime
                            .sessions
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .remove(&number);
                        Response::Ok
                    }
                    Err(error) => Response::Error {
                        message: error.to_string(),
                    },
                },
                None => Response::Error {
                    message: format!("unknown session {id}"),
                },
            },
            Request::Attach {
                session: id,
                force,
                cols,
                rows,
            } => {
                if cols == 0 || rows == 0 {
                    Response::Error {
                        message: "attachment dimensions must be nonzero".into(),
                    }
                } else if let Some((_, session)) = lookup(&runtime, &id) {
                    if attach(&mut connection, session, force, Size { cols, rows }).await? {
                        return Ok(());
                    }
                    continue;
                } else {
                    Response::Error {
                        message: format!("unknown session {id}"),
                    }
                }
            }
            Request::ListAgents => {
                let agents = runtime
                    .sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .map(|session| AgentSummary {
                        address: messaging::address(
                            session.name(),
                            &session.id().0,
                            &runtime.host_name,
                        ),
                        state: if session.exit_code().is_some() {
                            "exited"
                        } else {
                            "running"
                        }
                        .into(),
                        attached: session.attached(),
                    })
                    .collect();
                Response::Agents { agents }
            }
            Request::SendMessage {
                to,
                subject,
                message,
            } => match &role {
                Role::Admin => Response::Error {
                    message: "send_message needs a session token".into(),
                },
                Role::Session(id) => runtime.send_message(id, to, subject, message).await,
            },
            Request::ReportPrompt { prompt } => match &role {
                Role::Admin => Response::Error {
                    message: "report_prompt needs a session token".into(),
                },
                Role::Session(id) => runtime.report_prompt(id, prompt).await?,
            },
            Request::MessageStatus { id } => runtime.message_status(&role, &id).await?,
            Request::ListMessages { session, state } => {
                let filter = state.as_deref().map(MessageState::parse);
                if matches!(filter, Some(None)) {
                    Response::Error {
                        message: "unknown message state".into(),
                    }
                } else {
                    let rows = runtime
                        .store
                        .list(&runtime.boot, session.as_deref(), filter.flatten())
                        .await?;
                    let mut messages = Vec::with_capacity(rows.len());
                    for row in rows {
                        messages.push(runtime.message_info(row).await?);
                    }
                    Response::Messages { messages }
                }
            }
            Request::CancelMessage { id } => match parse_message_id(&id) {
                Some(seq) => match runtime.store.cancel(seq).await? {
                    CancelResult::Cancelled => Response::Ok,
                    CancelResult::NotCancellable(state) => Response::Error {
                        message: format!(
                            "message {id} is {} and cannot be cancelled",
                            state.as_str()
                        ),
                    },
                    CancelResult::Unknown => Response::Error {
                        message: format!("unknown message {id}"),
                    },
                },
                None => Response::Error {
                    message: format!("unknown message {id}"),
                },
            },
        };
        response(&mut connection, result).await?;
    }
    Ok(())
}

fn lookup(runtime: &Runtime, id: &str) -> Option<(u64, Arc<Session>)> {
    let number = id.strip_prefix('s')?.parse::<u64>().ok()?;
    runtime
        .sessions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&number)
        .filter(|session| session.id().0 == id)
        .map(|session| (number, session.clone()))
}

struct AttachmentGuard {
    session: Arc<Session>,
    generation: u64,
    closed: watch::Sender<bool>,
}

impl Drop for AttachmentGuard {
    fn drop(&mut self) {
        let mut state = self.session.lock();
        if state
            .attachment
            .as_ref()
            .is_some_and(|slot| slot.generation == self.generation)
        {
            state.attachment = None;
        }
        if state.pending_attachment == Some(self.generation) {
            state.pending_attachment = None;
        }
        let _ = self.closed.send(true);
    }
}

async fn attach(
    connection: &mut Framed,
    session: Arc<Session>,
    force: bool,
    size: Size,
) -> anyhow::Result<bool> {
    let (takeover, mut taken_over) = watch::channel(false);
    let (closed, completion) = watch::channel(false);
    let reservation = {
        let mut state = session.lock();
        if (state.attachment.is_some() || state.pending_attachment.is_some()) && !force {
            None
        } else {
            let previous = state
                .attachment
                .as_ref()
                .map(|slot| (slot.takeover.clone(), slot.closed.clone()));
            state.generation += 1;
            let generation = state.generation;
            state.pending_attachment = Some(generation);
            Some((generation, previous))
        }
    };
    let Some((generation, previous)) = reservation else {
        response(
            connection,
            Response::Error {
                message: format!(
                    "session {} is attached elsewhere (use --force to take over)",
                    session.id().0
                ),
            },
        )
        .await?;
        return Ok(false);
    };
    let _guard = AttachmentGuard {
        session: session.clone(),
        generation,
        closed,
    };
    if let Some((previous, mut previous_closed)) = previous {
        let _ = previous.send(true);
        while !*previous_closed.borrow() {
            if previous_closed.changed().await.is_err() {
                break;
            }
        }
    }
    let snapshot = {
        let mut state = session.lock();
        if state.pending_attachment != Some(generation) {
            Err(anyhow::anyhow!("attachment was taken over"))
        } else {
            session.resize_locked(&mut state, size).map(|()| {
                let bytes = state.emulator.render_full();
                state.dirty = false;
                state.attachment = Some(AttachmentSlot {
                    generation,
                    takeover,
                    closed: completion,
                });
                state.pending_attachment = None;
                (bytes, state.exit_code)
            })
        }
    };
    let (bytes, exited) = match snapshot {
        Ok(snapshot) => snapshot,
        Err(error) => {
            response(
                connection,
                Response::Error {
                    message: error.to_string(),
                },
            )
            .await?;
            return Ok(false);
        }
    };
    response(connection, Response::Attached).await?;
    send_data(connection, &bytes).await?;
    if let Some(code) = exited {
        connection.send(&ServerFrame::Exit(code).encode()).await?;
        connection.stream.shutdown().await?;
        return Ok(true);
    }
    loop {
        if *taken_over.borrow() {
            connection
                .send(&ServerFrame::Detached("taken over by another attachment".into()).encode())
                .await?;
            connection.stream.shutdown().await?;
            return Ok(true);
        }
        tokio::select! {
            _ = taken_over.changed() => {},
            _ = session.notify().notified() => {
                let (bytes, exited) = {
                    let mut state = session.lock();
                    let bytes = state.emulator.render_update();
                    if !state.emulator.is_scrolled() { state.dirty = false; }
                    let exited = state.exit_code.filter(|_| !state.dirty);
                    (bytes, exited)
                };
                send_data(connection, &bytes).await?;
                if let Some(code) = exited {
                    connection.send(&ServerFrame::Exit(code).encode()).await?;
                    connection.stream.shutdown().await?;
                    return Ok(true);
                }
            },
            payload = connection.recv() => {
                let Some(payload) = payload? else { return Ok(true); };
                match ClientFrame::decode(&payload)? {
                    ClientFrame::Detach => return Ok(true),
                    ClientFrame::Input(bytes) => {
                        session.note_human_input(&bytes);
                        if !session.enqueue(bytes).await? {
                            session.notify().notify_one();
                        }
                    },
                    ClientFrame::Release => session.release(),
                    frame => {
                        let (bytes, exited) = {
                            let mut state = session.lock();
                            match frame {
                                ClientFrame::Resize { cols, rows } if state.exit_code.is_none() =>
                                    session.resize_locked(&mut state, Size { cols, rows })?,
                                ClientFrame::Resize { .. } => {},
                                ClientFrame::Scroll(scroll) => state.emulator.scroll(scroll),
                                ClientFrame::Redraw => {},
                                _ => unreachable!(),
                            }
                            state.dirty = false;
                            (state.emulator.render_full(), state.exit_code)
                        };
                        send_data(connection, &bytes).await?;
                        if let Some(code) = exited {
                            connection.send(&ServerFrame::Exit(code).encode()).await?;
                            connection.stream.shutdown().await?;
                            return Ok(true);
                        }
                    },
                }
            },
        }
    }
}

async fn send_data(connection: &mut Framed, bytes: &[u8]) -> anyhow::Result<()> {
    for chunk in bytes.chunks(64 * 1024) {
        connection.send_tagged(0x01, chunk).await?;
    }
    Ok(())
}
