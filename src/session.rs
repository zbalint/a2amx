//! Session runtime: one PTY, its child, and its emulator.
//!
//! Blocking PTY reads, PTY writes, and the drop of the PTY (which blocks on the
//! child) stay off the async runtime. The PTY master is non-blocking and ends with
//! EIO when the child closes the slave. Resize uses our own ioctl, never
//! `Pty::on_resize`, which would exit the process on failure.

use std::fs::File;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::sync::{Notify, mpsc, oneshot, watch};

use crate::emulator::{Emulator, Size};
use crate::harness::{self, Deliver, Harness};
use crate::messaging;
use crate::wire::{Activity, BridgeDown, ConfineInfo, QuotaWindow};

mod exit;
mod native;
mod reset;
mod spawn;

pub(crate) use native::NativeGuard;
pub(crate) use spawn::{mark_inherited_fds_cloexec, spawn_on_worker};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);
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
    pub activity: Option<crate::wire::ActivityWatch>,
    pub heartbeat: Option<Duration>,
    pub team: Option<messaging::TeamScope>,
    pub role: Option<String>,
    pub remote: Option<messaging::RemoteMode>,
    pub token: String,
    pub(crate) codex: Option<Arc<crate::codex::Link>>,
    pub confine: Option<crate::confine::Plan>,
}

pub struct Session {
    id: SessionId,
    started: Instant,
    pub(crate) argv: Vec<String>,
    cwd: Option<std::path::PathBuf>,
    name: Option<String>,
    harness: Harness,
    channel: bool,
    deliver: Deliver,
    reset: Vec<String>,
    control_from: Vec<String>,
    watch: Vec<String>,
    activity: Option<crate::wire::ActivityWatch>,
    heartbeat: Option<Duration>,
    team: Option<messaging::TeamScope>,
    role: Option<String>,
    remote: Option<messaging::RemoteMode>,
    token: String,
    codex: Option<Arc<crate::codex::Link>>,
    confined: bool,
    confine: Option<ConfineInfo>,
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
    ready_latched: AtomicBool,
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
    pub needs_input_since: Option<(Instant, &'static str)>,
    pub activity_since: Option<(Activity, Instant)>,
    pub hold: Option<Hold>,
    pub last_submit: Option<Instant>,
    pub last_human_input: Option<Instant>,
    pub restore: Option<String>,
    pub corrupted: u32,
    pub quota_report: (Option<QuotaWindow>, Option<QuotaWindow>),
    pub native: NativeSlot,
}

#[derive(Default)]
pub(crate) struct NativeSlot {
    connected: bool,
    refused: bool,
    idle: bool,
    draft: bool,
    in_flight: bool,
    idle_since: Option<Instant>,
    reported_at: Option<Instant>,
    provider: Option<String>,
    diagnostics: Option<(u32, bool)>,
    link: Option<mpsc::UnboundedSender<BridgeDown>>,
    waiting: Option<(String, oneshot::Sender<Result<(), String>>)>,
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

pub(crate) struct AttachmentSlot {
    pub generation: u64,
    pub takeover: watch::Sender<bool>,
    pub closed: watch::Receiver<bool>,
    pub viewer: bool,
}

pub fn observe_activity(
    slot: &mut Option<(Activity, Instant)>,
    current: Activity,
    now: Instant,
) -> Duration {
    if let Some((activity, since)) = *slot
        && activity == current
    {
        return now.saturating_duration_since(since);
    }
    *slot = Some((current, now));
    Duration::ZERO
}

impl Session {
    pub(crate) fn codex(&self) -> Option<&Arc<crate::codex::Link>> {
        self.codex.as_ref()
    }

    pub fn confined(&self) -> bool {
        self.confined
    }
    pub fn confine_info(&self) -> Option<ConfineInfo> {
        self.confine.clone()
    }
    pub fn confine_auto(&self) -> bool {
        self.confine.as_ref().is_some_and(|info| info.auto)
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

    pub(crate) fn set_quota_report(
        &self,
        five_hour: Option<QuotaWindow>,
        weekly: Option<QuotaWindow>,
    ) {
        let mut state = self.lock();
        if five_hour.is_some() {
            state.quota_report.0 = five_hour;
        }
        if weekly.is_some() {
            state.quota_report.1 = weekly;
        }
    }

    pub(crate) fn quota_report(&self, now_unix: u64) -> (Option<QuotaWindow>, Option<QuotaWindow>) {
        let state = self.lock();
        (
            state
                .quota_report
                .0
                .filter(|window| window.resets_at > now_unix),
            state
                .quota_report
                .1
                .filter(|window| window.resets_at > now_unix),
        )
    }

    pub(crate) fn observe_ready(&self, state: &State) -> bool {
        let ready = harness::ready(
            self.harness,
            &state.emulator.screen(),
            state.emulator.is_scrolled(),
        );
        if ready {
            self.ready_latched.store(true, Ordering::Release);
        }
        ready
    }

    pub(crate) fn starting(&self) -> bool {
        self.exit_code().is_none()
            && !matches!(self.harness, Harness::Generic | Harness::Agy)
            && !self.ready_latched.load(Ordering::Acquire)
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
    pub(crate) fn activity(&self) -> Option<&crate::wire::ActivityWatch> {
        self.activity.as_ref()
    }
    pub(crate) fn heartbeat(&self) -> Option<Duration> {
        self.heartbeat
    }

    pub fn team(&self) -> Option<&messaging::TeamScope> {
        self.team.as_ref()
    }
    pub fn party(&self) -> messaging::Party<'_> {
        messaging::Party {
            name: self.name.as_deref(),
            team: self.team(),
        }
    }

    pub fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    pub fn remote(&self) -> Option<messaging::RemoteMode> {
        self.remote
    }

    pub(crate) fn note_activity(&self, current: Activity, now: Instant) -> Duration {
        let mut state = self.lock();
        observe_activity(&mut state.activity_since, current, now)
    }

    pub(crate) fn note_needs_input(&self, signal: Option<&'static str>, now: Instant) {
        let mut state = self.lock();
        match signal {
            Some(signal)
                if state
                    .needs_input_since
                    .is_some_and(|(_, previous)| previous == signal) => {}
            Some(signal) => state.needs_input_since = Some((now, signal)),
            None => state.needs_input_since = None,
        }
    }

    pub fn needs_input(&self) -> Option<&'static str> {
        let state = self.lock();
        if state.exit_code.is_some() {
            return None;
        }
        state.needs_input_since.and_then(|(since, signal)| {
            (Instant::now().saturating_duration_since(since) >= crate::daemon::NEEDS_INPUT_AFTER)
                .then_some(signal)
        })
    }

    pub(crate) fn started(&self) -> Instant {
        self.started
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
            state.last_human_input = Some(Instant::now());
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

    pub(crate) async fn begin_delivery(&self) -> Option<Delivery<'_>> {
        let gate = self.input_gate.lock().await;
        let mut state = self.lock();
        if self.harness == Harness::Agy && self.draft_is_stale(&state) {
            state.hold = None;
        }
        if state.exit_code.is_some()
            || state.hold.is_some()
            || !harness::ready(
                self.harness,
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            )
            || (self.harness == Harness::Agy
                && !harness::agy_composer_idle(
                    &state.emulator.screen(),
                    state.emulator.is_scrolled(),
                ))
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

    pub fn attachment_viewer(&self) -> Option<bool> {
        self.lock().attachment.as_ref().map(|slot| slot.viewer)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.shared
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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
        if !self.child_done.load(Ordering::Acquire)
            && let Err(error) = kill_process_group(self.pid, Signal::HUP)
            && error != rustix::io::Errno::SRCH
        {
            tracing::warn!(%error, "PTY drop signal failed");
        }
    }
}
