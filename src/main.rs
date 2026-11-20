use std::collections::{BTreeMap, BTreeSet};

use std::io::{self, BufRead, IsTerminal, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use clap::Parser;
use tracing_subscriber::prelude::*;

use a2amx::attach::{self, AttachOptions};
use a2amx::bridge;
use a2amx::cli::{
    Cli, Command, ConfigAction, ConfineAction, DaemonAction, GatewayAction, TeamAction,
};
use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::daemon::{self, Daemon, DaemonConfig};
use a2amx::discord::DiscordConfig;
use a2amx::harness::{Deliver, Harness};
use a2amx::hook;
use a2amx::lookup::{request_sessions, resolve_reference, resolve_session};
use a2amx::mcp;
use a2amx::messaging::{self, TeamScope, display_duration};
use a2amx::names;
use a2amx::relay::{Io as RelayIo, PairIo};
use a2amx::relay_files;
use a2amx::statusline;
use a2amx::table::{format_messages_table, format_session_table, session_confinement_mode};
use a2amx::team::{self, TeamSession};
use a2amx::terminal::{
    EchoGuard, stdin_is_terminal, terminal_size_with_default, write_stderr, write_stdout,
};
use a2amx::wire::{Activity, Request, Response, SessionSummary};

const DAEMON_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_AUTO_GRANTS_BYTES: usize = 65_536;

fn read_accepted_grants(path: &Path) -> anyhow::Result<a2amx::grant::Accepted> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(BTreeMap::new());
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", path.display()));
        }
    };
    if !metadata.is_file() {
        bail!(
            "accepted auto-grants.toml file {} is not a regular file",
            path.display()
        );
    }
    let file =
        std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut bytes = Vec::with_capacity(MAX_AUTO_GRANTS_BYTES + 1);
    let read = file
        .take((MAX_AUTO_GRANTS_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if read > MAX_AUTO_GRANTS_BYTES {
        bail!(
            "accepted auto-grants.toml file {} is larger than {} bytes",
            path.display(),
            MAX_AUTO_GRANTS_BYTES
        );
    }
    let text = String::from_utf8(bytes).with_context(|| {
        format!(
            "accepted auto-grants.toml file {} is not UTF-8",
            path.display()
        )
    })?;
    a2amx::grant::parse_accepted(&text)
        .with_context(|| format!("invalid accepted auto-grants.toml file {}", path.display()))
}

async fn load_accepted_grants(home: &Path) -> anyhow::Result<a2amx::grant::Accepted> {
    let path = a2amx::layout::Layout::new(home.to_owned()).auto_grants();
    tokio::task::spawn_blocking(move || read_accepted_grants(&path)).await?
}

async fn save_accepted_grants(
    home: &Path,
    accepted: &a2amx::grant::Accepted,
) -> anyhow::Result<()> {
    let path = a2amx::layout::Layout::new(home.to_owned()).auto_grants();
    let content = a2amx::grant::render_accepted(accepted);
    tokio::task::spawn_blocking(move || a2amx::omp::write_if_changed(&path, content.as_bytes()))
        .await??;
    Ok(())
}

fn log_target_allowed(target: &str) -> bool {
    ![
        "tungstenite",
        "tokio_tungstenite",
        "ureq",
        "ureq_proto",
        "rustls",
    ]
    .iter()
    .any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("::"))
    })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::EnvFilter::from_default_env())
                .with_filter(tracing_subscriber::filter::FilterFn::new(|metadata| {
                    log_target_allowed(metadata.target())
                })),
        )
        .init();
    let result = match Cli::try_parse() {
        Ok(cli) => match a2amx::privilege::ensure_not_root() {
            Ok(()) => dispatch(cli).await,
            Err(error) => Err(error),
        },
        Err(error) if !error.use_stderr() => write_stdout(error.to_string().into_bytes())
            .await
            .map(|_| std::process::ExitCode::SUCCESS),
        Err(error) => Err(error.into()),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            let _ = write_stderr(format!("a2amx: {error:#}\n").into_bytes()).await;
            std::process::ExitCode::FAILURE
        }
    }
}

async fn dispatch(cli: Cli) -> anyhow::Result<std::process::ExitCode> {
    let home_arg = cli.home;
    match &cli.command {
        Command::Mcp { channel, optional } => {
            return mcp::run(*channel, *optional)
                .await
                .map(|_| std::process::ExitCode::SUCCESS);
        }
        Command::Hook => return hook::run().await.map(|_| std::process::ExitCode::SUCCESS),
        Command::Statusline { report, harness } => {
            return statusline::run(*report, *harness)
                .await
                .map(|_| std::process::ExitCode::SUCCESS);
        }
        Command::OmpBridge => return bridge::run().await.map(|_| std::process::ExitCode::SUCCESS),
        Command::Confine {
            action: ConfineAction::List,
        } => {
            write_stdout(a2amx::grant::render_list().into_bytes()).await?;
            return Ok(std::process::ExitCode::SUCCESS);
        }
        Command::Confine {
            action: ConfineAction::Suggest { dir },
        } => {
            let dir = match dir {
                Some(dir) => dir.clone(),
                None => std::env::current_dir()?,
            };
            let host_home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let found = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let metadata = std::fs::metadata(&dir)
                    .with_context(|| format!("cannot read project directory {}", dir.display()))?;
                if !metadata.is_dir() {
                    bail!("cannot read project directory {}", dir.display());
                }
                Ok(auto_grants_for(&dir, &host_home))
            })
            .await??;
            let grants = found.iter().map(|(name, _)| *name).collect::<Vec<_>>();
            write_stdout(a2amx::suggest::render(&grants).into_bytes()).await?;
            return Ok(std::process::ExitCode::SUCCESS);
        }
        _ => {}
    }
    let explicit_home = home_arg.is_some();
    let home = resolve_home(home_arg)?;
    let client_overrides = a2amx::config::ClientOverrides {
        prefix: cli.prefix,
        viewer_after_secs: cli.viewer_after,
    };
    let show_client_overrides = client_overrides.clone();
    let client_layout = a2amx::layout::Layout::new(home.clone());
    let loaded_client = tokio::task::spawn_blocking(move || {
        a2amx::config::load_client(&client_layout, &client_overrides)
    })
    .await??;
    let attach_options = AttachOptions {
        prefix: loaded_client.settings.prefix,
        viewer_after: loaded_client.settings.viewer_after,
    };
    let result = match cli.command {
        Command::Gateway { action } => match action {
            GatewayAction::Token => run_gateway_token(&home, explicit_home).await,
            GatewayAction::Up { pair, foreground } => {
                run_gateway_up(home, pair, foreground, explicit_home).await
            }
            GatewayAction::Stop => return run_gateway_stop(&home).await,
            GatewayAction::Status => return run_gateway_status(&home).await,
            GatewayAction::Run { pair, pair_only } => run_gateway(home, pair, pair_only).await,
        },
        Command::Config {
            action: ConfigAction::Show,
        } => {
            let env_value = match std::env::var("A2AMX_OWNER_AWAY_SECS") {
                Ok(value) => Some(
                    value
                        .parse::<u64>()
                        .with_context(|| "A2AMX_OWNER_AWAY_SECS must be an integer")?,
                ),
                Err(std::env::VarError::NotPresent) => None,
                Err(error) => return Err(anyhow!("cannot read A2AMX_OWNER_AWAY_SECS: {error}")),
            };
            let layout = a2amx::layout::Layout::new(home.clone());
            let client_overrides = show_client_overrides.clone();
            let output = tokio::task::spawn_blocking(move || {
                let mut output = a2amx::config::show_daemon(&layout, env_value)?;
                output.push('\n');
                output.push_str(&a2amx::config::show_gateway(&layout)?);
                output.push('\n');
                output.push_str(&a2amx::config::show_client(&layout, &client_overrides)?);
                Ok::<_, anyhow::Error>(output)
            })
            .await??;
            write_stdout(output.into_bytes()).await
        }
        Command::Daemon { action } => match action {
            DaemonAction::Start {
                listen,
                host_name,
                foreground,
                owner_away_secs,
            } => {
                if foreground {
                    run_daemon(home, listen, host_name, owner_away_secs, explicit_home).await
                } else {
                    run_daemon_background(home, listen, host_name, owner_away_secs, explicit_home)
                        .await
                }
            }
            DaemonAction::Status => return run_daemon_status(home).await,
            DaemonAction::Stop { yes, now } => run_stop(home, yes, now).await,
        },
        options @ Command::New { .. } => run_new(home, attach_options, options).await,
        Command::List {
            details,
            team,
            viewer,
        } => run_list(home, details, team, viewer).await,
        Command::Team { action } => match action {
            TeamAction::Status { file } => return run_team_status(home, file).await,
            TeamAction::Up {
                file,
                detach,
                dry_run,
                yes,
                items,
            } => run_team_up(home, attach_options, file, detach, dry_run, yes, items).await,
            TeamAction::Down { file, names, now } => run_team_down(home, file, names, now).await,
            TeamAction::Reset {
                file,
                names,
                except,
                include_attached,
                yes,
            } => run_team_reset(home, file, names, except, include_attached, yes).await,
        },
        Command::Attach { session, force } => {
            run_attach_command(home, attach_options, session, force).await
        }
        Command::Kill {
            session,
            exited,
            yes,
            now,
        } => run_kill(home, session, exited, yes, now).await,
        Command::Reset { session } => run_reset(home, session).await,
        Command::Explain { session } => run_explain(home, session).await,
        Command::Screen { session, rows } => run_screen(home, session, rows).await,
        Command::Messages { session, state } => run_messages(home, session, state).await,
        Command::Cancel { message } => run_cancel(home, message).await,
        Command::Confine {
            action: ConfineAction::Show { session },
        } => run_confine_show(home, session).await,
        Command::Confine { .. }
        | Command::Mcp { .. }
        | Command::Hook
        | Command::Statusline { .. }
        | Command::OmpBridge => {
            unreachable!("early dispatch returned before resolving home")
        }
    };
    result.map(|_| std::process::ExitCode::SUCCESS)
}

fn resolve_home(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    Ok(
        a2amx::layout::Layout::resolve(explicit, std::env::var_os("HOME").map(PathBuf::from))?
            .root()
            .to_owned(),
    )
}

const MAX_GATEWAY_TOKEN_BYTES: usize = 64 * 1024;

async fn migrate_old_state(
    explicit_home: bool,
    layout: &a2amx::layout::Layout,
) -> anyhow::Result<()> {
    if explicit_home {
        return Ok(());
    }
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    let old = a2amx::migrate::old_dir(
        std::env::var_os("XDG_STATE_HOME").as_deref(),
        user_home.as_deref(),
    );
    let Some(old) = old else { return Ok(()) };
    let layout_for_task = layout.clone();
    let old_for_task = old.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        a2amx::migrate::migrate(&layout_for_task, &old_for_task)
    })
    .await??;
    if let a2amx::migrate::Outcome::Migrated(report) = outcome {
        for (old, new) in report.moved {
            eprintln!("moved {} -> {}", old.display(), new.display());
        }
        for warning in report.warnings {
            eprintln!("{warning}");
        }
        eprintln!(
            "a2amx now uses {}; {} is no longer used and can be deleted: rm -r {}",
            layout.root().display(),
            old.display(),
            shell_quote(&old)
        );
    }
    Ok(())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

fn warn_explicit_old_layout(home: &Path) {
    let mut names = Vec::new();
    if home.join("messages.db").exists() {
        names.push("messages.db");
    }
    if home.join("team.toml").exists() {
        names.push("team.toml");
    }
    if home.join("gateway").exists() {
        names.push("gateway/");
    }
    if !names.is_empty() {
        eprintln!(
            "{} holds files in the old layout ({}); a2amx now reads {}/state, {}/config and {}/log; see docs/install.md",
            home.display(),
            names.join(", "),
            home.display(),
            home.display(),
            home.display()
        );
    }
}

async fn run_gateway_token(home: &Path, explicit_home: bool) -> anyhow::Result<()> {
    let layout = a2amx::layout::Layout::new(home.to_owned());
    migrate_old_state(explicit_home, &layout).await?;
    layout.ensure()?;
    let token = tokio::task::spawn_blocking(read_gateway_token).await??;
    let home = home.to_owned();
    tokio::task::spawn_blocking(move || {
        relay_files::write_token(&home, &token)?;
        a2amx::config::ensure_gateway_file(&a2amx::layout::Layout::new(home))?;
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

async fn run_gateway_up(
    home: PathBuf,
    pair: bool,
    foreground: bool,
    explicit_home: bool,
) -> anyhow::Result<()> {
    let layout = a2amx::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &layout).await?;
    tokio::task::spawn_blocking(move || a2amx::gateway_launch::up(&home, pair, foreground))
        .await??;
    Ok(())
}

async fn run_gateway_status(home: &Path) -> anyhow::Result<std::process::ExitCode> {
    let state = tokio::task::spawn_blocking({
        let home = home.to_owned();
        move || a2amx::gateway_proc::state(&home)
    })
    .await??;
    match state {
        a2amx::gateway_proc::State::Running { pid } => {
            let pid = pid.map_or_else(|| "unknown".to_owned(), |pid| pid.to_string());
            write_stdout(format!("running (pid {pid})\n").into_bytes()).await?;
            Ok(std::process::ExitCode::SUCCESS)
        }
        a2amx::gateway_proc::State::NotRunning => {
            write_stdout(b"not running\n".to_vec()).await?;
            Ok(std::process::ExitCode::from(1))
        }
    }
}

async fn run_gateway_stop(home: &Path) -> anyhow::Result<std::process::ExitCode> {
    let outcome = tokio::task::spawn_blocking({
        let home = home.to_owned();
        move || a2amx::gateway_proc::stop(&home)
    })
    .await??;
    match outcome {
        a2amx::gateway_proc::StopOutcome::Stopped { pid } => {
            write_stdout(format!("stopped (pid {pid})\n").into_bytes()).await?;
        }
        a2amx::gateway_proc::StopOutcome::NotRunning => {
            write_stdout(b"no gateway running\n".to_vec()).await?;
        }
    }
    Ok(std::process::ExitCode::SUCCESS)
}

async fn run_gateway(home: PathBuf, pair: bool, pair_only: bool) -> anyhow::Result<()> {
    let loaded = a2amx::relay::load(&home)?;
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
        return a2amx::relay::pair_only(
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
    a2amx::relay::run(
        loaded,
        discord,
        RelayIo {
            stop,
            pair: pair_io,
        },
    )
    .await
}

async fn run_daemon(
    home: PathBuf,
    listen: Vec<std::net::SocketAddr>,
    host_name: Option<String>,
    owner_away_secs: Option<u64>,
    explicit_home: bool,
) -> anyhow::Result<()> {
    let migration_layout = a2amx::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &migration_layout).await?;
    if explicit_home {
        warn_explicit_old_layout(&home);
    }
    let overrides = a2amx::config::Overrides {
        listen,
        host_name,
        owner_away_secs,
    };
    let layout = a2amx::layout::Layout::new(home.clone());
    let loaded = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
        layout.ensure()?;
        a2amx::config::ensure_daemon_file(&layout)?;
        a2amx::config::ensure_client_file(&layout)?;
        a2amx::config::load_daemon(&layout, &overrides)
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

async fn run_daemon_background(
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
    let migration_layout = a2amx::layout::Layout::new(home.clone());
    migrate_old_state(explicit_home, &migration_layout).await?;
    if explicit_home {
        warn_explicit_old_layout(&home);
    }
    let log_path = a2amx::layout::Layout::new(home.clone()).daemon_log();
    let (log, exe) = {
        let home = home.clone();
        let log_path = log_path.clone();
        let overrides = a2amx::config::Overrides {
            listen: listen.clone(),
            host_name: host_name.clone(),
            owner_away_secs,
        };
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            let layout = a2amx::layout::Layout::new(home.clone());
            layout.ensure()?;
            a2amx::config::load_daemon(&layout, &overrides)?;
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
    let path = a2amx::layout::Layout::new(home.to_owned()).addr();
    Ok(tokio::task::spawn_blocking(move || std::fs::read_to_string(path)).await??)
}

async fn run_daemon_status(home: PathBuf) -> anyhow::Result<std::process::ExitCode> {
    let mut client = match Client::connect(&home).await {
        Ok(client) => client,
        Err(error) if error.to_string() == a2amx::client::UNREACHABLE => {
            write_stdout(b"not running\n".to_vec()).await?;
            return Ok(std::process::ExitCode::from(1));
        }
        Err(error) => return Err(error),
    };
    let addrs = read_addrs(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    let daemon_version = match client.request(Request::Version).await? {
        Response::Version { version } => Some(version),
        Response::Error { .. } => None,
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
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
    if let Some(warning) = a2amx::client::version_warning(a2amx::VERSION, daemon_version.as_deref())
    {
        write_stderr(format!("{warning}\n").into_bytes()).await?;
    }
    Ok(std::process::ExitCode::SUCCESS)
}

async fn confirm(prompt: &str, refusal: &str) -> anyhow::Result<bool> {
    if !io::stdin().is_terminal() {
        return Err(anyhow!(refusal.to_owned()));
    }
    write_stderr(prompt.as_bytes().to_vec()).await?;
    let answer = tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        io::stdin().read_line(&mut line).map(|_| line)
    })
    .await??;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

fn project_key(dir: &Path) -> anyhow::Result<&str> {
    dir.to_str().ok_or_else(|| {
        anyhow!(
            "project directory is not valid UTF-8: {:?}",
            dir.as_os_str()
        )
    })
}
/// `--yes` applies to this run only and is not saved.
async fn confirm_new_grants(
    home: &Path,
    project: &Path,
    accepted: &a2amx::grant::Accepted,
    new_grants: &[(String, String)],
    yes: bool,
) -> anyhow::Result<()> {
    let lines = a2amx::grant::consent_lines(new_grants, a2amx::grant::all());
    if yes {
        write_stderr(lines.into_bytes()).await?;
        return Ok(());
    }
    let prompt = format!("{lines}{}", a2amx::grant::consent_question(project));
    let refusal = a2amx::grant::consent_refusal(new_grants);
    if !confirm(&prompt, &refusal).await? {
        bail!("auto grants not accepted; nothing started");
    }
    let mut accepted = accepted.clone();
    accepted
        .entry(project_key(project)?.to_owned())
        .or_default()
        .extend(new_grants.iter().map(|(grant, _)| grant.clone()));
    // shortcut: concurrent team up runs can lose one project's acceptance; add a file lock if repeated prompts matter.
    save_accepted_grants(home, &accepted).await
}

async fn run_stop(home: PathBuf, yes: bool, now: bool) -> anyhow::Result<()> {
    let Ok(mut client) = Client::connect(&home).await else {
        return write_stdout(b"no daemon running\n".to_vec()).await;
    };
    let sessions = match client.request(Request::List).await? {
        Response::Sessions { sessions } => sessions,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
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
    match client.request(Request::Shutdown { now }).await? {
        Response::Ok => {}
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }
    let deadline = tokio::time::Instant::now() + DAEMON_STOP_TIMEOUT;
    while read_addrs(&home).await.is_ok() {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("daemon did not stop within 30s"));
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    write_stdout(b"stopped\n".to_vec()).await
}

fn parse_activity_watch(
    working_after: Option<&str>,
    idle_after: Option<&str>,
) -> anyhow::Result<Option<a2amx::wire::ActivityWatch>> {
    let working_after = working_after
        .map(|value| a2amx::messaging::parse_interval(value, "activity working_after"))
        .transpose()?
        .map(|duration| duration.as_secs());
    let idle_after = idle_after
        .map(|value| a2amx::messaging::parse_interval(value, "activity idle_after"))
        .transpose()?
        .map(|duration| duration.as_secs());
    Ok(match (working_after, idle_after) {
        (None, None) => None,
        (working_after, idle_after) => Some(a2amx::wire::ActivityWatch {
            working_after,
            idle_after,
        }),
    })
}

async fn run_new(
    home: PathBuf,
    attach_options: AttachOptions,
    options: Command,
) -> anyhow::Result<()> {
    let Command::New {
        detach,
        name,
        harness,
        deliver,
        no_authorize_peers,
        no_channel,
        confine,
        remote,
        confine_write,
        confine_hide,
        confine_unhide,
        confine_default_hides,
        mut confine_grant,
        confine_keep_env,
        reset,
        control_from,
        watch,
        activity_working_after,
        activity_idle_after,
        heartbeat,
        command,
    } = options
    else {
        return Err(anyhow!("expected new session options"));
    };
    let mut unique_grants = Vec::with_capacity(confine_grant.len());
    for grant in confine_grant {
        if !unique_grants.contains(&grant) {
            unique_grants.push(grant);
        }
    }
    confine_grant = unique_grants;
    let has_confine_options = !confine_write.is_empty()
        || !confine_hide.is_empty()
        || !confine_unhide.is_empty()
        || confine_default_hides.is_some()
        || !confine_grant.is_empty()
        || !confine_keep_env.is_empty();
    if has_confine_options && !confine {
        return Err(anyhow!("--confine-* flags require --confine"));
    }
    if confine {
        a2amx::grant::validate(&confine_grant)?;
    }

    let remote = remote
        .map(|value| messaging::RemoteMode::parse(&value))
        .transpose()?;
    let activity = parse_activity_watch(
        activity_working_after.as_deref(),
        activity_idle_after.as_deref(),
    )?;
    if let Some(activity) = &activity {
        activity.validate(!watch.is_empty())?;
    }
    let heartbeat = heartbeat
        .as_deref()
        .map(|value| a2amx::messaging::parse_interval(value, "heartbeat"))
        .transpose()?
        .map(|duration| duration.as_secs());
    if heartbeat.is_some() && watch.is_empty() {
        return Err(anyhow!("heartbeat needs a non-empty watch"));
    }
    let mut client = Client::connect(&home).await?;
    let mut generated_name = None;
    let name = match name {
        Some(name) => Some(name),
        None => {
            let taken = request_sessions(&mut client)
                .await?
                .into_iter()
                .filter_map(|session| session.name)
                .collect::<Vec<_>>();
            let random = tokio::task::spawn_blocking(daemon::random_hex::<8>).await??;
            let seed = u64::from_str_radix(&random, 16)?;
            // shortcut: ceiling is concurrent `new` on one daemon and very churned registry;
            // upgrade trigger is daemon-side generation if either is reported.
            let generated = names::pick(&taken, seed)
                .ok_or_else(|| anyhow!("no free generated session name; pass --name"))?;
            generated_name = Some(generated.clone());
            Some(generated)
        }
    };
    let (cols, rows) = terminal_size_with_default()?;
    let cwd = std::env::current_dir()?;
    let confine = if confine {
        let request = ConfineRequest {
            write: confine_write,
            hide: confine_hide,
            unhide: confine_unhide,
            no_default_hides: confine_default_hides == Some(false),
            grants: confine_grant,
            keep_env: confine_keep_env,
            auto: false,
            reasons: Vec::new(),
            team_file: None,
        };
        Some(resolved_confine_request(&request, &cwd, None)?)
    } else {
        None
    };
    let session = create_session(
        &home,
        &mut client,
        NewOptions {
            name,
            harness,
            role: None,
            remote,
            team: None,
            deliver,
            no_authorize_peers,
            no_channel,
            confine,
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            command,
            cwd,
            cols,
            rows,
        },
    )
    .await?;
    if let Some(name) = generated_name {
        write_stderr(format!("name: {name}\n").into_bytes()).await?;
    }
    if detach {
        write_stdout(format!("{session}\n").into_bytes()).await?;
        return Ok(());
    }
    if !stdin_is_terminal() {
        return Err(anyhow!("attach needs a terminal on stdin"));
    }
    attach::run(client, &home, attach_options, session, false, cols, rows).await
}

struct NewOptions {
    name: Option<String>,
    role: Option<String>,
    remote: Option<messaging::RemoteMode>,
    team: Option<TeamScope>,
    harness: Option<Harness>,
    deliver: Option<Deliver>,
    no_authorize_peers: bool,
    no_channel: bool,
    confine: Option<ConfineRequest>,
    reset: Vec<String>,
    control_from: Vec<String>,
    watch: Vec<String>,
    activity: Option<a2amx::wire::ActivityWatch>,
    heartbeat: Option<u64>,
    command: Vec<String>,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
}

/// D6 fixes this argument shape so launch wiring is shared by real and dry-run paths.
#[allow(clippy::too_many_arguments)]
async fn wired_command(
    home: &Path,
    command: Vec<String>,
    harness: Harness,
    cwd: &Path,
    role: Option<&str>,
    no_authorize_peers: bool,
    no_channel: bool,
    install_omp: bool,
) -> anyhow::Result<Vec<String>> {
    match harness {
        Harness::Claude => {
            let exe = std::env::current_exe()?;
            let wire_cwd = cwd.to_owned();
            let wire_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
                })
                .unwrap_or_default();
            let wire_role = role.map(str::to_owned);
            let authorize_peers = !no_authorize_peers;
            if no_channel {
                Ok(tokio::task::spawn_blocking(move || {
                    a2amx::harness::wire_claude_argv(
                        command,
                        &exe,
                        &wire_cwd,
                        &wire_config_dir,
                        authorize_peers,
                        wire_role.as_deref(),
                    )
                })
                .await??)
            } else {
                Ok(tokio::task::spawn_blocking(move || {
                    a2amx::harness::wire_claude_channel_argv(
                        command,
                        &exe,
                        &wire_cwd,
                        &wire_config_dir,
                        authorize_peers,
                        wire_role.as_deref(),
                    )
                })
                .await??)
            }
        }
        Harness::Agy => {
            let exe = std::env::current_exe()?;
            let wire_cwd = cwd.to_owned();
            let gemini_dir = std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".gemini"))
                .unwrap_or_default();
            if install_omp {
                let layout = a2amx::layout::Layout::new(home.to_owned());
                // shortcut: relative command[0] resolves from the launcher's directory; canonicalizing against cwd would change the install API.
                let agy_program = std::ffi::OsString::from(
                    command
                        .first()
                        .cloned()
                        .ok_or_else(|| anyhow!("empty command"))?,
                );
                tokio::task::spawn_blocking(move || a2amx::agy::install(&layout, &agy_program))
                    .await??;
            }
            Ok(tokio::task::spawn_blocking(move || {
                a2amx::harness::wire_agy_argv(command, &exe, &wire_cwd, &gemini_dir)
            })
            .await??)
        }
        Harness::Omp if !install_omp => Ok(command),
        Harness::Omp => {
            let install_layout = a2amx::layout::Layout::new(home.to_owned());
            let installed =
                tokio::task::spawn_blocking(move || a2amx::omp::install(&install_layout)).await??;
            Ok(a2amx::harness::wire_omp_argv(
                command,
                &installed.extension,
                &installed.overlay,
                !no_authorize_peers,
                role,
            ))
        }
        Harness::Generic | Harness::Codex => Ok(command),
    }
}

fn resolve_confine_paths(
    base: &Path,
    option: &str,
    paths: &[String],
) -> anyhow::Result<Vec<String>> {
    paths
        .iter()
        .map(|entry| {
            a2amx::confine::check_entry_syntax(option, entry)?;
            let path = if entry == "~" || entry.starts_with("~/") {
                PathBuf::from(entry)
            } else {
                let path = Path::new(entry);
                if path.is_absolute() {
                    path.to_owned()
                } else {
                    base.join(path)
                }
            };
            Ok(path.to_string_lossy().into_owned())
        })
        .collect()
}

/// Resolve a confinement request's relative paths and attach its team file.
fn resolved_confine_request(
    request: &ConfineRequest,
    cwd: &Path,
    team_file: Option<&Path>,
) -> anyhow::Result<ConfineRequest> {
    Ok(ConfineRequest {
        write: resolve_confine_paths(cwd, "write", &request.write)?,
        hide: resolve_confine_paths(cwd, "hide", &request.hide)?,
        unhide: resolve_confine_paths(cwd, "unhide", &request.unhide)?,
        no_default_hides: request.no_default_hides,
        grants: request.grants.clone(),
        auto: request.auto,
        reasons: request.reasons.clone(),
        keep_env: request.keep_env.clone(),
        team_file: team_file
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| request.team_file.clone()),
    })
}

const MAX_GIT_CONFIG_BYTES: usize = 65_536;

fn auto_grants_for(dir: &Path, home: &Path) -> Vec<(&'static str, String)> {
    let canonical_dir = std::fs::canonicalize(dir).ok();
    let canonical_home = if home.as_os_str().is_empty() {
        home.to_owned()
    } else {
        std::fs::canonicalize(home).unwrap_or_else(|_| home.to_owned())
    };
    let git_config = canonical_dir.as_deref().and_then(|canonical_dir| {
        let git_dir = a2amx::confine::git_common_dir(canonical_dir, &canonical_home)
            .map(|(common, _)| common)?;
        let config_path = git_dir.join("config");
        let regular = std::fs::metadata(&config_path)
            .map(|metadata| metadata.is_file())
            .unwrap_or(false);
        if !regular {
            return None;
        }
        let mut bytes = Vec::new();
        std::fs::File::open(config_path).ok().and_then(|file| {
            let read = file
                .take((MAX_GIT_CONFIG_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .ok()?;
            (read <= MAX_GIT_CONFIG_BYTES)
                .then(|| String::from_utf8(bytes).ok())
                .flatten()
        })
    });
    a2amx::suggest::explain(
        |name| dir.join(name).exists(),
        git_config.as_deref(),
        |path| {
            if canonical_home.as_os_str().is_empty() {
                return false;
            }
            let relative = path.strip_prefix("~/").unwrap_or(path);
            canonical_home.join(relative).exists()
        },
    )
}

async fn resolve_auto_grants(
    actions: &mut [team::Action],
    origin: TeamOrigin<'_>,
) -> anyhow::Result<()> {
    let host_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for action in actions {
        let team::Action::Start(session) = action else {
            continue;
        };
        if session.auto_confine().is_none() {
            continue;
        }
        let resolved_dir = session
            .cwd
            .as_deref()
            .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
        let scan_home = host_home.clone();
        let found =
            tokio::task::spawn_blocking(move || auto_grants_for(&resolved_dir, &scan_home)).await?;
        if let Some(confine) = session.auto_confine_mut() {
            confine.apply_auto(found);
        }
    }
    Ok(())
}

async fn create_session(
    home: &Path,
    client: &mut Client,
    options: NewOptions,
) -> anyhow::Result<String> {
    let NewOptions {
        name,
        harness,
        deliver,
        role,
        remote,
        team,
        no_authorize_peers,
        no_channel,
        confine,
        reset,
        control_from,
        watch,
        activity,
        heartbeat,
        command,
        cwd,
        cols,
        rows,
    } = options;
    a2amx::messaging::validate_reset_steps(&reset)?;
    for controller in &control_from {
        a2amx::messaging::validate_name(controller)?;
    }
    for name in &watch {
        a2amx::messaging::validate_name(name)?;
    }
    if let Some(seconds) = heartbeat {
        if watch.is_empty() {
            return Err(anyhow!("heartbeat needs a non-empty watch"));
        }
        a2amx::messaging::parse_interval(&format!("{seconds}s"), "heartbeat")?;
    }
    if let Some(activity) = &activity {
        activity.validate(!watch.is_empty())?;
    }
    let harness = harness.unwrap_or_else(|| Harness::infer(&command));
    let command = wired_command(
        home,
        command,
        harness,
        &cwd,
        role.as_deref(),
        no_authorize_peers,
        no_channel,
        true,
    )
    .await?;
    let cwd = cwd.to_string_lossy().into_owned();
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.retain(|(key, _)| key != a2amx::codex::ROLE_ENV);
    if harness == Harness::Omp {
        env.push((
            "A2AMX_BIN".to_owned(),
            std::env::current_exe()?.to_string_lossy().into_owned(),
        ));
    }
    if harness == Harness::Codex && no_authorize_peers {
        env.push((a2amx::codex::NO_AUTHORIZE_ENV.to_owned(), "1".to_owned()));
    }
    if harness == Harness::Codex
        && let Some(role) = &role
    {
        env.push((a2amx::codex::ROLE_ENV.to_owned(), role.clone()));
    }
    if harness == Harness::Claude && !no_channel {
        env.push((a2amx::harness::CHANNEL_ENV.to_owned(), "1".to_owned()));
    }
    let request = Request::NewSession {
        argv: command,
        cols,
        rows,
        cwd: Some(cwd),
        env,
        reset,
        control_from,
        watch,
        activity,
        name,
        harness,
        deliver,
        team,
        role,
        remote,
        heartbeat,
    };
    let confined = confine.is_some();
    let request = match confine {
        Some(confine) => Request::Confine {
            confine,
            request: Box::new(request),
        },
        None => request,
    };
    let response = client.request(request).await?;
    match response {
        Response::Created { session } => Ok(session),
        Response::Error { message } if confined && message.contains("invalid control request") => {
            Err(anyhow!(
                "the running daemon does not support --confine; restart it"
            ))
        }
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

#[derive(Clone)]
struct TeamSource {
    path: PathBuf,
    dir: PathBuf,
    global: bool,
}

#[derive(Clone, Copy)]
struct TeamOrigin<'a> {
    dir: &'a Path,
    file: Option<&'a Path>,
}

async fn resolve_team_file(home: &Path, file: Option<PathBuf>) -> anyhow::Result<TeamSource> {
    let invoking_cwd = std::env::current_dir()?;
    if let Some(file) = file {
        let path = invoking_cwd.join(file);
        let parent = path.parent().unwrap_or(&invoking_cwd).to_owned();
        let dir = tokio::task::spawn_blocking(move || {
            std::fs::canonicalize(&parent).with_context(|| {
                format!(
                    "cannot canonicalize team file directory {}",
                    parent.display()
                )
            })
        })
        .await??;
        return Ok(TeamSource {
            path,
            dir,
            global: false,
        });
    }
    let state_home = home.to_owned();
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<TeamSource> {
        let cwd = std::fs::canonicalize(&invoking_cwd).with_context(|| {
            format!(
                "cannot canonicalize invoking directory {}",
                invoking_cwd.display()
            )
        })?;
        let state_home = std::fs::canonicalize(&state_home).unwrap_or(state_home);
        let user_home = user_home.map(|path| {
            if path.is_absolute() {
                std::fs::canonicalize(&path).unwrap_or(path)
            } else {
                path
            }
        });
        let local = invoking_cwd.join("a2amx.toml");
        match std::fs::metadata(&local) {
            Ok(_) => {
                return Ok(TeamSource {
                    path: local,
                    dir: cwd,
                    global: false,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(anyhow!(
                    "cannot inspect team file {}: {error}",
                    local.display()
                ));
            }
        }
        let global = a2amx::layout::Layout::new(state_home.clone()).team_file();
        match std::fs::metadata(&global) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!(
                    "no a2amx.toml in {} and no global team file at {}",
                    cwd.display(),
                    global.display()
                );
            }
            Err(error) => {
                return Err(anyhow!(
                    "cannot inspect global team file {}: {error}",
                    global.display()
                ));
            }
        }
        team::check_global_allowed(&cwd, user_home.as_deref(), &state_home)?;
        Ok(TeamSource {
            path: global,
            dir: cwd,
            global: true,
        })
    })
    .await??;
    if result.global {
        let name = team::team_name_for(&result.dir, "create an a2amx.toml here, or pass --file")?;
        write_stderr(
            format!(
                "using global team file {} as team {name}\n",
                result.path.display()
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(result)
}

async fn read_team(source: TeamSource) -> anyhow::Result<Vec<TeamSession>> {
    tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&source.path)
            .with_context(|| format!("cannot read team file {}", source.path.display()))?;
        team::parse(&text, &source.dir, source.global)
            .with_context(|| format!("team file {}", source.path.display()))
    })
    .await?
}

fn conflict_text(reason: team::ConflictReason, name: &str, id: &str) -> String {
    match reason {
        team::ConflictReason::Exited => {
            format!("session {name} has exited ({id}); run a2amx kill {name} first")
        }
        team::ConflictReason::TeamMismatch => {
            format!(
                "session {name} ({id}) is running with a different team setting; run a2amx team down first"
            )
        }
    }
}

async fn write_conflicts(conflicts: &[team::Conflict]) -> anyhow::Result<()> {
    for conflict in conflicts {
        write_stderr(
            format!(
                "{}\n",
                conflict_text(conflict.reason.clone(), &conflict.name, &conflict.id,)
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(())
}

fn team_label(team: Option<&TeamScope>) -> String {
    let Some(team) = team else {
        return "-".to_owned();
    };
    if team.private {
        format!("{} (private)", team.name)
    } else {
        team.name.clone()
    }
}

fn team_detail(team: Option<&TeamScope>) -> Option<String> {
    team.map(|team| {
        let visibility = if team.private { "private" } else { "public" };
        let allow = if team.allow.is_empty() {
            "-".to_owned()
        } else {
            team.allow.join(",")
        };
        format!("{}, {visibility}, allow: {allow}", team.name)
    })
}

async fn run_team_status(
    home: PathBuf,
    file: Option<PathBuf>,
) -> anyhow::Result<std::process::ExitCode> {
    let source = resolve_team_file(&home, file).await?;
    let wanted = read_team(source).await?;
    let mut client = Client::connect(&home).await?;
    let existing = request_sessions(&mut client).await?;
    let entries = team::inspect(&wanted, &existing);
    let mut output = String::from("NAME TEAM STATE CONFINE NOTE\n");
    let mut has_conflict = false;
    for (session, entry) in wanted.iter().zip(entries) {
        let (state, confine, note) = match entry.state {
            team::EntryState::Missing => ("missing".to_owned(), String::new(), "-".to_owned()),
            team::EntryState::Running { id } => {
                let current = existing.iter().find(|current| current.id == id);
                let mode = current.map_or("off", session_confinement_mode).to_owned();
                let note = if current.is_some_and(|summary| summary.starting) {
                    format!("{id} (not ready: starting)")
                } else {
                    id
                };
                ("running".to_owned(), mode, note)
            }
            team::EntryState::Conflict { id, reason } => {
                has_conflict = true;
                let state = match &reason {
                    team::ConflictReason::Exited => "exited",
                    team::ConflictReason::TeamMismatch => "team-mismatch",
                };
                let full = conflict_text(reason, &entry.name, &id);
                let note = full
                    .split_once("; ")
                    .map_or(full.as_str(), |(_, note)| note)
                    .to_owned();
                (state.to_owned(), String::new(), note)
            }
        };
        output.push_str(&format!(
            "{} {} {} {} {}\n",
            session.name,
            team_label(session.team.as_ref()),
            state,
            confine,
            note
        ));
    }
    write_stdout(output.into_bytes()).await?;
    note_lost_messages(&home, &wanted).await;
    Ok(if has_conflict {
        std::process::ExitCode::from(1)
    } else {
        std::process::ExitCode::SUCCESS
    })
}

fn is_redactable_flag(value: &str) -> bool {
    if let Some(long) = value.strip_prefix("--") {
        let mut chars = long.bytes();
        return matches!(chars.next(), Some(byte) if byte.is_ascii_alphabetic())
            && chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    }
    if let Some(short) = value.strip_prefix('-') {
        return short.len() == 1 && short.as_bytes()[0].is_ascii_alphabetic();
    }
    false
}

fn redact_argv(argv: &[String]) -> String {
    let Some(first) = argv.first() else {
        return String::new();
    };
    let mut output = vec![first.clone()];
    let mut after_separator = false;
    for argument in &argv[1..] {
        if after_separator {
            output.push(format!("<{} bytes>", argument.len()));
            continue;
        }
        if argument == "--" {
            after_separator = true;
            output.push(argument.clone());
            continue;
        }
        let (flag, value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(flag, value)| {
                (flag, Some(value))
            });
        if is_redactable_flag(flag) {
            match value {
                Some(value) => output.push(format!("{flag}=<redacted, {} bytes>", value.len())),
                None => output.push(argument.clone()),
            }
        } else {
            output.push(format!("<{} bytes>", argument.len()));
        }
    }
    output.join(" ")
}

async fn note_lost_messages(home: &Path, wanted: &[TeamSession]) {
    let Ok(mut client) = Client::connect(home).await else {
        return;
    };
    let Ok(Response::Messages { messages }) = client
        .request(Request::ListMessages {
            session: None,
            state: Some("undeliverable".to_owned()),
        })
        .await
    else {
        return;
    };
    let count = messages
        .iter()
        .filter(|message| message.detail.as_deref() == Some("daemon_restarted"))
        .filter(|message| {
            let Some(local) = message.to.split_once('@').map(|(local, _)| local) else {
                return false;
            };
            wanted.iter().any(|session| session.name == local)
        })
        .count();
    if count != 0 {
        let _ = write_stderr(
            format!(
                "note: {count} message(s) to this team were lost in a daemon restart; see a2amx messages --state undeliverable\n"
            )
            .into_bytes(),
        )
        .await;
    }
}

async fn dry_run_report(
    home: &Path,
    origin: TeamOrigin<'_>,
    actions: &[team::Action],
    existing: &[SessionSummary],
    accepted: &BTreeSet<String>,
) -> anyhow::Result<String> {
    let mut output = String::new();
    for action in actions {
        match action {
            team::Action::AlreadyRunning { name, id } => {
                let mode = existing
                    .iter()
                    .find(|session| session.id == *id)
                    .map_or("off", session_confinement_mode);
                output.push_str(&format!("already running {name} {id} (confine {mode})\n"));
            }
            team::Action::Start(session) => {
                output.push_str(&dry_run_start(home, origin, session, accepted).await?);
            }
        }
    }
    Ok(output)
}

async fn dry_run_start(
    home: &Path,
    origin: TeamOrigin<'_>,
    session: &TeamSession,
    accepted: &BTreeSet<String>,
) -> anyhow::Result<String> {
    let harness = Harness::infer(&session.command);
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(session, &resolved_cwd)?;
    let confine_plan = if let Some(request) = session.confine.as_ref() {
        let host_home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        let runtime_dir =
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?);
        let request = resolved_confine_request(request, &resolved_cwd, origin.file)?;
        let host = a2amx::confine::Host {
            home: host_home,
            layout: a2amx::layout::Layout::new(home.to_owned()),
            runtime_dir,
        };
        let preflight = a2amx::confine::ensure_grant_dirs(&request, harness, &host, false);
        Some(
            a2amx::confine::plan_with_preflight(
                &request,
                harness,
                &resolved_cwd,
                &host,
                std::env::var_os("PATH"),
                preflight,
            )?
            .with_bwrap(PathBuf::from("bwrap")),
        )
    } else {
        None
    };
    let command = wired_command(
        home,
        session.command.clone(),
        harness,
        &resolved_cwd,
        session.role.as_deref(),
        false,
        false,
        false,
    )
    .await?;
    let mut output = format!(
        "would start {}\n  harness: {}\n  cwd: {}\n",
        session.name,
        harness.as_str(),
        resolved_cwd.display()
    );
    for (label, value) in session_details(session) {
        output.push_str(&format!("  {label}: {value}\n"));
    }
    if let Some(plan) = &confine_plan {
        output.push_str(&a2amx::confine::dry_run_lines(
            plan,
            &resolved_cwd,
            accepted,
        ));
    } else {
        output.push_str("  confinement: off\n");
    }
    output.push_str(&format!("  command: {}\n", redact_argv(&command)));
    if harness == Harness::Omp {
        output.push_str("omp extension install skipped (dry run)\n");
    }
    Ok(output)
}

fn validate_team_session(
    home: &Path,
    origin: TeamOrigin<'_>,
    session: &TeamSession,
) -> anyhow::Result<()> {
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(session, &resolved_cwd)?;
    let Some(request) = session.confine.as_ref() else {
        return Ok(());
    };
    let host_home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
    let runtime_dir =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?);
    let request = resolved_confine_request(request, &resolved_cwd, origin.file)?;
    let harness = Harness::infer(&session.command);
    let host = a2amx::confine::Host {
        home: host_home,
        layout: a2amx::layout::Layout::new(home.to_owned()),
        runtime_dir,
    };
    let preflight = a2amx::confine::ensure_grant_dirs(&request, harness, &host, false);
    a2amx::confine::plan_with_preflight(
        &request,
        harness,
        &resolved_cwd,
        &host,
        std::env::var_os("PATH"),
        preflight,
    )?;
    Ok(())
}

fn session_details(session: &TeamSession) -> Vec<(&'static str, String)> {
    [
        team_detail(session.team.as_ref()).map(|value| ("team", value)),
        session.role.as_ref().map(|value| ("role", value.clone())),
        session.attach.then(|| ("attach", "true".to_owned())),
        (!session.watch.is_empty()).then(|| ("watch", session.watch.join(", "))),
        (!session.control_from.is_empty())
            .then(|| ("control_from", session.control_from.join(", "))),
        session
            .heartbeat
            .as_ref()
            .map(|value| ("heartbeat", value.clone())),
        session
            .activity
            .as_ref()
            .map(|value| ("activity", activity_summary(value))),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn activity_summary(activity: &team::TeamActivity) -> String {
    [
        activity
            .working_after
            .as_ref()
            .map(|value| format!("working_after {value}")),
        activity
            .idle_after
            .as_ref()
            .map(|value| format!("idle_after {value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}

async fn start_planned_session(
    home: &Path,
    client: &mut Client,
    session: Box<TeamSession>,
    origin: TeamOrigin<'_>,
    cols: u16,
    rows: u16,
) -> anyhow::Result<(String, String)> {
    let heartbeat = session
        .heartbeat
        .as_deref()
        .map(|value| a2amx::messaging::parse_interval(value, "heartbeat"))
        .transpose()?
        .map(|duration| duration.as_secs());
    let activity = session
        .activity
        .as_ref()
        .map(|value| {
            parse_activity_watch(value.working_after.as_deref(), value.idle_after.as_deref())
        })
        .transpose()?
        .flatten();
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(&session, &resolved_cwd)?;
    let confine = session
        .confine
        .as_ref()
        .map(|request| resolved_confine_request(request, &resolved_cwd, origin.file))
        .transpose()?;
    let name = session.name;
    let result = create_session(
        home,
        client,
        NewOptions {
            name: Some(name.clone()),
            harness: None,
            role: session.role,
            remote: session.remote,
            team: session.team,
            no_authorize_peers: false,
            no_channel: false,
            confine,
            deliver: None,
            control_from: session.control_from,
            reset: session.reset.unwrap_or_default(),
            watch: session.watch,
            activity,
            heartbeat,
            command: session.command,
            cwd: resolved_cwd,
            cols,
            rows,
        },
    )
    .await
    .with_context(|| format!("failed {name}"))?;
    write_stdout(format!("started {name} {result}\n").into_bytes()).await?;
    Ok((name, result))
}

async fn write_auto_summaries(actions: &[team::Action]) -> anyhow::Result<()> {
    for action in actions {
        let team::Action::Start(session) = action else {
            continue;
        };
        let Some(confine) = session.confine.as_ref() else {
            continue;
        };
        if confine.reasons.is_empty() {
            continue;
        }
        let grants = confine
            .reasons
            .iter()
            .map(|reason| reason.grant.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write_stderr(
            format!(
                "auto confinement for session {}: grants {grants}\n",
                session.name
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(())
}

async fn issue_lease_code(client: &mut Client, remote_team: Option<String>) -> anyhow::Result<()> {
    if let Some(team) = remote_team {
        match client.request(Request::IssueLeaseCode { team }).await {
            Ok(Response::LeaseCode {
                code,
                agents,
                valid_secs,
            }) => {
                write_stdout(format!("remote messaging for: {}\n", agents.join(", ")).into_bytes())
                    .await?;
                write_stdout(
                    format!(
                        "post this code in the control channel within {}: {code}\n",
                        display_duration(Duration::from_secs(valid_secs))
                    )
                    .into_bytes(),
                )
                .await?;
            }
            Ok(Response::Error { message }) => {
                write_stderr(format!("warning: {message}\n").into_bytes()).await?;
            }
            Ok(other) => {
                write_stderr(
                    format!("warning: unexpected lease response: {other:?}\n").into_bytes(),
                )
                .await?;
            }
            Err(error) => {
                write_stderr(
                    format!("warning: could not obtain lease code: {error}\n").into_bytes(),
                )
                .await?;
            }
        }
    } else {
        write_stderr(
            b"remote messaging is set but the file has no team name; no lease can start\n".to_vec(),
        )
        .await?;
    }
    Ok(())
}

async fn wait_for_enter() -> anyhow::Result<()> {
    write_stderr(b"press Enter to attach to the session".to_vec()).await?;
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        io::stdin().read_line(&mut line)
    })
    .await??;
    Ok(())
}

async fn run_team_up(
    home: PathBuf,
    attach_options: AttachOptions,
    file: Option<PathBuf>,
    detach: bool,
    dry_run: bool,
    yes: bool,
    items: Vec<String>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let (wanted, file_dir, team_file) = if items.is_empty() {
        let source = resolve_team_file(&home, file).await?;
        let file_dir = source.dir.clone();
        let team_path = source.path.clone();
        let team_file =
            Some(tokio::task::spawn_blocking(move || std::fs::canonicalize(team_path)).await??);
        (read_team(source).await?, file_dir, team_file)
    } else {
        (team::flag_sessions(&items)?, cwd.clone(), None)
    };
    let attach_name = wanted
        .iter()
        .find(|session| session.attach)
        .map(|session| session.name.as_str());
    let mut client = Client::connect(&home).await?;
    let existing = request_sessions(&mut client).await?;
    let mut actions = match team::plan(&wanted, &existing) {
        Ok(actions) => actions,
        Err(conflicts) => {
            note_lost_messages(&home, &wanted).await;
            write_conflicts(&conflicts).await?;
            return Err(anyhow!("team has session conflicts"));
        }
    };
    let origin = TeamOrigin {
        dir: &file_dir,
        file: team_file.as_deref(),
    };
    resolve_auto_grants(&mut actions, origin).await?;
    for action in &actions {
        if let team::Action::Start(session) = action {
            validate_team_session(&home, origin, session)?;
        }
    }
    let has_auto = actions.iter().any(|action| {
        matches!(
            action,
            team::Action::Start(session) if session.auto_confine().is_some()
        )
    });
    let remote_requested = wanted.iter().any(|session| session.remote.is_some());
    let remote_team = wanted
        .iter()
        .find_map(|session| session.team.as_ref().map(|team| team.name.clone()));
    let accepted_all = if has_auto {
        load_accepted_grants(&home).await?
    } else {
        BTreeMap::new()
    };
    let project_key = if has_auto {
        Some(project_key(origin.dir)?.to_owned())
    } else {
        None
    };
    let accepted = project_key
        .as_ref()
        .and_then(|key| accepted_all.get(key))
        .cloned()
        .unwrap_or_default();
    let new_grants = team::new_auto_grants(&actions, &accepted);
    for output in team::escape_notices(&actions, a2amx::grant::all()) {
        write_stderr(output.into_bytes()).await?;
    }
    if !dry_run && !new_grants.is_empty() {
        confirm_new_grants(&home, origin.dir, &accepted_all, &new_grants, yes).await?;
    }
    write_auto_summaries(&actions).await?;
    if dry_run {
        let output = dry_run_report(&home, origin, &actions, &existing, &accepted).await?;
        write_stdout(output.into_bytes()).await?;
        note_lost_messages(&home, &wanted).await;
        return Ok(());
    }
    note_lost_messages(&home, &wanted).await;
    let (cols, rows) = terminal_size_with_default()?;
    let mut attached_id = None;
    // shortcut: the conflict check and the spawns are not atomic; a name taken in between
    // fails the spawn. A daemon-side batch request if concurrent clients make this matter.
    for action in actions {
        let (name, id) = match action {
            team::Action::AlreadyRunning { name, id } => {
                let mode = existing
                    .iter()
                    .find(|session| session.id == id)
                    .map_or("off", session_confinement_mode);
                write_stdout(
                    format!("already running {name} {id} (confine {mode})\n").into_bytes(),
                )
                .await?;
                (name, id)
            }
            team::Action::Start(session) => {
                start_planned_session(&home, &mut client, session, origin, cols, rows).await?
            }
        };
        if attach_name == Some(name.as_str()) {
            attached_id = Some(id);
        }
    }
    if remote_requested {
        issue_lease_code(&mut client, remote_team).await?;
    }
    if !detach
        && stdin_is_terminal()
        && let Some(session) = attached_id
    {
        let (attach_cols, attach_rows) = if remote_requested {
            wait_for_enter().await?;
            terminal_size_with_default()?
        } else {
            (cols, rows)
        };
        attach::run(
            client,
            &home,
            attach_options,
            session,
            false,
            attach_cols,
            attach_rows,
        )
        .await?;
    }
    Ok(())
}

async fn run_team_down(
    home: PathBuf,
    file: Option<PathBuf>,
    names: Vec<String>,
    now: bool,
) -> anyhow::Result<()> {
    let explicit = !names.is_empty();
    let names = if names.is_empty() {
        let source = resolve_team_file(&home, file).await?;
        read_team(source)
            .await?
            .into_iter()
            .map(|session| session.name)
            .collect()
    } else {
        names
    };
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    let mut failed = false;
    let mut missing_explicit = false;
    for name in names {
        let Some(session) = sessions
            .iter()
            .find(|session| session.name.as_deref() == Some(&name))
        else {
            write_stdout(format!("no session {name}\n").into_bytes()).await?;
            if explicit {
                missing_explicit = true;
            }
            continue;
        };
        let result = client
            .request(Request::Kill {
                session: session.id.clone(),
                now,
            })
            .await;
        let error = match result {
            Ok(Response::Ok) => {
                write_stdout(format!("killed {name} {}\n", session.id).into_bytes()).await?;
                continue;
            }
            Ok(Response::Error { message }) => message,
            Ok(other) => format!("unexpected daemon response: {other:?}"),
            Err(error) => error.to_string(),
        };
        write_stderr(format!("failed {name}: {error}\n").into_bytes()).await?;
        failed = true;
    }
    if missing_explicit {
        write_stderr(
            b"hint: names given to team down are full session names (with any team prefix); see a2amx list\n"
                .to_vec(),
        )
        .await?;
    }
    if failed {
        return Err(anyhow!("one or more team sessions could not be killed"));
    }
    Ok(())
}

async fn run_team_reset(
    home: PathBuf,
    file: Option<PathBuf>,
    names: Vec<String>,
    except: Vec<String>,
    include_attached: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let explicit = !names.is_empty();
    let names = if explicit {
        names
    } else {
        let source = resolve_team_file(&home, file).await?;
        read_team(source)
            .await?
            .into_iter()
            .map(|session| session.name)
            .collect()
    };
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    let targets = team::reset_targets(&names, explicit, &except, include_attached, &sessions);
    let running = targets
        .iter()
        .filter(|target| matches!(target.state, team::ResetState::Reset { .. }))
        .count();
    if running > 0 && !yes {
        let mut preview = String::new();
        for target in &targets {
            let team::ResetState::Reset { id } = &target.state else {
                continue;
            };
            let attached = sessions
                .iter()
                .find(|session| session.id == *id)
                .is_some_and(|session| session.attached);
            preview.push_str(&format!(
                "{} {}{}\n",
                target.name,
                id,
                if attached { " (attached)" } else { "" }
            ));
        }
        write_stdout(preview.into_bytes()).await?;
        let prompt = format!("Reset {running} session(s)? This clears their conversations. [y/N] ");
        if !confirm(
            &prompt,
            &format!("refusing to reset: {running} running session(s); pass --yes to reset them"),
        )
        .await?
        {
            return write_stdout(b"not reset\n".to_vec()).await;
        }
    }

    let mut failed = false;
    for target in targets {
        match target.state {
            team::ResetState::Reset { id } => match reset_one(&mut client, &id).await {
                Ok(steps) => {
                    let noun = if steps == 1 { "step" } else { "steps" };
                    write_stdout(
                        format!("reset {} {id}: {steps} {noun}\n", target.name).into_bytes(),
                    )
                    .await?;
                }
                Err(error) => {
                    write_stderr(format!("failed {}: {error}\n", target.name).into_bytes()).await?;
                    failed = true;
                }
            },
            team::ResetState::Missing => {
                write_stdout(format!("no session {}\n", target.name).into_bytes()).await?;
            }
            team::ResetState::Exited { id } => {
                write_stdout(format!("skipped {} {id} (exited)\n", target.name).into_bytes())
                    .await?;
            }
            team::ResetState::SkippedAttached { id } => {
                write_stdout(
                    format!(
                        "skipped {} {id} (attached; use --include-attached or name it)\n",
                        target.name
                    )
                    .into_bytes(),
                )
                .await?;
            }
        }
    }
    if failed {
        return Err(anyhow!("one or more team sessions could not be reset"));
    }
    Ok(())
}

async fn run_list(
    home: PathBuf,
    details: bool,
    team_filter: Option<String>,
    viewer: Option<String>,
) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::List).await?;
    let mut sessions = match response {
        Response::Sessions { sessions } => sessions,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
    let viewer_info = viewer.map(|name| {
        let session = sessions
            .iter()
            .find(|session| session.name.as_deref() == Some(name.as_str()))
            .or_else(|| sessions.iter().find(|session| session.id == name))
            .ok_or_else(|| anyhow!("unknown session {name}"))?;
        Ok::<_, anyhow::Error>((
            session.id.clone(),
            session.name.clone(),
            session.team.clone(),
        ))
    });
    let viewer_info = viewer_info.transpose()?;
    sessions.retain(|session| {
        let in_team = team_filter.as_deref().is_none_or(|team| {
            session
                .team
                .as_ref()
                .is_some_and(|scope| scope.name == team)
        });
        let visible_to_viewer =
            viewer_info
                .as_ref()
                .is_none_or(|(viewer_id, viewer_name, viewer_scope)| {
                    session.id == *viewer_id
                        || messaging::visible(
                            messaging::Party {
                                name: viewer_name.as_deref(),
                                team: viewer_scope.as_ref(),
                            },
                            messaging::Party {
                                name: session.name.as_deref(),
                                team: session.team.as_ref(),
                            },
                        )
                });
        in_team && visible_to_viewer
    });
    write_stdout(format_session_table(&sessions, details).into_bytes()).await
}
async fn run_confine_show(home: PathBuf, reference: Option<String>) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    if sessions.is_empty() {
        if let Some(reference) = reference {
            bail!("unknown session {reference}");
        }
        write_stdout(b"no sessions\n".to_vec()).await?;
        return Ok(());
    }
    let selected = match reference.as_deref() {
        Some(reference) => {
            let session = sessions
                .iter()
                .find(|session| {
                    session.id == reference || session.name.as_deref() == Some(reference)
                })
                .ok_or_else(|| anyhow!("unknown session {reference}"))?;
            vec![session]
        }
        None => sessions.iter().collect(),
    };
    let mut output = String::new();
    for session in selected {
        let name = session.name.as_deref().unwrap_or("-");
        let mode = session_confinement_mode(session);
        output.push_str(&format!(
            "{name} {} {} confine {mode}\n",
            session.id,
            session.harness.as_str()
        ));
        let Some(info) = &session.confine else {
            continue;
        };
        output.push_str("GRANT  SOURCE\n");
        for grant in &info.grants {
            let source = info
                .reasons
                .iter()
                .find(|reason| reason.grant == *grant)
                .map_or_else(
                    || "set".to_owned(),
                    |reason| format!("auto: {}", reason.reason),
                );
            output.push_str(&format!("{grant}  {source}\n"));
        }
        if !info.write.is_empty() {
            output.push_str(&format!("write: {}\n", info.write.join(", ")));
        }
    }
    write_stdout(output.into_bytes()).await
}

async fn run_kill(
    home: PathBuf,
    reference: Option<String>,
    exited: bool,
    yes: bool,
    now: bool,
) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    if exited {
        let mut found = false;
        let mut failed = false;
        for summary in sessions
            .iter()
            .filter(|summary| summary.exit_code.is_some())
        {
            found = true;
            let label = summary.name.as_deref().unwrap_or(&summary.id);
            match kill_session(&mut client, summary.id.clone(), false).await {
                Ok(()) => write_stdout(format!("removed {label}\n").into_bytes()).await?,
                Err(error) => {
                    write_stderr(format!("{label}: {error}\n").into_bytes()).await?;
                    failed = true;
                }
            }
        }
        if !found {
            return write_stdout(b"no exited sessions\n".to_vec()).await;
        }
        if failed {
            return Err(anyhow!("one or more exited sessions could not be removed"));
        }
        return Ok(());
    }

    let reference = reference.ok_or_else(|| anyhow!("kill requires a session or --exited"))?;
    let session = resolve_reference(&sessions, &reference);
    let running = sessions
        .iter()
        .any(|summary| summary.id == session && summary.exit_code.is_none());
    if running && !yes {
        let prompt = format!("Killing {session} ends a running session. Continue? [y/N] ");
        if !confirm(
            &prompt,
            &format!("refusing to kill: session {session} is running; pass --yes to end it"),
        )
        .await?
        {
            return write_stdout(b"not killed\n".to_vec()).await;
        }
    }
    kill_session(&mut client, session, now).await
}

async fn kill_session(client: &mut Client, session: String, now: bool) -> anyhow::Result<()> {
    match client.request(Request::Kill { session, now }).await? {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

async fn reset_one(client: &mut Client, session: &str) -> anyhow::Result<usize> {
    match client
        .request(Request::Reset {
            session: session.to_owned(),
        })
        .await?
    {
        Response::Reset { steps } => {
            usize::try_from(steps).map_err(|_| anyhow!("reset step count does not fit in usize"))
        }
        Response::Failed { code, message } => Err(anyhow!("{code}: {message}")),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

async fn run_reset(home: PathBuf, reference: String) -> anyhow::Result<()> {
    let session = resolve_session(&home, &reference).await?;
    let mut client = Client::connect(&home).await?;
    let steps = reset_one(&mut client, &session).await?;
    let noun = if steps == 1 { "step" } else { "steps" };
    write_stdout(format!("reset {session}: {steps} {noun}\n").into_bytes()).await?;
    Ok(())
}

async fn run_explain(home: PathBuf, reference: String) -> anyhow::Result<()> {
    let session = resolve_session(&home, &reference).await?;
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::Explain {
            session: session.clone(),
        })
        .await?;
    let info = match response {
        Response::Explain { info } => info,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected explain response: {other:?}")),
    };
    let mut output = String::new();
    if let Some(code) = info.exit_code {
        output.push_str(&format!(
            "{} ({}): {}, exited (code {code})\n",
            info.id,
            info.address,
            info.harness.as_str(),
        ));
    } else {
        output.push_str(&format!(
            "{} ({}): {}, running\n",
            info.id,
            info.address,
            info.harness.as_str()
        ));
        let activity = info.activity.map(Activity::as_str).unwrap_or("unknown");
        let cause = info.activity_cause.as_deref().unwrap_or("unknown");
        output.push_str(&format!("activity: {activity} ({cause})\n"));
        let ready = match info.ready {
            Some(true) => "yes",
            Some(false) if info.starting => "no (starting)",
            Some(false) => "no",
            None => "unknown",
        };
        output.push_str(&format!("ready: {ready}\n"));
        output.push_str(&format!(
            "needs input: {}\n",
            info.needs_input.as_deref().unwrap_or("no")
        ));
        if let Some(hold) = info.hold {
            output.push_str("hold: ");
            output.push_str(&hold.reason);
            if let Some(explanation) = hold.explanation {
                output.push_str(" - ");
                output.push_str(&explanation);
            }
            output.push('\n');
        } else {
            output.push_str("hold: none\n");
        }
        output.push_str(&format!("queued: {}\n", info.pending));
        if info.reset.allowed {
            output.push_str("reset: gate open\n");
        } else {
            output.push_str("reset: refused (");
            output.push_str(info.reset.code.as_deref().unwrap_or("unknown"));
            output.push_str("): ");
            output.push_str(info.reset.reason.as_deref().unwrap_or("unknown"));
            output.push('\n');
        }
    }

    let sessions = request_sessions(&mut client).await?;
    if let Some(summary) = sessions.into_iter().find(|summary| summary.id == session)
        && let Some(confine) = summary.confine
    {
        let default_hides = a2amx::confine::default_hide_names(summary.harness);
        let cwd = summary.cwd;
        let user_home = std::env::var_os("HOME").map(PathBuf::from);
        let help = tokio::task::spawn_blocking(move || {
            let exists = |path: &str| {
                if path == "~" {
                    return user_home.as_ref().is_some_and(|home| home.exists());
                }
                if let Some(relative) = path.strip_prefix("~/") {
                    return user_home
                        .as_ref()
                        .is_some_and(|home| home.join(relative).exists());
                }
                Path::new(path).exists()
            };
            a2amx::grant::denial_help(
                &confine,
                cwd.as_deref(),
                &default_hides,
                a2amx::grant::all(),
                &exists,
            )
        })
        .await?;
        output.push_str(&help);
    }
    write_stdout(output.into_bytes()).await
}

async fn run_screen(home: PathBuf, session: String, rows: Option<usize>) -> anyhow::Result<()> {
    let session = resolve_session(&home, &session).await?;
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::Screen { session }).await?;
    let lines = match response {
        Response::Screen { lines } => lines,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected screen response: {other:?}")),
    };
    let lines = if let Some(rows) = rows {
        let start = lines.len().saturating_sub(rows);
        lines.into_iter().skip(start).collect::<Vec<_>>()
    } else {
        lines
    };
    if lines.is_empty() {
        return write_stdout(Vec::new()).await;
    }
    write_stdout(format!("{}\n", lines.join("\n")).into_bytes()).await
}

async fn run_messages(
    home: PathBuf,
    session: Option<String>,
    state: Option<String>,
) -> anyhow::Result<()> {
    let session = match session {
        Some(reference) => Some(resolve_session(&home, &reference).await?),
        None => None,
    };
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::ListMessages { session, state })
        .await?;
    let messages = match response {
        Response::Messages { messages } => messages,
        Response::Error { message } | Response::Failed { message, .. } => {
            return Err(anyhow!(message));
        }
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
    write_stdout(format_messages_table(&messages).into_bytes()).await
}

async fn run_cancel(home: PathBuf, message: String) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::CancelMessage { id: message })
        .await?;
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } | Response::Failed { message, .. } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

async fn run_attach_command(
    home: PathBuf,
    attach_options: AttachOptions,
    session: String,
    force: bool,
) -> anyhow::Result<()> {
    if !stdin_is_terminal() {
        return Err(anyhow!("attach needs a terminal on stdin"));
    }
    let session = resolve_session(&home, &session).await?;
    let client = Client::connect(&home).await?;
    let (cols, rows) = terminal_size_with_default()?;
    attach::run(client, &home, attach_options, session, force, cols, rows).await
}
