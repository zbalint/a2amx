use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use rustix::process::{Pid, Signal};

pub const READY_LINE: &str = "gateway ready";
pub const READY_WAIT: Duration = Duration::from_secs(10);
pub const STOP_WAIT: Duration = Duration::from_secs(5);
const TAIL_LINES: usize = 20;
const TAIL_LINE_CHARS: usize = 200;
const MAX_PROGRESS_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum LockError {
    Held,
    Other(anyhow::Error),
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Held => f.write_str("gateway lock is held"),
            Self::Other(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for LockError {}

#[derive(Debug)]
pub struct GatewayLock {
    file: File,
}

impl GatewayLock {
    pub fn as_fd(&self) -> BorrowedFd<'_> {
        self.file.as_fd()
    }
}

fn layout_for(home: &Path) -> crate::layout::Layout {
    crate::layout::Layout::new(home.to_owned())
}

pub fn acquire(home: &Path) -> std::result::Result<GatewayLock, LockError> {
    let layout = layout_for(home);
    layout.ensure().map_err(LockError::Other)?;
    let path = layout.gateway_lock();
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .mode(0o600)
        .open(&path)
    {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(LockError::Other(error.into())),
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| LockError::Other(error.into()))?;
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|error| LockError::Other(error.into()))?;
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(GatewayLock { file }),
        Err(error) if error == rustix::io::Errno::AGAIN => Err(LockError::Held),
        Err(error) => Err(LockError::Other(error.into())),
    }
}

pub fn write_pid(home: &Path, pid: u32) -> Result<()> {
    crate::relay_files::atomic_write(
        &layout_for(home).gateway_pid(),
        format!("{pid}\n").as_bytes(),
        0o600,
    )
}

fn read_pid(home: &Path) -> Option<u32> {
    let mut bytes = Vec::new();
    File::open(layout_for(home).gateway_pid())
        .ok()?
        .take(33)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > 32 {
        return None;
    }
    let text = std::str::from_utf8(&bytes).ok()?.trim();
    (!text.is_empty()).then(|| text.parse().ok()).flatten()
}

pub fn open_log(home: &Path) -> Result<(File, u64)> {
    let path = layout_for(home).gateway_log();
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;
    let metadata = file.metadata()?;
    let valid = metadata.file_type().is_file()
        && metadata.uid() == rustix::process::getuid().as_raw()
        && metadata.nlink() == 1;
    if !valid {
        bail!(
            "refusing to use {}: not a regular file owned by you with one link",
            path.display()
        );
    }
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok((file, metadata.len()))
}

#[derive(Debug, Eq, PartialEq)]
pub struct LogProgress {
    pub ready: bool,
    pub tail: String,
}

pub fn log_progress(log: &File, from: u64) -> Result<LogProgress> {
    let mut bytes = vec![0_u8; MAX_PROGRESS_BYTES];
    let mut total = 0;
    while total < bytes.len() {
        let count = log.read_at(&mut bytes[total..], from + total as u64)?;
        if count == 0 {
            break;
        }
        total += count;
    }
    bytes.truncate(total);
    let ready = String::from_utf8_lossy(&bytes)
        .lines()
        .any(|line| line == READY_LINE);
    Ok(LogProgress {
        ready,
        tail: sanitize_tail(&bytes),
    })
}

pub fn sanitize_tail(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let lines: Vec<String> = text
        .lines()
        .map(|line| {
            line.chars()
                .take(TAIL_LINE_CHARS)
                .map(|ch| if ch.is_control() { '?' } else { ch })
                .collect()
        })
        .collect();
    let start = lines.len().saturating_sub(TAIL_LINES);
    lines.into_iter().skip(start).collect::<Vec<_>>().join("\n")
}

#[derive(Debug, Eq, PartialEq)]
pub enum State {
    Running { pid: Option<u32> },
    NotRunning,
}

pub fn state(home: &Path) -> Result<State> {
    let path = layout_for(home).gateway_lock();
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(State::NotRunning),
        Err(error) => return Err(error.into()),
    };
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(State::NotRunning),
        Err(error) if error == rustix::io::Errno::AGAIN => Ok(State::Running {
            pid: read_pid(home),
        }),
        Err(error) => Err(error.into()),
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum StopOutcome {
    NotRunning,
    Stopped { pid: u32 },
}

pub fn stop(home: &Path) -> Result<StopOutcome> {
    let State::Running { pid } = state(home)? else {
        return Ok(StopOutcome::NotRunning);
    };
    let pid = pid.ok_or_else(|| {
        anyhow::anyhow!(
            "the gateway is running but gateway.pid is missing or unreadable; stop it by hand (the process whose command line contains \"gateway run\")"
        )
    })?;
    let command_line = match File::open(format!("/proc/{pid}/cmdline")) {
        Ok(file) => {
            let mut bytes = Vec::new();
            file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 64 * 1024 {
                bail!(
                    "refusing to signal pid {pid}: it is not the gateway for this a2amx directory"
                );
            }
            bytes
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "the gateway holds the lock but its process {pid} is gone (killed by hand?); end it with: kill -TERM -- -{pid}"
            )
        }
        Err(_) => {
            bail!("refusing to signal pid {pid}: it is not the gateway for this a2amx directory")
        }
    };
    let args = command_line
        .split(|byte| *byte == 0)
        .filter(|arg| !arg.is_empty())
        .collect::<Vec<_>>();
    let canonical = fs::canonicalize(home)?;
    let expected = [
        b"--home".as_slice(),
        canonical.as_os_str().as_encoded_bytes(),
        b"gateway",
        b"run",
    ];
    let verified = args
        .windows(expected.len())
        .any(|window| window == expected);
    if !verified {
        // shortcut: this command-line match guards stale pid files; it is not process authentication.
        bail!("refusing to signal pid {pid}: it is not the gateway for this a2amx directory");
    }
    let target =
        Pid::from_raw(pid as i32).ok_or_else(|| anyhow::anyhow!("invalid process id {pid}"))?;
    let target_group = rustix::process::getpgid(Some(target))?;
    let own = Pid::from_raw(std::process::id() as i32)
        .ok_or_else(|| anyhow::anyhow!("invalid own process id"))?;
    let own_group = rustix::process::getpgid(Some(own))?;
    let signal_result = if target_group == target && target_group != own_group {
        rustix::process::kill_process_group(target, Signal::TERM)
    } else {
        rustix::process::kill_process(target, Signal::TERM)
    };
    if let Err(error) = signal_result
        && error != rustix::io::Errno::SRCH
    {
        return Err(error.into());
    }
    let deadline = Instant::now() + STOP_WAIT;
    loop {
        if matches!(state(home)?, State::NotRunning) {
            let _ = fs::remove_file(layout_for(home).gateway_pid());
            return Ok(StopOutcome::Stopped { pid });
        }
        if Instant::now() >= deadline {
            bail!("the gateway (pid {pid}) did not stop within 5s");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
