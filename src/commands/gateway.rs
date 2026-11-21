//! Gateway run, up, status, stop, and token commands.

use super::home::migrate_old_state;
use crate::discord::DiscordConfig;
use crate::relay::{Io as RelayIo, PairIo};
use crate::relay_files;
use crate::terminal::{EchoGuard, stdin_is_terminal, write_stdout};
use anyhow::{anyhow, bail};
use std::io::{self, BufRead, Read, Write};
use std::path::{Path, PathBuf};

const MAX_GATEWAY_TOKEN_BYTES: usize = 64 * 1024;

pub(super) async fn run_gateway_token(home: &Path, explicit_home: bool) -> anyhow::Result<()> {
    let layout = crate::layout::Layout::new(home.to_owned());
    migrate_old_state(explicit_home, &layout).await?;
    layout.ensure()?;
    let token = tokio::task::spawn_blocking(read_gateway_token).await??;
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || {
        relay_files::write_token(&home, &token)?;
        crate::config::ensure_gateway_file(&crate::layout::Layout::new(home))?;
        Ok::<_, anyhow::Error>(())
    })
    .await??;
    Ok(())
}

fn read_gateway_token() -> anyhow::Result<String> {
    let token = if stdin_is_terminal() {
        let guard = EchoGuard::enter()?;
        let result = read_gateway_token_line();
        drop(guard);
        result?
    } else {
        read_gateway_token_line()?
    };
    let token = token.trim().to_owned();
    if token.is_empty() {
        bail!("gateway token is empty");
    }
    Ok(token)
}

fn read_gateway_token_line() -> anyhow::Result<String> {
    let stdin = io::stdin();
    let mut stdin = stdin.lock();
    let mut bytes = Vec::with_capacity(MAX_GATEWAY_TOKEN_BYTES);
    for _ in 0..=MAX_GATEWAY_TOKEN_BYTES {
        let mut byte = [0_u8; 1];
        let count = stdin.read(&mut byte)?;
        if count == 0 {
            break;
        }
        bytes.push(byte[0]);
        if byte[0] == b'\n' {
            break;
        }
    }
    if bytes.len() > MAX_GATEWAY_TOKEN_BYTES {
        bail!("gateway token exceeds 64 KiB");
    }
    Ok(String::from_utf8(bytes)?)
}

pub(super) async fn run_gateway_up(
    home: PathBuf,
    pair: bool,
    foreground: bool,
    explicit_home: bool,
) -> anyhow::Result<()> {
    let layout = crate::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &layout).await?;
    tokio::task::spawn_blocking(move || crate::gateway_launch::up(&home, pair, foreground))
        .await??;
    Ok(())
}

pub(super) async fn run_gateway_status(home: &Path) -> anyhow::Result<std::process::ExitCode> {
    let state = tokio::task::spawn_blocking({
        let home = home.to_owned();
        move || crate::gateway_proc::state(&home)
    })
    .await??;
    match state {
        crate::gateway_proc::State::Running { pid } => {
            let pid = pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            write_stdout(format!("running (pid {pid})\n").into_bytes()).await?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        crate::gateway_proc::State::NotRunning => {
            write_stdout(b"not running\n".to_vec()).await?;
            Ok(std::process::ExitCode::from(1))
        }
    }
}

pub(super) async fn run_gateway_stop(home: &Path) -> anyhow::Result<std::process::ExitCode> {
    let outcome = tokio::task::spawn_blocking({
        let home = home.to_owned();
        move || crate::gateway_proc::stop(&home)
    })
    .await??;
    match outcome {
        crate::gateway_proc::StopOutcome::Stopped { pid } => {
            write_stdout(format!("stopped (pid {pid})\n").into_bytes()).await?;
        }
        crate::gateway_proc::StopOutcome::NotRunning => {
            write_stdout(b"no gateway running\n".to_vec()).await?;
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

pub(super) async fn run_gateway(home: PathBuf, pair: bool, pair_only: bool) -> anyhow::Result<()> {
    let loaded = crate::relay::load(&home)?;
    let mut discord = DiscordConfig::new(loaded.token.clone(), loaded.guild_id.clone());
    discord.api_base = loaded.config.api_base.clone();
    discord.gateway_url = loaded.config.gateway_url.clone();
    let pair_io = (pair || pair_only).then(|| PairIo {
        announce: Box::new(|line| eprintln!("{line}")),
        confirm: Box::new(|prompt| {
            eprint!("{prompt} ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            if io::stdin().lock().read_line(&mut line).is_err() {
                return false;
            }
            matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        }),
    });
    if pair_only {
        return crate::relay::pair_only(
            loaded,
            discord,
            pair_io.ok_or_else(|| anyhow!("pair-only mode requires pairing"))?,
        )
        .await;
    }
    let stop = Box::pin(async {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(error) => {
                    tracing::error!(%error, "cannot install SIGTERM handler");
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = sigterm.recv() => {},
        }
    });
    crate::relay::run(
        loaded,
        discord,
        RelayIo {
            stop,
            pair: pair_io,
        },
    )
    .await
}
