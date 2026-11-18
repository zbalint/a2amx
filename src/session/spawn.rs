use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::BorrowedFd;
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use alacritty_terminal::tty::{self, ChildEvent, EventedPty};
use anyhow::{Context, bail};
use rustix::process::{Pid, Signal, kill_process_group};
use tokio::sync::{Notify, mpsc, oneshot};

use crate::emulator::{Emulator, window_size};

use super::{NativeSlot, Session, SessionId, SessionSpec, Shared, State};

type SpawnTask = Box<dyn FnOnce() + Send + 'static>;

struct SpawnJob {
    task: SpawnTask,
}

static SPAWN_JOBS: LazyLock<Result<std_mpsc::Sender<SpawnJob>, String>> = LazyLock::new(|| {
    let (sender, receiver) = std_mpsc::channel();
    match std::thread::Builder::new()
        .name("a2amx-spawn".to_owned())
        .spawn(move || spawn_worker(receiver))
    {
        Ok(_) => Ok(sender),
        Err(error) => Err(error.to_string()),
    }
});

fn spawn_worker(receiver: std_mpsc::Receiver<SpawnJob>) {
    while let Ok(job) = receiver.recv() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job.task));
    }
}

fn confined_spawn_sender() -> anyhow::Result<&'static std_mpsc::Sender<SpawnJob>> {
    SPAWN_JOBS
        .as_ref()
        .map_err(|error| anyhow::anyhow!("cannot start confined spawn worker: {error}"))
}

pub(crate) async fn spawn_on_worker<T, F>(task: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    let (reply, result) = oneshot::channel();
    confined_spawn_sender()?
        .send(SpawnJob {
            task: Box::new(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(task))
                    .unwrap_or_else(|_| {
                        Err(anyhow::anyhow!("confined spawn worker task panicked"))
                    });
                let _ = reply.send(result);
            }),
        })
        .map_err(|_| anyhow::anyhow!("confined spawn worker stopped"))?;
    result
        .await
        .context("confined spawn worker dropped its reply")?
}

fn writer_loop(mut writes: mpsc::Receiver<Option<Vec<u8>>>, mut writer: File) {
    while let Some(Some(bytes)) = writes.blocking_recv() {
        if let Err(error) = writer.write_all(&bytes) {
            tracing::debug!(%error, "PTY writer stopped");
            break;
        }
    }
}

fn reader_loop(mut reader: File, state: Arc<Shared>, replies: mpsc::Sender<Option<Vec<u8>>>) {
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
            let mut state = state.state.lock().unwrap_or_else(PoisonError::into_inner);
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
        state.notify.notify_waiters();
        state.notify.notify_one();
    }
}

struct MonitorArgs {
    pty: tty::Pty,
    pid: Pid,
    drop_requested: Arc<AtomicBool>,
    child_done: Arc<AtomicBool>,
    state: Arc<Shared>,
    stop_writer: mpsc::Sender<Option<Vec<u8>>>,
    reader_thread: std::thread::JoinHandle<()>,
}

fn monitor_loop(args: MonitorArgs) {
    let MonitorArgs {
        mut pty,
        pid,
        drop_requested,
        child_done,
        state,
        stop_writer,
        reader_thread,
    } = args;
    let mut dropping_since = None;
    let mut kill_sent = false;
    let status = loop {
        if let Some(ChildEvent::Exited(Some(status))) = pty.next_child_event() {
            child_done.store(true, Ordering::Release);
            break status.into_raw();
        }
        if drop_requested.load(Ordering::Acquire) {
            let since = dropping_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= Duration::from_secs(3) && !kill_sent {
                kill_sent = true;
                if let Err(error) = kill_process_group(pid, Signal::KILL)
                    && error != rustix::io::Errno::SRCH
                {
                    tracing::warn!(%error, "PTY teardown signal failed");
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    if reader_thread.join().is_err() {
        tracing::error!("PTY reader panicked");
    }
    let code = exit_code_from_status(status);
    state
        .state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .exit_code = Some(code);
    state.notify.notify_waiters();
    state.notify.notify_one();
    // Wake the writer even when the exited session remains in the registry.
    let _ = stop_writer.blocking_send(None);
    drop(pty);
}

fn exit_code_from_status(status: i32) -> i32 {
    if (status & 0x7f) == 0 {
        status >> 8
    } else {
        128 + (status & 0x7f)
    }
}

pub(crate) fn mark_inherited_fds_cloexec() -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        const CLOSE_RANGE_CLOEXEC: libc::c_uint = 4;
        // libc is used because rustix 1.1 has no close_range wrapper.
        // SAFETY: close_range only changes this process's descriptor flags;
        // the kernel applies it atomically without dereferencing user pointers.
        let result = unsafe {
            libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                CLOSE_RANGE_CLOEXEC,
            )
        };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOSYS) {
            return Err(error).context("marking inherited descriptors close-on-exec");
        }
        for entry in std::fs::read_dir("/proc/self/fd").context("enumerating descriptors")? {
            let entry = entry?;
            let Ok(fd) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            if fd <= 2 {
                continue;
            }
            // SAFETY: the descriptor was returned by the live `/proc/self/fd`
            // directory and is borrowed only for these fcntl calls.
            let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
            let flags = rustix::io::fcntl_getfd(borrowed)?;
            rustix::io::fcntl_setfd(borrowed, flags | rustix::io::FdFlags::CLOEXEC)?;
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

impl Session {
    pub(crate) async fn spawn_confined(id: SessionId, spec: SessionSpec) -> anyhow::Result<Self> {
        spawn_on_worker(move || Self::spawn(id, spec)).await
    }

    pub fn spawn(id: SessionId, spec: SessionSpec) -> anyhow::Result<Self> {
        let confined = spec.confine.is_some();
        let confine = spec
            .confine
            .as_ref()
            .map(crate::confine::Plan::confine_info);
        let original_argv = spec.argv;
        let launch_argv = if let Some(plan) = spec.confine.as_ref() {
            let notice = crate::confine::notice(plan);
            crate::confine::launch_argv(plan, &notice, original_argv.clone(), spec.harness)?
        } else {
            original_argv.clone()
        };
        let Some(program) = launch_argv.first() else {
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
            shell: Some(tty::Shell::new(program.clone(), launch_argv[1..].to_vec())),
            working_directory: spec.cwd,
            env,
            drain_on_exit: true,
        };
        let pty = tty::new(&options, window_size(spec.size), 0)?;
        let reader = pty.file().try_clone()?;
        let writer = pty.file().try_clone()?;
        let master = pty.file().try_clone()?;
        let flags = rustix::fs::fcntl_getfl(&reader)?;
        rustix::fs::fcntl_setfl(&reader, flags & !rustix::fs::OFlags::NONBLOCK)?;
        let pid = Pid::from_raw(pty.child().id() as i32).context("invalid child pid")?;
        let started = Instant::now();
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
                needs_input_since: None,
                activity_since: None,
                hold: None,
                last_submit: None,
                last_human_input: None,
                restore: None,
                corrupted: 0,
                quota_report: (None, None),
                native: NativeSlot::default(),
            }),
            notify: Notify::new(),
        });
        let (input, writes) = mpsc::channel::<Option<Vec<u8>>>(256);
        std::thread::spawn(move || writer_loop(writes, writer));
        let reader_state = shared.clone();
        let replies = input.clone();
        let reader_thread = std::thread::spawn(move || reader_loop(reader, reader_state, replies));
        let drop_requested = Arc::new(AtomicBool::new(false));
        let child_done = Arc::new(AtomicBool::new(false));
        let monitor_drop = drop_requested.clone();
        let monitor_done = child_done.clone();
        let monitor_state = shared.clone();
        let stop_writer = input.clone();
        std::thread::spawn(move || {
            monitor_loop(MonitorArgs {
                pty,
                pid,
                drop_requested: monitor_drop,
                child_done: monitor_done,
                state: monitor_state,
                stop_writer,
                reader_thread,
            });
        });
        Ok(Self {
            id,
            started,
            argv: original_argv,
            cwd,
            name: spec.name,
            harness: spec.harness,
            channel: spec.channel,
            deliver: spec.deliver,
            reset: spec.reset,
            control_from: spec.control_from,
            watch: spec.watch,
            activity: spec.activity,
            heartbeat: spec.heartbeat,
            team: spec.team,
            role: spec.role,
            remote: spec.remote,
            token: spec.token,
            codex: spec.codex,
            confined,
            confine,
            shared,
            input_gate: tokio::sync::Mutex::new(()),
            message_notify: Notify::new(),
            input,
            master,
            pid,
            drop_requested,
            child_done,
            daemon_ended: AtomicBool::new(false),
            resetting: AtomicBool::new(false),
            ready_latched: AtomicBool::new(false),
        })
    }
}
