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

pub struct SessionSpec {
    pub argv: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub size: Size,
    pub env: Vec<(String, String)>,
    pub name: Option<String>,
    pub harness: Harness,
    pub channel: bool,
    pub deliver: Deliver,
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
        self.deliver
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

    pub(crate) fn native_state(&self, draft: bool, pending: bool) {
        let mut state = self.lock();
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

    pub(crate) async fn kill(&self) -> anyhow::Result<()> {
        if self.exit_code().is_some() {
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

    async fn wait_exit(&self) {
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
