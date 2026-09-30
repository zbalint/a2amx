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
use tokio::sync::{Notify, mpsc, watch};

use crate::emulator::{Emulator, Size, window_size};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

pub struct SessionSpec {
    pub argv: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
    pub size: Size,
    pub env: Vec<(String, String)>,
}

pub struct Session {
    id: SessionId,
    pub(crate) argv: Vec<String>,
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
            shared,
            input,
            master,
            pid,
            drop_requested,
            child_done,
        })
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    /// Blocking enqueue for synchronous callers; async callers use `enqueue`.
    pub fn write_input(&self, bytes: &[u8]) -> anyhow::Result<()> {
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
