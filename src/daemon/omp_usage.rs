use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process_group};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::sync::watch;

use crate::quota;
use crate::wire::QuotaInfo;

const OMP_USAGE_REFRESH: Duration = Duration::from_secs(300);
const OMP_USAGE_STALE: Duration = Duration::from_secs(900);
const OMP_USAGE_TIMEOUT: Duration = Duration::from_secs(20);
const OMP_USAGE_MAX_OUTPUT: usize = 1024 * 1024;

pub(super) struct OmpUsage {
    cache: Mutex<Option<(Instant, BTreeMap<String, QuotaInfo>)>>,
    attempt: Mutex<Option<Instant>>,
}

impl OmpUsage {
    pub(super) fn new() -> Self {
        Self {
            cache: Mutex::new(None),
            attempt: Mutex::new(None),
        }
    }

    pub(super) fn current(&self) -> Option<BTreeMap<String, QuotaInfo>> {
        let cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        let (at, quota) = cache.as_ref()?;
        (at.elapsed() < OMP_USAGE_STALE).then_some(quota.clone())
    }

    pub(super) async fn poll(
        &self,
        command: Option<(String, Option<PathBuf>)>,
        bwrap_path: &OsStr,
        stopping: &mut watch::Receiver<bool>,
    ) -> bool {
        let Some((argv0, cwd)) = command else {
            return false;
        };
        {
            let mut attempt = self.attempt.lock().unwrap_or_else(PoisonError::into_inner);
            if attempt.is_some_and(|at| at.elapsed() < OMP_USAGE_REFRESH) {
                return false;
            }
            *attempt = Some(Instant::now());
        }
        match run_omp_usage(&argv0, cwd.as_deref(), bwrap_path, stopping).await {
            Ok(quota) => {
                *self.cache.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((Instant::now(), quota));
                false
            }
            Err("shutdown") => true,
            Err(failure) => {
                tracing::debug!(failure, "omp usage poll failed");
                false
            }
        }
    }
}

async fn run_omp_usage(
    argv0: &str,
    cwd: Option<&Path>,
    path_var: &OsStr,
    stopping: &mut watch::Receiver<bool>,
) -> Result<BTreeMap<String, QuotaInfo>, &'static str> {
    let executable = if argv0.contains('/') {
        PathBuf::from(argv0)
    } else {
        let argv0 = argv0.to_owned();
        let path_var = path_var.to_os_string();
        tokio::task::spawn_blocking(move || crate::confine::find_executable(&argv0, &path_var))
            .await
            .map_err(|_| "spawn")?
            .ok_or("spawn")?
    };
    let mut command = Command::new(executable);
    command
        .args(["usage", "--json", "--no-extensions", "--redact"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_remove("A2AMX_TOKEN")
        .env_remove("A2AMX_ADDR")
        .process_group(0)
        .kill_on_drop(true);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let mut child = command.spawn().map_err(|_| "spawn")?;
    let pid = child.id();
    let stdout = child.stdout.take().ok_or("stdout")?;
    let mut output = Vec::with_capacity(OMP_USAGE_MAX_OUTPUT + 1);
    let result = tokio::select! {
        _ = stopping.changed() => {
            kill_omp_process_group(pid);
            let _ = child.wait().await;
            return Err("shutdown");
        }
        result = tokio::time::timeout(OMP_USAGE_TIMEOUT, async {
            let mut limited = stdout.take((OMP_USAGE_MAX_OUTPUT + 1) as u64);
            limited.read_to_end(&mut output).await.map_err(|_| "read")?;
            if output.len() > OMP_USAGE_MAX_OUTPUT {
                return Err("output");
            }
            child.wait().await.map_err(|_| "wait")
        }) => result.map_err(|_| "timeout").and_then(|result| result),
    };
    let status = match result {
        Ok(status) => status,
        Err(failure) => {
            kill_omp_process_group(pid);
            let _ = child.wait().await;
            return Err(failure);
        }
    };
    if !status.success() {
        return Err("exit");
    }
    let output = std::str::from_utf8(&output).map_err(|_| "parse")?;
    quota::parse_omp_usage(output).ok_or("parse")
}

fn kill_omp_process_group(pid: Option<u32>) {
    let Some(pid) = pid else {
        return;
    };
    let Some(pid) = Pid::from_raw(pid as _) else {
        return;
    };
    if let Err(error) = kill_process_group(pid, Signal::KILL)
        && error != rustix::io::Errno::SRCH
    {
        tracing::debug!(%error, "omp usage process-group kill failed");
    }
}
