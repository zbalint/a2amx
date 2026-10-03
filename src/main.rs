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
use a2amx::cli::{Cli, Command, TeamAction};
use a2amx::client::{Attachment, Client};
use a2amx::daemon::{self, Daemon, DaemonConfig};
use a2amx::emulator::Scroll;
use a2amx::harness::{Deliver, Harness};
use a2amx::hook;
use a2amx::mcp;
use a2amx::messaging::sgr_mouse_report_len;
use a2amx::prefix::{Action, Command as PrefixCommand, PrefixMachine};
use a2amx::status;
use a2amx::team::{self, TeamSession};
use a2amx::wire::{
    ClientFrame, MessageInfo, QuotaInfo, Request, Response, ServerFrame, SessionSummary, StatusInfo,
};

const DEFAULT_COLS: u16 = 80;
const DEFAULT_ROWS: u16 = 24;
const INPUT_CHUNK: usize = 8 * 1024;
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
        Err(error) if !error.use_stderr() => write_stdout(error.to_string().into_bytes()).await,
        Err(error) => Err(error.into()),
    };
    if let Err(error) = result {
        let _ = write_stderr(format!("a2amx: {error:#}\n").into_bytes()).await;
        return std::process::ExitCode::FAILURE;
    }
    // Returning lets Tokio wait for blocking terminal cleanup, including unwind guards.
    std::process::ExitCode::SUCCESS
}

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    let prefix = cli.prefix;
    let home_arg = cli.home;
    match &cli.command {
        Command::Mcp { channel } => return mcp::run(*channel).await,
        Command::Hook => return hook::run().await,
        Command::OmpBridge => return bridge::run().await,
        _ => {}
    }
    let home = resolve_home(home_arg)?;
    match cli.command {
        Command::Daemon {
            listen,
            host_name,
            background,
        } => {
            if background {
                run_daemon_background(home, listen, host_name).await
            } else {
                run_daemon(home, listen, host_name).await
            }
        }
        options @ Command::New { .. } => run_new(home, prefix, options).await,
        Command::List { details } => run_list(home, details).await,
        Command::Team { action } => match action {
            TeamAction::Up {
                file,
                detach,
                items,
            } => run_team_up(home, prefix, file, detach, items).await,
            TeamAction::Down { file, names } => run_team_down(home, file, names).await,
        },
        Command::Attach { session, force } => {
            run_attach_command(home, prefix, session, force).await
        }
        Command::Kill { session } => run_kill(home, session).await,
        Command::Stop { yes } => run_stop(home, yes).await,
        Command::Mcp { .. } | Command::Hook | Command::OmpBridge => {
            unreachable!("early dispatch returned before resolving home")
        }
        Command::Messages { session, state } => run_messages(home, session, state).await,
        Command::Cancel { message } => run_cancel(home, message).await,
    }
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
    command.arg("--home").arg(&home).arg("daemon");
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

async fn run_stop(home: PathBuf, yes: bool) -> anyhow::Result<()> {
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
        if !io::stdin().is_terminal() {
            return Err(anyhow!(
                "refusing to stop: {running} running session(s); pass --yes to end them"
            ));
        }
        write_stderr(
            format!("Stopping the daemon ends {running} running session(s). Continue? [y/N] ")
                .into_bytes(),
        )
        .await?;
        let answer = tokio::task::spawn_blocking(|| {
            let mut line = String::new();
            io::stdin().read_line(&mut line).map(|_| line)
        })
        .await??;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            return write_stdout(b"not stopped\n".to_vec()).await;
        }
    }
    match client.request(Request::Shutdown).await? {
        Response::Ok => {}
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while Client::connect(&home).await.is_ok() {
        if tokio::time::Instant::now() >= deadline {
            return Err(anyhow!("daemon did not stop within 15s"));
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
        command,
    } = options
    else {
        return Err(anyhow!("expected new session options"));
    };
    let mut client = Client::connect(&home).await?;
    let (cols, rows) = terminal_size_with_default()?;
    let session = create_session(
        &home,
        &mut client,
        NewOptions {
            name,
            harness,
            deliver,
            no_authorize_peers,
            no_channel,
            command,
            cwd: std::env::current_dir()?,
            cols,
            rows,
        },
    )
    .await?;
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
    harness: Option<Harness>,
    deliver: Option<Deliver>,
    no_authorize_peers: bool,
    no_channel: bool,
    command: Vec<String>,
    cwd: PathBuf,
    cols: u16,
    rows: u16,
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
        no_authorize_peers,
        no_channel,
        command,
        cwd,
        cols,
        rows,
    } = options;
    let harness = harness.unwrap_or_else(|| Harness::infer(&command));
    let command = match harness {
        Harness::Claude => {
            let exe = std::env::current_exe()?;
            if no_channel {
                a2amx::harness::wire_claude_argv(command, &exe, !no_authorize_peers)
            } else {
                a2amx::harness::wire_claude_channel_argv(command, &exe, !no_authorize_peers)
            }
        }
        Harness::Omp => {
            let install_home = home.to_owned();
            let installed =
                tokio::task::spawn_blocking(move || a2amx::omp::install(&install_home)).await??;
            a2amx::harness::wire_omp_argv(
                command,
                &installed.extension,
                &installed.overlay,
                !no_authorize_peers,
            )
        }
        Harness::Generic | Harness::Codex => command,
    };
    let cwd = cwd.to_string_lossy().into_owned();
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    if harness == Harness::Omp {
        env.push((
            "A2AMX_BIN".to_owned(),
            std::env::current_exe()?.to_string_lossy().into_owned(),
        ));
    }
    if harness == Harness::Codex && no_authorize_peers {
        env.push((a2amx::codex::NO_AUTHORIZE_ENV.to_owned(), "1".to_owned()));
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
            name,
            harness,
            deliver,
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

async fn run_team_up(
    home: PathBuf,
    prefix: u8,
    file: Option<PathBuf>,
    detach: bool,
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
            for conflict in conflicts {
                write_stderr(
                    format!(
                        "session {} has exited ({}); run a2amx kill {} first\n",
                        conflict.name, conflict.id, conflict.name,
                    )
                    .into_bytes(),
                )
                .await?;
            }
            return Err(anyhow!("team has exited session conflicts"));
        }
    };
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
                let name = session.name;
                let result = create_session(
                    &home,
                    &mut client,
                    NewOptions {
                        name: Some(name.clone()),
                        harness: None,
                        deliver: None,
                        no_authorize_peers: false,
                        no_channel: false,
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
) -> anyhow::Result<()> {
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
    for name in names {
        let Some(session) = sessions
            .iter()
            .find(|session| session.name.as_deref() == Some(&name))
        else {
            write_stdout(format!("no session {name}\n").into_bytes()).await?;
            continue;
        };
        let result = client
            .request(Request::Kill {
                session: session.id.clone(),
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
    if failed {
        return Err(anyhow!("one or more team sessions could not be killed"));
    }
    Ok(())
}

async fn run_list(home: PathBuf, details: bool) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::List).await?;
    let sessions = match response {
        Response::Sessions { sessions } => sessions,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
    write_stdout(format_session_table(&sessions, details).into_bytes()).await
}

async fn run_kill(home: PathBuf, session: String) -> anyhow::Result<()> {
    let session = resolve_session(&home, &session).await?;
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::Kill { session }).await?;
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
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
    if visible && rows >= 3 { rows - 1 } else { rows }
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

const SESSION_HEADERS: [&str; 9] = [
    "ID", "NAME", "HARNESS", "STATE", "ATTACHED", "PENDING", "HELD", "QUOTA", "SIZE",
];
const DETAIL_HEADERS: [&str; 11] = [
    "ID", "NAME", "HARNESS", "STATE", "ATTACHED", "PENDING", "HELD", "QUOTA", "SIZE", "CWD",
    "COMMAND",
];
const PICKER_HEADERS: [&str; 5] = ["ID", "STATE", "ATTACHED", "SIZE", "COMMAND"];
const MESSAGE_HEADERS: [&str; 6] = ["ID", "FROM", "TO", "STATE", "DETAIL", "SUBJECT"];

fn session_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 9]> {
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
                quota_cell(session.quota),
                format!("{}x{}", session.cols, session.rows),
            ]
        })
        .collect()
}

fn quota_cell(quota: Option<QuotaInfo>) -> String {
    let Some(quota) = quota else {
        return "-".to_owned();
    };
    let windows = [("5h", quota.five_hour), ("wk", quota.weekly)];
    windows
        .into_iter()
        .filter_map(|(label, percent)| percent.map(|percent| format!("{label} {percent}%")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn picker_value_rows(sessions: &[SessionSummary]) -> Vec<[String; 5]> {
    sessions
        .iter()
        .map(|session| {
            [
                session.id.clone(),
                match session.exit_code {
                    Some(code) => format!("exited({code})"),
                    None => "running".to_owned(),
                },
                if session.attached {
                    "yes".to_owned()
                } else {
                    "no".to_owned()
                },
                format!("{}x{}", session.cols, session.rows),
                session.argv.join(" "),
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

fn column_widths<const N: usize>(headers: &[&str; N], rows: &[[String; N]]) -> [usize; N] {
    let mut widths = headers.map(str::len);
    for row in rows {
        for (index, value) in row.iter().enumerate() {
            widths[index] = widths[index].max(value.len());
        }
    }
    widths
}

fn format_table<const N: usize>(headers: &[&str; N], rows: &[[String; N]]) -> String {
    let widths = column_widths(headers, rows);
    let mut output = String::new();
    output.push_str(&format_row(headers, &widths));
    output.push('\n');
    for row in rows {
        output.push_str(&format_row(row, &widths));
        output.push('\n');
    }
    output
}

fn format_session_table(sessions: &[SessionSummary], details: bool) -> String {
    let rows = session_value_rows(sessions);
    if details {
        let rows: Vec<[String; 11]> = rows
            .into_iter()
            .zip(sessions)
            .map(|(row, session)| {
                let [
                    id,
                    name,
                    harness,
                    state,
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
                    attached,
                    pending,
                    held,
                    quota,
                    size,
                    session.cwd.clone().unwrap_or_else(|| "-".to_owned()),
                    session.argv.join(" "),
                ]
            })
            .collect();
        format_table(&DETAIL_HEADERS, &rows)
    } else {
        format_table(&SESSION_HEADERS, &rows)
    }
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
    parser: PickerParser,
    footer: Option<String>,
}
enum PickerAction {
    Cancel,
    Move(isize),
    Select,
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
        let state = Self {
            control,
            sessions,
            current: current.to_owned(),
            cols,
            rows,
            selection,
            parser: PickerParser::default(),
            footer: None,
        };
        state.render(output)?;
        Ok(state)
    }

    fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        self.parser.feed(bytes, escape_alone)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.sessions.is_empty() {
            self.selection = 0;
            return;
        }
        let max = self.sessions.len() - 1;
        self.selection = if delta < 0 {
            self.selection.saturating_sub(delta.unsigned_abs())
        } else {
            (self.selection + delta as usize).min(max)
        };
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
        Ok(())
    }

    fn render(&self, output: &OutputHandle) -> anyhow::Result<()> {
        let mut text = String::from("\x1b[2J\x1b[?25l");
        picker_line(
            &mut text,
            1,
            "a2amx sessions: Enter attach, Esc or q cancel, Up/Down or j/k move",
        );
        let rows = session_rows(&self.sessions);
        for (index, row) in rows.iter().enumerate() {
            let row_number = index + 2;
            if row_number >= usize::from(self.rows) {
                break;
            }
            let marker = if index == self.selection { '>' } else { ' ' };
            let mut line = String::with_capacity(row.len() + 12);
            line.push(marker);
            line.push_str(row);
            if self.sessions[index].id == self.current {
                line.push_str(" (current)");
            }
            picker_line(&mut text, row_number as u16, &line);
        }
        picker_line(
            &mut text,
            self.rows.max(1),
            self.footer.as_deref().unwrap_or(""),
        );
        output.send(text.into_bytes())
    }
}

fn picker_line(text: &mut String, row: u16, line: &str) {
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

#[derive(Default)]
struct PickerParser {
    pending: Vec<u8>,
}

impl PickerParser {
    fn feed(&mut self, bytes: &[u8], escape_alone: bool) -> Vec<PickerAction> {
        // shortcut: ESC-alone detection is intentionally one-read based.
        if escape_alone {
            self.pending.clear();
            return vec![PickerAction::Cancel];
        }
        let mut actions = Vec::new();
        for byte in bytes {
            if self.pending.is_empty() && *byte == b'q' {
                actions.push(PickerAction::Cancel);
                continue;
            }
            if self.pending.is_empty() && *byte == b'j' {
                actions.push(PickerAction::Move(1));
                continue;
            }
            if self.pending.is_empty() && *byte == b'k' {
                actions.push(PickerAction::Move(-1));
                continue;
            }
            if self.pending.is_empty() && *byte == b'\r' {
                actions.push(PickerAction::Select);
                continue;
            }
            self.pending.push(*byte);
            if self.pending == b"\x1b[A" || self.pending == b"\x1bOA" {
                self.pending.clear();
                actions.push(PickerAction::Move(-1));
            } else if self.pending == b"\x1b[B" || self.pending == b"\x1bOB" {
                self.pending.clear();
                actions.push(PickerAction::Move(1));
            } else if self.pending.len() > 3 || self.pending.first() != Some(&0x1b) {
                self.pending.clear();
            }
        }
        actions
    }
}

fn session_rows(sessions: &[SessionSummary]) -> Vec<String> {
    let rows = picker_value_rows(sessions);
    let widths = column_widths(&PICKER_HEADERS, &rows);
    rows.iter().map(|row| format_row(row, &widths)).collect()
}
