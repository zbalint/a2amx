//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyhow::Context;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::{JoinHandle, JoinSet};

use crate::client::Framed;
use crate::confine;
use crate::delivery::{AnyChannel, Channel};
use crate::emulator::Size;
use crate::harness::{self, Deliver, Harness};
use crate::messaging::{self, Limits, MessageState, code};
use crate::session::{Session, mark_inherited_fds_cloexec};
use crate::store::{self, CancelResult, Message, RejectOutcome, Store};
use crate::wire::{MessageInfo, QuotaWindow, Request, Response};
mod admission;
mod attach;
mod bridge;
mod launch;
mod notices;
mod omp_usage;
mod registry;
mod remote;
mod trust;
mod views;
use attach::attach;
use bridge::bridge_attach;
use launch::kill_session;
use omp_usage::OmpUsage;
use registry::Sessions;
use remote::Leases;
use trust::{request_permitted, session_id};
use views::{explain_of, list_agents, list_messages, list_sessions, screen_of};
const ACTIVITY_QUIET: Duration = Duration::from_secs(10);
pub(crate) const NEEDS_INPUT_AFTER: Duration = Duration::from_secs(10);
const OMP_USAGE_CHECK: Duration = Duration::from_secs(2);
// shortcut: output-free tool calls over the quiet window read as idle; use a per-session or
// harness-specific working marker if that misleads in practice.
pub struct DaemonConfig {
    /// a2amx directory (`A2AMX_HOME` or `--home`).
    pub root: PathBuf,
    /// Listen addresses; loopback by default. Port 0 lets the OS choose.
    pub listen: Vec<SocketAddr>,
    pub host_name: Option<String>,
    pub limits: Limits,
    /// Daemon PATH value used to resolve bubblewrap and the OMP quota probe.
    pub bwrap_path: Option<OsString>,
    /// `env` program probed by the Generic confinement preflight; `None` means `confine::ENV_PATH`.
    /// Only the probe uses it: the launched argv always names `confine::ENV_PATH`.
    pub env_path: Option<PathBuf>,
}

pub struct Daemon {
    addrs: Vec<SocketAddr>,
    runtime: Arc<Runtime>,
    shutdown: watch::Sender<bool>,
    listeners: Vec<JoinHandle<()>>,
    purge: Option<JoinHandle<()>>,
    omp_usage: Option<JoinHandle<()>>,
    presence: Option<JoinHandle<()>>,
    layout: crate::layout::Layout,
    lock: Option<File>,
}

struct Runtime {
    sessions: Sessions,
    token: String,
    gateway_token: String,
    store: Store,
    host_name: String,
    boot: String,
    limits: Limits,
    address: String,
    layout: crate::layout::Layout,
    bwrap_path: OsString,
    env_path: PathBuf,
    exe: PathBuf,
    omp_usage: OmpUsage,
    rate: tokio::sync::Mutex<HashMap<String, VecDeque<Instant>>>,
    presence: Arc<Mutex<HashMap<String, messaging::Presence>>>,
    leases: Leases,
    gateway_connections: AtomicUsize,
    // shortcut: a stopped gateway leaves messages delivering until daemon restart;
    // each stuck message consumes the shared human queue limit, so add a timeout
    // when gateway flakiness proves this path needs recovery.
    human_attempts: Mutex<HashMap<i64, i64>>,
    deliveries: Mutex<Vec<JoinHandle<()>>>,
    stop_requested: tokio::sync::Notify,
    stop_now: AtomicBool,
}

enum Role {
    Admin,
    Gateway,
    Session(String),
}

struct GatewayConnectionGuard<'a> {
    connections: &'a AtomicUsize,
}

impl Drop for GatewayConnectionGuard<'_> {
    fn drop(&mut self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
    }
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
    fn omp_usage_command(&self) -> Option<(String, Option<PathBuf>)> {
        self.sessions.snapshot().iter().find_map(|session| {
            if session.exit_code().is_some() || session.harness() != Harness::Omp {
                return None;
            }
            let argv0 = session.argv.first()?;
            if Path::new(argv0)
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                != Some("omp")
            {
                return None;
            }
            Some((argv0.clone(), session.cwd().map(Path::to_owned)))
        })
    }

    fn authenticate(&self, token: &str) -> Option<Role> {
        let mut role = constant_time_eq(token, &self.token).then_some(Role::Admin);
        for session in self.sessions.snapshot() {
            if session.exit_code().is_none() {
                let matches = constant_time_eq(token, session.token());
                if matches && role.is_none() {
                    role = Some(Role::Session(session.id().0.clone()));
                }
            }
        }
        if role.is_none() && constant_time_eq(token, &self.gateway_token) {
            role = Some(Role::Gateway);
        }
        role
    }

    fn owner_cause(
        &self,
        session_id: &str,
        now: Instant,
    ) -> Option<(messaging::PresenceCause, u64)> {
        self.presence
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(session_id)
            .and_then(|presence| {
                presence
                    .cause(now)
                    .map(|cause| (cause, presence.away_secs()))
            })
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
            .snapshot()
            .iter()
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
            Role::Gateway => "gateway".to_owned(),
            Role::Session(id) => lookup(self, id).map_or_else(
                || id.clone(),
                |(_, session)| messaging::address(session.name(), id, &self.host_name),
            ),
        };
        let Some(target) = self.reset_target(&reference) else {
            self.reset_audit(&sender, &reference, messaging::code::UNKNOWN_SESSION, None);
            return failed(messaging::code::UNKNOWN_SESSION, "unknown session");
        };
        if let Role::Session(sender_id) = role {
            let Some((_, sender_session)) = lookup(self, sender_id) else {
                self.reset_audit(&sender, &reference, messaging::code::UNKNOWN_SESSION, None);
                return failed(messaging::code::UNKNOWN_SESSION, "unknown session");
            };
            if !messaging::visible(sender_session.party(), target.party()) {
                let target_address =
                    messaging::address(target.name(), &target.id().0, &self.host_name);
                self.reset_audit(
                    &sender,
                    &target_address,
                    messaging::code::UNKNOWN_SESSION,
                    None,
                );
                return failed(messaging::code::UNKNOWN_SESSION, "unknown session");
            }
        }
        let target_address = messaging::address(target.name(), &target.id().0, &self.host_name);
        let permitted = match role {
            Role::Admin => true,
            Role::Gateway => false,
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

    async fn report_prompt(&self, role: &Role, prompt: String) -> anyhow::Result<Response> {
        let session_id = match session_id(role, "report_prompt") {
            Ok(id) => id,
            Err(message) => return Ok(Response::Error { message }),
        };
        let (_, session) =
            lookup(self, session_id).ok_or_else(|| anyhow::anyhow!("session is unavailable"))?;
        if session.channel()
            && let Some(inner) = messaging::unwrap_channel(&prompt)
        {
            self.channel_prompt_verdict(&session, session_id, &inner)
                .await?;
            return Ok(Response::PromptVerdict {
                verdict: "allow".into(),
                reason: None,
            });
        }
        let text = messaging::unwrap_pastes(&prompt);
        let known = self.known_envelopes(&session, session_id, &text).await?;
        if known.is_empty() {
            session.note_submit(false);
        } else if known.len() == 1 && text == known[0].1 {
            self.store.record_receipt(known[0].0).await?;
            session.note_submit(true);
        } else {
            return self.block_corrupted_submission(&session, text, known).await;
        }
        Ok(Response::PromptVerdict {
            verdict: "allow".into(),
            reason: None,
        })
    }

    async fn report_quota(
        &self,
        role: &Role,
        five_hour: Option<QuotaWindow>,
        weekly: Option<QuotaWindow>,
    ) -> anyhow::Result<Response> {
        let session_id = match session_id(role, "report_quota") {
            Ok(id) => id,
            Err(message) => return Ok(Response::Error { message }),
        };
        let now = store::now();
        let latest = now.saturating_add(8 * 24 * 60 * 60);
        for window in [five_hour, weekly].into_iter().flatten() {
            if window.left > 100 {
                return Ok(Response::Error {
                    message: "quota left must be at most 100".into(),
                });
            }
            if window.resets_at > latest {
                return Ok(Response::Error {
                    message: "quota reset is too far in the future".into(),
                });
            }
        }
        let Some((_, session)) = lookup(self, session_id) else {
            return Ok(Response::Error {
                message: "session is unavailable".into(),
            });
        };
        if matches!(session.harness(), Harness::Claude | Harness::Agy) {
            session.set_quota_report(five_hour, weekly);
        }
        Ok(Response::Ok)
    }

    fn is_awaiting_receipt(&self, message: &Message, session_id: &str) -> bool {
        message.boot == self.boot
            && message.recipient_session == session_id
            && matches!(
                message.state,
                MessageState::Delivering | MessageState::Submitted | MessageState::Unsubmitted
            )
            && !message.observed
    }

    async fn channel_prompt_verdict(
        &self,
        session: &Arc<Session>,
        session_id: &str,
        inner: &str,
    ) -> anyhow::Result<()> {
        let ids = messaging::envelope_ids(inner);
        if let [seq] = ids.as_slice()
            && let Some(message) = self.store.get(*seq).await?
            && self.is_awaiting_receipt(&message, session_id)
            && inner == messaging::channel_view(&message.envelope())
        {
            self.store.record_receipt(*seq).await?;
            session.native_received();
        }
        Ok(())
    }

    async fn known_envelopes(
        &self,
        session: &Arc<Session>,
        session_id: &str,
        text: &str,
    ) -> anyhow::Result<Vec<(i64, String)>> {
        let mut known = Vec::new();
        // shortcut: a session-token holder can forge prompts for that session;
        // stronger attestation requires a different same-user trust contract.
        for seq in messaging::envelope_ids(text) {
            if let Some(message) = self.store.get(seq).await?
                && self.is_awaiting_receipt(&message, session_id)
            {
                // The hook sees the envelope as the harness transformed it on paste.
                let envelope = harness::paste_view(session.harness(), &message.envelope());
                known.push((seq, envelope));
            }
        }
        Ok(known)
    }

    async fn block_corrupted_submission(
        &self,
        session: &Arc<Session>,
        text: String,
        known: Vec<(i64, String)>,
    ) -> anyhow::Result<Response> {
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
        Ok(Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some(
                if undeliverable {
                    messaging::UNMATCHABLE_SUBMISSION_REASON
                } else {
                    messaging::CORRUPTED_SUBMISSION_REASON
                }
                .into(),
            ),
        })
    }

    async fn message_status(&self, role: &Role, id: &str) -> anyhow::Result<Response> {
        let Some(seq) = parse_message_id(id) else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        let Some(message) = self.store.get(seq).await? else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        if let Role::Session(sender) = role
            && (message.boot != self.boot || message.sender_session != *sender)
        {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
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

/// The running executable's path, without the suffix Linux appends once the
/// file has been replaced on disk.
pub fn current_exe() -> std::io::Result<PathBuf> {
    let mut exe = std::env::current_exe()?;
    if let Some(path) = exe.to_string_lossy().strip_suffix(" (deleted)") {
        exe = PathBuf::from(path);
    }
    Ok(exe)
}

impl Daemon {
    /// Bind every listen address, write the chosen addresses to a file in the
    /// a2amx directory, write the admin token file (mode 0600), and start serving.
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self> {
        mark_inherited_fds_cloexec()?;
        let exe = tokio::task::spawn_blocking(current_exe).await??;
        let bwrap_path = config
            .bwrap_path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default();
        let env_path = config
            .env_path
            .clone()
            .unwrap_or_else(|| PathBuf::from(confine::ENV_PATH));
        let layout = crate::layout::Layout::new(config.root.clone());
        layout.ensure()?;
        let lock_path = layout.daemon_lock();
        let lock = tokio::task::spawn_blocking(move || -> anyhow::Result<File> {
            let lock = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(lock_path)?;
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| {
                    anyhow::anyhow!("another a2amx daemon is running for this a2amx directory")
                })?;
            Ok(lock)
        })
        .await??;
        let host_name = config.host_name;
        let (token, gateway_token, host_name, boot) =
            tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let token = random_hex::<32>()?;
                let gateway_token = random_hex::<32>()?;
                let host = match host_name {
                    Some(host) => {
                        messaging::validate_host(&host).context("invalid host name")?;
                        host
                    }
                    None => messaging::default_host_name(),
                };
                Ok((token, gateway_token, host, random_hex::<8>()?))
            })
            .await??;
        let layout = crate::layout::Layout::new(config.root.clone());
        let store = Store::open(&layout, config.limits).await?;
        // Written only after the steps that can fail without binding, so an early
        // startup error never leaves a token for a daemon that is not running.
        let gateway_path = layout.gateway_token();
        let link_gateway_path = layout.link_gateway_token();
        let token_path = layout.admin_token();
        let gateway_text = gateway_token.clone();
        let token_text = token.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            write_state_file(&gateway_path, gateway_text.as_bytes())?;
            if let Err(error) = write_state_file(&link_gateway_path, gateway_text.as_bytes()) {
                return match remove_state_files(&[&gateway_path, &link_gateway_path]) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(anyhow::anyhow!(
                        "{error}; failed to remove gateway tokens after link write failure: {cleanup}"
                    )),
                };
            }
            if let Err(error) = write_state_file(&token_path, token_text.as_bytes()) {
                return match remove_state_files(&[&gateway_path, &link_gateway_path]) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(anyhow::anyhow!(
                        "{error}; failed to remove gateway tokens after admin.token write failure: {cleanup}"
                    )),
                };
            }
            Ok(())
        })
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
                    let admin_path = layout.admin_token();
                    let gateway_path = layout.gateway_token();
                    let link_gateway_path = layout.link_gateway_token();
                    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
                        remove_state_files(&[&admin_path, &gateway_path, &link_gateway_path])
                    })
                    .await??;
                    return Err(error.into());
                }
            };
            addrs.push(listener.local_addr()?);
            bound.push(listener);
        }
        let path = layout.addr();
        let link_path = layout.link_addr();
        let addresses = addrs
            .iter()
            .map(|addr| format!("{addr}\n"))
            .collect::<String>();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            write_state_file(&path, addresses.as_bytes())?;
            if let Err(error) = write_state_file(&link_path, addresses.as_bytes()) {
                if let Err(cleanup) = remove_state_files(&[&path, &link_path]) {
                    return Err(anyhow::anyhow!(
                        "{error}; failed to remove daemon addresses after link write failure: {cleanup}"
                    ));
                }
                return Err(error);
            }
            Ok(())
        })
        .await??;
        // shortcut: sessions live only in this daemon; add persistence only with
        // a later lifecycle specification.
        let (shutdown, _) = watch::channel(false);
        let runtime = Arc::new(Runtime {
            sessions: Sessions::new(),
            token,
            gateway_token,
            store,
            host_name,
            boot,
            limits: config.limits,
            address: addrs[0].to_string(),
            layout: layout.clone(),
            bwrap_path,
            env_path,
            exe,
            omp_usage: OmpUsage::new(),
            // shortcut: accepted-send timestamps are memory-only; persist them if
            // rate limits must survive daemon restarts.
            rate: tokio::sync::Mutex::new(HashMap::new()),
            presence: Arc::new(Mutex::new(HashMap::new())),
            leases: Leases::new(),
            gateway_connections: AtomicUsize::new(0),
            human_attempts: Mutex::new(HashMap::new()),
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
        let omp_usage = {
            let runtime = runtime.clone();
            let mut stopping = shutdown.subscribe();
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(OMP_USAGE_CHECK);
                loop {
                    tokio::select! {
                        _ = stopping.changed() => break,
                        _ = interval.tick() => {
                            if runtime.omp_usage.poll(
                                runtime.omp_usage_command(),
                                runtime.bwrap_path.as_os_str(),
                                &mut stopping,
                            ).await
                            {
                                break;
                            }
                        }
                    }
                }
            })
        };
        let presence = {
            let runtime = runtime.clone();
            let stopping = shutdown.subscribe();
            tokio::spawn(runtime.poll_presence(stopping))
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
            omp_usage: Some(omp_usage),
            presence: Some(presence),
            layout,
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
        if let Some(omp_usage) = self.omp_usage.take() {
            omp_usage.await?;
        }
        if let Some(presence) = self.presence.take() {
            presence.await?;
        }
        let mut failure = None;
        let deliveries = std::mem::take(
            &mut *self
                .runtime
                .deliveries
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
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
        let sessions = self.runtime.sessions.snapshot();
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
        self.runtime.sessions.clear();
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
        let addr = self.layout.addr();
        let link_addr = self.layout.link_addr();
        let lock = self.lock.take();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            remove_state_files(&[&addr, &link_addr])?;
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
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
        {
            delivery.abort();
        }
    }
}

pub fn random_hex<const N: usize>() -> anyhow::Result<String> {
    let mut random = [0; N];
    File::open("/dev/urandom")?.read_exact(&mut random)?;
    let mut text = String::with_capacity(N * 2);
    use std::fmt::Write as _;
    for byte in random {
        write!(&mut text, "{byte:02x}")?;
    }
    Ok(text)
}

fn remove_state_files(paths: &[&Path]) -> anyhow::Result<()> {
    let mut first_error = None;
    for path in paths {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                first_error.get_or_insert_with(|| {
                    anyhow::anyhow!("cannot remove state file {}: {error}", path.display())
                });
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
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
    let Some(role) = authenticate_hello(&mut connection, &runtime).await? else {
        return Ok(());
    };
    let _gateway_guard = if matches!(role, Role::Gateway) {
        runtime.gateway_connections.fetch_add(1, Ordering::Relaxed);
        Some(GatewayConnectionGuard {
            connections: &runtime.gateway_connections,
        })
    } else {
        None
    };
    response(&mut connection, Response::Ok).await?;
    while let Some(bytes) = connection.recv().await? {
        if session_has_exited(&runtime, &role) {
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
        if !request_permitted(&role, &request) {
            let message = if matches!(&role, Role::Session(_)) {
                "not permitted for a session token"
            } else {
                "not permitted for this token"
            };
            response(
                &mut connection,
                Response::Error {
                    message: message.into(),
                },
            )
            .await?;
            continue;
        }
        let result = match request {
            Request::Hello { .. } => Response::Error {
                message: "already authenticated".into(),
            },
            Request::List => list_sessions(&runtime).await?,
            Request::Version => Response::Version {
                version: crate::VERSION.to_owned(),
            },
            request @ Request::Confine { .. } => runtime.create_session(request).await?,
            request @ Request::NewSession { .. } => runtime.create_session(request).await?,
            Request::Shutdown { now } => {
                // Answer first: shutdown closes every connection, this one included.
                runtime.stop_now.store(now, Ordering::Relaxed);
                let sent = response(&mut connection, Response::Ok).await;
                runtime.stop_requested.notify_one();
                sent?;
                continue;
            }
            Request::Kill { session: id, now } => kill_session(&runtime, &id, now).await,
            Request::Reset { session } => runtime.reset_session(&role, session).await,
            Request::Screen { session: id } => screen_of(&runtime, &id),
            Request::Explain { session: id } => explain_of(&runtime, &id).await,
            Request::Attach {
                session: id,
                force,
                cols,
                rows,
                status,
                top_rows,
                viewer,
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
                        top_rows,
                        viewer,
                        status,
                        runtime.clone(),
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
            Request::ListAgents => list_agents(&runtime, &role),
            Request::SendMessage {
                to,
                subject,
                message,
            } => runtime.send_message(&role, to, subject, message).await,
            Request::ReportPrompt { prompt } => runtime.report_prompt(&role, prompt).await?,
            Request::ReportQuota { five_hour, weekly } => {
                runtime.report_quota(&role, five_hour, weekly).await?
            }
            Request::BridgeAttach => {
                if let Some(result) = bridge_attach(&mut connection, &runtime, &role).await? {
                    result
                } else {
                    return Ok(());
                }
            }
            Request::MessageStatus { id } => runtime.message_status(&role, &id).await?,
            Request::HumanInbound { to, body } => runtime.human_inbound(to, body).await,
            Request::GatewayAgents => runtime.gateway_agents(),
            Request::HumanOutboundNext => runtime.human_outbound_next().await,
            Request::HumanOutboundDone { id, sent } => runtime.human_outbound_done(id, sent).await,
            Request::IssueLeaseCode { team } => runtime.issue_lease_code(team).await?,
            Request::RedeemLeaseCode { code } => runtime.redeem_lease_code(code),
            Request::ListMessages { session, state } => {
                list_messages(&runtime, session, state).await?
            }
            Request::CancelMessage { id } => cancel_message(&runtime, &id).await?,
        };
        response(&mut connection, result).await?;
    }
    Ok(())
}

fn session_has_exited(runtime: &Runtime, role: &Role) -> bool {
    matches!(role, Role::Session(id) if !lookup(runtime, id).is_some_and(|(_, session)| session.exit_code().is_none()))
}

async fn authenticate_hello(
    connection: &mut Framed,
    runtime: &Runtime,
) -> anyhow::Result<Option<Role>> {
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
            connection,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            },
        )
        .await?;
        connection.stream.shutdown().await?;
        return Ok(None);
    };
    Ok(Some(role))
}

async fn cancel_message(runtime: &Runtime, id: &str) -> anyhow::Result<Response> {
    Ok(match parse_message_id(id) {
        Some(seq) => match runtime.store.cancel(seq).await? {
            CancelResult::Cancelled => Response::Ok,
            CancelResult::NotCancellable(state) => Response::Error {
                message: format!("message {id} is {} and cannot be cancelled", state.as_str()),
            },
            CancelResult::Unknown => Response::Error {
                message: format!("unknown message {id}"),
            },
        },
        None => Response::Error {
            message: format!("unknown message {id}"),
        },
    })
}

fn lookup(runtime: &Runtime, id: &str) -> Option<(u64, Arc<Session>)> {
    let number = id.strip_prefix('s')?.parse::<u64>().ok()?;
    runtime
        .sessions
        .get(number)
        .filter(|session| session.id().0 == id)
        .map(|session| (number, session))
}
