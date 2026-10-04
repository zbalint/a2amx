//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::client::Framed;
use crate::codex;
use crate::delivery::{AnyChannel, Channel};
use crate::emulator::Size;
use crate::harness::{self, Deliver, Harness};
use crate::messaging::{self, Limits, MessageState, code};
use crate::quota;
use crate::session::{AttachmentSlot, NativeGuard, Session, SessionId, SessionSpec};
use crate::store::{self, CancelResult, InsertError, Message, NewMessage, RejectOutcome, Store};
use crate::wire::{
    Activity, AgentSummary, BRIDGE_PROTOCOL, BridgeDown, BridgeUp, ClientFrame, MessageInfo,
    QuotaInfo, Request, Response, ServerFrame, SessionSummary, StatusInfo,
};

const HEARTBEAT_POLL: Duration = Duration::from_secs(1);
const ACTIVITY_QUIET: Duration = Duration::from_secs(10);
// shortcut: output-free tool calls over the quiet window read as idle; use a per-session or
// harness-specific working marker if that misleads in practice.
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
    state_dir: PathBuf,
    exe: PathBuf,
    rate: tokio::sync::Mutex<HashMap<String, VecDeque<Instant>>>,
    new_session_gate: tokio::sync::Mutex<()>,
    deliveries: Mutex<Vec<JoinHandle<()>>>,
    stop_requested: tokio::sync::Notify,
    stop_now: AtomicBool,
}

struct HeartbeatPeer {
    name: String,
    address: String,
    busy_for: Option<Duration>,
    unchanged_for: Option<Duration>,
    hold: Option<String>,
    last_message_age: Option<Duration>,
    queued: u32,
}

fn heartbeat_body(interval: Duration, peers: &[HeartbeatPeer]) -> String {
    let mut body = format!(
        "Heartbeat: you have been idle for {}. Watched peers:\n",
        display_duration(interval)
    );
    for peer in peers {
        body.push_str(&format!("- {} ({}): ", peer.name, peer.address));
        if let Some(busy_for) = peer.busy_for {
            body.push_str(&format!("busy for {}", display_duration(busy_for)));
        } else {
            body.push_str("ready");
        }
        if let Some(unchanged_for) = peer.unchanged_for.filter(|age| age.as_secs() >= 60) {
            body.push_str(&format!(
                ", screen unchanged for {}",
                display_duration(unchanged_for)
            ));
        }
        if let Some(hold) = peer.hold.as_deref() {
            body.push_str(&format!(", held: {hold}"));
        }
        if let Some(age) = peer.last_message_age {
            body.push_str(&format!(", last message {} ago", display_duration(age)));
        } else {
            body.push_str(", last message none");
        }
        body.push_str(&format!(", {} queued\n", peer.queued));
    }
    body.push_str("Run a2amx screen <peer> to look before messaging a busy peer.");
    body
}

fn display_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}s", seconds.max(1))
    }
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

/// Stores a native receipt when `id` is an open message of this daemon run addressed
/// to `session` and no receipt exists yet; reports whether one was stored.
pub(crate) async fn record_receipt(
    store: &Store,
    boot: &str,
    session: &Session,
    id: &str,
) -> anyhow::Result<bool> {
    let Some(seq) = parse_message_id(id) else {
        return Ok(false);
    };
    let Some(message) = store.get(seq).await? else {
        return Ok(false);
    };
    if message.boot != boot
        || message.recipient_session != session.id().0
        || !matches!(
            message.state,
            MessageState::Delivering | MessageState::Submitted | MessageState::Unsubmitted
        )
        || message.observed
    {
        return Ok(false);
    }
    store.record_receipt(seq).await?;
    Ok(true)
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

    async fn create_session(self: &Arc<Self>, request: Request) -> anyhow::Result<Response> {
        let Request::NewSession {
            argv,
            cols,
            rows,
            cwd,
            mut env,
            reset,
            control_from,
            watch,
            heartbeat,
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
        if let Err(error) = messaging::validate_reset_steps(&reset) {
            return Ok(Response::Error {
                message: error.to_string(),
            });
        }
        for controller in &control_from {
            if let Err(error) = messaging::validate_name(controller) {
                return Ok(Response::Error {
                    message: format!("control_from entry {controller:?}: {error}"),
                });
            }
        }
        for watcher in &watch {
            if let Err(error) = messaging::validate_name(watcher) {
                return Ok(Response::Error {
                    message: format!("watch entry {watcher:?}: {error}"),
                });
            }
            if name.as_deref() == Some(watcher.as_str()) {
                return Ok(Response::Error {
                    message: format!("session {watcher} watches itself"),
                });
            }
        }
        let heartbeat = match heartbeat
            .map(|seconds| messaging::parse_interval(&format!("{seconds}s")))
            .transpose()
        {
            Ok(heartbeat) => heartbeat,
            Err(error) => {
                return Ok(Response::Error {
                    message: error.to_string(),
                });
            }
        };
        if heartbeat.is_some() && watch.is_empty() {
            return Ok(Response::Error {
                message: format!(
                    "session {}: heartbeat needs a non-empty watch",
                    name.as_deref().unwrap_or("unnamed")
                ),
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
        let token = tokio::task::spawn_blocking(random_hex::<32>).await??;
        env.push(("A2AMX_TOKEN".into(), token.clone()));
        env.push(("A2AMX_ADDR".into(), address));
        let mut argv = argv;
        let mut codex = None;
        let channel = harness == Harness::Claude
            && env
                .iter()
                .any(|(key, value)| key == harness::CHANNEL_ENV && value == "1");
        env.retain(|(key, _)| key != harness::CHANNEL_ENV);
        if harness == Harness::Codex {
            // shortcut: the client signals --no-authorize-peers through the request
            // environment; give the wire request a field if more flags need this.
            let authorize = !env
                .iter()
                .any(|(key, value)| key == codex::NO_AUTHORIZE_ENV && value == "1");
            env.retain(|(key, _)| key != codex::NO_AUTHORIZE_ENV);
            let exe = self.exe.clone();
            if let Some(message) =
                tokio::task::spawn_blocking(move || codex::executable_error(&exe)).await?
            {
                return Ok(Response::Error { message });
            }
            let started = codex::start(
                &argv[0],
                &self.state_dir,
                &id.0,
                cwd.as_deref().map(Path::new),
                &env,
                codex::server_config(&self.exe, authorize),
            )
            .await;
            let link = match started {
                Ok(link) => link,
                Err(error) => {
                    return Ok(Response::Error {
                        message: format!("{error:#}"),
                    });
                }
            };
            argv = codex::wire_argv(argv, link.socket());
            codex = Some(link);
        }
        let session = tokio::task::spawn_blocking(move || {
            Session::spawn(
                id,
                SessionSpec {
                    argv,
                    cwd: cwd.map(PathBuf::from),
                    size: Size { cols, rows },
                    env,
                    name,
                    harness,
                    channel,
                    deliver: deliver.unwrap_or_else(|| harness.default_deliver()),
                    reset,
                    control_from,
                    watch,
                    heartbeat,
                    token,
                    codex,
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
                let mut tasks = vec![tokio::spawn(crate::delivery::run(
                    AnyChannel::for_session(session.clone()),
                    store.clone(),
                    boot.clone(),
                ))];
                tasks.push(tokio::spawn(self.clone().observe_exit(session.clone())));
                if let Some(interval) = session.heartbeat() {
                    tasks.push(tokio::spawn(
                        self.clone().heartbeat(session.clone(), interval),
                    ));
                }
                if channel {
                    tasks.push(tokio::spawn(accept_channel_dialog(session.clone())));
                }
                if let Some(link) = session.codex().cloned() {
                    tasks.push(tokio::spawn(codex::run(session, link, store, boot)));
                }
                self.deliveries
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .extend(tasks);
                Ok(Response::Created { session: id })
            }
            Err(error) => Ok(Response::Error {
                message: error.to_string(),
            }),
        }
    }

    async fn observe_exit(self: Arc<Self>, session: Arc<Session>) {
        session.wait_exit().await;
        if session.daemon_ended() {
            return;
        }
        let Some(name) = session.name() else {
            return;
        };
        let Some(code) = session.exit_code() else {
            return;
        };
        let watchers = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .filter(|watcher| {
                watcher.exit_code().is_none()
                    && watcher.watch().iter().any(|watched| watched == name)
            })
            .cloned()
            .collect::<Vec<_>>();
        let peer_address = messaging::address(session.name(), &session.id().0, &self.host_name);
        let sender_address = messaging::address(
            Some(messaging::SYSTEM_SENDER),
            messaging::SYSTEM_SENDER,
            &self.host_name,
        );
        let subject = format!("peer exited: {name} (code {code})");
        let body = format!(
            "{name} ({peer_address}) exited with code {code}.\nRun a2amx screen {name} to see its last screen."
        );
        for watcher in watchers {
            let message = NewMessage {
                boot: self.boot.clone(),
                sender_session: "daemon".into(),
                sender_address: sender_address.clone(),
                recipient_session: watcher.id().0.clone(),
                recipient_address: messaging::address(
                    watcher.name(),
                    &watcher.id().0,
                    &self.host_name,
                ),
                subject: subject.clone(),
                body: body.clone(),
            };
            match self.store.insert_message(message).await {
                Ok(_) => watcher.message_notify().notify_one(),
                Err(InsertError::QueueFull) => {
                    tracing::warn!(recipient = %watcher.id().0, "exit event message queue is full");
                }
                Err(InsertError::Internal(error)) => {
                    tracing::error!(%error, recipient = %watcher.id().0, "exit event message acceptance failed");
                }
            }
        }
    }
    async fn heartbeat(self: Arc<Self>, session: Arc<Session>, interval: Duration) {
        // shortcut: poll resolution is one second; peers busy before this task starts are bounded from the first sight, not an exact ready-since clock.
        let mut ticker = tokio::time::interval(HEARTBEAT_POLL);
        let mut busy_since = HashMap::<String, Instant>::new();
        loop {
            ticker.tick().await;
            if session.exit_code().is_some() {
                return;
            }
            let now = Instant::now();
            let watched = session.watch().to_vec();
            let live_peers = {
                let sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                watched
                    .iter()
                    .filter_map(|name| {
                        sessions
                            .values()
                            .find(|peer| {
                                peer.exit_code().is_none() && peer.name() == Some(name.as_str())
                            })
                            .cloned()
                    })
                    .collect::<Vec<_>>()
            };
            let mut observed = Vec::with_capacity(live_peers.len());
            for peer in live_peers {
                let activity = session_activity(&peer);
                let (idle, unchanged_for) = {
                    let state = peer.lock();
                    (
                        !activity.is_some_and(|value| value != Activity::Idle),
                        now.saturating_duration_since(state.last_change),
                    )
                };
                let hold = AnyChannel::for_session(peer.clone())
                    .hold_reason()
                    .map(str::to_owned);
                let id = peer.id().0.clone();
                let busy_for = if activity.is_some_and(|value| value != Activity::Idle) {
                    let since = *busy_since.entry(id).or_insert(now);
                    Some(now.saturating_duration_since(since))
                } else {
                    busy_since.remove(&id);
                    None
                };
                observed.push((peer, idle, busy_for, unchanged_for, hold));
            }
            let watcher_ready = {
                let state = session.lock();
                harness::ready(
                    session.harness(),
                    &state.emulator.screen(),
                    state.emulator.is_scrolled(),
                )
            };
            if !watcher_ready
                || session.resetting()
                || AnyChannel::for_session(session.clone())
                    .hold_reason()
                    .is_some()
                || now.saturating_duration_since(session.last_change()) < interval
            {
                continue;
            }
            let open = match self
                .store
                .open_system_heartbeat(&self.boot, &session.id().0)
                .await
            {
                Ok(open) => open,
                Err(error) => {
                    tracing::error!(%error, recipient = %session.id().0, "heartbeat state lookup failed");
                    continue;
                }
            };
            if open {
                continue;
            }
            let counts = match self.store.open_counts(&self.boot).await {
                Ok(counts) => counts,
                Err(error) => {
                    tracing::error!(%error, "heartbeat queue lookup failed");
                    continue;
                }
            };
            let mut digest_peers = Vec::with_capacity(observed.len());
            let mut has_busy_peer = false;
            for (peer, idle, busy_for, unchanged_for, hold) in observed {
                let queued = counts.get(&peer.id().0).copied().unwrap_or_default();
                if !idle || queued > 0 {
                    has_busy_peer = true;
                }
                let last_message_age =
                    match self.store.last_message_age(&self.boot, &peer.id().0).await {
                        Ok(age) => age.map(Duration::from_secs),
                        Err(error) => {
                            tracing::error!(
                                %error,
                                peer = %peer.id().0,
                                "heartbeat message age lookup failed"
                            );
                            None
                        }
                    };
                digest_peers.push(HeartbeatPeer {
                    name: peer.name().unwrap_or(&peer.id().0).to_owned(),
                    address: messaging::address(peer.name(), &peer.id().0, &self.host_name),
                    busy_for,
                    unchanged_for: Some(unchanged_for),
                    hold,
                    last_message_age,
                    queued,
                });
            }
            if !has_busy_peer || digest_peers.is_empty() {
                continue;
            }
            if session.exit_code().is_some() {
                return;
            }
            let body = heartbeat_body(interval, &digest_peers);
            let message = NewMessage {
                boot: self.boot.clone(),
                sender_session: "daemon".into(),
                sender_address: messaging::address(
                    Some(messaging::SYSTEM_SENDER),
                    messaging::SYSTEM_SENDER,
                    &self.host_name,
                ),
                recipient_session: session.id().0.clone(),
                recipient_address: messaging::address(
                    session.name(),
                    &session.id().0,
                    &self.host_name,
                ),
                subject: messaging::HEARTBEAT_SUBJECT.into(),
                body,
            };
            match self.store.insert_message(message).await {
                Ok(_) => session.message_notify().notify_one(),
                Err(InsertError::QueueFull) => {
                    tracing::warn!(
                        recipient = %session.id().0,
                        "heartbeat message queue is full"
                    );
                }
                Err(InsertError::Internal(error)) => {
                    tracing::error!(
                        %error,
                        recipient = %session.id().0,
                        "heartbeat message acceptance failed"
                    );
                }
            }
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
                    recipient_hold: if recipient.deliver() == Deliver::Hold {
                        Some("deliver_hold".into())
                    } else {
                        AnyChannel::for_session(recipient.clone())
                            .hold_reason()
                            .map(str::to_owned)
                    },
                    recipient_quota: session_quota(&recipient)
                        .and_then(|quota| quota.exhausted())
                        .map(|window| format!("{window} quota exhausted")),
                }
            }
            Err(InsertError::QueueFull) => failed(code::QUEUE_FULL, "message queue is full"),
            Err(InsertError::Internal(error)) => {
                tracing::error!(%error, "message acceptance failed");
                failed(code::INTERNAL, "message storage failed")
            }
        }
    }

    fn reset_target(&self, reference: &str) -> Option<Arc<Session>> {
        if let Some((_, session)) = lookup(self, reference) {
            return Some(session);
        }
        let local = messaging::local_part(reference, &self.host_name)?;
        if let Some((_, session)) = lookup(self, local) {
            return Some(session);
        }
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find(|session| session.name() == Some(local))
            .cloned()
    }

    fn reset_audit(&self, sender: &str, target: &str, outcome: &str, step: Option<(usize, usize)>) {
        tracing::info!(
            sender,
            target,
            outcome,
            step = ?step,
            "session reset attempt"
        );
    }

    async fn reset_session(&self, role: &Role, reference: String) -> Response {
        let sender = match role {
            Role::Admin => "admin".to_owned(),
            Role::Session(id) => lookup(self, id).map_or_else(
                || id.clone(),
                |(_, session)| messaging::address(session.name(), id, &self.host_name),
            ),
        };
        let Some(target) = self.reset_target(&reference) else {
            self.reset_audit(&sender, &reference, messaging::code::UNKNOWN_SESSION, None);
            return failed(messaging::code::UNKNOWN_SESSION, "unknown session");
        };
        let target_address = messaging::address(target.name(), &target.id().0, &self.host_name);
        let permitted = match role {
            Role::Admin => true,
            Role::Session(sender_id) => {
                sender_id != &target.id().0
                    && lookup(self, sender_id)
                        .and_then(|(_, session)| session.name().map(str::to_owned))
                        .is_some_and(|name| target.control_from().iter().any(|item| item == &name))
            }
        };
        if !permitted {
            self.reset_audit(
                &sender,
                &target_address,
                messaging::code::NOT_PERMITTED,
                None,
            );
            return failed(
                messaging::code::NOT_PERMITTED,
                "sender is not permitted to reset this session",
            );
        }
        if target.exit_code().is_some() {
            self.reset_audit(&sender, &target_address, messaging::code::EXITED, None);
            return failed(messaging::code::EXITED, "session has exited");
        }
        match target.reset_sequence().await {
            Ok(steps) => {
                self.reset_audit(&sender, &target_address, "ok", None);
                Response::Reset { steps }
            }
            Err(error) => {
                self.reset_audit(&sender, &target_address, error.code, error.step);
                failed(error.code, &error.message)
            }
        }
    }

    async fn report_prompt(&self, session_id: &str, prompt: String) -> anyhow::Result<Response> {
        let (_, session) =
            lookup(self, session_id).ok_or_else(|| anyhow::anyhow!("session is unavailable"))?;
        if session.channel() {
            if let Some(inner) = messaging::unwrap_channel(&prompt) {
                let ids = messaging::envelope_ids(&inner);
                if let [seq] = ids.as_slice() {
                    if let Some(message) = self.store.get(*seq).await? {
                        if message.boot == self.boot
                            && message.recipient_session == session_id
                            && matches!(
                                message.state,
                                MessageState::Delivering
                                    | MessageState::Submitted
                                    | MessageState::Unsubmitted
                            )
                            && !message.observed
                            && inner
                                == messaging::channel_view(&messaging::render_envelope(
                                    &format!("m_{seq}"),
                                    &message.sender_address,
                                    &message.subject,
                                    &message.body,
                                ))
                        {
                            self.store.record_receipt(*seq).await?;
                            session.native_received();
                        }
                    }
                }
                return Ok(Response::PromptVerdict {
                    verdict: "allow".into(),
                    reason: None,
                });
            }
        }
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
                    // The hook sees the envelope as the harness transformed it on paste.
                    let envelope = harness::paste_view(
                        session.harness(),
                        &messaging::render_envelope(
                            &format!("m_{seq}"),
                            &message.sender_address,
                            &message.subject,
                            &message.body,
                        ),
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
            // A leftover wrapper tag means unwrapping failed; pasting it back would put
            // harness markup in the human's composer.
            let draft = (complete
                && !remainder.contains("<pasted_content")
                && !remainder.contains("</pasted_content"))
            .then(|| messaging::sanitize_draft(remainder.trim()));
            session.note_corrupted(draft);
            let mut undeliverable = false;
            for (seq, _) in known {
                if self
                    .store
                    .reject_attempt(seq, messaging::MAX_MESSAGE_REJECTIONS)
                    .await?
                    == RejectOutcome::Undeliverable
                {
                    undeliverable = true;
                }
            }
            session.message_notify().notify_one();
            return Ok(Response::PromptVerdict {
                verdict: "block".into(),
                reason: Some(
                    if undeliverable {
                        messaging::UNMATCHABLE_SUBMISSION_REASON
                    } else {
                        messaging::CORRUPTED_SUBMISSION_REASON
                    }
                    .into(),
                ),
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
            let channel = AnyChannel::for_session(session.clone());
            if session.exit_code().is_some() {
                None
            } else if session.deliver() == Deliver::Hold {
                Some("deliver_hold")
            } else if let Some(reason) = channel.hold_reason() {
                Some(reason)
            } else if queued {
                Some("queued")
            } else {
                channel.unready_reason()
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
            hold_explanation: hold_reason
                .and_then(messaging::hold_explanation)
                .map(str::to_owned),
            accepted_at: Some(message.accepted_at),
            updated_at: Some(message.updated_at),
            evidence: if message.state == MessageState::Submitted {
                Some(
                    if message.observed {
                        // shortcut: a removed session reports submission_observed for a
                        // native receipt; persist the channel with the receipt if that matters.
                        if lookup(self, &message.recipient_session).is_some_and(|(_, session)| {
                            matches!(session.harness(), Harness::Omp | Harness::Codex)
                                || session.channel()
                        }) {
                            "native_receipt"
                        } else {
                            "submission_observed"
                        }
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

async fn accept_channel_dialog(session: Arc<Session>) {
    // shortcut: fixed 60 s window and one string match; re-probe the dialog text
    // when Claude Code changes it.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if session.exit_code().is_some() {
            return;
        }
        let visible = harness::channel_dialog_visible(&session.lock().emulator.screen());
        if visible {
            match tokio::task::spawn_blocking(move || session.write_input(b"\r")).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::debug!(%error, "channel dialog Enter failed"),
                Err(error) => tracing::debug!(%error, "channel dialog input task failed"),
            }
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// The running executable's path, without the suffix Linux appends once the
/// file has been replaced on disk.
pub fn current_exe() -> std::io::Result<PathBuf> {
    let mut exe = std::env::current_exe()?;
    if let Some(path) = exe.to_string_lossy().strip_suffix(" (deleted)") {
        exe = PathBuf::from(path);
    }
    Ok(exe)
}

/// Create the state directory (and missing parents) owner-only.
pub fn ensure_state_dir(path: &Path) -> anyhow::Result<()> {
    let mut missing = Vec::new();
    let mut ancestor = path;
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
                if error.kind() == std::io::ErrorKind::AlreadyExists && directory.is_dir() => {}
            Err(error) => return Err(error.into()),
        }
        // A restrictive umask must not prevent creation of the next child.
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    }
    if missing.is_empty() {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// state dir, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        let exe = tokio::task::spawn_blocking(current_exe).await??;
        let path = config.state_dir.clone();
        let lock = tokio::task::spawn_blocking(move || -> anyhow::Result<File> {
            ensure_state_dir(&path)?;
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
            state_dir: config.state_dir.clone(),
            exe,
            // shortcut: accepted-send timestamps are memory-only; persist them if
            // rate limits must survive daemon restarts.
            rate: tokio::sync::Mutex::new(HashMap::new()),
            new_session_gate: tokio::sync::Mutex::new(()),
            deliveries: Mutex::new(Vec::new()),
            stop_requested: tokio::sync::Notify::new(),
            stop_now: AtomicBool::new(false),
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

    /// Completes when an admin client asked the daemon to shut down.
    pub async fn stop_requested(&self) {
        self.runtime.stop_requested.notified().await;
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
        let graceful = !self.runtime.stop_now.load(Ordering::Relaxed);
        let mut kills = JoinSet::new();
        for session in sessions {
            kills.spawn(async move { session.kill(graceful).await });
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
                    | Request::Reset { .. }
                    | Request::ListAgents
                    | Request::MessageStatus { .. }
                    | Request::ReportPrompt { .. }
                    | Request::BridgeAttach,
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
                        let activity = session_activity(session);
                        let hold_reason = AnyChannel::for_session(session.clone()).hold_reason();
                        let quota = session_quota(session);
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
                            held: hold_reason.is_some(),
                            hold_reason: hold_reason.map(str::to_owned),
                            quota,
                            harness: session.harness(),
                            cwd: session
                                .cwd()
                                .map(|path| path.to_string_lossy().into_owned()),
                            activity,
                        }
                    })
                    .collect();
                Response::Sessions { sessions }
            }
            request @ Request::NewSession { .. } => runtime.create_session(request).await?,
            Request::Shutdown { now } => {
                // Answer first: shutdown closes every connection, this one included.
                runtime.stop_now.store(now, Ordering::Relaxed);
                let sent = response(&mut connection, Response::Ok).await;
                runtime.stop_requested.notify_one();
                sent?;
                continue;
            }
            Request::Kill { session: id, now } => match lookup(&runtime, &id) {
                Some((number, session)) => match session.kill(!now).await {
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
            Request::Reset { session } => runtime.reset_session(&role, session).await,
            Request::Screen { session: id } => match lookup(&runtime, &id) {
                Some((_, session)) => {
                    let screen = {
                        let state = session.lock();
                        state.emulator.screen()
                    };
                    Response::Screen {
                        lines: screen.text_lines(),
                    }
                }
                None => Response::Error {
                    message: format!("unknown session {id}"),
                },
            },
            Request::Attach {
                session: id,
                force,
                cols,
                rows,
                status,
            } => {
                if cols == 0 || rows == 0 {
                    Response::Error {
                        message: "attachment dimensions must be nonzero".into(),
                    }
                } else if let Some((_, session)) = lookup(&runtime, &id) {
                    if attach(
                        &mut connection,
                        session,
                        force,
                        Size { cols, rows },
                        status.then(|| runtime.clone()),
                    )
                    .await?
                    {
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
                    .map(|session| {
                        let activity = session_activity(session);
                        AgentSummary {
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
                            quota: session_quota(session),
                            harness: session.harness(),
                            cwd: session
                                .cwd()
                                .map(|path| path.to_string_lossy().into_owned()),
                            activity,
                        }
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
            Request::BridgeAttach => match &role {
                Role::Admin => Response::Error {
                    message: "bridge_attach needs a session token".into(),
                },
                Role::Session(id) => {
                    let Some((_, session)) = lookup(&runtime, id) else {
                        anyhow::bail!("bridge session disappeared");
                    };
                    if session.harness() != Harness::Omp && !session.channel() {
                        Response::Error {
                            message: "session is not an omp session".into(),
                        }
                    } else if let Some(guard) = session.native_attach() {
                        response(&mut connection, Response::Attached).await?;
                        bridge(&mut connection, &runtime, session, guard).await?;
                        return Ok(());
                    } else {
                        Response::Error {
                            message: "a bridge is already attached to this session".into(),
                        }
                    }
                }
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

async fn bridge(
    connection: &mut Framed,
    runtime: &Arc<Runtime>,
    session: Arc<Session>,
    mut guard: NativeGuard,
) -> anyhow::Result<()> {
    let first = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let Ok(Ok(Some(bytes))) = first else {
        return Ok(());
    };
    let Ok(BridgeUp::Hello {
        protocol,
        omp_version,
        missing,
    }) = serde_json::from_slice(&bytes)
    else {
        return Ok(());
    };
    let refusal = if protocol != BRIDGE_PROTOCOL {
        Some("protocol_mismatch".to_owned())
    } else if !missing.is_empty() {
        Some(format!("missing_apis: {}", missing.join(", ")))
    } else {
        None
    };
    if let Some(reason) = refusal {
        session.native_refuse();
        connection
            .send(&serde_json::to_vec(&BridgeDown::Refused { reason })?)
            .await?;
        return Ok(());
    }
    session.native_accept();
    connection
        .send(&serde_json::to_vec(&BridgeDown::Ready)?)
        .await?;
    tracing::info!(%omp_version, session = %session.id().0, "native bridge accepted");
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            bytes = connection.recv() => {
                let Some(bytes) = bytes? else {
                    return Ok(());
                };
                let Ok(frame) = serde_json::from_slice::<BridgeUp>(&bytes) else {
                    return Ok(());
                };
                match frame {
                    BridgeUp::Hello { .. } => return Ok(()),
                    BridgeUp::State { idle, draft, pending } => session.native_state(idle, draft, pending),
                    BridgeUp::Ack { id } => session.native_ack(&id, Ok(())),
                    BridgeUp::Nack { id, reason } => session.native_ack(&id, Err(reason)),
                    BridgeUp::Receipt { id } => {
                        if record_receipt(&runtime.store, &runtime.boot, &session, &id).await? {
                            session.native_received();
                        }
                    }
                }
            }
            frame = guard.down.recv() => {
                let Some(frame) = frame else {
                    return Ok(());
                };
                connection.send(&serde_json::to_vec(&frame)?).await?;
            }
            _ = interval.tick() => {
                if session.exit_code().is_some() {
                    return Ok(());
                }
            }
        }
    }
}

/// The quota a running Claude, Codex or OMP session reports.
fn session_quota(session: &Session) -> Option<QuotaInfo> {
    if session.exit_code().is_some()
        || !matches!(
            session.harness(),
            Harness::Claude | Harness::Codex | Harness::Omp
        )
    {
        return None;
    }
    quota::read(&session.lock().emulator.screen(), session.harness())
}

fn session_activity(session: &Arc<Session>) -> Option<Activity> {
    if session.exit_code().is_some() {
        return None;
    }
    let now = Instant::now();
    let (ready, last_change) = {
        let state = session.lock();
        (
            harness::ready(
                session.harness(),
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            ),
            state.last_change,
        )
    };
    if AnyChannel::for_session(session.clone())
        .hold_reason()
        .is_some()
        || session.resetting()
    {
        Some(Activity::Busy)
    } else if !ready || now.saturating_duration_since(last_change) < ACTIVITY_QUIET {
        Some(Activity::Working)
    } else {
        Some(Activity::Idle)
    }
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

async fn attachment_status(
    session: &Arc<Session>,
    runtime: &Runtime,
) -> anyhow::Result<StatusInfo> {
    let counts = runtime.store.open_counts(&runtime.boot).await?;
    Ok(StatusInfo {
        address: messaging::address(session.name(), &session.id().0, &runtime.host_name),
        pending: counts.get(&session.id().0).copied().unwrap_or(0),
        hold: AnyChannel::for_session(session.clone())
            .hold_reason()
            .map(str::to_owned),
    })
}

async fn attach(
    connection: &mut Framed,
    session: Arc<Session>,
    force: bool,
    size: Size,
    status: Option<Arc<Runtime>>,
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
    let mut last = None;
    if let Some(runtime) = status.as_ref() {
        let info = attachment_status(&session, runtime).await?;
        connection
            .send(&ServerFrame::Status(info.clone()).encode())
            .await?;
        last = Some(info);
    }
    let period = Duration::from_secs(1);
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        if *taken_over.borrow() {
            connection
                .send(&ServerFrame::Detached("taken over by another attachment".into()).encode())
                .await?;
            connection.stream.shutdown().await?;
            return Ok(true);
        }
        // Delivery shares this notifier and can consume its stored permit while
        // a snapshot is being sent. Register before checking pending render state.
        let output = session.notify().notified();
        tokio::pin!(output);
        output.as_mut().enable();
        let output_pending = {
            let state = session.lock();
            (state.dirty && !state.emulator.is_scrolled())
                || (state.exit_code.is_some() && !state.dirty)
        };
        tokio::select! {
            _ = taken_over.changed() => {},
            // shortcut: one-second poll per attachment; push from the store's write path if latency
            // or load matters.
            _ = ticker.tick() => {
                if let Some(runtime) = status.as_ref() {
                    let info = attachment_status(&session, runtime).await?;
                    if last.as_ref() != Some(&info) {
                        connection.send(&ServerFrame::Status(info.clone()).encode()).await?;
                        last = Some(info);
                    }
                }
            },
            _ = async { if !output_pending { output.await; } } => {
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

#[cfg(test)]
mod tests {
    use super::{HeartbeatPeer, heartbeat_body};
    use std::time::Duration;

    #[test]
    fn heartbeat_body_matches_digest_example() {
        let body = heartbeat_body(
            Duration::from_secs(30 * 60),
            &[
                HeartbeatPeer {
                    name: "developer".into(),
                    address: "developer@host-a".into(),
                    busy_for: Some(Duration::from_secs(12 * 60)),
                    unchanged_for: Some(Duration::from_secs(11 * 60)),
                    hold: None,
                    last_message_age: Some(Duration::from_secs(18 * 60)),
                    queued: 0,
                },
                HeartbeatPeer {
                    name: "tester".into(),
                    address: "tester@host-a".into(),
                    busy_for: None,
                    unchanged_for: Some(Duration::from_secs(45)),
                    hold: Some("human_draft".into()),
                    last_message_age: None,
                    queued: 2,
                },
            ],
        );
        assert_eq!(
            body,
            "Heartbeat: you have been idle for 30m. Watched peers:\n\
- developer (developer@host-a): busy for 12m, screen unchanged for 11m, last message 18m ago, 0 queued\n\
- tester (tester@host-a): ready, held: human_draft, last message none, 2 queued\n\
Run a2amx screen <peer> to look before messaging a busy peer."
        );
    }
}
