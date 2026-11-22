//! The host daemon: owns session runtimes and serves the local TCP protocol.

use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::fs::File;
use std::io::Read;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::harness::Harness;
use crate::messaging::{self, Limits};
use crate::session::Session;
use crate::store::Store;
use crate::wire::Response;
mod admission;
mod attach;
mod bridge;
mod launch;
mod lifecycle;
mod notices;
mod omp_usage;
mod registry;
mod remote;
mod reports;
mod serve;
mod trust;
mod views;
use omp_usage::OmpUsage;
use registry::Sessions;
use remote::Leases;
pub(crate) use reports::record_receipt;
use serve::response;
use trust::session_id;
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
    omp_usage_poller: Option<JoinHandle<()>>,
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

fn lookup(runtime: &Runtime, id: &str) -> Option<(u64, Arc<Session>)> {
    let number = id.strip_prefix('s')?.parse::<u64>().ok()?;
    runtime
        .sessions
        .get(number)
        .filter(|session| session.id().0 == id)
        .map(|session| (number, session))
}
