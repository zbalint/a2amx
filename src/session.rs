//! Session runtime: one PTY, its child, and its emulator.
//!
//! Blocking PTY reads, PTY writes, and the drop of the PTY (which blocks on the
//! child) stay off the async runtime. The PTY master is non-blocking and ends with
//! EIO when the child closes the slave. Resize uses our own ioctl, never
//! `Pty::on_resize`, which would exit the process on failure.

use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use alacritty_terminal::tty::{self, ChildEvent, EventedPty};
use anyhow::{Context, bail};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::sync::{Notify, mpsc, oneshot, watch};

use crate::emulator::{Emulator, Size, window_size};
use crate::harness::{self, Deliver, Harness};
use crate::messaging;
use crate::wire::BridgeDown;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);
const RESET_SETTLE: Duration = Duration::from_millis(1500); // shortcut: fixed settle; replace with a per-harness completion marker if a harness reports readiness early or needs longer.
const RESET_POLL: Duration = Duration::from_millis(100);
const RESET_TIMEOUT: Duration = Duration::from_secs(30);
const GRACEFUL_TIMEOUT: Duration = Duration::from_secs(10);
const CLAUDE_EXIT_KEY_GAP: Duration = Duration::from_millis(300);

pub struct SessionSpec {
    pub argv: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub size: Size,
    pub env: Vec<(String, String)>,
    pub name: Option<String>,
    pub harness: Harness,
    pub channel: bool,
    pub deliver: Deliver,
    pub reset: Vec<String>,
    pub control_from: Vec<String>,
    pub watch: Vec<String>,
    pub heartbeat: Option<Duration>,
    pub token: String,
    pub(crate) codex: Option<Arc<crate::codex::Link>>,
}

pub struct Session {
    id: SessionId,
    pub(crate) argv: Vec<String>,
    cwd: Option<std::path::PathBuf>,
    name: Option<String>,
    harness: Harness,
    channel: bool,
    deliver: Deliver,
    reset: Vec<String>,
    control_from: Vec<String>,
    watch: Vec<String>,
    heartbeat: Option<Duration>,
    token: String,
    codex: Option<Arc<crate::codex::Link>>,
    input_gate: tokio::sync::Mutex<()>,
    message_notify: Notify,
    shared: Arc<Shared>,
    input: mpsc::Sender<Option<Vec<u8>>>,
    master: File,
    pid: Pid,
    drop_requested: Arc<AtomicBool>,
    child_done: Arc<AtomicBool>,
    daemon_ended: AtomicBool,
    // shortcut: Codex in-flight delivery is not gated; add a Codex in-flight flag to Session if a message lands inside a reset.
    resetting: AtomicBool,
}

struct Shared {
    state: Mutex<State>,
    notify: Notify,
}

pub(crate) struct State {
    pub emulator: Emulator,
    pub size: Size,
    pub exit_code: Option<i32>,
    pub attachment: Option<AttachmentSlot>,
    pub generation: u64,
    pub pending_attachment: Option<u64>,
    pub dirty: bool,
    pub last_change: Instant,
    pub hold: Option<Hold>,
    pub last_submit: Option<Instant>,
    pub restore: Option<String>,
    pub corrupted: u32,
    pub native: NativeSlot,
}

#[derive(Default)]
pub(crate) struct NativeSlot {
    connected: bool,
    refused: bool,
    idle: bool,
    draft: bool,
    in_flight: bool,
    link: Option<mpsc::UnboundedSender<BridgeDown>>,
    waiting: Option<(String, oneshot::Sender<Result<(), String>>)>,
}

pub(crate) struct NativeGuard {
    session: Arc<Session>,
    pub(crate) down: mpsc::UnboundedReceiver<BridgeDown>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Hold {
    HumanDraft,
    UnsubmittedEnvelope,
    CorruptedSubmissions,
}

pub(crate) struct Delivery<'a> {
    session: &'a Session,
    _gate: tokio::sync::MutexGuard<'a, ()>,
}

pub(crate) enum DeliveryOutcome {
    Submitted,
    Unsubmitted(String),
    Failed(String),
}

#[derive(Debug)]
pub(crate) struct ResetError {
    pub(crate) code: &'static str,
    pub(crate) step: Option<(usize, usize)>,
    pub(crate) message: String,
}

pub(crate) struct ResetGuard<'a>(&'a Session);

impl Drop for ResetGuard<'_> {
    fn drop(&mut self) {
        self.0.resetting.store(false, Ordering::Release);
        self.0.message_notify.notify_one();
    }
}

pub(crate) struct AttachmentSlot {
    pub generation: u64,
    pub takeover: watch::Sender<bool>,
    pub closed: watch::Receiver<bool>,
}

impl Session {
    pub fn spawn(id: SessionId, spec: SessionSpec) -> anyhow::Result<Self> {
        let Some(program) = spec.argv.first() else {
            bail!("session command is empty");
        };
        if spec.size.cols == 0 || spec.size.rows == 0 {
            bail!("session dimensions must be nonzero");
        }
        // shortcut: daemon-only environment variables remain visible; use an exact
        // environment if a later isolation contract requires it.
        let mut env = spec
            .env
            .into_iter()
            .collect::<std::collections::HashMap<_, _>>();
        env.insert("TERM".into(), "xterm-256color".into());
        env.insert("COLORTERM".into(), "truecolor".into());
        let cwd = spec.cwd.clone();
        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), spec.argv[1..].to_vec())),
            working_directory: spec.cwd,
            env,
            drain_on_exit: true,
        };
        let mut pty = tty::new(&options, window_size(spec.size), 0)?;
        let mut reader = pty.file().try_clone()?;
        let mut writer = pty.file().try_clone()?;
        let master = pty.file().try_clone()?;
        let flags = rustix::fs::fcntl_getfl(&reader)?;
        rustix::fs::fcntl_setfl(&reader, flags & !rustix::fs::OFlags::NONBLOCK)?;
        let pid = Pid::from_raw(pty.child().id() as i32).context("invalid child pid")?;
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                emulator: Emulator::new(spec.size),
                size: spec.size,
                exit_code: None,
                attachment: None,
                generation: 0,
                pending_attachment: None,
                dirty: true,
                last_change: Instant::now(),
                hold: None,
                last_submit: None,
                restore: None,
                corrupted: 0,
                native: NativeSlot::default(),
            }),
            notify: Notify::new(),
        });
        let (input, mut writes) = mpsc::channel::<Option<Vec<u8>>>(256);
        std::thread::spawn(move || {
            while let Some(Some(bytes)) = writes.blocking_recv() {
                if let Err(error) = writer.write_all(&bytes) {
                    tracing::debug!(%error, "PTY writer stopped");
                    break;
                }
            }
        });
        let reader_state = shared.clone();
        let replies = input.clone();
        let reader_thread = std::thread::spawn(move || {
            let mut bytes = [0; 8192];
            loop {
                let count = match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(count) => count,
                    Err(error) if error.raw_os_error() == Some(5) => break,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        tracing::warn!(%error, "PTY reader stopped");
                        break;
                    }
                };
                let response = {
                    let mut state = reader_state.state.lock().unwrap_or_else(|e| e.into_inner());
                    let response = state.emulator.feed(&bytes[..count]);
                    state.dirty = true;
                    state.last_change = Instant::now();
                    response
                };
                if !response.is_empty() && replies.blocking_send(Some(response)).is_err() {
                    tracing::debug!("PTY reply writer closed");
                }
                // Rendering and delivery both observe output; wake both, retaining
                // one permit for a observer registering after this update.
                reader_state.notify.notify_waiters();
                reader_state.notify.notify_one();
            }
        });
        let drop_requested = Arc::new(AtomicBool::new(false));
        let child_done = Arc::new(AtomicBool::new(false));
        let monitor_drop = drop_requested.clone();
        let monitor_done = child_done.clone();
        let monitor_state = shared.clone();
        let stop_writer = input.clone();
        std::thread::spawn(move || {
            let mut dropping_since = None;
            let mut kill_sent = false;
            let status = loop {
                if let Some(ChildEvent::Exited(Some(status))) = pty.next_child_event() {
                    monitor_done.store(true, Ordering::Release);
                    break status.into_raw();
                }
                if monitor_drop.load(Ordering::Acquire) {
                    let since = dropping_since.get_or_insert_with(Instant::now);
                    if since.elapsed() >= Duration::from_secs(3) && !kill_sent {
                        kill_sent = true;
                        if let Err(error) = kill_process_group(pid, Signal::KILL) {
                            if error != rustix::io::Errno::SRCH {
                                tracing::warn!(%error, "PTY teardown signal failed");
                            }
                        }
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if reader_thread.join().is_err() {
                tracing::error!("PTY reader panicked");
            }
            let code = if status & 0x7f == 0 {
                status >> 8
            } else {
                128 + (status & 0x7f)
            };
            monitor_state
                .state
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .exit_code = Some(code);
            monitor_state.notify.notify_waiters();
            monitor_state.notify.notify_one();
            // Wake the writer even when the exited session remains in the registry.
            let _ = stop_writer.blocking_send(None);
            drop(pty);
        });
        Ok(Self {
            id,
            argv: spec.argv,
            cwd,
            name: spec.name,
            harness: spec.harness,
            channel: spec.channel,
            deliver: spec.deliver,
            reset: spec.reset,
            control_from: spec.control_from,
            watch: spec.watch,
            heartbeat: spec.heartbeat,
            token: spec.token,
            codex: spec.codex,
            input_gate: tokio::sync::Mutex::new(()),
            message_notify: Notify::new(),
            shared,
            input,
            master,
            pid,
            drop_requested,
            child_done,
            daemon_ended: AtomicBool::new(false),
            resetting: AtomicBool::new(false),
        })
    }

    pub(crate) fn codex(&self) -> Option<&Arc<crate::codex::Link>> {
        self.codex.as_ref()
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.cwd.as_deref()
    }

    pub fn harness(&self) -> Harness {
        self.harness
    }

    pub(crate) fn channel(&self) -> bool {
        self.channel
    }

    pub fn deliver(&self) -> Deliver {
        if self.resetting.load(Ordering::Acquire) {
            Deliver::Hold
        } else {
            self.deliver
        }
    }

    pub(crate) fn control_from(&self) -> &[String] {
        &self.control_from
    }
    pub(crate) fn watch(&self) -> &[String] {
        &self.watch
    }
    pub(crate) fn heartbeat(&self) -> Option<Duration> {
        self.heartbeat
    }

    pub(crate) fn last_change(&self) -> Instant {
        self.lock().last_change
    }

    pub(crate) fn resetting(&self) -> bool {
        self.resetting.load(Ordering::Acquire)
    }
    pub(crate) fn daemon_ended(&self) -> bool {
        self.daemon_ended.load(Ordering::Acquire)
    }

    pub(crate) fn token(&self) -> &str {
        &self.token
    }

    pub fn held(&self) -> bool {
        self.lock().hold.is_some()
    }

    pub(crate) fn note_human_input(&self, bytes: &[u8]) {
        if messaging::input_is_typing(bytes) {
            let mut state = self.lock();
            state.restore = None;
            state.hold.get_or_insert(Hold::HumanDraft);
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.lock();
        state.hold = None;
        state.restore = None;
        state.corrupted = 0;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn note_submit(&self, clean_envelope: bool) {
        let mut state = self.lock();
        // shortcut: typing between submit and report can have its hold cleared;
        // use harness-side composition epochs if this observed race becomes noisy.
        if matches!(
            state.hold,
            Some(Hold::HumanDraft | Hold::UnsubmittedEnvelope)
        ) {
            state.hold = None;
            state.restore = None;
        }
        if clean_envelope {
            state.corrupted = 0;
        }
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn note_corrupted(&self, draft: Option<String>) -> bool {
        let mut state = self.lock();
        state.corrupted += 1;
        let capped = state.corrupted >= messaging::MAX_CORRUPTED_SUBMISSIONS;
        state.restore =
            draft.filter(|text| !text.is_empty() && text.len() <= messaging::MAX_RESTORE_BYTES);
        if capped {
            state.hold = Some(Hold::CorruptedSubmissions);
        } else if state.restore.is_some() {
            state.hold = Some(Hold::HumanDraft);
        }
        capped
    }

    pub(crate) async fn restore_draft(&self) {
        if self.lock().restore.is_none() {
            return;
        }
        let _gate = self.input_gate.lock().await;
        {
            let mut state = self.lock();
            if state.exit_code.is_some() {
                state.restore = None;
                return;
            }
            if state.restore.is_none()
                || !harness::ready(
                    self.harness,
                    &state.emulator.screen(),
                    state.emulator.is_scrolled(),
                )
            {
                return;
            }
        }
        // shortcut: a queue that never drains can hold this gate indefinitely;
        // add a timeout if an unattended session stalls on input backpressure.
        let Ok(permit) = self.input.reserve().await else {
            self.lock().restore = None;
            return;
        };
        let mut state = self.lock();
        if state.exit_code.is_some() {
            state.restore = None;
            return;
        }
        // Human input can cancel restoration while reserving waits; never paste
        // an old draft over that input, or into a newly displayed dialog.
        if !harness::ready(
            self.harness,
            &state.emulator.screen(),
            state.emulator.is_scrolled(),
        ) {
            return;
        }
        if let Some(text) = state.restore.take() {
            permit.send(Some(messaging::paste_bytes(&text)));
        }
    }

    pub(crate) fn message_notify(&self) -> &Notify {
        &self.message_notify
    }

    pub(crate) fn native_attach(self: &Arc<Self>) -> Option<NativeGuard> {
        let mut state = self.lock();
        if state.native.link.is_some() {
            return None;
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        state.native.link = Some(sender);
        drop(state);
        Some(NativeGuard {
            session: self.clone(),
            down: receiver,
        })
    }

    pub(crate) fn native_accept(&self) {
        let mut state = self.lock();
        state.native.connected = true;
        state.native.refused = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_refuse(&self) {
        let mut state = self.lock();
        state.native.refused = true;
        state.native.connected = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_state(&self, idle: bool, draft: bool, pending: bool) {
        let mut state = self.lock();
        state.native.idle = idle;
        state.native.draft = draft;
        state.native.in_flight = pending;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_ack(&self, id: &str, result: Result<(), String>) {
        let sender = {
            let mut state = self.lock();
            let matches_id = state
                .native
                .waiting
                .as_ref()
                .is_some_and(|(waiting_id, _)| waiting_id == id);
            if !matches_id {
                return;
            }
            let Some((_, sender)) = state.native.waiting.take() else {
                return;
            };
            if result.is_ok() {
                state.native.in_flight = true;
            }
            sender
        };
        let _ = sender.send(result);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_received(&self) {
        let mut state = self.lock();
        state.native.in_flight = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_send(
        &self,
        id: &str,
        envelope: &str,
    ) -> Option<oneshot::Receiver<Result<(), String>>> {
        let mut state = self.lock();
        let native = &mut state.native;
        let link = native.link.as_ref()?;
        let (sender, receiver) = oneshot::channel();
        native.waiting = Some((id.to_owned(), sender));
        if link
            .send(BridgeDown::Deliver {
                id: id.to_owned(),
                envelope: envelope.to_owned(),
            })
            .is_err()
        {
            native.waiting = None;
            return None;
        }
        drop(state);
        Some(receiver)
    }

    pub(crate) fn native_reason(&self) -> Option<&'static str> {
        let state = self.lock();
        if state.native.refused {
            Some("channel_refused")
        } else if !state.native.connected {
            Some("channel_down")
        } else if state.native.draft {
            Some("draft_present")
        } else if state.native.in_flight {
            Some("in_flight")
        } else {
            None
        }
    }
    fn reset_native_reason(&self) -> Option<&'static str> {
        if self.harness != Harness::Omp {
            return None;
        }
        let state = self.lock();
        if !state.native.connected || state.native.refused {
            Some("not_ready")
        } else if state.native.draft {
            Some("draft_present")
        } else if !state.native.idle || state.native.in_flight || state.native.waiting.is_some() {
            Some("busy")
        } else {
            None
        }
    }

    fn reset_gate_reason(&self) -> Option<&'static str> {
        if self.exit_code().is_some() {
            return Some(messaging::code::EXITED);
        }
        {
            let state = self.lock();
            if let Some(hold) = state.hold {
                return Some(match hold {
                    Hold::HumanDraft => messaging::code::DRAFT_PRESENT,
                    Hold::UnsubmittedEnvelope | Hold::CorruptedSubmissions => messaging::code::HELD,
                });
            }
            if !harness::ready(
                self.harness,
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            ) {
                return Some(messaging::code::NOT_READY);
            }
        }
        self.reset_native_reason()
    }

    fn reset_error(
        code: &'static str,
        step: Option<(usize, usize)>,
        message: impl Into<String>,
    ) -> ResetError {
        ResetError {
            code,
            step,
            message: message.into(),
        }
    }

    pub(crate) fn begin_reset(&self) -> Result<ResetGuard<'_>, ResetError> {
        self.resetting
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| {
                Self::reset_error(messaging::code::BUSY, None, "reset is already running")
            })?;
        self.message_notify.notify_one();
        Ok(ResetGuard(self))
    }

    pub(crate) async fn reset_sequence(&self) -> Result<u32, ResetError> {
        let _guard = self.begin_reset()?;
        let steps = if self.reset.is_empty() {
            vec!["/clear".to_owned()]
        } else {
            self.reset.clone()
        };
        let total = steps.len();
        for (index, step) in steps.iter().enumerate() {
            let number = index + 1;
            if let Some(code) = self.reset_gate_reason() {
                return Err(Self::reset_error(
                    code,
                    Some((number, total)),
                    format!("reset gate rejected step {number} of {total}"),
                ));
            }
            let Some(delivery) = self.begin_delivery().await else {
                let code = self.reset_gate_reason().unwrap_or(messaging::code::HELD);
                return Err(Self::reset_error(
                    code,
                    Some((number, total)),
                    format!("reset gate rejected step {number} of {total}"),
                ));
            };
            match delivery
                .submit(messaging::paste_bytes(step), messaging::PASTE_GAP)
                .await
            {
                DeliveryOutcome::Submitted => {}
                DeliveryOutcome::Unsubmitted(reason) => {
                    return Err(Self::reset_error(
                        messaging::code::HELD,
                        Some((number, total)),
                        format!("step {number} of {total} was not submitted: {reason}"),
                    ));
                }
                DeliveryOutcome::Failed(reason) => {
                    return Err(Self::reset_error(
                        messaging::code::WRITE_FAILED,
                        Some((number, total)),
                        format!("step {number} of {total} failed: {reason}"),
                    ));
                }
            }
            let deadline = Instant::now() + RESET_TIMEOUT;
            tokio::time::sleep(RESET_SETTLE).await;
            loop {
                if self.exit_code().is_some() {
                    return Err(Self::reset_error(
                        messaging::code::EXITED,
                        Some((number, total)),
                        format!("session exited during step {number} of {total}"),
                    ));
                }
                if self.reset_gate_reason().is_none() {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(Self::reset_error(
                        messaging::code::STEP_TIMEOUT,
                        Some((number, total)),
                        format!("step {number} of {total} did not become ready"),
                    ));
                }
                tokio::time::sleep(RESET_POLL).await;
            }
        }
        Ok(total as u32)
    }

    pub(crate) async fn begin_delivery(&self) -> Option<Delivery<'_>> {
        let gate = self.input_gate.lock().await;
        let state = self.lock();
        if state.exit_code.is_some()
            || state.hold.is_some()
            || !harness::ready(
                self.harness,
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            )
        {
            return None;
        }
        Some(Delivery {
            session: self,
            _gate: gate,
        })
    }

    /// Blocking enqueue for synchronous callers; async callers use `enqueue`.
    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
        let _gate = self.input_gate.blocking_lock();
        let permit = self.input.blocking_send(Some(bytes.to_vec()));
        permit.context("session input is closed")
    }

    pub fn resize(&self, size: Size) -> anyhow::Result<()> {
        self.resize_locked(&mut self.lock(), size)?;
        self.notify().notify_one();
        Ok(())
    }

    /// Exit code once the child has exited.
    pub fn exit_code(&self) -> Option<i32> {
        self.lock().exit_code
    }

    pub fn notify(&self) -> &Notify {
        &self.shared.notify
    }

    pub fn size(&self) -> Size {
        self.lock().size
    }

    pub fn attached(&self) -> bool {
        self.lock().attachment.is_some()
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) async fn enqueue(&self, bytes: Vec<u8>) -> anyhow::Result<bool> {
        let _gate = self.input_gate.lock().await;
        let permit = match self.input.reserve().await {
            Ok(permit) => permit,
            Err(_) if self.exit_code().is_some() => return Ok(false),
            Err(error) => return Err(error).context("session input is closed"),
        };
        let state = self.lock();
        if state.exit_code.is_some() {
            return Ok(false);
        }
        // Publishing exit and committing input are serialized by the same lock.
        permit.send(Some(bytes));
        Ok(true)
    }

    pub(crate) fn resize_locked(&self, state: &mut State, size: Size) -> anyhow::Result<()> {
        if size.cols == 0 || size.rows == 0 {
            bail!("session dimensions must be nonzero");
        }
        // Exited sessions retain a drawable screen but no longer have a slave.
        if state.exit_code.is_none() {
            rustix::termios::tcsetwinsize(
                &self.master,
                rustix::termios::Winsize {
                    ws_row: size.rows,
                    ws_col: size.cols,
                    ws_xpixel: 0,
                    ws_ypixel: 0,
                },
            )?;
        }
        state.emulator.resize(size);
        state.size = size;
        state.dirty = true;
        Ok(())
    }

    pub(crate) async fn kill(&self, graceful: bool) -> anyhow::Result<()> {
        self.daemon_ended.store(true, Ordering::Release);
        if self.exit_code().is_some() {
            return Ok(());
        }
        if graceful && self.harness != Harness::Generic && self.graceful_exit().await {
            return Ok(());
        }
        if let Err(error) = kill_process_group(self.pid, Signal::HUP) {
            if error != rustix::io::Errno::SRCH {
                return Err(error.into());
            }
        }
        if tokio::time::timeout(Duration::from_secs(3), self.wait_exit())
            .await
            .is_err()
        {
            if !self.child_done.load(Ordering::Acquire) {
                if let Err(error) = kill_process_group(self.pid, Signal::KILL) {
                    if error != rustix::io::Errno::SRCH {
                        return Err(error.into());
                    }
                }
            }
            self.wait_exit().await;
        }
        Ok(())
    }

    fn log_graceful(&self, outcome: &str, code: Option<&str>, error: Option<&anyhow::Error>) {
        match (code, error) {
            (Some(code), Some(error)) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                code = code,
                %error,
                "graceful exit"
            ),
            (Some(code), None) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                code = code,
                "graceful exit"
            ),
            (None, Some(error)) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                %error,
                "graceful exit"
            ),
            (None, None) => tracing::info!(
                session = %self.id.0,
                outcome = outcome,
                "graceful exit"
            ),
        }
    }

    async fn graceful_exit(&self) -> bool {
        let _guard = match self.begin_reset() {
            Ok(guard) => guard,
            Err(error) => {
                self.log_graceful("skipped", Some(error.code), None);
                return false;
            }
        };
        // shortcut: a human can start typing between this gate check and the key;
        // re-check inside the input lock if a composer corruption is observed.
        if let Some(code) = self.reset_gate_reason() {
            self.log_graceful("skipped", Some(code), None);
            return false;
        }
        let key = vec![0x04];
        match self.enqueue(key.clone()).await {
            Ok(true) => {}
            Ok(false) => {
                self.log_graceful("exited", None, None);
                return true;
            }
            Err(error) => {
                self.log_graceful("skipped", Some("input_closed"), Some(&error));
                return false;
            }
        }
        if self.harness == Harness::Claude {
            // shortcut: fixed 300 ms against Claude's pending-exit window; use a
            // screen-based wait if a Claude release shortens that window.
            tokio::time::sleep(CLAUDE_EXIT_KEY_GAP).await;
            match self.enqueue(key).await {
                Ok(true) => {}
                Ok(false) => {
                    self.log_graceful("exited", None, None);
                    return true;
                }
                Err(error) => {
                    self.log_graceful("skipped", Some("input_closed"), Some(&error));
                    return false;
                }
            }
        }
        let exited = tokio::time::timeout(GRACEFUL_TIMEOUT, self.wait_exit())
            .await
            .is_ok();
        self.log_graceful(if exited { "exited" } else { "timeout" }, None, None);
        exited
    }

    pub(crate) async fn wait_exit(&self) {
        loop {
            let notified = self.notify().notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.exit_code().is_some() {
                return;
            }
            notified.await;
        }
    }
}

impl Drop for NativeGuard {
    fn drop(&mut self) {
        let mut state = self.session.lock();
        state.native.connected = false;
        state.native.link = None;
        state.native.waiting = None;
        state.native.draft = false;
        state.native.in_flight = false;
        drop(state);
        self.session.message_notify.notify_one();
    }
}

impl Delivery<'_> {
    pub(crate) async fn submit(self, paste: Vec<u8>, gap: Duration) -> DeliveryOutcome {
        // shortcut: a child that never reads can hold this gate indefinitely under
        // input backpressure; add a timeout if an unattended session stalls on it.
        let Ok(permit) = self.session.input.reserve().await else {
            return DeliveryOutcome::Failed("session input is closed".into());
        };
        permit.send(Some(paste));
        tokio::time::sleep(gap).await;
        {
            let mut state = self.session.lock();
            if let Some(outcome) = self.recheck(&mut state) {
                return outcome;
            }
        }
        let Ok(permit) = self.session.input.reserve().await else {
            return DeliveryOutcome::Failed("session input is closed".into());
        };
        let mut state = self.session.lock();
        // Reserving may wait under backpressure; observe again before committing CR.
        if let Some(outcome) = self.recheck(&mut state) {
            return outcome;
        }
        permit.send(Some(vec![b'\r']));
        state.last_submit = Some(Instant::now());
        DeliveryOutcome::Submitted
    }

    fn recheck(&self, state: &mut State) -> Option<DeliveryOutcome> {
        if state.exit_code.is_some() {
            return Some(DeliveryOutcome::Failed(
                "recipient exited during delivery".into(),
            ));
        }
        let human_input = state.hold.is_some();
        if human_input
            || !harness::ready(
                self.session.harness,
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            )
        {
            state.hold = Some(Hold::UnsubmittedEnvelope);
            return Some(DeliveryOutcome::Unsubmitted(
                if human_input {
                    "human_input"
                } else {
                    "screen_not_ready"
                }
                .into(),
            ));
        }
        None
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.drop_requested.store(true, Ordering::Release);
        if !self.child_done.load(Ordering::Acquire) {
            if let Err(error) = kill_process_group(self.pid, Signal::HUP) {
                if error != rustix::io::Errno::SRCH {
                    tracing::warn!(%error, "PTY drop signal failed");
                }
            }
        }
    }
}
