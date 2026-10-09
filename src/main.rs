use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::Duration;

use anyhow::{Context, anyhow};
use clap::Parser;
use tokio::sync::mpsc;

use a2amx::bridge;
use a2amx::cli::{Cli, Command, DaemonAction, TeamAction};
use a2amx::client::{Attachment, Client};
use a2amx::daemon::{self, Daemon, DaemonConfig};
use a2amx::emulator::Scroll;
use a2amx::harness::{Deliver, Harness};
use a2amx::hook;
use a2amx::mcp;
use a2amx::messaging::{self, TeamScope, display_duration, sgr_mouse_report_len};
use a2amx::names;
use a2amx::picker::{PickerAction, PickerParser, scroll_into_view, target};
use a2amx::prefix::{Action, Command as PrefixCommand, PrefixMachine};
use a2amx::quota;
use a2amx::status;
use a2amx::team::{self, TeamSession};
use a2amx::wire::{
    Activity, ClientFrame, MessageInfo, Request, Response, ServerFrame, SessionSummary, StatusInfo,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const INPUT_CHUNK: usize = 8 * 1024;
const DAEMON_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const MOUSE_CAPTURE: &[u8] = b"\x1b[?1000h\x1b[?1006h";
const RESTORE_TERMINAL: &[u8] = b"\x1b[0m\x1b[?2004l\x1b[?1004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1l\x1b>\x1b[?25h\x1b[?1049l";

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let result = match Cli::try_parse() {
        Ok(cli) => dispatch(cli).await,
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
    let prefix = cli.prefix;
    let home_arg = cli.home;
    match &cli.command {
        Command::Mcp { channel } => {
            return mcp::run(*channel)
                .await
                .map(|_| std::process::ExitCode::SUCCESS);
        }
        Command::Hook => return hook::run().await.map(|_| std::process::ExitCode::SUCCESS),
        Command::OmpBridge => return bridge::run().await.map(|_| std::process::ExitCode::SUCCESS),
        _ => {}
    }
    let home = resolve_home(home_arg)?;
    let result = match cli.command {
        Command::Daemon { action } => match action {
            DaemonAction::Start {
                listen,
                host_name,
                foreground,
            } => {
                if foreground {
                    run_daemon(home, listen, host_name).await
                } else {
                    run_daemon_background(home, listen, host_name).await
                }
            }
            DaemonAction::Status => return run_daemon_status(home).await,
            DaemonAction::Stop { yes, now } => run_stop(home, yes, now).await,
        },
        options @ Command::New { .. } => run_new(home, prefix, options).await,
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
                items,
            } => run_team_up(home, prefix, file, detach, dry_run, items).await,
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
            run_attach_command(home, prefix, session, force).await
        }
        Command::Kill {
            session,
            exited,
            yes,
            now,
        } => run_kill(home, session, exited, yes, now).await,
        Command::Reset { session } => run_reset(home, session).await,
        Command::Screen { session, rows } => run_screen(home, session, rows).await,
        Command::Messages { session, state } => run_messages(home, session, state).await,
        Command::Cancel { message } => run_cancel(home, message).await,
        Command::Mcp { .. } | Command::Hook | Command::OmpBridge => {
            unreachable!("early dispatch returned before resolving home")
        }
    };
    result.map(|_| std::process::ExitCode::SUCCESS)
}

fn resolve_home(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(home) = explicit {
        return Ok(home);
    }
    if let Some(home) = std::env::var_os("A2AMX_HOME") {
        return Ok(PathBuf::from(home));
    }
    if let Some(base) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(base).join("a2amx"));
    }
    if let Some(base) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(base).join(".local/state/a2amx"));
    }
    Err(anyhow!("cannot determine the a2amx state directory"))
}

async fn run_daemon(
    home: PathBuf,
    listen: Vec<std::net::SocketAddr>,
    host_name: Option<String>,
) -> anyhow::Result<()> {
    let listen = if listen.is_empty() {
        vec![
            "127.0.0.1:0"
                .parse()
                .context("default daemon listen address")?,
        ]
    } else {
        listen
    };
    let daemon = Daemon::start(DaemonConfig {
        state_dir: home,
        listen,
        host_name,
        limits: a2amx::messaging::Limits::default(),
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
) -> anyhow::Result<()> {
    if Client::connect(&home).await.is_ok() {
        let running = read_addrs(&home).await.unwrap_or_default();
        return Err(anyhow!(
            "a2amx daemon is already running (listening on {})",
            running.lines().next().unwrap_or("unknown address")
        ));
    }
    let log_path = home.join("daemon.log");
    let (log, exe) = {
        let home = home.clone();
        let log_path = log_path.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
            daemon::ensure_state_dir(&home)?;
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
    let path = home.join("addr");
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

async fn run_new(home: PathBuf, prefix: u8, options: Command) -> anyhow::Result<()> {
    let Command::New {
        detach,
        name,
        harness,
        deliver,
        no_authorize_peers,
        no_channel,
        reset,
        control_from,
        watch,
        heartbeat,
        command,
    } = options
    else {
        return Err(anyhow!("expected new session options"));
    };
    let heartbeat = heartbeat
        .as_deref()
        .map(a2amx::messaging::parse_interval)
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
    let session = create_session(
        &home,
        &mut client,
        NewOptions {
            name,
            harness,
            role: None,
            team: None,
            deliver,
            no_authorize_peers,
            no_channel,
            reset,
            control_from,
            watch,
            heartbeat,
            command,
            cwd: std::env::current_dir()?,
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
    run_attachment(client, &home, prefix, session, false, cols, rows).await
}

struct NewOptions {
    name: Option<String>,
    role: Option<String>,
    team: Option<TeamScope>,
    harness: Option<Harness>,
    deliver: Option<Deliver>,
    no_authorize_peers: bool,
    no_channel: bool,
    reset: Vec<String>,
    control_from: Vec<String>,
    watch: Vec<String>,
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
            let wire_role = role.map(str::to_owned);
            let authorize_peers = !no_authorize_peers;
            if no_channel {
                Ok(tokio::task::spawn_blocking(move || {
                    a2amx::harness::wire_claude_argv(
                        command,
                        &exe,
                        &wire_cwd,
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
                        authorize_peers,
                        wire_role.as_deref(),
                    )
                })
                .await??)
            }
        }
        Harness::Omp if !install_omp => Ok(command),
        Harness::Omp => {
            let install_home = home.to_owned();
            let installed =
                tokio::task::spawn_blocking(move || a2amx::omp::install(&install_home)).await??;
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
        team,
        no_authorize_peers,
        no_channel,
        reset,
        control_from,
        watch,
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
        a2amx::messaging::parse_interval(&format!("{seconds}s"))?;
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
    if harness == Harness::Codex {
        if let Some(role) = &role {
            env.push((a2amx::codex::ROLE_ENV.to_owned(), role.clone()));
        }
    }
    if harness == Harness::Claude && !no_channel {
        env.push((a2amx::harness::CHANNEL_ENV.to_owned(), "1".to_owned()));
    }
    let response = client
        .request(Request::NewSession {
            argv: command,
            cols,
            rows,
            cwd: Some(cwd),
            env,
            reset,
            control_from,
            watch,
            name,
            harness,
            deliver,
            team,
            role,
            heartbeat,
        })
        .await?;
    match response {
        Response::Created { session } => Ok(session),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

async fn read_team(file: PathBuf) -> anyhow::Result<Vec<TeamSession>> {
    tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("cannot read team file {}", file.display()))?;
        team::parse(&text).with_context(|| format!("team file {}", file.display()))
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
    let cwd = std::env::current_dir()?;
    let file = cwd.join(file.unwrap_or_else(|| PathBuf::from("a2amx.toml")));
    let wanted = read_team(file).await?;
    let mut client = Client::connect(&home).await?;
    let existing = request_sessions(&mut client).await?;
    let entries = team::inspect(&wanted, &existing);
    let mut output = String::from("NAME TEAM STATE NOTE\n");
    let mut has_conflict = false;
    for (session, entry) in wanted.iter().zip(entries) {
        let (state, note) = match entry.state {
            team::EntryState::Missing => ("missing".to_owned(), "-".to_owned()),
            team::EntryState::Running { id } => ("running".to_owned(), id),
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
                (state.to_owned(), note)
            }
        };
        output.push_str(&format!(
            "{} {} {} {}\n",
            session.name,
            team_label(session.team.as_ref()),
            state,
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

async fn run_team_up(
    home: PathBuf,
    prefix: u8,
    file: Option<PathBuf>,
    detach: bool,
    dry_run: bool,
    items: Vec<String>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let (wanted, file_dir) = if items.is_empty() {
        let file = cwd.join(file.unwrap_or_else(|| PathBuf::from("a2amx.toml")));
        let file_dir = file.parent().unwrap_or(&cwd).to_owned();
        (read_team(file).await?, file_dir)
    } else {
        (team::flag_sessions(&items)?, cwd.clone())
    };
    let attach_name = wanted
        .iter()
        .find(|session| session.attach)
        .map(|session| session.name.as_str());
    let mut client = Client::connect(&home).await?;
    let existing = request_sessions(&mut client).await?;
    let actions = match team::plan(&wanted, &existing) {
        Ok(actions) => actions,
        Err(conflicts) => {
            note_lost_messages(&home, &wanted).await;
            write_conflicts(&conflicts).await?;
            return Err(anyhow!("team has session conflicts"));
        }
    };
    if dry_run {
        let mut output = String::new();
        for action in actions {
            match action {
                team::Action::AlreadyRunning { name, id } => {
                    output.push_str(&format!("already running {name} {id}\n"));
                }
                team::Action::Start(session) => {
                    let harness = Harness::infer(&session.command);
                    let resolved_cwd = session
                        .cwd
                        .as_deref()
                        .map_or_else(|| cwd.clone(), |path| file_dir.join(path));
                    let command = wired_command(
                        &home,
                        session.command.clone(),
                        harness,
                        &resolved_cwd,
                        session.role.as_deref(),
                        false,
                        false,
                        false,
                    )
                    .await?;
                    output.push_str(&format!("would start {}\n", session.name));
                    output.push_str(&format!("  harness: {}\n", harness.as_str()));
                    output.push_str(&format!("  cwd: {}\n", resolved_cwd.display()));
                    if let Some(team) = team_detail(session.team.as_ref()) {
                        output.push_str(&format!("  team: {team}\n"));
                    }
                    if let Some(role) = &session.role {
                        output.push_str(&format!("  role: {role}\n"));
                    }
                    if session.attach {
                        output.push_str("  attach: true\n");
                    }
                    if !session.watch.is_empty() {
                        output.push_str(&format!("  watch: {}\n", session.watch.join(", ")));
                    }
                    if !session.control_from.is_empty() {
                        output.push_str(&format!(
                            "  control_from: {}\n",
                            session.control_from.join(", ")
                        ));
                    }
                    if let Some(heartbeat) = &session.heartbeat {
                        output.push_str(&format!("  heartbeat: {heartbeat}\n"));
                    }
                    output.push_str(&format!("  command: {}\n", redact_argv(&command)));
                    if harness == Harness::Omp {
                        output.push_str("omp extension install skipped (dry run)\n");
                    }
                }
            }
        }
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
                write_stdout(format!("already running {name} {id}\n").into_bytes()).await?;
                (name, id)
            }
            team::Action::Start(session) => {
                let heartbeat = session
                    .heartbeat
                    .as_deref()
                    .map(a2amx::messaging::parse_interval)
                    .transpose()?
                    .map(|duration| duration.as_secs());
                let name = session.name;
                let result = create_session(
                    &home,
                    &mut client,
                    NewOptions {
                        name: Some(name.clone()),
                        harness: None,
                        role: session.role,
                        team: session.team,
                        deliver: None,
                        no_authorize_peers: false,
                        no_channel: false,
                        reset: session.reset.unwrap_or_default(),
                        control_from: session.control_from,
                        watch: session.watch,
                        heartbeat,
                        command: session.command,
                        cwd: session
                            .cwd
                            .map_or_else(|| cwd.clone(), |path| file_dir.join(path)),
                        cols,
                        rows,
                    },
                )
                .await;
                let id = result.with_context(|| format!("failed {name}"))?;
                write_stdout(format!("started {name} {id}\n").into_bytes()).await?;
                (name, id)
            }
        };
        if attach_name == Some(name.as_str()) {
            attached_id = Some(id);
        }
    }
    if !detach && stdin_is_terminal() {
        if let Some(session) = attached_id {
            run_attachment(client, &home, prefix, session, false, cols, rows).await?;
        }
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
        let file =
            std::env::current_dir()?.join(file.unwrap_or_else(|| PathBuf::from("a2amx.toml")));
        read_team(file)
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
        let file =
            std::env::current_dir()?.join(file.unwrap_or_else(|| PathBuf::from("a2amx.toml")));
        read_team(file)
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
    prefix: u8,
    session: String,
    force: bool,
) -> anyhow::Result<()> {
    if !stdin_is_terminal() {
        return Err(anyhow!("attach needs a terminal on stdin"));
    }
    let session = resolve_session(&home, &session).await?;
    let client = Client::connect(&home).await?;
    let (cols, rows) = terminal_size_with_default()?;
    run_attachment(client, &home, prefix, session, force, cols, rows).await
}

async fn run_attachment(
    client: Client,
    home: &Path,
    prefix: u8,
    session: String,
    force: bool,
    cols: u16,
    rows: u16,
) -> anyhow::Result<()> {
    let attachment = client
        .attach_status(&session, force, cols, pty_rows(rows, true))
        .await?;
    let output = Output::start()?;
    let mut terminal = TerminalGuard::enter(output.handle()?)?;
    output.send(b"\x1b[?1049h".to_vec())?;
    // shortcut: the outer terminal title is not restored on exit.
    output.send(format!("\x1b]2;a2amx: {session}\x07").into_bytes())?;

    let loop_result = run_attachment_loop(
        attachment,
        home,
        prefix,
        session,
        cols,
        rows,
        output.handle()?,
    )
    .await;
    let restore_result = terminal.restore().await;
    let finish_result = tokio::task::spawn_blocking(move || output.finish()).await?;
    restore_result?;
    finish_result?;
    let outcome = loop_result?;
    write_stderr(format!("{}\n", outcome.message).into_bytes()).await
}

fn pty_rows(rows: u16, visible: bool) -> u16 {
    if visible && rows >= 4 {
        rows - 2
    } else if visible && rows == 3 {
        rows - 1
    } else {
        rows
    }
}

struct AttachOutcome {
    message: String,
}

struct AttachmentState {
    attachment: Option<Attachment>,
    home: PathBuf,
    session: String,
    cols: u16,
    rows: u16,
    prefix_machine: PrefixMachine,
    prefix_byte: u8,
    after_prefix: bool,
    status_visible: bool,
    status: Option<StatusInfo>,
    session_mouse: bool,
    scroll_mode: bool,
    scroll_parser: ScrollParser,
    picker: Option<PickerState>,
    output: OutputHandle,
}

impl AttachmentState {
    fn new(
        attachment: Attachment,
        home: &Path,
        prefix: u8,
        session: String,
        cols: u16,
        rows: u16,
        output: OutputHandle,
    ) -> Self {
        Self {
            attachment: Some(attachment),
            home: home.to_owned(),
            session,
            cols,
            rows,
            prefix_machine: PrefixMachine::new(prefix),
            prefix_byte: prefix,
            after_prefix: false,
            status_visible: true,
            status: None,
            session_mouse: false,
            scroll_mode: false,
            scroll_parser: ScrollParser::default(),
            picker: None,
            output,
        }
    }

    fn draw_status(&self) -> anyhow::Result<()> {
        if self.status_visible && self.picker.is_none() && !self.scroll_mode {
            self.output.send(status::render(
                &self.session,
                self.status.as_ref(),
                self.prefix_byte,
                self.cols,
                self.rows,
            ))?;
        }
        Ok(())
    }

    async fn handle_input(&mut self, bytes: &[u8]) -> anyhow::Result<Option<AttachOutcome>> {
        let mut offset = 0;
        while offset < bytes.len() {
            let report = if self.session_mouse {
                None
            } else {
                find_mouse_report(bytes, offset)
            };
            if let Some((start, end)) = report {
                if start == offset {
                    let button = bytes[offset + 3..]
                        .iter()
                        .take_while(|byte| byte.is_ascii_digit())
                        .fold(0u16, |value, byte| {
                            value
                                .saturating_mul(10)
                                .saturating_add(u16::from(byte - b'0'))
                        });
                    self.handle_mouse_button(button).await?;
                    offset = end;
                    continue;
                }
            }
            let segment_end = report.map_or(bytes.len(), |(start, _)| start);
            if self.picker.is_some() {
                if let Some(outcome) = self
                    .handle_picker_input(&bytes[offset..offset + 1], bytes == [0x1b])
                    .await?
                {
                    return Ok(Some(outcome));
                }
                offset += 1;
            } else if self.scroll_mode {
                for key in self.scroll_parser.feed(&bytes[offset..offset + 1]) {
                    match key {
                        ScrollKey::Scroll(scroll) => {
                            if let Some(attached) = self.attachment.as_mut() {
                                attached.send(ClientFrame::Scroll(scroll)).await?;
                            }
                        }
                        ScrollKey::Quit => {
                            if let Some(attached) = self.attachment.as_mut() {
                                attached.send(ClientFrame::Scroll(Scroll::Bottom)).await?;
                                attached.send(ClientFrame::Redraw).await?;
                            }
                            self.scroll_mode = false;
                            self.scroll_parser.clear();
                        }
                    }
                }
                offset += 1;
            } else {
                // End a batch at the prefix and consume its next byte separately:
                // a command can change how the remaining bytes must be interpreted.
                let count = if self.after_prefix {
                    self.after_prefix = false;
                    1
                } else if let Some(index) = bytes[offset..segment_end]
                    .iter()
                    .position(|b| *b == self.prefix_byte)
                {
                    self.after_prefix = true;
                    index + 1
                } else {
                    segment_end - offset
                };
                let actions = self.prefix_machine.feed(&bytes[offset..offset + count]);
                if let Some(outcome) = self.handle_prefix_actions(actions).await? {
                    return Ok(Some(outcome));
                }
                offset += count;
            }
        }
        Ok(None)
    }

    async fn handle_prefix_actions(
        &mut self,
        actions: Vec<Action>,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        for action in actions {
            match action {
                Action::Forward(input) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Input(input)).await?;
                    }
                }
                Action::Command(PrefixCommand::Detach) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Detach).await?;
                    }
                    return Ok(Some(AttachOutcome {
                        message: format!("[detached from {}]", self.session),
                    }));
                }
                Action::Command(PrefixCommand::ScrollMode) => {
                    self.enter_scroll_mode().await?;
                }
                Action::Command(PrefixCommand::SessionPicker) => {
                    let state = PickerState::open(
                        &self.home,
                        &self.session,
                        self.cols,
                        self.rows,
                        &self.output,
                    )
                    .await?;
                    self.picker = Some(state);
                }
                Action::Command(PrefixCommand::StatusLine) => {
                    self.status_visible = !self.status_visible;
                    if let Some(attached) = self.attachment.as_mut() {
                        attached
                            .send(ClientFrame::Resize {
                                cols: self.cols,
                                rows: pty_rows(self.rows, self.status_visible),
                            })
                            .await?;
                    }
                }
                Action::Command(PrefixCommand::Release) => {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Release).await?;
                    }
                }
            }
        }
        Ok(None)
    }

    async fn enter_scroll_mode(&mut self) -> anyhow::Result<()> {
        self.scroll_mode = true;
        self.scroll_parser.clear();
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Redraw).await?;
        }
        Ok(())
    }

    async fn handle_mouse_button(&mut self, button: u16) -> anyhow::Result<()> {
        if self.picker.is_some() {
            return Ok(());
        }
        match button {
            64 => {
                if !self.scroll_mode {
                    self.enter_scroll_mode().await?;
                }
                // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
                for _ in 0..3 {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(Scroll::LineUp)).await?;
                    }
                }
            }
            65 if self.scroll_mode => {
                // shortcut: the wheel uses a fixed three-line step; make it configurable only if asked.
                for _ in 0..3 {
                    if let Some(attached) = self.attachment.as_mut() {
                        attached.send(ClientFrame::Scroll(Scroll::LineDown)).await?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn scan_session_mouse(&mut self, bytes: &[u8]) -> bool {
        let mut found = false;
        let mut offset = 0;
        while offset + 3 < bytes.len() {
            if bytes[offset] == 0x1b
                && bytes.get(offset + 1) == Some(&b'[')
                && bytes.get(offset + 2) == Some(&b'?')
            {
                let first = offset + 3;
                let mut end = first;
                while end < bytes.len() && bytes[end].is_ascii_digit() {
                    end += 1;
                }
                if end > first && matches!(bytes.get(end), Some(b'h' | b'l')) {
                    let mode = bytes[first..end].iter().fold(0u16, |value, byte| {
                        value
                            .saturating_mul(10)
                            .saturating_add(u16::from(byte - b'0'))
                    });
                    if matches!(mode, 1000 | 1002 | 1003) {
                        found = true;
                        self.session_mouse = bytes[end] == b'h';
                    }
                }
            }
            offset += 1;
        }
        found
    }

    async fn restore_current(&mut self) -> anyhow::Result<bool> {
        if self.attachment.is_none() {
            let attach_result = match Client::connect(&self.home).await {
                Ok(client) => {
                    client
                        .attach_status(
                            &self.session,
                            false,
                            self.cols,
                            pty_rows(self.rows, self.status_visible),
                        )
                        .await
                }
                Err(error) => Err(error),
            };
            match attach_result {
                Ok(next) => self.attachment = Some(next),
                Err(error) => {
                    if let Some(state) = self.picker.as_mut() {
                        state.footer = Some(error.to_string());
                        let _ = state.refresh().await;
                        state.render(&self.output)?;
                    }
                    return Ok(false);
                }
            }
        }
        if let Some(attached) = self.attachment.as_mut() {
            attached.send(ClientFrame::Redraw).await?;
        }
        self.picker = None;
        Ok(true)
    }

    async fn handle_picker_input(
        &mut self,
        bytes: &[u8],
        escape_alone: bool,
    ) -> anyhow::Result<Option<AttachOutcome>> {
        let actions = match self.picker.as_mut() {
            Some(state) => state.feed(bytes, escape_alone),
            None => return Ok(None),
        };
        for action in actions {
            match action {
                PickerAction::Cancel => {
                    if !self.restore_current().await? {
                        continue;
                    }
                }
                PickerAction::Move(delta) => {
                    if let Some(state) = self.picker.as_mut() {
                        state.move_selection(delta);
                        state.render(&self.output)?;
                    }
                }
                PickerAction::Page(_) | PickerAction::Home | PickerAction::End => {
                    if let Some(state) = self.picker.as_mut() {
                        state.set_target(action);
                        state.render(&self.output)?;
                    }
                }
                PickerAction::Select => {
                    let Some(selected) = self.picker.as_ref().and_then(PickerState::selected_id)
                    else {
                        continue;
                    };
                    if selected == self.session {
                        if !self.restore_current().await? {
                            continue;
                        }
                        continue;
                    }
                    if let Some(attached) = self.attachment.as_mut() {
                        let _ = attached.send(ClientFrame::Detach).await;
                        let _ = tokio::time::timeout(Duration::from_secs(5), attached.recv()).await;
                    }
                    self.attachment = None;
                    let attach_result = match Client::connect(&self.home).await {
                        Ok(client) => {
                            client
                                .attach_status(
                                    &selected,
                                    false,
                                    self.cols,
                                    pty_rows(self.rows, self.status_visible),
                                )
                                .await
                        }
                        Err(error) => Err(error),
                    };
                    match attach_result {
                        Ok(next) => {
                            self.attachment = Some(next);
                            self.session = selected;
                            self.status = None;
                            self.output
                                .send(format!("\x1b]2;a2amx: {}\x07", self.session).into_bytes())?;
                            self.picker = None;
                        }
                        Err(error) => {
                            if let Some(state) = self.picker.as_mut() {
                                state.footer = Some(error.to_string());
                                state.refresh().await?;
                                state.render(&self.output)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(None)
    }
}

// shortcut: a report split across input chunks is typed input; reassemble only if it proves noisy.
fn find_mouse_report(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    let mut offset = start;
    while offset < bytes.len() {
        if bytes[offset] == 0x1b
            && bytes.get(offset + 1) == Some(&b'[')
            && bytes.get(offset + 2) == Some(&b'<')
            && let Some(end) = sgr_mouse_report_len(bytes, offset)
        {
            return Some((offset, end));
        }
        offset += 1;
    }
    None
}

async fn run_attachment_loop(
    attachment: Attachment,
    home: &Path,
    prefix: u8,
    session: String,
    cols: u16,
    rows: u16,
    output: OutputHandle,
) -> anyhow::Result<AttachOutcome> {
    let mut state = AttachmentState::new(attachment, home, prefix, session, cols, rows, output);
    let (input_tx, mut input_rx) = mpsc::channel::<Option<Vec<u8>>>(8);
    spawn_stdin_reader(input_tx)?;
    let mut sigwinch =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())?;

    loop {
        let has_attachment = state.attachment.is_some();
        tokio::select! {
            input = input_rx.recv() => {
                let Some(input) = input else {
                    if let Some(attached) = state.attachment.as_mut() {
                        let _ = attached.send(ClientFrame::Detach).await;
                    }
                    return Ok(AttachOutcome {
                        message: format!("[detached from {}]", state.session),
                    });
                };
                let Some(bytes) = input else {
                    if let Some(attached) = state.attachment.as_mut() {
                        let _ = attached.send(ClientFrame::Detach).await;
                    }
                    return Ok(AttachOutcome {
                        message: format!("[detached from {}]", state.session),
                    });
                };
                if let Some(outcome) = state.handle_input(&bytes).await? {
                    return Ok(outcome);
                }
            }
            _ = sigwinch.recv() => {
                if let Ok((new_cols, new_rows)) = terminal_size_with_default() {
                    state.cols = new_cols;
                    state.rows = new_rows;
                    if let Some(picker) = state.picker.as_mut() {
                        picker.cols = new_cols;
                        picker.rows = new_rows;
                        picker.recompute_offset();
                        picker.render(&state.output)?;
                    }
                    if let Some(attached) = state.attachment.as_mut() {
                        attached.send(ClientFrame::Resize { cols: new_cols, rows: pty_rows(new_rows, state.status_visible) }).await?;
                    }
                }
            }
            frame = recv_attachment(&mut state.attachment), if has_attachment => {
                let Some(frame) = frame else { continue; };
                let frame = frame?;
                let Some(frame) = frame else {
                    return Ok(AttachOutcome {
                        message: format!("[detached from {}]", state.session),
                    });
                };
                match frame {
                    ServerFrame::Data(data) => {
                        let has_mouse_mode = state.scan_session_mouse(&data);
                        if state.picker.is_none() {
                            state.output.send(data)?;
                            if has_mouse_mode && !state.session_mouse {
                                state.output.send(MOUSE_CAPTURE.to_vec())?;
                            }
                            state.draw_status()?;
                            if state.scroll_mode {
                                state.output.send(scroll_status(state.cols, state.rows))?;
                            }
                        }
                    }
                    ServerFrame::Status(info) => {
                        state.status = Some(info);
                        state.draw_status()?;
                    }
                    ServerFrame::Exit(code) => {
                        return Ok(AttachOutcome {
                            message: format!("[session {} exited with code {code}]", state.session),
                        });
                    }
                    ServerFrame::Detached(reason) => {
                        return Ok(AttachOutcome {
                            message: format!("[detached: {reason}]"),
                        });
                    }
                }
            }
        }
        let output = state.output.clone();
        tokio::task::spawn_blocking(move || output.flush()).await??;
    }
}

async fn recv_attachment(
    attachment: &mut Option<Attachment>,
) -> Option<anyhow::Result<Option<ServerFrame>>> {
    let attachment = attachment.as_mut()?;
    Some(attachment.recv().await)
}

fn spawn_stdin_reader(sender: mpsc::Sender<Option<Vec<u8>>>) -> anyhow::Result<()> {
    thread::Builder::new()
        .name("a2amx-stdin-reader".to_owned())
        .spawn(move || {
            let stdin = io::stdin();
            let mut stdin = stdin.lock();
            let mut buf = [0u8; INPUT_CHUNK];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => {
                        let _ = sender.blocking_send(None);
                        return;
                    }
                    Ok(len) => {
                        if sender.blocking_send(Some(buf[..len].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = sender.blocking_send(None);
                        return;
                    }
                }
            }
        })
        .context("start stdin reader")?;
    Ok(())
}

fn stdin_is_terminal() -> bool {
    let stdin = io::stdin();
    rustix::termios::isatty(stdin.as_fd())
}

fn terminal_size_with_default() -> anyhow::Result<(u16, u16)> {
    if !stdin_is_terminal() {
        return Ok((DEFAULT_COLS, DEFAULT_ROWS));
    }
    let stdin = io::stdin();
    let size = rustix::termios::tcgetwinsize(stdin.as_fd())?;
    Ok((
        if size.ws_col == 0 {
            DEFAULT_COLS
        } else {
            size.ws_col
        },
        if size.ws_row == 0 {
            DEFAULT_ROWS
        } else {
            size.ws_row
        },
    ))
}

const SESSION_HEADERS: [&str; 12] = [
    "ID", "NAME", "HARNESS", "STATE", "UPTIME", "ACTIVITY", "IN-STATE", "ATTACHED", "PENDING",
    "HELD", "QUOTA", "SIZE",
];
const DETAIL_HEADERS: [&str; 14] = [
    "ID", "NAME", "HARNESS", "STATE", "UPTIME", "ACTIVITY", "IN-STATE", "ATTACHED", "PENDING",
    "HELD", "QUOTA", "SIZE", "CWD", "COMMAND",
];
const PICKER_HEADERS: [&str; 11] = [
    "ID", "NAME", "HARNESS", "STATE", "ACTIVITY", "ATTACHED", "PENDING", "HELD", "QUOTA", "SIZE",
    "CWD",
];
const MESSAGE_HEADERS: [&str; 6] = ["ID", "FROM", "TO", "STATE", "DETAIL", "SUBJECT"];

fn session_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 10]> {
    sessions
        .iter()
        .map(|session| {
            [
                session.id.clone(),
                session.name.clone().unwrap_or_else(|| "-".to_owned()),
                session.harness.as_str().to_owned(),
                match session.exit_code {
                    Some(code) => format!("exited({code})"),
                    None => "running".to_owned(),
                },
                session.activity.map_or("-", Activity::as_str).to_owned(),
                if session.attached {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
                session.pending.to_string(),
                session
                    .hold_reason
                    .clone()
                    .unwrap_or_else(|| "-".to_owned()),
                quota::cell(session.quota),
                format!("{}x{}", session.cols, session.rows),
            ]
        })
        .collect()
}

fn picker_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 11]> {
    session_value_rows(sessions)
        .into_iter()
        .zip(sessions)
        .map(|(row, session)| {
            let [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
            ] = row;
            [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
                session.cwd.clone().unwrap_or_else(|| "-".to_owned()),
            ]
        })
        .collect()
}

fn message_value_rows(messages: &[MessageInfo]) -> Vec<[String; 6]> {
    messages
        .iter()
        .map(|message| {
            [
                message.id.clone(),
                message.from.clone(),
                message.to.clone(),
                message.state.clone(),
                message
                    .detail
                    .clone()
                    .or_else(|| message.hold_reason.clone())
                    .or_else(|| match message.evidence.as_deref() {
                        Some("submission_observed") | Some("native_receipt") => {
                            message.evidence.clone()
                        }
                        _ => None,
                    })
                    .unwrap_or_else(|| "-".to_owned()),
                message.subject.clone(),
            ]
        })
        .collect()
}

fn column_widths<R: AsRef<[String]>>(headers: &[&str], rows: &[R]) -> Vec<usize> {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (index, value) in row.as_ref().iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    widths
}

fn format_table<R: AsRef<[String]>>(headers: &[&str], rows: &[R]) -> String {
    let widths = column_widths(headers, rows);
    let mut output = String::new();
    output.push_str(&format_row(headers, &widths));
    output.push('\n');
    for row in rows {
        output.push_str(&format_row(row.as_ref(), &widths));
        output.push('\n');
    }
    output
}

fn session_team_cells(sessions: &[SessionSummary]) -> Option<Vec<String>> {
    sessions
        .iter()
        .any(|session| session.team.is_some())
        .then(|| {
            sessions
                .iter()
                .map(|session| {
                    session.team.as_ref().map_or_else(
                        || "-".to_owned(),
                        |team| {
                            if team.private {
                                format!("{} (private)", team.name)
                            } else {
                                team.name.clone()
                            }
                        },
                    )
                })
                .collect()
        })
}

fn insert_team_column<'a>(
    headers: &'a [&'a str],
    mut rows: Vec<Vec<String>>,
    teams: Option<Vec<String>>,
) -> (Vec<&'a str>, Vec<Vec<String>>) {
    let Some(teams) = teams else {
        return (headers.to_vec(), rows);
    };
    let mut headers = headers.to_vec();
    headers.insert(2, "TEAM");
    for (row, team) in rows.iter_mut().zip(teams) {
        row.insert(2, team);
    }
    (headers, rows)
}

fn format_session_table(sessions: &[SessionSummary], details: bool) -> String {
    let rows: Vec<Vec<String>> = session_value_rows(sessions)
        .into_iter()
        .zip(sessions)
        .map(|(row, session)| {
            let [
                id,
                name,
                harness,
                state,
                activity,
                attached,
                pending,
                held,
                quota,
                size,
            ] = row;
            vec![
                id,
                name,
                harness,
                state,
                session
                    .uptime_secs
                    .map(|seconds| display_duration(Duration::from_secs(seconds)))
                    .unwrap_or_else(|| "-".to_owned()),
                activity,
                session
                    .activity_secs
                    .map(|seconds| display_duration(Duration::from_secs(seconds)))
                    .unwrap_or_else(|| "-".to_owned()),
                attached,
                pending,
                held,
                quota,
                size,
            ]
        })
        .collect();
    let (headers, rows) = if details {
        let rows = rows
            .into_iter()
            .zip(sessions)
            .map(|(mut row, session)| {
                row.push(session.cwd.clone().unwrap_or_else(|| "-".to_owned()));
                row.push(session.argv.join(" "));
                row
            })
            .collect();
        insert_team_column(&DETAIL_HEADERS, rows, session_team_cells(sessions))
    } else {
        insert_team_column(&SESSION_HEADERS, rows, session_team_cells(sessions))
    };
    format_table(&headers, &rows)
}

fn format_messages_table(messages: &[MessageInfo]) -> String {
    format_table(&MESSAGE_HEADERS, &message_value_rows(messages))
}

fn format_row<T: AsRef<str>>(values: &[T], widths: &[usize]) -> String {
    let mut output = String::new();
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            output.push_str("  ");
        }
        let value = value.as_ref();
        output.push_str(value);
        if index + 1 < values.len() {
            for _ in value.len()..widths[index] {
                output.push(' ');
            }
        }
    }
    output
}

async fn write_stdout(bytes: Vec<u8>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        let mut stdout = io::stdout().lock();
        stdout.write_all(&bytes)?;
        stdout.flush()
    })
    .await??;
    Ok(())
}

async fn write_stderr(bytes: Vec<u8>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || -> io::Result<()> {
        let mut stderr = io::stderr().lock();
        stderr.write_all(&bytes)?;
        stderr.flush()
    })
    .await??;
    Ok(())
}

struct Output {
    handle: Option<OutputHandle>,
    join: Option<thread::JoinHandle<io::Result<()>>>,
}

#[derive(Clone)]
struct OutputHandle {
    tx: std_mpsc::Sender<OutputMessage>,
}

enum OutputMessage {
    Write(Vec<u8>),
    Flush(std_mpsc::SyncSender<io::Result<()>>),
}

impl Output {
    fn start() -> anyhow::Result<Self> {
        let (tx, rx) = std_mpsc::channel();
        let join = thread::Builder::new()
            .name("a2amx-output".to_owned())
            .spawn(move || {
                let mut stdout = io::stdout().lock();
                let mut result: io::Result<()> = Ok(());
                for message in rx {
                    match message {
                        OutputMessage::Write(bytes) => {
                            if result.is_ok() {
                                if let Err(error) =
                                    stdout.write_all(&bytes).and_then(|()| stdout.flush())
                                {
                                    result = Err(error);
                                }
                            }
                        }
                        OutputMessage::Flush(sender) => {
                            if result.is_ok() {
                                if let Err(error) = stdout.flush() {
                                    result = Err(error);
                                }
                            }
                            let response = result
                                .as_ref()
                                .map(|_| ())
                                .map_err(|error| io::Error::new(error.kind(), error.to_string()));
                            let _ = sender.send(response);
                        }
                    }
                }
                if result.is_ok() {
                    stdout.flush()?;
                }
                result
            })?;
        Ok(Self {
            handle: Some(OutputHandle { tx }),
            join: Some(join),
        })
    }

    fn handle(&self) -> anyhow::Result<OutputHandle> {
        self.handle
            .as_ref()
            .cloned()
            .ok_or_else(|| anyhow!("terminal output thread handle missing"))
    }

    fn send(&self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.handle
            .as_ref()
            .ok_or_else(|| anyhow!("terminal output thread handle missing"))?
            .send(bytes)
    }

    fn finish(mut self) -> anyhow::Result<()> {
        if let Some(handle) = self.handle.take() {
            handle.flush()?;
        }
        if let Some(join) = self.join.take() {
            join.join()
                .map_err(|_| anyhow!("terminal output thread panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        let handle = self.handle.take();
        let join = self.join.take();
        if handle.is_none() && join.is_none() {
            return;
        }
        run_cleanup(move || {
            if let Some(handle) = handle {
                if let Err(error) = handle.flush() {
                    tracing::debug!(%error, "terminal output cleanup failed");
                }
            }
            if let Some(join) = join {
                if join.join().is_err() {
                    tracing::error!("terminal output thread panicked");
                }
            }
        });
    }
}
impl OutputHandle {
    fn send(&self, bytes: Vec<u8>) -> anyhow::Result<()> {
        self.tx
            .send(OutputMessage::Write(bytes))
            .map_err(|_| anyhow!("terminal output thread stopped"))?;
        Ok(())
    }

    fn flush(&self) -> anyhow::Result<()> {
        let (sender, receiver) = std_mpsc::sync_channel(1);
        self.tx
            .send(OutputMessage::Flush(sender))
            .map_err(|_| anyhow!("terminal output thread stopped"))?;
        receiver
            .recv()
            .map_err(|_| anyhow!("terminal output thread stopped"))??;
        Ok(())
    }
}

struct TerminalGuard {
    original: rustix::termios::Termios,
    output: Option<OutputHandle>,
    restored: bool,
}

impl TerminalGuard {
    fn enter(output: OutputHandle) -> anyhow::Result<Self> {
        let stdin = io::stdin();
        let original = rustix::termios::tcgetattr(stdin.as_fd())?;
        let mut raw = original.clone();
        raw.make_raw();
        rustix::termios::tcsetattr(stdin.as_fd(), rustix::termios::OptionalActions::Now, &raw)?;
        Ok(Self {
            original,
            output: Some(output),
            restored: false,
        })
    }

    async fn restore(&mut self) -> anyhow::Result<()> {
        if self.restored {
            return Ok(());
        }
        let output = self.output.take();
        let original = self.original.clone();
        let result =
            tokio::task::spawn_blocking(move || restore_terminal(output, &original)).await?;
        self.restored = result.is_ok();
        result
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        let output = self.output.take();
        let original = self.original.clone();
        run_cleanup(move || {
            if let Err(error) = restore_terminal(output, &original) {
                tracing::debug!(%error, "terminal restoration failed");
            }
        });
    }
}

fn restore_terminal(
    output: Option<OutputHandle>,
    original: &rustix::termios::Termios,
) -> anyhow::Result<()> {
    let output_result = match output {
        Some(output) => output
            .send(RESTORE_TERMINAL.to_vec())
            .and_then(|()| output.flush()),
        None => Ok(()),
    };
    // Input restoration is required even when the output terminal is broken.
    let stdin = io::stdin();
    rustix::termios::tcsetattr(
        stdin.as_fd(),
        rustix::termios::OptionalActions::Now,
        original,
    )?;
    output_result
}

fn run_cleanup(cleanup: impl FnOnce() + Send + 'static) {
    match tokio::runtime::Handle::try_current() {
        Ok(runtime) => {
            runtime.spawn_blocking(cleanup);
        }
        Err(_) => cleanup(),
    }
}

#[derive(Default)]
struct ScrollParser {
    pending: Vec<u8>,
}

enum ScrollKey {
    Scroll(Scroll),
    Quit,
}

impl ScrollParser {
    fn clear(&mut self) {
        self.pending.clear();
    }

    fn feed(&mut self, bytes: &[u8]) -> Vec<ScrollKey> {
        let mut keys = Vec::new();
        for byte in bytes {
            if self.pending.is_empty() && *byte == b'q' {
                keys.push(ScrollKey::Quit);
                continue;
            }
            self.pending.push(*byte);
            if let Some(key) = parse_scroll_sequence(&self.pending) {
                self.pending.clear();
                keys.push(ScrollKey::Scroll(key));
            } else if self.pending.len() > 5 || self.pending.first() != Some(&0x1b) {
                self.pending.clear();
            }
        }
        keys
    }
}

fn parse_scroll_sequence(bytes: &[u8]) -> Option<Scroll> {
    match bytes {
        b"\x1b[5~" => Some(Scroll::PageUp),
        b"\x1b[6~" => Some(Scroll::PageDown),
        b"\x1b[A" | b"\x1bOA" => Some(Scroll::LineUp),
        b"\x1b[B" | b"\x1bOB" => Some(Scroll::LineDown),
        b"\x1b[H" | b"\x1b[1~" | b"\x1bOH" => Some(Scroll::Top),
        b"\x1b[F" | b"\x1b[4~" | b"\x1bOF" => Some(Scroll::Bottom),
        _ => None,
    }
}

fn scroll_status(cols: u16, rows: u16) -> Vec<u8> {
    let status = "[scroll: q to exit]";
    let width = status.len() as u16;
    let col = cols.saturating_sub(width).saturating_add(1).max(1);
    format!("\x1b[{rows};{col}H\x1b[7m{status}\x1b[0m\x1b[?25l").into_bytes()
}

struct PickerState {
    control: Client,
    sessions: Vec<SessionSummary>,
    current: String,
    cols: u16,
    rows: u16,
    selection: usize,
    offset: usize,
    parser: PickerParser,
    footer: Option<String>,
}

impl PickerState {
    async fn open(
        home: &Path,
        current: &str,
        cols: u16,
        rows: u16,
        output: &OutputHandle,
    ) -> anyhow::Result<Self> {
        let mut control = Client::connect(home).await?;
        let sessions = request_sessions(&mut control).await?;
        let selection = sessions
            .iter()
            .position(|session| session.id == current)
            .unwrap_or(0);
        let offset = scroll_into_view(
            0,
            selection,
            usize::from(rows.saturating_sub(3)),
            sessions.len(),
        );
        let state = Self {
            control,
            sessions,
            current: current.to_owned(),
            cols,
            rows,
            selection,
            offset,
            parser: PickerParser::default(),
            footer: None,
        };
        state.render(output)?;
        Ok(state)
    }

    fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        self.parser.feed(bytes, escape_alone)
    }

    fn height(&self) -> usize {
        usize::from(self.rows.saturating_sub(3))
    }

    fn recompute_offset(&mut self) {
        self.offset = scroll_into_view(
            self.offset,
            self.selection,
            self.height(),
            self.sessions.len(),
        );
    }

    fn set_target(&mut self, action: PickerAction) {
        self.selection = target(self.selection, action, self.height(), self.sessions.len());
        self.recompute_offset();
    }

    fn move_selection(&mut self, delta: isize) {
        self.set_target(PickerAction::Move(delta));
    }

    fn selected_id(&self) -> Option<String> {
        self.sessions
            .get(self.selection)
            .map(|session| session.id.clone())
    }

    async fn refresh(&mut self) -> anyhow::Result<()> {
        self.sessions = request_sessions(&mut self.control).await?;
        if self.selection >= self.sessions.len() {
            self.selection = self.sessions.len().saturating_sub(1);
        }
        self.recompute_offset();
        Ok(())
    }

    fn render(&self, output: &OutputHandle) -> anyhow::Result<()> {
        let mut text = String::from("\x1b[2J\x1b[?25l");
        picker_line(
            &mut text,
            1,
            "a2amx sessions: Enter attach, Esc or q cancel, Up/Down or j/k move",
            self.cols,
        );
        let (header, rows) = session_rows(&self.sessions, &self.current, self.selection, self.cols);
        picker_line(&mut text, 2, &header, self.cols);
        let height = self.height();
        let start = self.offset.min(rows.len());
        let end = start.saturating_add(height).min(rows.len());
        for (index, row) in rows[start..end].iter().enumerate() {
            picker_line(&mut text, (index + 3) as u16, row, self.cols);
        }
        let footer = self.footer.clone().unwrap_or_else(|| {
            if rows.len() > height {
                format!("{}/{}", self.selection + 1, rows.len())
            } else {
                String::new()
            }
        });
        picker_line(&mut text, self.rows.max(1), &footer, self.cols);
        output.send(text.into_bytes())
    }
}

fn picker_line(text: &mut String, row: u16, line: &str, cols: u16) {
    // shortcut: picker clipping counts Unicode scalar values, not terminal cell widths.
    let line = line.chars().take(usize::from(cols)).collect::<String>();
    text.push_str(&format!("\x1b[{row};1H\x1b[2K{line}"));
}

async fn request_sessions(client: &mut Client) -> anyhow::Result<Vec<SessionSummary>> {
    match client.request(Request::List).await? {
        Response::Sessions { sessions } => Ok(sessions),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

/// A name resolves to its id; other references remain for the daemon to judge.
fn resolve_reference(sessions: &[SessionSummary], reference: &str) -> String {
    sessions
        .iter()
        .find(|session| session.name.as_deref() == Some(reference))
        .map_or_else(|| reference.to_owned(), |session| session.id.clone())
}

async fn resolve_session(home: &Path, reference: &str) -> anyhow::Result<String> {
    let mut client = Client::connect(home).await?;
    Ok(resolve_reference(
        &request_sessions(&mut client).await?,
        reference,
    ))
}

// shortcut: picker widths use byte/scalar counts rather than terminal cell widths.
fn session_rows(
    sessions: &[SessionSummary],
    current: &str,
    selection: usize,
    cols: u16,
) -> (String, Vec<String>) {
    let rows: Vec<Vec<String>> = picker_value_rows(sessions)
        .into_iter()
        .map(|row| row.into_iter().collect())
        .collect();
    let (headers, rows) = insert_team_column(&PICKER_HEADERS, rows, session_team_cells(sessions));
    let widths = column_widths(&headers, &rows);
    let cwd = headers.len() - 1;
    let fixed_prefix_width = widths[..cwd].iter().sum::<usize>() + 2 * cwd;
    let header_has_cwd = usize::from(cols) >= 1 + fixed_prefix_width + headers[cwd].len();
    let mut header = String::new();
    header.push(' ');
    if header_has_cwd {
        header.push_str(&format_row(&headers, &widths));
    } else {
        header.push_str(&format_row(&headers[..cwd], &widths[..cwd]));
    }
    let lines = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let suffix = if sessions[index].id == current {
                " (current)"
            } else {
                ""
            };
            let marker = if index == selection { '>' } else { ' ' };
            let mut line = String::new();
            line.push(marker);
            let available = usize::from(cols).saturating_sub(1 + fixed_prefix_width + suffix.len());
            if available >= 4 {
                let mut row = row.clone();
                row[cwd] = shorten_left(&row[cwd], available);
                line.push_str(&format_row(&row, &widths));
            } else {
                line.push_str(&format_row(&row[..cwd], &widths[..cwd]));
            }
            line.push_str(suffix);
            line
        })
        .collect();
    (header, lines)
}

fn shorten_left(value: &str, width: usize) -> String {
    if value.chars().count() <= width {
        return value.to_owned();
    }
    if width <= 1 {
        return "…".chars().take(width).collect();
    }
    let tail = value
        .chars()
        .rev()
        .take(width - 1)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    format!("…{tail}")
}
