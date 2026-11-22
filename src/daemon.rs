//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::{HashMap, HashSet, VecDeque};
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
use crate::codex;
use crate::confine::{self, Host, Plan};
use crate::delivery::{AnyChannel, Channel, RecheckFn};
use crate::emulator::Size;
use crate::harness::{self, Deliver, Harness};
use crate::messaging::{self, Limits, MessageState, code};
use crate::session::{Session, SessionId, SessionSpec, mark_inherited_fds_cloexec};
use crate::store::{self, CancelResult, InsertError, Message, NewMessage, RejectOutcome, Store};
use crate::wire::{HumanOutboundMessage, MessageInfo, QuotaWindow, Request, Response};
mod attach;
mod bridge;
mod notices;
mod omp_usage;
mod registry;
mod remote;
mod trust;
mod views;
use attach::attach;
use bridge::bridge_attach;
use omp_usage::OmpUsage;
use registry::Sessions;
use remote::Leases;
use trust::{request_permitted, session_id};
use views::{
    explain_of, list_agents, list_messages, list_sessions, screen_of, session_activity,
    session_quota,
};
const ACTIVITY_QUIET: Duration = Duration::from_secs(10);
const ACTIVITY_POLL: Duration = Duration::from_secs(1);
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

    fn split_confine(
        request: Request,
    ) -> Result<(Option<confine::ConfineRequest>, Request), String> {
        match request {
            Request::Confine { confine, request } => {
                if matches!(request.as_ref(), Request::Confine { .. }) {
                    return Err("nested confinement requests are not allowed".into());
                }
                Ok((Some(confine), *request))
            }
            request => Ok((None, request)),
        }
    }

    /// Resolve bubblewrap and build the mount plan off the async runtime.  `Err` carries the
    /// message for the client; `Ok(None)` means the session is not confined.
    async fn plan_confinement(
        &self,
        confine_request: Option<confine::ConfineRequest>,
        harness: Harness,
        cwd: Option<&str>,
    ) -> anyhow::Result<Result<Option<Plan>, String>> {
        let Some(confine_request) = confine_request else {
            return Ok(Ok(None));
        };
        let Some(cwd) = cwd else {
            return Ok(Err("confinement requires an explicit cwd".into()));
        };
        let cwd = PathBuf::from(cwd);
        let layout = self.layout.clone();
        let bwrap_path = self.bwrap_path.clone();
        let env_path = self.env_path.clone();
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Plan> {
            let bwrap = confine::resolve_bwrap(Some(&bwrap_path))?;
            if matches!(harness, Harness::Generic) {
                confine::preflight_env(&env_path)?;
            }
            confine::preflight(&bwrap)?;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
            let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
            confine::ensure_empty_file(&layout)?;
            let host = Host {
                layout: layout.clone(),
                home: home.clone(),
                runtime_dir,
            };
            confine::ensure_data_dirs(harness, &home)?;
            confine::ensure_data_files(harness, &home)?;
            let preflight = confine::ensure_grant_dirs(&confine_request, harness, &host, true);
            let mut plan = confine::plan_with_preflight(
                &confine_request,
                harness,
                &cwd,
                &host,
                std::env::var_os("PATH"),
                preflight,
            )?;
            if !plan.overlay_parents().is_empty()
                && let Err(reason) = confine::supports_tmp_overlay(&bwrap, plan.overlay_parents())
            {
                plan = plan.without_overlays(&reason);
            }
            Ok(plan.with_bwrap(bwrap))
        })
        .await?;
        Ok(result.map(Some).map_err(|error| error.to_string()))
    }

    async fn create_session(self: &Arc<Self>, request: Request) -> anyhow::Result<Response> {
        let (confine_request, request) = match Self::split_confine(request) {
            Ok(split) => split,
            Err(message) => return Ok(Response::Error { message }),
        };
        let Request::NewSession {
            argv,
            cols,
            rows,
            cwd,
            mut env,
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            name,
            harness,
            deliver,
            team,
            role,
            remote,
        } = request
        else {
            return Ok(Response::Error {
                message: "expected new_session".into(),
            });
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
        if let Some(message) = Self::validate_watch_lists(&control_from, &watch, name.as_deref()) {
            return Ok(Response::Error { message });
        }
        let heartbeat = match heartbeat
            .map(|seconds| messaging::parse_interval(&format!("{seconds}s"), "heartbeat"))
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
        if let Some(activity) = &activity
            && let Err(error) = activity.validate(!watch.is_empty())
        {
            return Ok(Response::Error {
                message: format!("session {}: {error}", name.as_deref().unwrap_or("unnamed")),
            });
        }

        if let Some(scope) = &team
            && let Some(message) = Self::validate_team_scope(scope, name.as_deref())
        {
            return Ok(Response::Error { message });
        }
        if let Some(role) = &role
            && let Err(error) = messaging::validate_role(role)
        {
            return Ok(Response::Error {
                message: error.to_string(),
            });
        }

        if remote.is_some() && confine_request.is_none() {
            return Ok(Response::Error {
                message: "remote messaging needs a confined session".into(),
            });
        }
        if confine_request.is_some() && cwd.is_none() {
            return Ok(Response::Error {
                message: "confinement requires an explicit cwd".into(),
            });
        }
        let _gate = self.sessions.begin_creation().await;
        if let Some(message) = self.check_team_and_name(team.as_ref(), name.as_deref()) {
            return Ok(Response::Error { message });
        }
        let number = self
            .sessions
            .next_number()
            .ok_or_else(|| anyhow::anyhow!("session id space exhausted"))?;
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
        let mut confine_plan = match self
            .plan_confinement(confine_request, harness, cwd.as_deref())
            .await?
        {
            Ok(plan) => plan,
            Err(message) => return Ok(Response::Error { message }),
        };
        if harness == Harness::Codex
            && let Some(plan) = confine_plan.take()
        {
            confine_plan = Some(plan.with_codex_dir(codex::session_dir(&self.layout, &id.0)));
        }
        if harness == Harness::Codex {
            (argv, codex) = match self
                .prepare_codex(&id, argv, cwd.as_deref(), &mut env, confine_plan.as_ref())
                .await?
            {
                Ok(prepared) => (prepared.0, Some(prepared.1)),
                Err(message) => return Ok(Response::Error { message }),
            };
        }
        let session_spec = SessionSpec {
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
            activity,
            heartbeat,
            team,
            role,
            remote,
            token,
            codex,
            confine: confine_plan,
        };
        let session = if session_spec.confine.is_some() {
            Session::spawn_confined(id, session_spec).await
        } else {
            tokio::task::spawn_blocking(move || Session::spawn(id, session_spec)).await?
        };
        match session {
            Ok(session) => {
                let session = Arc::new(session);
                let id = session.id().0.clone();
                self.sessions.insert(number, session.clone());
                self.start_session_tasks(session.clone(), channel);
                Ok(Response::Created { session: id })
            }
            Err(error) => Ok(Response::Error {
                message: error.to_string(),
            }),
        }
    }

    fn validate_watch_lists(
        control_from: &[String],
        watch: &[String],
        name: Option<&str>,
    ) -> Option<String> {
        for controller in control_from {
            if let Err(error) = messaging::validate_name(controller) {
                return Some(format!("control_from entry {controller:?}: {error}"));
            }
        }
        for watcher in watch {
            if let Err(error) = messaging::validate_name(watcher) {
                return Some(format!("watch entry {watcher:?}: {error}"));
            }
            if name == Some(watcher.as_str()) {
                return Some(format!("session {watcher} watches itself"));
            }
        }
        None
    }

    fn validate_team_scope(scope: &messaging::TeamScope, name: Option<&str>) -> Option<String> {
        if let Err(error) = messaging::validate_name(&scope.name) {
            return Some(format!("team: {error}"));
        }
        if scope.name.ends_with('-') {
            return Some("team must not end in '-'".into());
        }
        if !scope.private && !scope.allow.is_empty() {
            return Some("allow needs a private team".into());
        }
        let mut seen = HashSet::with_capacity(scope.allow.len());
        for entry in &scope.allow {
            if let Err(error) = messaging::validate_name(entry) {
                return Some(format!("team allow entry {entry:?}: {error}"));
            }
            if entry == &scope.name {
                return Some("team allow entry must not be the team itself".into());
            }
            if !seen.insert(entry) {
                return Some(format!("team allow entry {entry:?} is repeated"));
            }
        }
        if !scope.private && !scope.agents.is_empty() {
            return Some("allow needs a private team".into());
        }
        if scope.agents.len() > 16 {
            return Some("session allow has at most 16 entries".into());
        }
        let mut seen_agents = HashSet::with_capacity(scope.agents.len());
        for entry in &scope.agents {
            if let Err(error) = messaging::validate_name(entry) {
                return Some(format!("session allow entry {entry:?}: {error}"));
            }
            if name == Some(entry.as_str()) {
                return Some("session allow entry must not name itself".into());
            }
            if !seen_agents.insert(entry) {
                return Some(format!("session allow entry {entry:?} is repeated"));
            }
        }
        None
    }

    fn check_team_and_name(
        &self,
        team: Option<&messaging::TeamScope>,
        name: Option<&str>,
    ) -> Option<String> {
        if let Some(scope) = team {
            let differs = self.sessions.snapshot().iter().any(|session| {
                session.exit_code().is_none()
                    && session.team().is_some_and(|existing| {
                        existing.name == scope.name
                            && (existing.private != scope.private || existing.allow != scope.allow)
                    })
            });
            if differs {
                return Some(format!(
                    "team {} is already running with different settings",
                    scope.name
                ));
            }
        }
        if let Some(name) = name {
            if let Err(error) = messaging::validate_name(name) {
                return Some(error.to_string());
            }
            if self
                .sessions
                .snapshot()
                .iter()
                .any(|session| session.name() == Some(name))
            {
                return Some(format!("session name {name} is already in use"));
            }
        }
        None
    }

    async fn prepare_codex(
        &self,
        id: &SessionId,
        mut argv: Vec<String>,
        cwd: Option<&str>,
        env: &mut Vec<(String, String)>,
        plan: Option<&confine::Plan>,
    ) -> anyhow::Result<Result<(Vec<String>, Arc<codex::Link>), String>> {
        // shortcut: the client signals --no-authorize-peers and the role through
        // the request environment; a wire field is deferred until a third
        // value needs this or the env path causes trouble. A field now would
        // force edits to the 29 Request::NewSession literals in tests plus
        // src/main.rs, which have no Default.
        // The role also travels in the additive wire field for display only.
        let authorize = !env
            .iter()
            .any(|(key, value)| key == codex::NO_AUTHORIZE_ENV && value == "1");
        env.retain(|(key, _)| key != codex::NO_AUTHORIZE_ENV);
        let exe = self.exe.clone();
        let codex_role = env
            .iter()
            .rev()
            .find_map(|(key, value)| (key == codex::ROLE_ENV).then(|| value.clone()));
        env.retain(|(key, _)| key != codex::ROLE_ENV);
        if let Some(message) =
            tokio::task::spawn_blocking(move || codex::executable_error(&exe)).await?
        {
            return Ok(Err(message));
        }
        let notice = plan.map(confine::notice);
        let started = codex::start(
            &argv[0],
            &self.layout,
            &id.0,
            cwd.map(Path::new),
            env,
            codex::server_config(
                &self.exe,
                authorize,
                codex_role.as_deref(),
                notice.as_deref(),
            ),
            plan,
        )
        .await;
        let link = match started {
            Ok(link) => link,
            Err(error) => return Ok(Err(format!("{error:#}"))),
        };
        argv = codex::wire_argv(argv, link.endpoint());
        Ok(Ok((argv, link)))
    }

    fn start_session_tasks(self: &Arc<Self>, session: Arc<Session>, channel: bool) {
        let store = self.store.clone();
        let boot = self.boot.clone();
        let runtime = Arc::downgrade(self);
        let recheck: RecheckFn = Arc::new(move |message| {
            if message.sender_session != messaging::HUMAN_SESSION {
                return None;
            }
            let Some(runtime) = runtime.upgrade() else {
                return Some("recipient is no longer eligible for remote messaging");
            };
            let Some((_, recipient)) = lookup(&runtime, &message.recipient_session) else {
                return Some("recipient is no longer eligible for remote messaging");
            };
            runtime
                .remote_ineligible(&recipient, Instant::now(), messaging::Direction::ToAgent)
                .map(|(_, detail)| detail)
        });
        let mut tasks = vec![tokio::spawn(crate::delivery::run(
            AnyChannel::for_session(session.clone()),
            store.clone(),
            boot.clone(),
            recheck,
        ))];
        tasks.push(tokio::spawn(self.clone().observe_exit(session.clone())));
        tasks.push(tokio::spawn(self.clone().sample_activity(session.clone())));
        if let Some(interval) = session.heartbeat() {
            tasks.push(tokio::spawn(
                self.clone().heartbeat(session.clone(), interval),
            ));
        }
        if session.activity().is_some() {
            tasks.push(tokio::spawn(self.clone().activity(session.clone())));
        }
        if channel {
            tasks.push(tokio::spawn(accept_channel_dialog(session.clone())));
        }
        if let Some(link) = session.codex().cloned() {
            tasks.push(tokio::spawn(codex::run(session, link, store, boot)));
        }
        self.deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(tasks);
    }

    async fn sample_activity(self: Arc<Self>, session: Arc<Session>) {
        // shortcut: sample once per second and read each session screen; use an output-path
        // transition hook if the cost matters.
        let mut ticker = tokio::time::interval(ACTIVITY_POLL);
        loop {
            ticker.tick().await;
            let now = Instant::now();
            let signal = match session.harness() {
                Harness::Codex => session
                    .codex()
                    .and_then(|link| harness::label_for_codex_reason(link.reason())),
                Harness::Claude => {
                    // shortcut: the Claude signal is a screen heuristic shared with
                    // `claude_ready`; replace it with a protocol signal if Claude Code ever
                    // exposes one.
                    let state = session.lock();
                    harness::needs_input(Harness::Claude, &state.emulator.screen())
                        .then_some("input?")
                }
                Harness::Omp | Harness::Generic | Harness::Agy => None,
            };
            session.note_needs_input(signal, now);
            if session.exit_code().is_some() {
                return;
            }
            let Some(activity) = session_activity(&session) else {
                return;
            };
            session.note_activity(activity, now);
        }
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

    fn recipient(&self, to: &str) -> Option<Arc<Session>> {
        let local = messaging::local_part(to, &self.host_name)?;
        self.sessions
            .snapshot()
            .iter()
            .find(|session| session.name().unwrap_or(&session.id().0) == local)
            .cloned()
    }

    async fn send_message(
        &self,
        role: &Role,
        to: String,
        subject: String,
        body: String,
    ) -> Response {
        let sender_id = match session_id(role, "send_message") {
            Ok(id) => id,
            Err(message) => return Response::Error { message },
        };
        if let Err(error) = messaging::validate_message(&subject, &body) {
            return failed(error.code, &error.message);
        }
        let Some((_, sender)) = lookup(self, sender_id) else {
            return failed(code::INTERNAL, "sender session is unavailable");
        };
        if to == messaging::HUMAN_SESSION {
            if let Some((error_code, message)) =
                self.remote_ineligible(&sender, Instant::now(), messaging::Direction::ToHuman)
            {
                return failed(error_code, message);
            }
            if let Some((cause, away_secs)) = self.owner_cause(sender_id, Instant::now()) {
                let message = match cause {
                    messaging::PresenceCause::Attached => {
                        "the owner is attached to this session in the terminal (viewer mode does not count); write to them there instead".to_owned()
                    }
                    messaging::PresenceCause::Grace => format!(
                        "the owner left this session less than {away_secs} seconds ago; try again later or write to them in the terminal"
                    ),
                };
                return failed(code::OWNER_PRESENT, &message);
            }
            let new = NewMessage {
                boot: self.boot.clone(),
                sender_session: sender_id.into(),
                sender_address: messaging::address(sender.name(), sender_id, &self.host_name),
                recipient_session: messaging::HUMAN_SESSION.into(),
                recipient_address: messaging::HUMAN_SESSION.into(),
                subject,
                body,
            };
            let windows = [
                (
                    sender_id,
                    self.limits.rate_per_minute,
                    "sender rate limit reached",
                ),
                (
                    "human:out",
                    self.limits.human_outbound_per_minute,
                    "human outbound rate limit reached",
                ),
            ];
            return match self.insert_message_rate_limited(&windows, new).await {
                Ok(Ok(seq)) => Response::Accepted {
                    id: format!("m_{seq}"),
                    recipient_hold: None,
                    recipient_quota: None,
                },
                Ok(Err(message)) => failed(code::RATE_LIMITED, message),
                Err(InsertError::QueueFull) => failed(code::QUEUE_FULL, "message queue is full"),
                Err(InsertError::Internal(error)) => {
                    tracing::error!(%error, "message acceptance failed");
                    failed(code::INTERNAL, "message storage failed")
                }
            };
        }
        let Some(recipient) = self.recipient(&to) else {
            return failed(code::UNKNOWN_RECIPIENT, "unknown recipient");
        };
        if !messaging::visible(sender.party(), recipient.party()) {
            return failed(code::UNKNOWN_RECIPIENT, "unknown recipient");
        }
        if recipient.id().0 == sender_id {
            return failed(code::UNKNOWN_RECIPIENT, "a session cannot message itself");
        }
        if recipient.exit_code().is_some() {
            return failed(code::RECIPIENT_EXITED, "recipient has exited");
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
        let windows = [(
            sender_id,
            self.limits.rate_per_minute,
            "sender rate limit reached",
        )];
        match self.insert_message_rate_limited(&windows, new).await {
            Ok(Ok(seq)) => {
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
                    recipient_quota: session_quota(&recipient, self.omp_usage.current().as_ref())
                        .and_then(|quota| quota.exhausted())
                        .map(|window| format!("{window} quota exhausted")),
                }
            }
            Ok(Err(message)) => failed(code::RATE_LIMITED, message),
            Err(InsertError::QueueFull) => failed(code::QUEUE_FULL, "message queue is full"),
            Err(InsertError::Internal(error)) => {
                tracing::error!(%error, "message acceptance failed");
                failed(code::INTERNAL, "message storage failed")
            }
        }
    }

    async fn insert_message_rate_limited(
        &self,
        windows: &[(&str, u32, &'static str)],
        new: NewMessage,
    ) -> std::result::Result<std::result::Result<i64, &'static str>, InsertError> {
        let mut rate = self.rate.lock().await;
        for &(key, limit, refusal) in windows {
            let timestamps = rate.entry(key.into()).or_default();
            while timestamps
                .front()
                .is_some_and(|time| time.elapsed() > Duration::from_secs(60))
            {
                timestamps.pop_front();
            }
            if timestamps.len() >= limit as usize {
                return Ok(Err(refusal));
            }
        }
        let sequence = self.store.insert_message(new).await?;
        let now = Instant::now();
        for &(key, _, _) in windows {
            rate.entry(key.into()).or_default().push_back(now);
        }
        Ok(Ok(sequence))
    }

    async fn human_inbound(&self, to: String, body: String) -> Response {
        if let Err(error) = messaging::validate_message(messaging::HUMAN_SUBJECT, &body) {
            return failed(error.code, &error.message);
        }
        if body.len() > messaging::MAX_HUMAN_BODY_BYTES {
            return failed(
                code::TOO_LARGE,
                "message exceeds the byte limit for a human message",
            );
        }
        let Some(recipient) = self.recipient(&to) else {
            return failed(code::UNKNOWN_RECIPIENT, "unknown recipient");
        };
        if let Some((error_code, message)) =
            self.remote_ineligible(&recipient, Instant::now(), messaging::Direction::ToAgent)
        {
            return failed(error_code, message);
        }
        let new = NewMessage {
            boot: self.boot.clone(),
            sender_session: messaging::HUMAN_SESSION.into(),
            sender_address: messaging::HUMAN_SESSION.into(),
            recipient_session: recipient.id().0.clone(),
            recipient_address: messaging::address(
                recipient.name(),
                &recipient.id().0,
                &self.host_name,
            ),
            subject: messaging::HUMAN_SUBJECT.into(),
            body,
        };
        let windows = [(
            "human:in",
            self.limits.human_inbound_per_minute,
            "human inbound rate limit reached",
        )];
        match self.insert_message_rate_limited(&windows, new).await {
            Ok(Ok(seq)) => {
                recipient.message_notify().notify_one();
                Response::Accepted {
                    id: format!("m_{seq}"),
                    recipient_hold: None,
                    recipient_quota: None,
                }
            }
            Ok(Err(message)) => failed(code::RATE_LIMITED, message),
            Err(InsertError::QueueFull) => failed(code::QUEUE_FULL, "message queue is full"),
            Err(InsertError::Internal(error)) => {
                tracing::error!(%error, "human message acceptance failed");
                failed(code::INTERNAL, "message storage failed")
            }
        }
    }

    async fn fail_human_bound(&self, seq: i64, detail: &'static str) -> anyhow::Result<()> {
        if let Some(attempt) = self.store.begin_attempt(seq).await? {
            self.store
                .finish_attempt(seq, attempt, "failed", Some(detail))
                .await?;
        }
        Ok(())
    }

    async fn next_eligible_human(&self) -> anyhow::Result<Option<(Message, i64)>> {
        for _ in 0..self.limits.max_pending_per_recipient {
            let Some(message) = self
                .store
                .next_pending(&self.boot, messaging::HUMAN_SESSION)
                .await?
            else {
                return Ok(None);
            };
            let eligible = lookup(self, &message.sender_session)
                .map(|(_, sender)| {
                    self.remote_ineligible(&sender, Instant::now(), messaging::Direction::ToHuman)
                        .is_none()
                })
                .unwrap_or(false);
            if !eligible {
                self.fail_human_bound(
                    message.seq,
                    "sender is no longer eligible for remote messaging",
                )
                .await?;
                continue;
            }
            let Some(attempt) = self.store.begin_attempt(message.seq).await? else {
                continue;
            };
            return Ok(Some((message, attempt)));
        }
        Ok(None)
    }

    async fn human_outbound_next(&self) -> Response {
        match self.next_eligible_human().await {
            Ok(Some((message, attempt))) => {
                self.human_attempts
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(message.seq, attempt);
                Response::HumanOutbound {
                    message: Some(HumanOutboundMessage {
                        id: format!("m_{}", message.seq),
                        from: message.sender_address,
                        subject: message.subject,
                        body: message.body,
                    }),
                }
            }
            Ok(None) => Response::HumanOutbound { message: None },
            Err(error) => {
                tracing::error!(%error, "human outbound lookup failed");
                Response::Error {
                    message: "human outbound lookup failed".into(),
                }
            }
        }
    }

    async fn human_outbound_done(&self, id: String, sent: bool) -> Response {
        let Some(seq) = parse_message_id(&id) else {
            return Response::Error {
                message: "unknown message".into(),
            };
        };
        let Some(attempt) = self
            .human_attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&seq)
        else {
            return Response::Error {
                message: "unknown message".into(),
            };
        };
        let outcome = if sent { "submitted" } else { "failed" };
        let detail = (!sent).then_some("the gateway could not send it");
        match self
            .store
            .finish_attempt(seq, attempt, outcome, detail)
            .await
        {
            Ok(()) => Response::Ok,
            Err(error) => {
                self.human_attempts
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(seq, attempt);
                tracing::error!(%error, "human outbound result failed");
                Response::Error {
                    message: "human outbound result failed".into(),
                }
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

async fn kill_session(runtime: &Runtime, id: &str, now: bool) -> Response {
    match lookup(runtime, id) {
        Some((number, session)) => match session.kill(!now).await {
            Ok(()) => {
                runtime.sessions.remove(number);
                Response::Ok
            }
            Err(error) => Response::Error {
                message: error.to_string(),
            },
        },
        None => Response::Error {
            message: format!("unknown session {id}"),
        },
    }
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
