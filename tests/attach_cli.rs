//! S5 acceptance tests for the real `a2amx` binary under a PTY.

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
use common::{eventually, new_agent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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

async fn fake_recv_frame(stream: &mut TcpStream) -> anyhow::Result<Option<Vec<u8>>> {
    let mut header = [0; 4];
    match stream.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let length = u32::from_be_bytes(header) as usize;
    let mut payload = vec![0; length];
    stream.read_exact(&mut payload).await?;
    Ok(Some(payload))
}

async fn fake_send_frame(stream: &mut TcpStream, payload: &[u8]) -> anyhow::Result<()> {
    stream.write_u32(payload.len() as u32).await?;
    stream.write_all(payload).await?;
    Ok(())
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
            done: Arc::new(tokio::sync::Notify::new()),
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
    std::fs::write(dir.path().join("addr"), format!("{address}\n"))?;
    std::fs::write(dir.path().join("admin.token"), "fake-token\n")?;
    let daemon = FakeDaemon::with_top_rows(fake_team(), forced_top_rows);
    let server = tokio::spawn(run_fake_daemon(listener, daemon.clone()));
    Ok((dir, daemon, server))
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_list_attach_detach_and_literal_prefix_use_real_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;

    let (created, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf ready; cat"],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"), "new output: {created:?}");

    let (listed, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(listed.contains("ID  NAME"), "list output: {listed:?}");
    assert!(
        listed.contains("s1") && listed.contains("running"),
        "list output: {listed:?}"
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"typed\n")?;
    attached.wait_for_text("typed", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );

    let (after_detach, code) = run_cli(dir.path(), &["list"])?;
    assert_eq!(code, 0);
    assert!(after_detach.contains("s1") && after_detach.contains("running"));

    // Ctrl-B twice is a literal 0x02, not a prefix command. The child
    // prints the received byte in hex and stays alive so the screen is observable.
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "stty raw -echo; dd if=/dev/tty bs=1 count=1 2>/dev/null | od -An -t x1; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s2"), "new output: {created:?}");
    let mut literal = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    literal.send(&[2, 2])?;
    literal.wait_for_text("02", WAIT)?;
    literal.send(&[2, b'd'])?;
    literal.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(literal.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn scroll_mode_parses_split_sequences_and_returns_to_live_bottom() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script =
        "i=1; while [ $i -le 30 ]; do printf 'line%02d\\r\\n' $i; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("line30", WAIT)?;
    attached.send(&[2, b'['])?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;

    // Send PageUp one byte at a time; the parser must retain the split CSI.
    for byte in b"\x1b[5~" {
        attached.send(&[*byte])?;
    }
    attached.wait_for_text("line01", WAIT)?;
    attached.send(b"qafter-scroll-input\n")?;
    attached.wait_for_text("after-scroll-input", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[test]
fn raw_wait_observes_late_bytes() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = [
        "-c".to_owned(),
        "printf first; sleep 1; printf second; sleep 30".to_owned(),
    ];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    pty.wait_for_text("first", WAIT)?;
    pty.wait_for_raw_contains(b"second", WAIT)?;
    Ok(())
}

#[test]
fn raw_wait_times_out_with_diagnostics() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = ["-c".to_owned(), "sleep 30".to_owned()];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    let error = pty
        .wait_for_raw_contains(b"ne\x01ver", Duration::from_millis(300))
        .expect_err("missing raw output should time out");
    let message = error.to_string();
    assert!(message.contains("timed out"), "error: {message}");
    assert!(message.contains("ne\\x01ver"), "error: {message}");
    assert!(message.contains("raw output:"), "error: {message}");
    Ok(())
}

#[test]
fn raw_wait_accepts_empty_needle() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let args = ["-c".to_owned(), "printf first; sleep 30".to_owned()];
    let mut pty = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
    pty.wait_for_text("first", WAIT)?;
    pty.wait_for_raw_contains(b"", Duration::from_millis(300))?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_wheel_scrolls_three_lines_and_q_returns_to_live() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script =
        "i=1; while [ $i -le 100 ]; do printf 'line-%03d\\r\\n' $i; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("line-100", WAIT)?;
    attached.wait_for_raw_contains(b"\x1b[?1000h\x1b[?1006h", WAIT)?;
    attached.expect_screen_lacks("line-076")?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("line-077", WAIT)?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.expect_screen_lacks("line-100")?;
    attached.send(b"\x1b[<65;10;5M")?;
    attached.wait_for_text("line-100", WAIT)?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.expect_screen_lacks("[scroll: q to exit]")?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_click_reports_are_dropped_without_entering_scroll_mode() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "stty raw -echo; cat -v",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(b"\x1b[<65;10;5M")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(b"\x1b[<0;10;5M\x1b[<0;10;5m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    assert!(!attached.screen_text().contains("^[[<0;10;5M"));
    attached.send(b"x")?;
    attached.wait_for_text("x", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_mouse_reporting_forwards_wheel_reports() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h\\033[?1006h'; printf ready; stty raw -echo; cat -v";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("^[[<64;10;5M", WAIT)?;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_reporting_reset_returns_to_wheel_capture() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "printf '\\033[?1000h\\033[?1006h'; printf on; stty raw -echo; read trigger; printf '\\033[?1000l\\033[?1006l'; printf off; cat -v";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("on", WAIT)?;
    attached.send(b"x\n")?;
    attached.wait_for_text("off", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    assert!(!attached.screen_text().contains("^[[<64;10;5M"));
    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_does_not_capture_session_already_in_drag_mode() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf '\\033[?1002h'; printf ready; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.expect_raw_lacks(b"\x1b[?1000h")?;
    attached.expect_raw_lacks(b"\x1b[?1006h")?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mouse_wheel_is_dropped_while_session_picker_is_open() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--",
            "sh",
            "-c",
            "printf ready; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ready", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[<64;10;5M")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"\x1b")?;
    attached.wait_for_text("ready", WAIT)?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!attached.screen_text().contains("[scroll: q to exit]"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_switches_sessions_and_exit_status_is_reported() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "aa",
            "--",
            "sh",
            "-c",
            "printf '%s%s' o ne; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "bb",
            "--",
            "sh",
            "-c",
            "printf '%s%s' t wo; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);

    let mut blocker = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s2"]), dir.path())?;
    blocker.wait_for_text("two", WAIT)?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("one", WAIT)?;

    // Entering and selecting the current row cancels and redraws the live session.
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    for header in ["NAME", "HARNESS", "ACTIVITY", "PENDING", "HELD", "QUOTA"] {
        attached.wait_for_text(header, WAIT)?;
    }
    let screen = attached.screen_text();
    let running_row = screen
        .lines()
        .find(|line| line.starts_with(">s1 "))
        .expect("current session missing from picker");
    let cells: Vec<_> = running_row
        .trim_start_matches('>')
        .split_whitespace()
        .collect();
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..4], ["generic", "running"]);
    assert!(matches!(cells[4], "idle" | "working" | "busy"));
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;

    // A held controller makes the first switch fail; the picker remains open.
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[B\r")?;
    attached.wait_for_text("attached elsewhere", WAIT)?;
    blocker.send(&[2, b'd'])?;
    blocker.wait_for_text("[detached from s2]", WAIT)?;
    blocker.wait_exit(WAIT)?;
    attached.send(b"\r")?;
    attached.wait_for_text("two", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    let (_, code) = run_cli(
        dir.path(),
        &["new", "--detach", "--", "sh", "-c", "printf bye; exit 7"],
    )?;
    assert_eq!(code, 0);
    let mut exited = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s3"]), dir.path())?;
    exited.wait_for_text("[session s3 exited with code 7]", WAIT)?;
    assert_eq!(exited.wait_exit(WAIT)?, 0);
    assert!(
        exited
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("one", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("exited(7)", WAIT)?;
    let screen = attached.screen_text();
    let exited_row = screen
        .lines()
        .find(|line| line.trim_start().starts_with("s3 "))
        .expect("exited session missing from picker");
    let cells: Vec<_> = exited_row.split_whitespace().collect();
    assert_eq!(cells[0], "s3");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "exited(7)", "-"]);
    attached.send(b"\r")?;
    attached.wait_for_text("one", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_scrolls_and_takes_page_home_end_keys() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    for name in [
        "pk-a", "pk-b", "pk-c", "pk-d", "pk-e", "pk-f", "pk-g", "pk-h",
    ] {
        let (_, code) = run_cli(
            dir.path(),
            &[
                "new", "--detach", "--name", name, "--", "sh", "-c", "sleep 30",
            ],
        )?;
        assert_eq!(code, 0);
    }

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "pk-a"]), dir.path())?;
    attached.resize(100, 8)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("1/8", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.contains("pk-a"), "picker screen: {screen:?}");
    assert!(!screen.contains("pk-h"), "picker screen: {screen:?}");

    attached.send(b"\x1b[F")?;
    attached.wait_for_text("8/8", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.contains("pk-h"), "picker screen: {screen:?}");
    assert!(!screen.contains("pk-a"), "picker screen: {screen:?}");

    attached.send(b"\x1b[H")?;
    attached.wait_for_text("1/8", WAIT)?;
    assert!(attached.screen_text().contains("pk-a"));
    attached.send(b"\x1b[6~")?;
    attached.wait_for_text("5/8", WAIT)?;

    attached.send(b"q")?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_clips_long_cwd_to_narrow_terminal() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let cwd_root = tempfile::tempdir()?;
    let long_cwd = cwd_root
        .path()
        .join("fictional-picker-working-directory")
        .join("tail-of-picker-path");
    std::fs::create_dir_all(&long_cwd)?;

    let mut client = Client::connect(dir.path()).await?;
    let response = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "stty size; trap 'stty size' WINCH; while :; do read line; done".into(),
            ],
            cols: 80,
            rows: 24,
            cwd: Some(long_cwd.to_string_lossy().into_owned()),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await?;
    let Response::Created { session } = response else {
        anyhow::bail!("new session response was not Created");
    };

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", &session]), dir.path())?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.resize(101, 10)?;
    attached.wait_for_text("8 101", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("CWD", WAIT)?;
    let screen = attached.screen_text();
    assert!(screen.lines().any(|line| line.contains("CWD")));
    for line in screen.lines() {
        assert!(
            line.chars().count() <= 101,
            "picker screen line exceeded width: {line:?}"
        );
    }
    assert!(
        screen
            .lines()
            .any(|line| line.contains("…-picker-path (current)")),
        "picker did not left-shorten CWD: {screen:?}"
    );
    Ok(())
}

#[test]
fn attach_without_terminal_is_rejected_with_status_one() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["attach", "s1"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "a2amx: attach needs a terminal on stdin\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_precedence_and_nonterminal_new_use_default_size() -> anyhow::Result<()> {
    let (first_dir, _first_daemon) = common::start_daemon().await;
    let second_dir = tempfile::tempdir()?;
    let bwrap_dir = second_dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir)?;
    let _second_daemon = a2amx::daemon::Daemon::start(a2amx::daemon::DaemonConfig {
        state_dir: second_dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse()?],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
        env_path: None,
    })
    .await?;
    let first_home = first_dir.path().to_string_lossy().into_owned();
    let second_home = second_dir.path().to_string_lossy().into_owned();
    let environment = [
        ("A2AMX_HOME", first_home.as_str()),
        ("XDG_STATE_HOME", second_home.as_str()),
    ];

    let created = run_binary(
        first_dir.path(),
        &["new", "--detach", "--", "sh", "-c", "sleep 30"],
        &environment,
    )?;
    assert_eq!(created.status.code(), Some(0));
    assert_eq!(String::from_utf8(created.stdout)?, "s1\n");
    let generated_name = String::from_utf8(created.stderr)?;
    let generated_name = generated_name
        .strip_prefix("name: ")
        .expect("generated name")
        .trim();
    let name_width = generated_name.len().max("NAME".len());
    let expected_header = format!(
        "ID  {:<name_width$}  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE",
        "NAME",
        name_width = name_width,
    );
    let listed = run_binary(first_dir.path(), &["list"], &environment)?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    let header = listing.lines().next().unwrap_or_default();
    assert_eq!(header, expected_header);
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells.len(), 13);
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "working");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }

    let explicit = run_binary(second_dir.path(), &["list"], &environment)?;
    assert_eq!(explicit.status.code(), Some(0));
    let explicit_listing = String::from_utf8(explicit.stdout)?;
    assert!(explicit_listing.starts_with(
        "ID  NAME  HARNESS  CONFINE  STATE  UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
    ));
    assert!(!explicit_listing.contains("s1"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn custom_prefix_and_sigwinch_resize_reach_attached_session() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);

    let mut attached = PtyHarness::spawn_with_env(
        &cli_args(dir.path(), &["attach", "s1"]),
        dir.path(),
        &[("A2AMX_PREFIX", "C-a")],
    )?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.resize(40, 10)?;
    attached.wait_for_text("8 40", WAIT)?;
    attached.send(&[1, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        attached
            .raw_output()
            .windows(b"\x1b[?1049l".len())
            .any(|window| window == b"\x1b[?1049l")
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_failure_restores_the_outer_terminal_before_returning_an_error() -> anyhow::Result<()>
{
    let (dir, _daemon) = common::start_daemon().await;
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "cat"])?;
    assert_eq!(code, 0);
    let before = dir.path().join("before");
    let after = dir.path().join("after");
    let script = "stty -g > \"$BEFORE\"; \"$A2AMX_BIN\" --home \"$A2AMX_HOME\" attach s1 > /dev/full; status=$?; stty -g > \"$AFTER\"; printf 'status=%s\\n' \"$status\"";
    let mut terminal = PtyHarness::spawn_program(
        "sh",
        &["-c".into(), script.into()],
        dir.path(),
        &[
            ("A2AMX_BIN", env!("CARGO_BIN_EXE_a2amx")),
            ("BEFORE", before.to_str().unwrap()),
            ("AFTER", after.to_str().unwrap()),
        ],
    )?;
    terminal.wait_for_text("status=1", WAIT)?;
    assert_eq!(terminal.wait_exit(WAIT)?, 0);
    assert_eq!(std::fs::read(before)?, std::fs::read(after)?);
    assert!(terminal.screen_text().contains("a2amx:"));
    Ok(())
}

#[test]
fn invalid_prefix_reports_the_named_reason_as_an_a2amx_error() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let output = run_binary(dir.path(), &["--prefix", "C-m", "list"], &[])?;
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr)?;
    assert!(stderr.starts_with("a2amx:"));
    assert!(stderr.contains("CR"));
    Ok(())
}
async fn wait_for_file(path: &Path, lines: usize) -> String {
    let path = path.to_owned();
    eventually(move || {
        let path = path.clone();
        async move {
            let content = std::fs::read_to_string(path).ok()?;
            (content.lines().count() == lines).then_some(content)
        }
    })
    .await
}

async fn wait_for_message_state(home: &Path, id: &str, state: &str) {
    let home = home.to_owned();
    let id = id.to_owned();
    let state = state.to_owned();
    eventually(move || {
        let home = home.clone();
        let id = id.clone();
        let state = state.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Status { message } =
                client.request(Request::MessageStatus { id }).await.ok()?
            else {
                return None;
            };
            (message.state == state).then_some(())
        }
    })
    .await;
}

async fn wait_for_held(home: &Path, session: &str, held: bool) {
    let home = home.to_owned();
    let session = session.to_owned();
    eventually(move || {
        let home = home.clone();
        let session = session.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let Response::Sessions { sessions } = client.request(Request::List).await.ok()? else {
                return None;
            };
            sessions
                .into_iter()
                .find(|summary| summary.id == session)
                .filter(|summary| summary.held == held)
                .map(|_| ())
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn named_session_appears_in_the_list_table() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let output = run_binary(dir.path(), &["list"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID  NAME        HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n",
    ));
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells.len(), 13);
    assert_eq!(
        &cells[..5],
        ["s1", "agent-plan", "generic", "off", "running"]
    );
    assert_eq!(cells[6], "working");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn claude_new_wires_mcp_and_optional_peer_authorization() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let first = tempfile::NamedTempFile::new_in(dir.path())?;
    let first_path = first.path().to_string_lossy().into_owned();
    let first_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &first_path)],
    )?;
    assert_eq!(first_output.status.code(), Some(0));
    let first_args = wait_for_file(first.path(), 8).await;
    let first_args: Vec<&str> = first_args.lines().collect();
    assert_eq!(first_args.len(), 8);
    assert_eq!(first_args[0], "--mcp-config");
    let config: serde_json::Value = serde_json::from_str(first_args[1])?;
    let expected_exe = std::fs::canonicalize(env!("CARGO_BIN_EXE_a2amx"))?
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        config["mcpServers"]["a2amx"]["command"],
        serde_json::Value::String(expected_exe.clone())
    );
    assert_eq!(
        config["mcpServers"]["a2amx"]["args"],
        serde_json::json!(["mcp"])
    );
    assert_eq!(first_args[2], "--allowedTools");
    assert_eq!(
        first_args[3],
        "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session"
    );
    assert_eq!(first_args[4], "--append-system-prompt");
    assert_eq!(first_args[5], a2amx::harness::PEER_AUTHORIZATION_PROMPT);
    assert_eq!(first_args[6], "--settings");
    let settings: serde_json::Value = serde_json::from_str(first_args[7])?;
    assert_eq!(
        settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
        format!("'{expected_exe}' hook")
    );

    let second = tempfile::NamedTempFile::new_in(dir.path())?;
    let second_path = second.path().to_string_lossy().into_owned();
    let second_output = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "claude",
            "--no-channel",
            "--no-authorize-peers",
            "--",
            "sh",
            "-c",
            "printf '%s\\n' \"$@\" >\"$OUT\"; sleep 30",
            "sh",
        ],
        &[("OUT", &second_path)],
    )?;
    assert_eq!(second_output.status.code(), Some(0));
    let second_args = wait_for_file(second.path(), 6).await;
    let second_args: Vec<&str> = second_args.lines().collect();
    assert_eq!(second_args.len(), 6);
    assert_eq!(
        second_args,
        vec![
            "--mcp-config",
            first_args[1],
            "--allowedTools",
            "mcp__a2amx__list_agents,mcp__a2amx__send_message,mcp__a2amx__message_status,mcp__a2amx__reset_session",
            "--settings",
            first_args[7],
        ]
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attached_input_sets_hold_and_prefix_release_clears_it() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "stty raw -echo; cat",
        ],
        &[],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(b"x")?;
    wait_for_held(dir.path(), "s1", true).await;
    let listed = run_binary(dir.path(), &["list", "--details"], &[])?;
    assert_eq!(listed.status.code(), Some(0));
    let listing = String::from_utf8(listed.stdout)?;
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "busy");
    assert_eq!(cells[8], "yes");
    assert_eq!(cells[9], "0");
    assert_eq!(cells[10], "human_draft");
    assert_eq!(cells[11], "-");
    assert_eq!(cells[12], "80x22");
    for index in [5, 7] {
        let bytes = cells[index].as_bytes();
        assert!(bytes.len() >= 2);
        assert!(
            bytes[..bytes.len() - 1]
                .iter()
                .all(|byte| byte.is_ascii_digit())
        );
        assert!(matches!(bytes.last(), Some(b's' | b'm' | b'h')));
    }
    assert!(listing.contains(&format!(
        "80x22  {}  sh -c stty raw -echo; cat\n",
        std::env::current_dir()?.display(),
    )));

    attached.send(&[2, b'r'])?;
    wait_for_held(dir.path(), "s1", false).await;
    let listed = run_binary(dir.path(), &["list"], &[])?;
    assert!(listed.status.success());
    let listing = String::from_utf8(listed.stdout)?;
    let cells = common::list_cells(&listing, "s1");
    assert_eq!(cells[0], "s1");
    assert_ne!(cells[1], "-");
    a2amx::messaging::validate_name(cells[1])?;
    assert_eq!(&cells[2..5], ["generic", "off", "running"]);
    assert_eq!(cells[6], "working");
    assert_eq!(cells[8], "yes");
    assert_eq!(cells[9], "0");
    assert_eq!(cells[10], "-");

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_cli_prints_submitted_and_undeliverable_rows() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let recipient_output = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-review",
            "--harness",
            "generic",
            "--deliver",
            "auto",
            "--",
            "sh",
            "-c",
            "printf '\\033[?2004h'; stty raw -echo; cat >/dev/null",
        ],
    )?;
    assert_eq!(recipient_output.1, 0);
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Parser issue".into(),
            message: "I found the regression.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    wait_for_message_state(dir.path(), "m_1", "submitted").await;

    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Second".into(),
            message: "Another issue.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_2".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    let accepted = sender
        .request(Request::SendMessage {
            to: "agent-review@host-a".into(),
            subject: "Trailing ".into(),
            message: "Subjects print as stored.".into(),
        })
        .await?;
    assert_eq!(
        accepted,
        Response::Accepted {
            id: "m_3".into(),
            recipient_hold: None,
            recipient_quota: None
        }
    );
    assert_eq!(
        admin
            .request(Request::Kill {
                session: "s2".into(),
                now: true,
            })
            .await?,
        Response::Ok
    );
    wait_for_message_state(dir.path(), "m_2", "undeliverable").await;
    wait_for_message_state(dir.path(), "m_3", "undeliverable").await;

    let output = run_binary(dir.path(), &["messages"], &[])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    assert!(listing.starts_with(
        "ID   FROM               TO                   STATE          DETAIL            SUBJECT\n"
    ));
    assert!(listing.contains("m_1  agent-plan@host-a  agent-review@host-a  submitted      -                 Parser issue\n"));
    assert!(listing.contains(
        "m_2  agent-plan@host-a  agent-review@host-a  undeliverable  recipient_exited  Second\n"
    ));
    assert!(listing.contains(
        "m_3  agent-plan@host-a  agent-review@host-a  undeliverable  recipient_exited  Trailing \n"
    ));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_is_silent_on_success_and_reports_errors() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, sender_token, sender_addr) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _, _) = new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &sender_token).await?;
    assert_eq!(
        sender
            .request(Request::SendMessage {
                to: "agent-review@host-a".into(),
                subject: "Pending".into(),
                message: "Cancel me.".into(),
            })
            .await?,
        Response::Accepted {
            id: "m_1".into(),
            recipient_hold: Some("deliver_hold".into()),
            recipient_quota: None
        }
    );

    let cancelled = run_binary(dir.path(), &["cancel", "m_1"], &[])?;
    assert_eq!(cancelled.status.code(), Some(0));
    assert!(cancelled.stdout.is_empty());
    assert!(cancelled.stderr.is_empty());

    let missing = run_binary(dir.path(), &["cancel", "m_99"], &[])?;
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr)?,
        "a2amx: unknown message m_99\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_line_toggles_and_shows_the_session_address() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "stty size; trap 'stty size' WINCH; while :; do read line; done";
    let (_, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            script,
        ],
    )?;
    assert_eq!(code, 0);
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("22 80", WAIT)?;
    attached.wait_for_text("agent-plan@", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(40, 10)?;
    attached.wait_for_text("8 40", WAIT)?;
    let after_toggle = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    if let Err(error) = attached.wait_for_text("10 40", WAIT) {
        eprintln!(
            "post-toggle outer PTY bytes: {:?}",
            String::from_utf8_lossy(&attached.raw_output()[after_toggle..]),
        );
        return Err(error);
    }
    assert!(
        !attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.resize(50, 12)?;
    attached.wait_for_text("12 50", WAIT)?;
    attached.send(&[2, b's'])?;
    attached.wait_for_text("10 50", WAIT)?;
    assert!(
        attached
            .screen_text()
            .lines()
            .last()
            .unwrap_or("")
            .contains("agent-plan@")
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_separator_keeps_session_output_above_the_bar() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let script = "read trigger; set -- $(stty size); i=1; while [ $i -le $1 ]; do printf '\\033[%s;1Hsession-row-%02d' \"$i\" \"$i\"; i=$((i+1)); done; sleep 30";
    let (_, code) = run_cli(dir.path(), &["new", "--detach", "--", "sh", "-c", script])?;
    assert_eq!(code, 0);
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("@host-a", WAIT)?;
    attached.send(b"draw\n")?;
    attached.wait_for_text("session-row-22", WAIT)?;
    let screen = attached.screen_text();
    let rows: Vec<_> = screen.lines().collect();
    assert_eq!(rows.len(), 24, "screen: {screen:?}");
    assert!(rows[23].contains("@host-a"), "screen: {screen:?}");
    assert_eq!(rows[22], "─".repeat(79), "screen: {screen:?}");
    for (row, marker) in rows[..22].iter().zip([
        "session-row-01",
        "session-row-02",
        "session-row-03",
        "session-row-04",
        "session-row-05",
        "session-row-06",
        "session-row-07",
        "session-row-08",
        "session-row-09",
        "session-row-10",
        "session-row-11",
        "session-row-12",
        "session-row-13",
        "session-row-14",
        "session-row-15",
        "session-row-16",
        "session-row-17",
        "session-row-18",
        "session-row-19",
        "session-row-20",
        "session-row-21",
        "session-row-22",
    ]) {
        assert_eq!(*row, marker, "screen: {screen:?}");
    }
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test]
async fn output_during_backpressured_snapshot_reaches_live_attachment_without_redraw()
-> anyhow::Result<()> {
    use a2amx::wire::{ClientFrame, ServerFrame, encode_frame};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    tokio::time::timeout(WAIT, async {
        let (dir, daemon) = common::start_daemon().await;
        let gate_path = dir.path().join("output-gate");
        let listener = tokio::net::UnixListener::bind(&gate_path)?;
        let script = r#"
import os, socket, sys, tty
tty.setraw(0)
gate = socket.socket(socket.AF_UNIX)
gate.connect(sys.argv[1])
def synced_write(data):
    sys.stdout.buffer.write(data + b'\x1b[6n')
    sys.stdout.buffer.flush()
    while os.read(0, 1) != b'R':
        pass
pair = b'\x1b[38;2;1;2;3mX\x1b[38;2;4;5;6mX'
synced_write(pair * 131072)
gate.sendall(b'ready')
gate.recv(1)
synced_write(b'\x1b[1;1H\x1b[0mOUTPUT-AFTER-SNAPSHOT')
gate.sendall(b'printed')
gate.recv(1)
"#;
        let mut control = Client::connect(dir.path()).await?;
        let Response::Created { session } = control
            .request(Request::NewSession {
                argv: vec![
                    "python3".into(),
                    "-c".into(),
                    script.into(),
                    gate_path.to_string_lossy().into_owned(),
                ],
                cols: 512,
                rows: 512,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                name: Some("render-regression".into()),
                harness: Harness::Generic,
                deliver: Some(Deliver::Hold),
                heartbeat: None,
                activity: None,
                team: None,
                role: None,
                remote: None,
            })
            .await?
        else {
            panic!("session creation");
        };
        let (mut gate, _) = listener.accept().await?;
        let mut ready = [0; 5];
        gate.read_exact(&mut ready).await?;
        assert_eq!(&ready, b"ready");

        // Alternating RGB cells make the snapshot exceed Linux's TCP send buffer.
        // A small receive window leaves the daemon sending it while the child writes.
        let socket = tokio::net::TcpSocket::new_v4()?;
        socket.set_recv_buffer_size(16384)?;
        let mut attachment = socket.connect(daemon.addrs()[0]).await?;
        let token = std::fs::read_to_string(dir.path().join("admin.token"))?;
        for (request, expected) in [
            (Request::Hello { token }, Response::Ok),
            (
                Request::Attach {
                    session,
                    force: false,
                    cols: 512,
                    rows: 512,
                    status: false,
                    top_rows: 0,
                    viewer: false,
                },
                Response::Attached,
            ),
        ] {
            attachment
                .write_all(&encode_frame(&serde_json::to_vec(&request)?)?)
                .await?;
            let length = attachment.read_u32().await?;
            let mut payload = vec![0; length as usize];
            attachment.read_exact(&mut payload).await?;
            assert_eq!(serde_json::from_slice::<Response>(&payload)?, expected);
        }
        gate.write_all(b"p").await?;
        let mut printed = [0; 7];
        gate.read_exact(&mut printed).await?;
        assert_eq!(&printed, b"printed");
        // Let the already-woken delivery observer consume the shared notification.
        assert!(matches!(
            control.request(Request::List).await?,
            Response::Sessions { .. }
        ));
        tokio::task::yield_now().await;
        tokio::time::timeout(WAIT, async {
            loop {
                let length = attachment.read_u32().await?;
                let mut payload = vec![0; length as usize];
                attachment.read_exact(&mut payload).await?;
                let ServerFrame::Data(data) = ServerFrame::decode(&payload)? else {
                    panic!("child remains live");
                };
                if data
                    .windows(b"OUTPUT-AFTER-SNAPSHOT".len())
                    .any(|window| window == b"OUTPUT-AFTER-SNAPSHOT")
                {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        attachment
            .write_all(&encode_frame(&ClientFrame::Detach.encode())?)
            .await?;
        gate.write_all(b"x").await?;
        daemon.shutdown().await?;
        Ok::<_, anyhow::Error>(())
    })
    .await?
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_groups_sessions_into_team_sections() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new", "--detach", "--name", "plain", "--", "sh", "-c", "sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"));
    std::fs::write(
        dir.path().join("team.toml"),
        "team = \"demo\"\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )?;
    let team_file = dir.path().join("team.toml");
    let team_file = team_file.to_string_lossy().into_owned();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", &team_file],
        &[],
    )?;
    assert!(
        started.status.success(),
        "team up output: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("NAME", WAIT)?;
    attached.wait_for_text("[-] demo (private) (1)", WAIT)?;
    attached.wait_for_text("worker", WAIT)?;
    attached.wait_for_text("[-] (no team) (1)", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("TEAM"), "picker screen: {screen:?}");
    assert!(screen.contains("plain"), "picker screen: {screen:?}");
    assert!(
        screen.find("NAME").unwrap_or(usize::MAX) < screen.find("worker").unwrap_or(0),
        "picker screen: {screen:?}"
    );
    attached.send(b"q")?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn picker_folds_teams_and_filters() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let (created, code) = run_cli(
        dir.path(),
        &[
            "new", "--detach", "--name", "base", "--", "sh", "-c", "sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(created.contains("s1"));
    for (team, sessions) in [
        ("alpha", [("al-1", "sleep 30"), ("al-2", "sleep 30")]),
        (
            "beta",
            [("be-1", "sleep 30"), ("be-2", "printf BE2-READY; sleep 30")],
        ),
        ("gamma", [("ga-1", "sleep 30"), ("ga-2", "sleep 30")]),
    ] {
        let path = dir.path().join(format!("{team}.toml"));
        let mut file = format!("team = \"{team}\"\n");
        for (name, command) in sessions {
            file.push_str(&format!(
                "[[session]]\nname = \"{name}\"\ncommand = [\"sh\", \"-c\", \"{command}\"]\n"
            ));
        }
        std::fs::write(&path, file)?;
        let path = path.to_string_lossy().into_owned();
        let started = run_binary(
            dir.path(),
            &["team", "up", "--detach", "--file", &path],
            &[],
        )?;
        assert!(
            started.status.success(),
            "team up {team} output: {}",
            String::from_utf8_lossy(&started.stderr)
        );
    }

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.wait_for_text("[+] alpha (private) (2)", WAIT)?;
    attached.wait_for_text("[+] beta (private) (2)", WAIT)?;
    attached.wait_for_text("[+] gamma (private) (2)", WAIT)?;
    attached.wait_for_text("[-] (no team) (1)", WAIT)?;
    attached.wait_for_text("base", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"\x1b[H\r")?;
    attached.wait_for_text("[-] alpha (private) (2)", WAIT)?;
    attached.wait_for_text("al-1", WAIT)?;
    attached.wait_for_text("al-2", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("be-1"), "picker screen: {screen:?}");

    attached.send(b" ")?;
    attached.wait_for_text("[+] alpha (private) (2)", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"/be-2")?;
    attached.wait_for_text("[-] beta (private) (1)", WAIT)?;
    attached.wait_for_text("be-2", WAIT)?;
    attached.wait_for_text("/be-2", WAIT)?;
    let screen = attached.screen_text();
    assert!(!screen.contains("al-1"), "picker screen: {screen:?}");

    attached.send(b"\r")?;
    attached.wait_for_text("BE2-READY", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from ", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_line_renders_clicks_switches_and_toggles_for_real_attach() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let team_file = dir.path().join("demo.toml");
    std::fs::write(
        &team_file,
        "team = \"demo\"\n[[session]]\nname = \"alpha\"\ncommand = [\"sh\", \"-c\", \"printf '\\\\033[?1000h\\\\033[?1006h'; printf ALPHA; stty raw -echo; cat -v\"]\n[[session]]\nname = \"beta\"\ncommand = [\"sh\", \"-c\", \"printf BETA; sleep 30\"]\n[[session]]\nname = \"dead\"\ncommand = [\"sh\", \"-c\", \"exit 0\"]\n",
    )?;
    let team_file = team_file.to_string_lossy().into_owned();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", &team_file],
        &[],
    )?;
    assert!(
        started.status.success(),
        "team up output: {}",
        String::from_utf8_lossy(&started.stderr)
    );

    let small_command = format!(
        "stty rows 4; exec {} attach demo-alpha",
        env!("CARGO_BIN_EXE_a2amx")
    );
    let small_args = vec!["-c".to_owned(), small_command];
    let mut small = PtyHarness::spawn_program("sh", &small_args, dir.path(), &[])?;
    small.wait_for_text("ALPHA", WAIT)?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let small_screen = small.screen_text();
    assert!(
        !small_screen
            .lines()
            .next()
            .unwrap_or_default()
            .contains("team demo"),
        "small screen: {small_screen:?}"
    );
    small.send(&[2, b'd'])?;
    small.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(small.wait_exit(WAIT)?, 0);

    for (rows, draws_rule) in [(5, false), (6, true)] {
        let command = format!(
            "stty rows {rows}; exec {} attach demo-alpha",
            env!("CARGO_BIN_EXE_a2amx")
        );
        let args = vec!["-c".to_owned(), command];
        let mut sized = PtyHarness::spawn_program("sh", &args, dir.path(), &[])?;
        sized.wait_for_text("ALPHA", WAIT)?;
        sized.wait_for_text("team demo", WAIT)?;
        let lines = sized.screen_text();
        let rows = lines.lines().collect::<Vec<_>>();
        assert!(rows[0].contains("team demo"), "screen: {lines:?}");
        if draws_rule {
            assert_eq!(rows[1], "─".repeat(79), "screen: {lines:?}");
            assert!(rows[2].contains("ALPHA"), "screen: {lines:?}");
        } else {
            assert_ne!(rows[1], "─".repeat(79), "screen: {lines:?}");
        }
        sized.send(&[2, b'd'])?;
        sized.wait_for_text("[detached from s1]", WAIT)?;
        assert_eq!(sized.wait_exit(WAIT)?, 0);
    }

    let mut attached =
        PtyHarness::spawn(&cli_args(dir.path(), &["attach", "demo-alpha"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("team demo", WAIT)?;
    let initial_screen = attached.screen_text();
    let initial_rows = initial_screen.lines().collect::<Vec<_>>();
    assert_eq!(
        initial_rows[1],
        "─".repeat(79),
        "screen: {initial_screen:?}"
    );
    assert!(
        initial_rows[2].contains("ALPHA"),
        "screen: {initial_screen:?}"
    );
    assert!(attached.screen_text().contains("demo-alpha"));
    attached.wait_for_text("demo-beta", WAIT)?;
    attached.wait_for_text("demo-dead", WAIT)?;
    attached.send(&[2, b'3'])?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(attached.screen_text().contains("demo-alpha@host-a"));

    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;

    attached.send(&[2, b'1'])?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("demo-alpha@host-a", WAIT)?;
    attached.send(b"\x1b[M\x20\x3e\x21\x1b[M\x23\x3e\x21")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'1'])?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("demo-alpha@host-a", WAIT)?;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(attached.screen_text().contains("demo-alpha@host-a"));

    attached.send(b"\x1b[<0;18;1M\x1b[<0;10;2m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let team_to_rule = attached.screen_text();
    assert!(!team_to_rule.contains("^[[<0;10;1m"));
    assert!(
        !team_to_rule.contains("a2amx sessions:"),
        "screen: {team_to_rule:?}"
    );

    attached.send(b"\x1b[<0;5;2M\x1b[<0;5;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rule_to_team = attached.screen_text();
    assert!(!rule_to_team.contains("^[[<0;5;1m"));
    assert!(
        !rule_to_team.contains("a2amx sessions:"),
        "screen: {rule_to_team:?}"
    );

    attached.send(b"\x1b[<0;12;2M\x1b[<0;12;2m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rule_click = attached.screen_text();
    assert!(!rule_click.contains("^[[<0;12;2m"));
    assert!(
        !rule_click.contains("a2amx sessions:"),
        "screen: {rule_click:?}"
    );

    attached.send(b"\x1b[<0;10;3M\x1b[<0;10;1m")?;
    attached.wait_for_text("^[[<0;10;1m", WAIT)?;
    attached.send(b"\x1b[<0;11;3M\x1b[<64;11;2M\x1b[<0;11;3m")?;
    attached.wait_for_text("^[[<0;11;1m", WAIT)?;

    attached.send(&[2, b'2'])?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b[<64;18;1M\x1b[<64;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"q")?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;
    attached.send(&[2, b'['])?;
    attached.send(b"\x1b[<0;18;1M\x1b[<0;18;1m")?;
    tokio::time::sleep(Duration::from_millis(200)).await;
    attached.send(b"q")?;
    attached.wait_for_text("demo-beta@host-a", WAIT)?;

    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let hidden_screen = attached.screen_text();
    let hidden_first_line = hidden_screen.lines().next().unwrap_or_default();
    assert!(
        hidden_first_line.contains("BETA"),
        "hidden screen: {hidden_screen:?}"
    );
    assert!(
        !hidden_first_line.contains("team demo"),
        "hidden screen: {hidden_screen:?}"
    );
    assert_ne!(
        hidden_screen.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "hidden screen: {hidden_screen:?}"
    );
    attached.send(&[2, b's'])?;
    attached.wait_for_text("team demo", WAIT)?;
    let restored_screen = attached.screen_text();
    assert_eq!(
        restored_screen.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "restored screen: {restored_screen:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);

    let (created, code) = run_cli(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "plain",
            "--",
            "sh",
            "-c",
            "printf PLAIN; sleep 30",
        ],
    )?;
    assert_eq!(code, 0);
    assert!(!created.is_empty());
    let mut plain = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "plain"]), dir.path())?;
    plain.wait_for_text("PLAIN", WAIT)?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!plain.screen_text().contains("team demo"));
    plain.send(&[2, b'd'])?;
    plain.wait_for_text("[detached from s4]", WAIT)?;
    assert_eq!(plain.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_toggle_clears_team_row_without_session_output() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let team_file = dir.path().join("silent.toml");
    std::fs::write(
        &team_file,
        "team = \"silent\"\n[[session]]\nname = \"alpha\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )?;
    let team_file = team_file.to_string_lossy().into_owned();
    let started = run_binary(
        dir.path(),
        &["team", "up", "--detach", "--file", &team_file],
        &[],
    )?;
    assert!(started.status.success());

    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["attach", "silent-alpha"]),
        dir.path(),
    )?;
    attached.wait_for_text("team silent", WAIT)?;
    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let hidden = attached.screen_text();
    assert!(
        !hidden
            .lines()
            .next()
            .unwrap_or_default()
            .contains("team silent"),
        "hidden screen: {hidden:?}"
    );
    assert_ne!(
        hidden.lines().nth(1).unwrap_or_default(),
        "─".repeat(79),
        "hidden screen: {hidden:?}"
    );
    let clear_rule = b"\x1b7\x1b[2;1H\x1b[0m\x1b[2K\x1b8";
    assert!(
        attached
            .raw_output()
            .windows(clear_rule.len())
            .any(|window| window == clear_rule),
        "raw output did not clear row 2: {hidden:?}"
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn status_toggle_reattaches_to_resize_the_session() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::write(dir.path().join("addr"), format!("{address}\n"))?;
    std::fs::write(dir.path().join("admin.token"), "fake-token\n")?;
    let daemon = FakeDaemon::new(fake_team());
    let server = tokio::spawn(run_fake_daemon(listener, daemon.clone()));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("alpha@host-a", WAIT)?;
    attached.send(&[2, b's'])?;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(daemon.attach_count.load(Ordering::SeqCst), 2);
    attached.send(&[2, b'd'])?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn switch_waits_for_old_attachment_eof_with_delayed_fake_daemon() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    std::fs::write(dir.path().join("addr"), format!("{address}\n"))?;
    std::fs::write(dir.path().join("admin.token"), "fake-token\n")?;
    let daemon = FakeDaemon::new(fake_team());
    let server = tokio::spawn(run_fake_daemon(listener, daemon.clone()));

    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("alpha@host-a", WAIT)?;
    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    attached.send(&[2, b'd'])?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    assert!(
        !daemon.early_switch.load(Ordering::SeqCst),
        "target attach arrived before old stream EOF"
    );
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test]
async fn attach_viewer_sends_no_resize_frame() -> anyhow::Result<()> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let hello = fake_recv_frame(&mut stream)
            .await?
            .context("missing hello")?;
        anyhow::ensure!(
            matches!(
                serde_json::from_slice::<Request>(&hello)?,
                Request::Hello { .. }
            ),
            "expected hello request"
        );
        fake_send_frame(&mut stream, &serde_json::to_vec(&Response::Ok)?).await?;
        let attach = fake_recv_frame(&mut stream)
            .await?
            .context("missing attach")?;
        let Request::Attach { viewer, .. } = serde_json::from_slice(&attach)? else {
            anyhow::bail!("expected attach request");
        };
        anyhow::ensure!(viewer, "attach_viewer did not set viewer mode");
        fake_send_frame(&mut stream, &serde_json::to_vec(&Response::Attached)?).await?;
        let next =
            tokio::time::timeout(Duration::from_millis(250), fake_recv_frame(&mut stream)).await;
        Ok::<_, anyhow::Error>(matches!(next, Ok(Ok(Some(_)))))
    });

    let attachment = Client::connect_addr(address, "fake-token")
        .await?
        .attach_viewer("s1", false, 80, 24, 0)
        .await?;
    let resize_or_other_frame = server.await??;
    assert!(
        !resize_or_other_frame,
        "attach_viewer wrote an unexpected frame after Attach"
    );
    drop(attachment);
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_toggle_blocks_input_and_shows_overlay() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;

    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    let raw_before_resize = attached.raw_output().len();
    attached.resize(90, 24)?;
    eventually(|| {
        let changed = attached.raw_output().len() > raw_before_resize;
        async move { changed.then_some(()) }
    })
    .await;
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { .. })),
        "viewer resize sent a daemon frame: {:?}",
        daemon.frames()
    );
    attached.send(b"\x1b[<0;10")?;
    attached.send(b";5M")?;

    let mode_frame_count = daemon
        .frames()
        .iter()
        .filter(|frame| matches!(frame, ClientFrame::Mode { .. }))
        .count();
    attached.send(b"x")?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(b"\x1b[200~paste\x1b[201~")?;
    attached.send(&[2, b'r'])?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(b"\x1b[<0;10")?;
    attached.send(b";5M")?;
    eventually(|| {
        let absent = !attached.screen_text().contains("VIEW MODE");
        async move { absent.then_some(()) }
    })
    .await;
    assert!(
        daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Redraw)),
        "overlay expiry did not request a repaint: {:?}",
        daemon.frames()
    );
    attached.send(b"\x1b[O")?;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b")?;
    eventually(|| {
        let screen = attached.screen_text();
        async move {
            (screen.contains("ALPHA") && screen.contains("VIEW") && !screen.contains("VIEW MODE"))
                .then_some(())
        }
    })
    .await;
    let frames = daemon.frames();
    assert_eq!(
        frames
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Mode { .. }))
            .count(),
        mode_frame_count
    );
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Release)),
        "viewer forwarded a release frame: {frames:?}"
    );
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer forwarded an input frame: {frames:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
async fn assert_viewer_after_stays_interactive(after: &str) -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", after, "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(b"x")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"x"))
                .then_some(())
        }
    })
    .await;
    let observe_until = Instant::now() + Duration::from_millis(2500);
    eventually(|| {
        let screen = attached.screen_text();
        let no_mode = !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }));
        async move {
            (Instant::now() >= observe_until && screen.contains("ALPHA") && no_mode).then_some(())
        }
    })
    .await;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_idle_switches_once_and_toggle_redraws_after_overlay() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "1", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"x")?;
    attached.wait_for_text("VIEW MODE", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        let ready = frames
            .iter()
            .position(|frame| matches!(frame, ClientFrame::Mode { viewer: false }))
            .is_some_and(|left| {
                matches!(frames.get(left + 1), Some(ClientFrame::Resize { .. }))
                    && matches!(frames.get(left + 2), Some(ClientFrame::Redraw))
            });
        async move { ready.then_some(()) }
    })
    .await;
    eventually(|| {
        let absent = !attached.screen_text().contains("VIEW MODE");
        async move { absent.then_some(()) }
    })
    .await;

    let frames = daemon.frames();
    let entered = frames
        .iter()
        .position(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
        .expect("automatic viewer mode frame");
    let left = frames
        .iter()
        .enumerate()
        .find_map(|(index, frame)| {
            (index > entered && matches!(frame, ClientFrame::Mode { viewer: false }))
                .then_some(index)
        })
        .expect("toggle back mode frame");
    assert!(left > entered);
    assert!(matches!(
        frames.get(left + 1),
        Some(ClientFrame::Resize { .. })
    ));
    assert!(matches!(frames.get(left + 2), Some(ClientFrame::Redraw)));
    assert!(
        frames[entered + 1..left]
            .iter()
            .all(|frame| !matches!(frame, ClientFrame::Mode { .. }))
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_idle_switch_ignores_daemon_data() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    daemon.schedule_data_after(Duration::from_millis(2500));
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "3", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.wait_for_text("scheduled daemon data", WAIT)?;
    tokio::time::timeout(
        Duration::from_secs(2),
        eventually(|| {
            let frames = daemon.frames();
            async move {
                frames
                    .iter()
                    .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                    .then_some(())
            }
        }),
    )
    .await?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn interactive_focus_report_forwards_without_resetting_idle_clock() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--viewer-after", "3", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(b"x")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"x"))
                .then_some(())
        }
    })
    .await;
    let focus_deadline = Instant::now() + Duration::from_millis(2500);
    eventually(|| async move { (Instant::now() >= focus_deadline).then_some(()) }).await;
    attached.send(b"\x1b[I")?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Input(bytes) if bytes == b"\x1b[I"))
                .then_some(())
        }
    })
    .await;
    tokio::time::timeout(
        Duration::from_secs(2),
        eventually(|| {
            let frames = daemon.frames();
            async move {
                frames
                    .iter()
                    .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                    .then_some(())
            }
        }),
    )
    .await?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_focus_report_does_not_show_overlay() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    assert!(!attached.screen_text().contains("VIEW MODE"));
    let raw_before = attached.raw_output().len();
    attached.send(b"\x1b[I")?;
    let showed_overlay = tokio::time::timeout(Duration::from_millis(300), async {
        loop {
            let has_overlay = {
                let raw = attached.raw_output();
                raw.len() > raw_before
                    && raw[raw_before..]
                        .windows(b"VIEW MODE".len())
                        .any(|window| window == b"VIEW MODE")
            };
            if has_overlay {
                return true;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap_or(false);
    assert!(!showed_overlay, "focus report showed the viewer overlay");
    assert!(!attached.screen_text().contains("VIEW MODE"));
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_after_zero_and_maximum_never_switch() -> anyhow::Result<()> {
    assert_viewer_after_stays_interactive("0").await?;
    assert_viewer_after_stays_interactive("18446744073709551615").await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_status_toggle_same_top_rows_redraws_without_resize() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(Some(0)).await?;
    daemon.hide_session_teams();
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.wait_for_text("VIEW", WAIT)?;
    let frames_before_hide = daemon.frames().len();

    let raw_before_hide = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let raw_changed = attached.raw_output().len() > raw_before_hide;
        let frames = daemon.frames();
        let no_resize = !frames[frames_before_hide..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { .. }));
        async move { (raw_changed && no_resize).then_some(()) }
    })
    .await;

    let frames_before_show = daemon.frames().len();
    let raw_before_show = attached.raw_output().len();
    attached.send(&[2, b's'])?;
    eventually(|| {
        let has_marker = {
            let raw = attached.raw_output();
            raw.len() > raw_before_show
                && raw
                    .windows(b"\x1b[1;30;43m VIEW \x1b[0m".len())
                    .any(|window| window == b"\x1b[1;30;43m VIEW \x1b[0m")
        };
        let frames = daemon.frames();
        let no_resize = !frames[frames_before_show..]
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { .. }));
        async move { (has_marker && no_resize).then_some(()) }
    })
    .await;
    assert_eq!(daemon.attach_viewers().len(), 1);
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_picker_switch_and_status_toggle_keep_viewer_attach() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(&[2, b'w'])?;
    attached.wait_for_text("a2amx sessions:", WAIT)?;
    attached.send(b"\x1b")?;
    eventually(|| {
        let screen = attached.screen_text();
        async move { (screen.contains("ALPHA") && screen.contains("VIEW")).then_some(()) }
    })
    .await;

    attached.send(&[2, b'n'])?;
    attached.wait_for_text("BETA", WAIT)?;
    eventually(|| {
        let attaches = daemon.attach_viewers();
        async move { (attaches.len() >= 2).then_some(()) }
    })
    .await;
    attached.send(&[2, b's'])?;
    eventually(|| {
        let attaches = daemon.attach_viewers();
        async move { (attaches.len() >= 3).then_some(()) }
    })
    .await;

    let attaches = daemon.attach_viewers();
    assert!(!attaches[0].1);
    assert!(
        attaches.iter().skip(1).all(|(_, viewer)| *viewer),
        "{attaches:?}"
    );
    let frames = daemon.frames();
    assert!(
        !frames
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Resize { .. })),
        "viewer switch or status toggle resized: {frames:?}"
    );

    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

async fn assert_viewer_mouse_is_not_forwarded(top_rows: Option<u16>) -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(top_rows).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    let resize_count = daemon
        .frames()
        .iter()
        .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
        .count();
    if top_rows == Some(0) {
        attached.send(&[2, b's'])?;
    }

    let raw_before_mouse = attached.raw_output().to_vec();
    attached.send(b"\x1b[<0;10;5M\x1b[<0;10;5m\x1b[<35;10;5M")?;
    attached.send(b"\x1b[<64;10;5M")?;
    eventually(|| {
        let changed = attached.raw_output().len() > raw_before_mouse.len();
        async move { changed.then_some(()) }
    })
    .await;
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    let raw_after_mouse = attached.raw_output();
    let suffix = &raw_after_mouse[raw_before_mouse.len()..];
    assert!(
        !suffix
            .windows(b"VIEW MODE".len())
            .any(|window| window == b"VIEW MODE"),
        "complete mouse report showed the viewer overlay in raw output"
    );
    attached.wait_for_text("[scroll: q to exit]", WAIT)?;
    assert!(
        !attached.screen_text().contains("VIEW MODE"),
        "mouse report showed the viewer overlay"
    );
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer forwarded a mouse frame: {:?}",
        daemon.frames()
    );
    assert_eq!(
        daemon
            .frames()
            .iter()
            .filter(|frame| matches!(frame, ClientFrame::Resize { .. }))
            .count(),
        resize_count,
        "viewer status toggle or mouse path resized the daemon: {:?}",
        daemon.frames()
    );
    attached.send(b"q")?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_team_click_acts_on_release_and_keeps_viewer_mode() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(&cli_args(dir.path(), &["attach", "s1"]), dir.path())?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[2, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"\x1b[<0;30;1M\x1b[<0;30;1m")?;
    attached.wait_for_text("BETA", WAIT)?;
    let attaches = daemon.attach_viewers();
    assert_eq!(attaches.get(1).map(|(_, viewer)| *viewer), Some(true));
    assert!(
        !daemon
            .frames()
            .iter()
            .any(|frame| matches!(frame, ClientFrame::Input(_))),
        "viewer team click forwarded input: {:?}",
        daemon.frames()
    );
    attached.send(&[2, b'd'])?;
    attached.wait_for_text("[detached from s2]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_overlay_uses_the_configured_prefix_name() -> anyhow::Result<()> {
    let (dir, daemon, server) = start_fake_daemon(None).await?;
    let mut attached = PtyHarness::spawn(
        &cli_args(dir.path(), &["--prefix", "C-space", "attach", "s1"]),
        dir.path(),
    )?;
    attached.wait_for_text("ALPHA", WAIT)?;
    attached.send(&[0, b'v'])?;
    eventually(|| {
        let frames = daemon.frames();
        async move {
            frames
                .iter()
                .any(|frame| matches!(frame, ClientFrame::Mode { viewer: true }))
                .then_some(())
        }
    })
    .await;
    attached.send(b"x")?;
    attached.wait_for_text("Ctrl-Space", WAIT)?;
    assert!(!attached.screen_text().contains("Ctrl-B"));
    attached.send(&[0, b'd'])?;
    attached.wait_for_text("[detached from s1]", WAIT)?;
    assert_eq!(attached.wait_exit(WAIT)?, 0);
    server.abort();
    let _ = server.await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_mouse_reports_are_dropped_with_and_without_team_rows() -> anyhow::Result<()> {
    assert_viewer_mouse_is_not_forwarded(None).await?;
    assert_viewer_mouse_is_not_forwarded(Some(0)).await?;
    Ok(())
}
