//! S5 acceptance tests for the real `a2amx` binary under a PTY.

#[path = "../common/mod.rs"]
mod common;

use anyhow::Context;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::TeamScope;
use a2amx::wire::{
    Activity, ClientFrame, Request, Response, ServerFrame, SessionSummary, StatusInfo, TeamMember,
};
use common::pty::PtyHarness;
use common::{eventually, fake_recv_frame, fake_send_frame, new_agent};
use tokio::net::{TcpListener, TcpStream};
const WAIT: Duration = Duration::from_secs(10);

fn cli_args(home: &std::path::Path, command: &[&str]) -> Vec<String> {
    let mut args = vec!["--home".to_owned(), home.to_string_lossy().into_owned()];
    args.extend(command.iter().map(|arg| (*arg).to_owned()));
    args
}

fn run_cli(home: &std::path::Path, command: &[&str]) -> anyhow::Result<(String, i32)> {
    let args = cli_args(home, command);
    let mut pty = PtyHarness::spawn(&args, home)?;
    let code = pty.wait_exit(WAIT)?;
    Ok((pty.screen_text(), code))
}

fn run_binary(
    home: &Path,
    command: &[&str],
    environment: &[(&str, &str)],
) -> anyhow::Result<Output> {
    let mut process = Command::new(env!("CARGO_BIN_EXE_a2amx"));
    process
        .args(cli_args(home, command))
        .envs(environment.iter().copied())
        .stdin(Stdio::null());
    Ok(process.output()?)
}

fn fake_team() -> TeamScope {
    TeamScope {
        name: "demo".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    }
}

fn fake_sessions(team: &TeamScope, include_team: bool) -> Vec<SessionSummary> {
    [
        ("s1", "demo-alpha", "alpha@host-a"),
        ("s2", "demo-beta", "beta@host-a"),
    ]
    .into_iter()
    .map(|(id, name, address)| SessionSummary {
        id: id.into(),
        argv: vec!["sh".into()],
        cols: 80,
        rows: 24,
        exit_code: None,
        attached: false,
        viewer: false,
        name: Some(name.into()),
        team: include_team.then(|| team.clone()),
        address: address.into(),
        pending: 0,
        held: false,
        hold_reason: None,
        quota: None,
        harness: Harness::Generic,
        confine: None,
        cwd: None,
        activity: Some(Activity::Idle),
        uptime_secs: Some(1),
        activity_secs: Some(1),
        starting: false,
    })
    .collect()
}

fn fake_status(session: &str, team: &TeamScope) -> StatusInfo {
    let alpha = TeamMember {
        id: "s1".into(),
        name: "demo-alpha".into(),
        activity: Some(Activity::Idle),
        exited: false,
        held: false,
        starting: false,
    };
    let beta = TeamMember {
        id: "s2".into(),
        name: "demo-beta".into(),
        activity: Some(Activity::Idle),
        exited: false,
        held: false,
        starting: false,
    };
    StatusInfo {
        address: if session == "s1" {
            "alpha@host-a".into()
        } else {
            "beta@host-a".into()
        },
        pending: 0,
        hold: None,
        harness: Some(Harness::Generic),
        activity: Some(Activity::Idle),
        confined: false,
        confine_auto: false,
        starting: false,
        quota: None,
        team: Some(team.clone()),
        role: None,
        members: vec![alpha, beta],
        remote: None,
    }
}

#[derive(Clone)]
struct FakeDaemon {
    team: TeamScope,
    sessions_have_team: Arc<AtomicBool>,
    forced_top_rows: Option<u16>,
    stop_on_detach: bool,
    old_open: Arc<AtomicBool>,
    early_switch: Arc<AtomicBool>,
    attach_count: Arc<AtomicUsize>,
    attach_viewers: Arc<Mutex<Vec<(String, bool)>>>,
    frames: Arc<Mutex<Vec<ClientFrame>>>,
    data_after: Arc<Mutex<Option<Duration>>>,
    refuse_attach: Arc<Mutex<Option<String>>>,
    done: Arc<tokio::sync::Notify>,
}

impl FakeDaemon {
    fn new(team: TeamScope) -> Self {
        let mut daemon = Self::with_top_rows(team, None);
        daemon.stop_on_detach = true;
        daemon
    }

    fn with_top_rows(team: TeamScope, forced_top_rows: Option<u16>) -> Self {
        Self {
            team,
            forced_top_rows,
            stop_on_detach: false,
            old_open: Arc::new(AtomicBool::new(false)),
            early_switch: Arc::new(AtomicBool::new(false)),
            attach_count: Arc::new(AtomicUsize::new(0)),
            attach_viewers: Arc::new(Mutex::new(Vec::new())),
            frames: Arc::new(Mutex::new(Vec::new())),
            sessions_have_team: Arc::new(AtomicBool::new(true)),
            data_after: Arc::new(Mutex::new(None)),
            refuse_attach: Arc::new(Mutex::new(None)),
            done: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn set_refuse_attach(&self, session: Option<&str>) {
        match self.refuse_attach.lock() {
            Ok(mut value) => *value = session.map(str::to_owned),
            Err(poisoned) => *poisoned.into_inner() = session.map(str::to_owned),
        }
    }

    fn attach_viewers(&self) -> Vec<(String, bool)> {
        match self.attach_viewers.lock() {
            Ok(values) => values.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn frames(&self) -> Vec<ClientFrame> {
        match self.frames.lock() {
            Ok(values) => values.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
    fn schedule_data_after(&self, delay: Duration) {
        match self.data_after.lock() {
            Ok(mut value) => *value = Some(delay),
            Err(poisoned) => *poisoned.into_inner() = Some(delay),
        }
    }

    fn scheduled_data_after(&self) -> Option<Duration> {
        match self.data_after.lock() {
            Ok(value) => *value,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
    fn hide_session_teams(&self) {
        self.sessions_have_team.store(false, Ordering::SeqCst);
    }
}

async fn fake_connection(mut stream: TcpStream, daemon: FakeDaemon) -> anyhow::Result<()> {
    while let Some(payload) = fake_recv_frame(&mut stream).await? {
        if let Ok(request) = serde_json::from_slice::<Request>(&payload) {
            match request {
                Request::Hello { .. } => {
                    fake_send_frame(&mut stream, &serde_json::to_vec(&Response::Ok)?).await?;
                }
                Request::List => {
                    let response = Response::Sessions {
                        sessions: fake_sessions(
                            &daemon.team,
                            daemon.sessions_have_team.load(Ordering::SeqCst),
                        ),
                    };
                    fake_send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
                }
                Request::Attach {
                    session,
                    top_rows,
                    viewer,
                    ..
                } => {
                    return fake_attach(&mut stream, &session, top_rows, viewer, &daemon).await;
                }
                _ => {
                    let response = Response::Error {
                        message: "unsupported fake request".into(),
                    };
                    fake_send_frame(&mut stream, &serde_json::to_vec(&response)?).await?;
                }
            }
        }
    }
    Ok(())
}

async fn fake_attach(
    stream: &mut TcpStream,
    session: &str,
    top_rows: u16,
    viewer: bool,
    daemon: &FakeDaemon,
) -> anyhow::Result<()> {
    let refused = match daemon.refuse_attach.lock() {
        Ok(value) => value.as_deref() == Some(session),
        Err(poisoned) => poisoned.into_inner().as_deref() == Some(session),
    };
    if refused {
        let response = Response::Error {
            message: "session refused".into(),
        };
        fake_send_frame(stream, &serde_json::to_vec(&response)?).await?;
        return Ok(());
    }
    match daemon.attach_viewers.lock() {
        Ok(mut values) => values.push((session.to_owned(), viewer)),
        Err(poisoned) => poisoned.into_inner().push((session.to_owned(), viewer)),
    }
    let attach_number = daemon.attach_count.fetch_add(1, Ordering::SeqCst) + 1;
    let actual_top_rows = daemon.forced_top_rows.unwrap_or(top_rows);
    if session == "s2" && daemon.old_open.load(Ordering::SeqCst) {
        daemon.early_switch.store(true, Ordering::SeqCst);
    }
    let response = Response::AttachedTop {
        top_rows: actual_top_rows,
    };
    fake_send_frame(stream, &serde_json::to_vec(&response)?).await?;
    let output = if session == "s1" {
        format!(
            "\x1b[{};1H\x1b[?1000hALPHA\n",
            actual_top_rows.saturating_add(1)
        )
    } else {
        format!(
            "\x1b[{};1H\x1b[?1000hBETA\n",
            actual_top_rows.saturating_add(1)
        )
    };
    fake_send_frame(stream, &ServerFrame::Data(output.into_bytes()).encode()).await?;
    fake_send_frame(
        stream,
        &ServerFrame::Status(fake_status(session, &daemon.team)).encode(),
    )
    .await?;
    if session == "s1" {
        daemon.old_open.store(true, Ordering::SeqCst);
    }
    fake_serve_detach(stream, session, attach_number, actual_top_rows, daemon).await?;
    Ok(())
}

async fn fake_serve_detach(
    stream: &mut TcpStream,
    session: &str,
    attach_number: usize,
    top_rows: u16,
    daemon: &FakeDaemon,
) -> anyhow::Result<()> {
    let data_deadline = daemon
        .scheduled_data_after()
        .map(|delay| Instant::now() + delay);
    let mut data_sent = data_deadline.is_none();
    loop {
        let payload = match data_deadline {
            Some(deadline) if !data_sent => {
                let timeout = deadline.saturating_duration_since(Instant::now());
                match tokio::time::timeout(timeout, fake_recv_frame(stream)).await {
                    Ok(payload) => payload?,
                    Err(_) => {
                        fake_send_frame(
                            stream,
                            &ServerFrame::Data(b"scheduled daemon data".to_vec()).encode(),
                        )
                        .await?;
                        data_sent = true;
                        continue;
                    }
                }
            }
            _ => fake_recv_frame(stream).await?,
        };
        let Some(payload) = payload else {
            break;
        };
        let frame = ClientFrame::decode(&payload)?;
        match daemon.frames.lock() {
            Ok(mut frames) => frames.push(frame.clone()),
            Err(poisoned) => poisoned.into_inner().push(frame.clone()),
        }
        match frame {
            ClientFrame::Redraw => {
                let label = if session == "s1" { "ALPHA" } else { "BETA" };
                let repaint = format!(
                    "\x1b[2J\x1b[{};1H\x1b[?1000h{label}\n",
                    top_rows.saturating_add(1)
                );
                fake_send_frame(stream, &ServerFrame::Data(repaint.into_bytes()).encode()).await?;
            }
            ClientFrame::Detach => {
                if session == "s1" && attach_number == 1 {
                    fake_send_frame(
                        stream,
                        &ServerFrame::Data(b"late old frame".to_vec()).encode(),
                    )
                    .await?;
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    daemon.old_open.store(false, Ordering::SeqCst);
                } else {
                    fake_send_frame(stream, &ServerFrame::Detached("done".into()).encode()).await?;
                    if daemon.stop_on_detach {
                        daemon.done.notify_one();
                    }
                }
                break;
            }
            _ => {}
        }
    }
    Ok(())
}

async fn run_fake_daemon(listener: TcpListener, daemon: FakeDaemon) -> anyhow::Result<()> {
    loop {
        let (stream, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            _ = daemon.done.notified() => return Ok(()),
        };
        tokio::spawn(fake_connection(stream, daemon.clone()));
    }
}

async fn start_fake_daemon(
    forced_top_rows: Option<u16>,
) -> anyhow::Result<(
    tempfile::TempDir,
    FakeDaemon,
    tokio::task::JoinHandle<anyhow::Result<()>>,
)> {
    let dir = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::create_dir_all(dir.path().join("run"))?;
    std::fs::write(dir.path().join("run/addr"), format!("{address}\n"))?;
    std::fs::write(dir.path().join("run/admin.token"), "fake-token\n")?;
    let daemon = FakeDaemon::with_top_rows(fake_team(), forced_top_rows);
    let server = tokio::spawn(run_fake_daemon(listener, daemon.clone()));
    Ok((dir, daemon, server))
}

mod attach_session;
mod chunked_input;
mod failed_switch;
mod list_and_messages;
mod mouse_and_resize;
mod picker;
mod scroll_and_mouse;
mod status_and_team_line;
mod viewer;
