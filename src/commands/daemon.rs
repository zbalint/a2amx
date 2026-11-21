//! Daemon foreground, background, status, and stop commands.

use super::home::{migrate_old_state, warn_explicit_old_layout};
use crate::client::Client;
use crate::daemon::{self, Daemon, DaemonConfig};
use crate::terminal::confirm;
use crate::terminal::{write_stderr, write_stdout};
use anyhow::{Context, anyhow};
use std::io::{self};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DAEMON_STOP_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) async fn run_daemon(
    home: PathBuf,
    listen: Vec<std::net::SocketAddr>,
    host_name: Option<String>,
    owner_away_secs: Option<u64>,
    explicit_home: bool,
) -> anyhow::Result<()> {
    let migration_layout = crate::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &migration_layout).await?;
    if explicit_home {
        warn_explicit_old_layout(&home);
    }
    let overrides = crate::config::Overrides {
        listen,
        host_name,
        owner_away_secs,
    };
    let layout = crate::layout::Layout::new(home.clone());
    let loaded = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        layout.ensure()?;
        crate::config::ensure_daemon_file(&layout)?;
        crate::config::ensure_client_file(&layout)?;
        crate::config::load_daemon(&layout, &overrides)
    })
    .await??;
    let daemon = Daemon::start(DaemonConfig {
        root: home,
        listen: loaded.settings.listen,
        host_name: loaded.settings.host_name,
        limits: loaded.settings.limits,
        bwrap_path: None,
        env_path: None,
    })
    .await?;
    for addr in daemon.addrs() {
        write_stdout(format!("listening on {addr}\n").into_bytes()).await?;
    }

    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => { result?; },
        _ = sigterm.recv() => {},
        _ = daemon.stop_requested() => {},
    }
    daemon.shutdown().await
}

pub(super) async fn run_daemon_background(
    home: PathBuf,
    listen: Vec<std::net::SocketAddr>,
    host_name: Option<String>,
    owner_away_secs: Option<u64>,
    explicit_home: bool,
) -> anyhow::Result<()> {
    if Client::connect(&home).await.is_ok() {
        let running = read_addrs(&home).await.unwrap_or_default();
        return Err(anyhow!(
            "a2amx daemon is already running (listening on {})",
            running.lines().next().unwrap_or("unknown address")
        ));
    }
    let migration_layout = crate::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &migration_layout).await?;
    if explicit_home {
        warn_explicit_old_layout(&home);
    }
    let log_path = crate::layout::Layout::new(home.clone()).daemon_log();
    let (log, exe) = {
        let home = home.clone();
        let log_path = log_path.clone();
        let overrides = crate::config::Overrides {
            listen: listen.clone(),
            host_name: host_name.clone(),
            owner_away_secs,
        };
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let layout = crate::layout::Layout::new(home.clone());
            layout.ensure()?;
            crate::config::load_daemon(&layout, &overrides)?;
            let log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(&log_path)
                .context("cannot open daemon.log")?;
            Ok((log, daemon::current_exe()?))
        })
        .await??
    };
    let mut command = std::process::Command::new(exe);
    command
        .arg("--home")
        .arg(&home)
        .args(["daemon", "start", "--foreground"]);
    if let Some(value) = owner_away_secs {
        command.arg("--owner-away-secs").arg(value.to_string());
    }
    for addr in &listen {
        command.arg("--listen").arg(addr.to_string());
    }
    if let Some(name) = &host_name {
        command.arg("--host-name").arg(name);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // SAFETY: the closure only calls `setsid`, which is async-signal-safe.
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()
                .map(|_| ())
                .map_err(io::Error::from)
        });
    }
    let mut child = command.spawn().context("cannot start the daemon")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(anyhow!(
                "daemon exited during startup ({status}); see {}",
                log_path.display()
            ));
        }
        if Client::connect(&home).await.is_ok() {
            for addr in read_addrs(&home).await?.lines() {
                write_stdout(format!("listening on {addr}\n").into_bytes()).await?;
            }
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            // shortcut: a slow start is reported, not reaped; add a kill
            // if a half-started daemon ever lingers.
            return Err(anyhow!(
                "daemon did not become ready within 10s; see {}",
                log_path.display()
            ));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn read_addrs(home: &Path) -> anyhow::Result<String> {
    let path = crate::layout::Layout::new(home.to_owned()).addr();
    Ok(tokio::task::spawn_blocking(move || std::fs::read_to_string(path)).await??)
}

pub(super) async fn run_daemon_status(home: PathBuf) -> anyhow::Result<std::process::ExitCode> {
    let mut client = match Client::connect(&home).await {
        Ok(client) => client,
        Err(error) if error.to_string() == crate::client::UNREACHABLE => {
            write_stdout(b"not running\n".to_vec()).await?;
            return Ok(std::process::ExitCode::from(1));
        }
        Err(error) => return Err(error),
    };
    let addrs = read_addrs(&home).await?;
    let sessions = client.sessions().await?;
    let daemon_version = client.version().await?;
    let running = sessions
        .iter()
        .filter(|session| session.exit_code.is_none())
        .count();
    let exited = sessions.len() - running;
    let mut output = String::from("running\n");
    output.push_str(&format!(
        "version: {}\n",
        daemon_version
            .as_deref()
            .unwrap_or("unknown (daemon predates version reporting)")
    ));
    for addr in addrs.lines() {
        output.push_str(&format!("listening on {addr}\n"));
    }
    output.push_str(&format!("sessions: {running} running, {exited} exited\n"));
    write_stdout(output.into_bytes()).await?;
    if let Some(warning) = crate::client::version_warning(crate::VERSION, daemon_version.as_deref())
    {
        write_stderr(format!("{warning}\n").into_bytes()).await?;
    }
    Ok(std::process::ExitCode::SUCCESS)
}

pub(super) async fn run_stop(home: PathBuf, yes: bool, now: bool) -> anyhow::Result<()> {
    let Ok(mut client) = Client::connect(&home).await else {
        return write_stdout(b"no daemon running\n".to_vec()).await;
    };
    let sessions = client.sessions().await?;
    let running = sessions.iter().filter(|s| s.exit_code.is_none()).count();
    if running > 0 && !yes {
        let prompt =
            format!("Stopping the daemon ends {running} running session(s). Continue? [y/N] ");
        if !confirm(
            &prompt,
            &format!("refusing to stop: {running} running session(s); pass --yes to end them"),
        )
        .await?
        {
            return write_stdout(b"not stopped\n".to_vec()).await;
        }
    }
    client.shutdown(now).await?;
    let deadline = tokio::time::Instant::now() + DAEMON_STOP_TIMEOUT;
    while read_addrs(&home).await.is_ok() {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("daemon did not stop within 30s"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    write_stdout(b"stopped\n".to_vec()).await
}
