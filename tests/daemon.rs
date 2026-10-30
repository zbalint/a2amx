// Shared test helpers are not themselves annotated with #[test].
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::os::unix::fs::{PermissionsExt, symlink};

use a2amx::daemon::{Daemon, DaemonConfig};
use std::time::Duration;

use a2amx::client::{Attachment, Client};
use a2amx::emulator::{Emulator, Scroll, Size};
use a2amx::messaging::{Limits, TeamScope};
use a2amx::wire::{ClientFrame, Request, Response, ServerFrame, SessionSummary, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn activity_age_preserves_same_state_and_resets_on_transition() {
    use a2amx::session::observe_activity;
    use a2amx::wire::Activity::{Idle, Working};
    use std::time::Instant;

    let t0 = Instant::now();
    let mut slot = None;
    assert_eq!(observe_activity(&mut slot, Idle, t0), Duration::ZERO);
    assert_eq!(slot, Some((Idle, t0)));
    assert_eq!(
        observe_activity(&mut slot, Idle, t0 + Duration::from_secs(5)),
        Duration::from_secs(5)
    );
    assert_eq!(slot, Some((Idle, t0)));
    let changed = t0 + Duration::from_secs(7);
    assert_eq!(
        observe_activity(&mut slot, Working, changed),
        Duration::ZERO
    );
    assert_eq!(slot, Some((Working, changed)));
    assert_eq!(
        observe_activity(&mut slot, Working, t0 + Duration::from_secs(9)),
        Duration::from_secs(2)
    );
    assert_eq!(observe_activity(&mut slot, Working, t0), Duration::ZERO);
    assert_eq!(slot, Some((Working, changed)));
}

#[test]
fn duration_display_floors_at_unit_boundaries() {
    for (seconds, expected) in [
        (0, "1s"),
        (1, "1s"),
        (59, "59s"),
        (60, "1m"),
        (3599, "59m"),
        (3600, "1h"),
        (90000, "25h"),
    ] {
        assert_eq!(
            a2amx::messaging::display_duration(Duration::from_secs(seconds)),
            expected
        );
    }
}

#[test]
fn shutdown_wire_defaults_graceful_and_omits_false_now() {
    let kill: Request = serde_json::from_str(r#"{"type":"kill","session":"s1"}"#).unwrap();
    assert_eq!(
        kill,
        Request::Kill {
            session: "s1".into(),
            now: false,
        }
    );
    let shutdown = Request::Shutdown { now: false };
    assert_eq!(
        serde_json::to_string(&shutdown).unwrap(),
        r#"{"type":"shutdown"}"#
    );
}
async fn create(client: &mut Client, argv: &[&str], env: Vec<(String, String)>) -> String {
    match client
        .request(Request::NewSession {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            cols: 40,
            rows: 5,
            cwd: None,
            env,
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

fn confine_new_session_request() -> Request {
    Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec!["true".into()],
            cols: 40,
            rows: 5,
            cwd: Some(
                std::env::current_dir()
                    .expect("current directory")
                    .to_string_lossy()
                    .into_owned(),
            ),
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        }),
    }
}

#[tokio::test]
async fn nested_confine_is_rejected_without_closing_daemon_connection() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let nested = Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(Request::Confine {
            confine: a2amx::confine::ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec!["true".into()],
                cols: 40,
                rows: 5,
                cwd: None,
                env: vec![],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: None,
                harness: a2amx::harness::Harness::Generic,
                deliver: None,
                team: None,
                role: None,
            }),
        }),
    };
    let response = client.request(nested).await.unwrap();
    assert!(matches!(
        response,
        Response::Error { message } if message.contains("nested confinement")
    ));
    let id = create(&mut client, &["true"], vec![]).await;
    assert!(!id.is_empty());
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn session_tokens_cannot_confine_and_admin_confine_rejects_invalid_targets() {
    use a2amx::harness::{Deliver, Harness};

    let dir = tempfile::tempdir().unwrap();
    let Ok(real_bwrap) = a2amx::confine::resolve_bwrap(None) else {
        return;
    };
    let bwrap_dir = dir.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).unwrap();
    symlink(real_bwrap, bwrap_dir.join("bwrap")).unwrap();
    let daemon = Daemon::start(DaemonConfig {
        state_dir: dir.path().to_path_buf(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: Some("host-a".into()),
        limits: Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
    })
    .await
    .unwrap();
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let before = list(&mut admin).await;
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(
            &encode_frame(&serde_json::to_vec(&confine_new_session_request()).unwrap()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "not permitted for a session token".into()
        }
    );
    assert_eq!(list(&mut admin).await, before);

    let created = admin.request(confine_new_session_request()).await.unwrap();
    let Response::Created { .. } = created else {
        panic!("admin confined creation failed: {created:?}");
    };
    let nested = Request::Confine {
        confine: a2amx::confine::ConfineRequest::default(),
        request: Box::new(confine_new_session_request()),
    };
    assert!(matches!(
        admin.request(nested).await.unwrap(),
        Response::Error { message } if message.contains("nested confinement")
    ));
    assert!(matches!(
        admin
            .request(Request::Confine {
                confine: a2amx::confine::ConfineRequest::default(),
                request: Box::new(Request::List),
            })
            .await
            .unwrap(),
        Response::Error { message } if message.contains("expected new_session")
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn attach_top_rows_acknowledges_and_resets_origin() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    for top_rows in [Some(9), Some(8), Some(1), Some(0), None] {
        let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
        socket
            .write_all(
                &encode_frame(
                    &serde_json::to_vec(&Request::Hello {
                        token: token.clone(),
                    })
                    .unwrap(),
                )
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(raw_response(&mut socket).await, Response::Ok);
        let mut request =
            serde_json::json!({"type":"attach","session":id,"force":true,"cols":40,"rows":5});
        if let Some(top_rows) = top_rows {
            request["top_rows"] = top_rows.into();
        }
        socket
            .write_all(&encode_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
            .await
            .unwrap();
        let response = raw_response(&mut socket).await;
        if top_rows == Some(9) {
            assert!(matches!(response, Response::Error { .. }));
            continue;
        }
        if let Some(rows @ 1..=8) = top_rows {
            assert_eq!(response, Response::AttachedTop { top_rows: rows });
        } else {
            assert_eq!(response, Response::Attached);
        }
        let length = socket.read_u32().await.unwrap();
        let mut frame = vec![0; length as usize];
        socket.read_exact(&mut frame).await.unwrap();
        let ServerFrame::Data(bytes) = ServerFrame::decode(&frame).unwrap() else {
            panic!("expected data")
        };
        if top_rows == Some(1) {
            assert!(bytes.starts_with(b"\x1b[2;1H\x1b[J"));
        } else if top_rows == Some(8) {
            assert!(bytes.starts_with(b"\x1b[9;1H\x1b[J"));
        } else {
            assert!(bytes.starts_with(b"\x1b[2J\x1b[H"));
        }
    }
    daemon.shutdown().await.unwrap();
}
#[tokio::test]
async fn resize_and_redraw_preserve_top_rows_origin() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    let request = Request::Attach {
        session: id,
        force: false,
        cols: 40,
        rows: 5,
        status: false,
        top_rows: 1,
    };
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&request).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::AttachedTop { top_rows: 1 }
    );
    let length = socket.read_u32().await.unwrap();
    let mut initial = vec![0; length as usize];
    socket.read_exact(&mut initial).await.unwrap();
    assert!(matches!(
        ServerFrame::decode(&initial).unwrap(),
        ServerFrame::Data(_)
    ));
    for request in [
        ClientFrame::Resize { cols: 42, rows: 6 },
        ClientFrame::Redraw,
    ] {
        socket
            .write_all(&encode_frame(&request.encode()).unwrap())
            .await
            .unwrap();
        let length = socket.read_u32().await.unwrap();
        let mut frame = vec![0; length as usize];
        socket.read_exact(&mut frame).await.unwrap();
        let ServerFrame::Data(bytes) = ServerFrame::decode(&frame).unwrap() else {
            panic!("expected data after {request:?}");
        };
        assert!(
            bytes.starts_with(b"\x1b[2;1H\x1b[J"),
            "after {request:?}: {bytes:?}"
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn new_session_validation_messages_and_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for (empty_argv, cols, rows, reset, control_from, watch, heartbeat, name, role, expected) in [
        (
            true,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            0,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            0,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            5,
            vec!["clear"],
            vec![],
            vec![],
            None,
            "worker",
            None,
            "reset step 1 must start with '/'",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec![],
            None,
            "worker",
            None,
            "control_from entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec!["Bad_Name"],
            None,
            "worker",
            None,
            "watch entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec!["peer"],
            Some(0),
            "worker",
            None,
            "invalid heartbeat \"0s\": expected a number followed by s, m or h, from 1s to 24h",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            Some(1),
            "worker",
            None,
            "session worker: heartbeat needs a non-empty watch",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "worker",
            Some(""),
            "role must not be empty",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "Bad_Name",
            None,
            "invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            true,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec![],
            None,
            "worker",
            None,
            "command and nonzero dimensions are required",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec!["Bad_Name"],
            vec!["worker"],
            None,
            "worker",
            None,
            "control_from entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            None,
            "Bad_Name",
            Some(""),
            "role must not be empty",
        ),
        (
            false,
            40,
            5,
            vec![],
            vec![],
            vec![],
            Some(1),
            "Bad_Name",
            None,
            "session Bad_Name: heartbeat needs a non-empty watch",
        ),
    ] {
        let response = client
            .request(Request::NewSession {
                argv: if empty_argv {
                    vec![]
                } else {
                    vec!["cat".into()]
                },
                cols,
                rows,
                cwd: None,
                env: vec![],
                reset: reset.into_iter().map(str::to_owned).collect(),
                control_from: control_from.into_iter().map(str::to_owned).collect(),
                watch: watch.into_iter().map(str::to_owned).collect(),
                heartbeat,
                activity: None,
                name: Some(name.into()),
                harness: a2amx::harness::Harness::Generic,
                deliver: None,
                team: None,
                role: role.map(str::to_owned),
            })
            .await
            .unwrap();
        assert_eq!(
            response,
            Response::Error {
                message: expected.into()
            },
            "{name}"
        );
    }
    daemon.shutdown().await.unwrap();
}

async fn create_claude(client: &mut Client, command: &str) -> String {
    match client
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), command.into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Claude,
            deliver: Some(a2amx::harness::Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

async fn create_claude_team(client: &mut Client, command: &str, team: TeamScope) -> String {
    match client
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), command.into()],
            cols: 40,
            rows: 5,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Claude,
            deliver: Some(a2amx::harness::Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: Some(team),
            role: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

async fn list(client: &mut Client) -> Vec<SessionSummary> {
    match client.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("unexpected list response: {response:?}"),
    }
}

async fn eventually_sessions(
    client: &mut Client,
    predicate: impl Fn(&[SessionSummary]) -> bool,
) -> Vec<SessionSummary> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let sessions = list(client).await;
            if predicate(&sessions) {
                return sessions;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("session condition within deadline")
}
#[tokio::test]
async fn list_activity_reports_working_then_idle_and_non_ready_working() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let prompt_id = create(
        &mut client,
        &["sh", "-i", "-c", "printf '\\033[?2004h'; read x"],
        vec![],
    )
    .await;
    let sleeping_id = create(&mut client, &["sleep", "30"], vec![]).await;

    let sessions = list(&mut client).await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == prompt_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Working)
    );
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == sleeping_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Working)
    );

    tokio::time::sleep(Duration::from_millis(10_200)).await;
    let sessions = list(&mut client).await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == prompt_id)
            .and_then(|session| session.activity),
        Some(a2amx::wire::Activity::Idle)
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn attach_status_reports_starting_latch_and_generic_never_starting() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "activity".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let claude = create_claude_team(
        &mut admin,
        r#"sleep 2; printf '\033[2J\033[?2004h\033[2;1H'; i=0; while [ "$i" -lt 40 ]; do printf '─'; i=$((i+1)); done; printf '\033[3;1H❯ \033[4;1H'; i=0; while [ "$i" -lt 40 ]; do printf '─'; i=$((i+1)); done; printf '\033[3;3H\033[?25h'; sleep 3; printf '\033[2J\033[HEnter to confirm · Esc to cancel'; sleep 30"#,
        team,
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&claude, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(initial) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    assert!(initial.starting);

    let mut saw_ready = false;
    while !saw_ready {
        match next(&mut attachment).await {
            ServerFrame::Status(info) => {
                if !info.starting {
                    saw_ready = true;
                }
            }
            ServerFrame::Data(_) => {}
            frame => panic!("unexpected frame while observing status: {frame:?}"),
        }
    }
    tokio::time::sleep(Duration::from_millis(14_500)).await;
    let mut held_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&claude, true, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(
        next(&mut held_attachment).await,
        ServerFrame::Data(_)
    ));
    let ServerFrame::Status(held_status) = next(&mut held_attachment).await else {
        panic!("held status frame");
    };
    assert!(held_status.hold.is_some());
    assert!(!held_status.starting);
    assert!(
        held_status
            .members
            .iter()
            .any(|member| member.id == claude && member.held)
    );

    let generic = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let mut generic_attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&generic, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(
        next(&mut generic_attachment).await,
        ServerFrame::Data(_)
    ));
    let ServerFrame::Status(generic_status) = next(&mut generic_attachment).await else {
        panic!("generic initial status frame");
    };
    assert!(!generic_status.starting);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_never_ready_team_member_is_not_starting() {
    use a2amx::harness::{Deliver, Harness};

    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "exited-starting".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let exited = create_claude_team(&mut admin, "sleep 0.2", team.clone()).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == exited)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;
    let live = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("live"),
        Harness::Generic,
        Deliver::Auto,
        Some(team),
        &[],
        &[],
    )
    .await
    .0;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&live, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(status) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    let member = status
        .members
        .iter()
        .find(|member| member.id == exited)
        .expect("exited Claude member in team status");
    assert!(member.exited);
    assert!(!member.starting);
    assert!(!member.held);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_needs_input_label_reaches_list_and_attach_status() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = create_claude(
        &mut admin,
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; sleep 30"#,
    )
    .await;
    let initial = list(&mut admin).await;
    let summary = initial.iter().find(|session| session.id == id).unwrap();
    assert_eq!(summary.hold_reason, None);
    assert!(!summary.held);

    tokio::time::sleep(Duration::from_millis(10_200)).await;
    let sessions = eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?") && session.held)
    })
    .await;
    let address = sessions
        .iter()
        .find(|session| session.id == id)
        .unwrap()
        .address
        .clone();

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&id, false, 40, 5, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut attachment).await else {
        panic!("initial status frame");
    };
    assert_eq!(info.hold.as_deref(), Some("input?"));
    let rendered = String::from_utf8(a2amx::status::render(&id, Some(&info), 2, 80, 24)).unwrap();
    assert!(rendered.contains("HELD input?"));
    assert!(!rendered.contains("^B r"));

    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    let sessions = eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("human_draft"))
    })
    .await;
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.id == id)
            .unwrap()
            .hold_reason
            .as_deref(),
        Some("human_draft")
    );

    let (_, token, sender_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        a2amx::harness::Harness::Generic,
        a2amx::harness::Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(sender_addr, &token).await.unwrap();
    let Response::Accepted { recipient_hold, .. } = sender
        .request(Request::SendMessage {
            to: address,
            subject: "Needs input status".into(),
            message: "hello".into(),
        })
        .await
        .unwrap()
    else {
        panic!("message was not accepted");
    };
    assert_eq!(recipient_hold.as_deref(), Some("human_draft"));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn claude_needs_input_clears_when_dialog_marker_disappears() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = create_claude(
        &mut admin,
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; sleep 12; printf '\033[2J\033[Hready'; sleep 30"#,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10_200)).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?"))
    })
    .await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.is_none() && !session.held)
    })
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn exited_claude_session_never_reports_needs_input() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let id = create_claude(
        &mut admin,
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; sleep 11"#,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(10_200)).await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.hold_reason.as_deref() == Some("input?"))
    })
    .await;
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.exit_code.is_some() && session.hold_reason.is_none())
    })
    .await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn list_reports_session_durations_until_natural_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "sleep 5"], vec![]).await;

    let sessions = list(&mut client).await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert!(summary.uptime_secs.is_some_and(|seconds| seconds <= 1));
    assert!(summary.activity_secs.is_some_and(|seconds| seconds <= 1));

    tokio::time::sleep(Duration::from_millis(2_200)).await;
    let sessions = list(&mut client).await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert!(
        summary
            .uptime_secs
            .is_some_and(|seconds| (2..=5).contains(&seconds))
    );
    assert!(
        summary
            .activity_secs
            .is_some_and(|seconds| (2..=5).contains(&seconds))
    );

    let sessions = eventually_sessions(&mut client, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == id)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;
    let summary = sessions.iter().find(|session| session.id == id).unwrap();
    assert_eq!(summary.uptime_secs, None);
    assert_eq!(summary.activity_secs, None);
    daemon.shutdown().await.unwrap();
}

fn text(emulator: &Emulator) -> String {
    emulator.screen().cells.iter().map(|cell| cell.ch).collect()
}

async fn next(attachment: &mut Attachment) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(10), attachment.recv())
        .await
        .expect("stream deadline")
        .unwrap()
        .expect("stream frame")
}

async fn wait_text(attachment: &mut Attachment, emulator: &mut Emulator, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !text(emulator).contains(expected) {
            match attachment.recv().await.unwrap().expect("data before close") {
                ServerFrame::Data(bytes) => {
                    emulator.feed(&bytes);
                }
                frame => panic!("expected text before {frame:?}"),
            }
        }
    })
    .await
    .expect("screen text within deadline");
}

async fn raw_response(stream: &mut TcpStream) -> Response {
    let size = stream.read_u32().await.unwrap() as usize;
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn serve_authentication_errors_preserve_connection_lifecycle() {
    let (dir, daemon) = common::start_daemon().await;
    for body in [
        br#"{"type":"list"}"#.as_slice(),
        br#"{"type":"hello","token":"wrong-token"}"#.as_slice(),
        b"invalid",
    ] {
        let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
        socket
            .write_all(&encode_frame(body).unwrap())
            .await
            .unwrap();
        assert_eq!(
            raw_response(&mut socket).await,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            }
        );
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read_u8())
                .await
                .unwrap()
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::UnexpectedEof
        );
    }
    let mut admin = Client::connect(dir.path()).await.unwrap();
    assert_eq!(
        admin
            .request(Request::Hello {
                token: "ignored".into()
            })
            .await
            .unwrap(),
        Response::Error {
            message: "already authenticated".into()
        }
    );
    assert_eq!(
        admin.request(Request::Version).await.unwrap(),
        Response::Version {
            version: a2amx::VERSION.into(),
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_control_validation_messages_and_attachment_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    for (request, expected) in [
        (
            Request::ListMessages {
                session: None,
                state: Some("bad-state".into()),
            },
            "unknown message state",
        ),
        (
            Request::CancelMessage { id: "abc".into() },
            "unknown message abc",
        ),
        (
            Request::Screen {
                session: "s999".into(),
            },
            "unknown session s999",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 40,
                rows: 5,
                status: false,
                top_rows: 0,
            },
            "unknown session s999",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 0,
                rows: 5,
                status: false,
                top_rows: 0,
            },
            "attachment dimensions must be nonzero",
        ),
        (
            Request::Attach {
                session: "s999".into(),
                force: false,
                cols: 40,
                rows: 0,
                status: false,
                top_rows: 0,
            },
            "attachment dimensions must be nonzero",
        ),
    ] {
        assert_eq!(
            admin.request(request).await.unwrap(),
            Response::Error {
                message: expected.into()
            }
        );
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_cancelled_message_reports_full_state_error() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    common::new_agent(
        &mut admin,
        dir.path(),
        Some("recipient"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(addr, &token).await.unwrap();
    let Response::Accepted { id, .. } = sender
        .request(Request::SendMessage {
            to: "recipient@host-a".into(),
            subject: "Cancel".into(),
            message: "body".into(),
        })
        .await
        .unwrap()
    else {
        panic!("message not accepted");
    };
    assert_eq!(id, "m_1");
    assert_eq!(
        admin
            .request(Request::CancelMessage { id: id.clone() })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(
        admin.request(Request::CancelMessage { id }).await.unwrap(),
        Response::Error {
            message: "message m_1 is cancelled and cannot be cancelled".into()
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_session_token_parse_precedes_permission_check() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(&encode_frame(b"invalid").unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "invalid control request".into()
        }
    );
    socket
        .write_all(&encode_frame(br#"{"type":"list"}"#).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "not permitted for a session token".into()
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn serve_exited_and_removed_sessions_close_before_parsing_requests() {
    for removed in [false, true] {
        for body in [
            b"invalid".as_slice(),
            br#"{"type":"list_agents"}"#.as_slice(),
        ] {
            let (dir, daemon) = common::start_daemon().await;
            let mut admin = Client::connect(dir.path()).await.unwrap();
            let credentials = dir.path().join("credentials");
            let exit = dir.path().join("exit");
            let id = create(&mut admin, &["sh", "-c",
                "printf '%s' \"$A2AMX_TOKEN\" > \"$OUT\"; while [ ! -f \"$EXIT\" ]; do sleep 0.01; done"],
                vec![("OUT".into(), credentials.to_string_lossy().into_owned()),
                     ("EXIT".into(), exit.to_string_lossy().into_owned())]).await;
            let token = common::eventually(|| async {
                let token = std::fs::read_to_string(&credentials).ok()?;
                (token.len() == 64).then_some(token)
            })
            .await;
            let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
            socket
                .write_all(
                    &encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(raw_response(&mut socket).await, Response::Ok);
            if removed {
                assert_eq!(
                    admin
                        .request(Request::Kill {
                            session: id.clone(),
                            now: true
                        })
                        .await
                        .unwrap(),
                    Response::Ok
                );
                assert!(list(&mut admin).await.is_empty());
            } else {
                std::fs::write(&exit, b"exit").unwrap();
                eventually_sessions(&mut admin, |sessions| {
                    sessions
                        .iter()
                        .any(|session| session.id == id && session.exit_code == Some(0))
                })
                .await;
            }
            socket
                .write_all(&encode_frame(body).unwrap())
                .await
                .unwrap();
            assert_eq!(
                raw_response(&mut socket).await,
                Response::Error {
                    message: "session has exited".into()
                }
            );
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(2), socket.read_u8())
                    .await
                    .unwrap()
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::UnexpectedEof
            );
            daemon.shutdown().await.unwrap();
        }
    }
}

#[tokio::test]
async fn state_is_owner_only_and_exclusively_locked() {
    let (dir, daemon) = common::start_daemon().await;
    assert_eq!(
        std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for file in ["admin.token", "addr"] {
        assert_eq!(
            std::fs::metadata(dir.path().join(file))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let second = Daemon::start(DaemonConfig {
        state_dir: dir.path().to_owned(),
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(dir.path().join("bwrap-path").into_os_string()),
    })
    .await;
    assert_eq!(
        second.err().unwrap().to_string(),
        "another a2amx daemon is running for this state dir"
    );
    daemon.shutdown().await.unwrap();
    assert!(!dir.path().join("addr").exists());
}

#[tokio::test]
async fn admin_version_request_reports_the_build_version() {
    let (dir, daemon) = common::start_daemon().await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);

    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Version).unwrap()).unwrap())
        .await
        .unwrap();
    match raw_response(&mut socket).await {
        Response::Version { version } => assert!(!version.is_empty()),
        response => panic!("unexpected version response: {response:?}"),
    }
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_control_request_is_recoverable_on_an_authenticated_socket() {
    let (dir, daemon) = common::start_daemon().await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);

    socket
        .write_all(&encode_frame(br#"{"type":"no_such_request"}"#).unwrap())
        .await
        .unwrap();
    assert_eq!(
        raw_response(&mut socket).await,
        Response::Error {
            message: "invalid control request".into(),
        }
    );

    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::List).unwrap()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        raw_response(&mut socket).await,
        Response::Sessions { .. }
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn every_connection_requires_a_valid_hello() {
    let (dir, daemon) = common::start_daemon().await;
    let addr = daemon.addrs()[0];
    let mut socket = TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::List).unwrap()).unwrap())
        .await
        .unwrap();
    assert!(matches!(
        raw_response(&mut socket).await,
        Response::Error { .. }
    ));
    assert_eq!(
        socket.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    let error = Client::connect_addr(addr, "wrong-token")
        .await
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("admin.token"));
    assert!(!error.contains("wrong-token"));
    let mut silent = TcpStream::connect(addr).await.unwrap();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(7), raw_response(&mut silent))
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(
        silent.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    let mut valid = Client::connect(dir.path()).await.unwrap();
    assert_eq!(list(&mut valid).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_spawns_and_zero_sizes_leave_no_sessions() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for (argv, cols, rows) in [
        (vec![], 40, 5),
        (vec!["/no-such-a2amx-program".into()], 40, 5),
        (vec!["cat".into()], 0, 5),
        (vec!["cat".into()], 40, 0),
    ] {
        assert!(matches!(
            client
                .request(Request::NewSession {
                    argv,
                    cols,
                    rows,
                    cwd: None,
                    env: vec![],
                    reset: vec![],
                    control_from: vec![],
                    watch: vec![],
                    name: None,
                    harness: a2amx::harness::Harness::Generic,
                    deliver: None,
                    heartbeat: None,
                    activity: None,
                    team: None,
                    role: None,
                })
                .await
                .unwrap(),
            Response::Error { .. }
        ));
        assert_eq!(list(&mut client).await, vec![]);
    }
    assert!(matches!(
        client
            .request(Request::Kill {
                session: "unknown".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    let id = create(&mut client, &["cat"], vec![]).await;
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach(&id, false, 0, 5)
            .await
            .is_err()
    );
    assert!(
        Client::connect(dir.path())
            .await
            .unwrap()
            .attach("unknown", false, 40, 5)
            .await
            .is_err()
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn input_resize_takeover_detach_and_disconnect_preserve_session() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    assert!(matches!(next(&mut first).await, ServerFrame::Data(_)));
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    first
        .send(ClientFrame::Input(b"consumer-input\n".to_vec()))
        .await
        .unwrap();
    wait_text(&mut first, &mut emu, "consumer-input").await;
    let error = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .err()
        .unwrap()
        .to_string();
    assert_eq!(
        error,
        format!("session {id} is attached elsewhere (use --force to take over)")
    );
    let mut second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 60, 8)
        .await
        .unwrap();
    let reason = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match first.recv().await.unwrap() {
                Some(ServerFrame::Data(_)) => continue,
                Some(ServerFrame::Detached(reason)) => break reason,
                frame => panic!("expected take-over notice, got {frame:?}"),
            }
        }
    })
    .await
    .expect("take-over notice within deadline");
    assert_eq!(reason, "taken over by another attachment");
    assert!(first.recv().await.unwrap().is_none());
    assert!(matches!(next(&mut second).await, ServerFrame::Data(_)));
    second
        .send(ClientFrame::Resize { cols: 30, rows: 6 })
        .await
        .unwrap();
    let mut resized = Emulator::new(Size { cols: 30, rows: 6 });
    match next(&mut second).await {
        ServerFrame::Data(bytes) => {
            resized.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&resized).contains("consumer-input"));
    eventually_sessions(&mut client, |s| {
        s[0].cols == 30 && s[0].rows == 6 && s[0].attached
    })
    .await;
    second.send(ClientFrame::Detach).await.unwrap();
    eventually_sessions(&mut client, |s| !s[0].attached).await;
    let third = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    drop(third);
    eventually_sessions(&mut client, |s| !s[0].attached && s[0].exit_code.is_none()).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn detached_queries_environment_and_final_output_are_preserved() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let out = dir.path().join("reply");
    let id = create(&mut client, &["sh", "-c", "stty raw -echo; printf '\\033[c'; head -c 5 > \"$OUT\"; printf '%s:%s:final' \"$TERM\" \"$COLORTERM\"; exit 7"],
        vec![("OUT".into(), out.to_str().unwrap().into()), ("TERM".into(), "wrong".into()), ("COLORTERM".into(), "wrong".into())]).await;
    let exited = eventually_sessions(&mut client, |s| s[0].exit_code == Some(7)).await;
    assert_eq!(exited[0].id, id);
    assert_eq!(std::fs::read(out).unwrap(), b"\x1b[?6c");
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    match next(&mut attachment).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        frame => panic!("snapshot first, not {frame:?}"),
    }
    assert!(text(&emu).contains("xterm-256color:truecolor:final"));
    assert_eq!(next(&mut attachment).await, ServerFrame::Exit(7));
    assert!(attachment.recv().await.unwrap().is_none());
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(list(&mut client).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn live_exit_sends_last_screen_before_status_and_retains_it() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &["sh", "-c", "read line; printf 'last-screen'; exit 23"],
        vec![],
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    attachment
        .send(ClientFrame::Input(b"go\n".to_vec()))
        .await
        .unwrap();
    loop {
        match next(&mut attachment).await {
            ServerFrame::Data(bytes) => {
                emu.feed(&bytes);
            }
            ServerFrame::Exit(code) => {
                assert_eq!(code, 23);
                assert!(text(&emu).contains("last-screen"));
                break;
            }
            frame => panic!("{frame:?}"),
        }
    }
    assert_eq!(list(&mut client).await[0].exit_code, Some(23));
    let mut late = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    match next(&mut late).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("last-screen"));
    assert_eq!(next(&mut late).await, ServerFrame::Exit(23));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn detached_output_snapshot_and_scroll_history_are_model_renders() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "printf hi; read line; i=1; while [ \"$i\" -le 30 ]; do printf '\\r\\nline%02d' \"$i\"; i=$((i + 1)); done; read line"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut first, &mut emu, "hi").await;
    first.send(ClientFrame::Detach).await.unwrap();
    eventually_sessions(&mut client, |s| !s[0].attached).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    match next(&mut attached).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("hi"));
    attached
        .send(ClientFrame::Input(b"go\n".to_vec()))
        .await
        .unwrap();
    wait_text(&mut attached, &mut emu, "line30").await;
    attached
        .send(ClientFrame::Scroll(Scroll::Top))
        .await
        .unwrap();
    loop {
        match next(&mut attached).await {
            ServerFrame::Data(bytes) => {
                emu.feed(&bytes);
                if text(&emu).contains("line01") {
                    break;
                }
            }
            f => panic!("{f:?}"),
        }
    }
    assert!(!emu.screen().cursor.visible);
    attached
        .send(ClientFrame::Scroll(Scroll::Bottom))
        .await
        .unwrap();
    wait_text(&mut attached, &mut emu, "line30").await;
    attached.send(ClientFrame::Redraw).await.unwrap();
    assert!(matches!(next(&mut attached).await, ServerFrame::Data(_)));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn kill_removes_running_session_and_shutdown_disconnects_attachments() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["sh", "-c", "sleep 30"], vec![]).await;
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(list(&mut client).await, vec![]);
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    next(&mut attachment).await;
    daemon.shutdown().await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_secs(10), attachment.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    assert!(!dir.path().join("addr").exists());
}

#[tokio::test]
async fn output_drains_when_the_attached_client_never_reads() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &[
            "sh",
            "-c",
            "read line; head -c 20000000 /dev/zero | tr '\\0' x",
        ],
        vec![],
    )
    .await;
    let mut stalled = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    stalled
        .send(ClientFrame::Input(b"start\n".to_vec()))
        .await
        .unwrap();
    let sessions = eventually_sessions(&mut client, |s| s[0].exit_code == Some(0)).await;
    assert_eq!(sessions[0].exit_code, Some(0));
    drop(stalled);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn missing_or_refused_daemon_has_the_exact_start_hint() {
    let temp = tempfile::tempdir().unwrap();
    let expected = "cannot reach the a2amx daemon: start it with `a2amx daemon start`";
    assert_eq!(
        Client::connect(temp.path())
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert_eq!(
        Client::connect_addr(address, "unused")
            .await
            .err()
            .unwrap()
            .to_string(),
        expected
    );
}

#[tokio::test]
async fn partial_bind_failure_releases_addresses_and_lock_and_parents_are_private() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("parent/state");
    let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = occupied.local_addr().unwrap();
    let bwrap_dir = home.join("bwrap-path");
    assert!(
        Daemon::start(DaemonConfig {
            state_dir: home.clone(),
            listen: vec!["127.0.0.1:0".parse().unwrap(), address],
            host_name: None,
            limits: a2amx::messaging::Limits::default(),
            bwrap_path: Some(bwrap_dir.clone().into_os_string()),
        })
        .await
        .is_err()
    );
    assert!(!home.join("addr").exists());
    assert!(!home.join("admin.token").exists());
    for path in [home.clone(), temp.path().join("parent")] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    std::fs::create_dir_all(&bwrap_dir).unwrap();
    let daemon = Daemon::start(DaemonConfig {
        state_dir: home,
        listen: vec!["127.0.0.1:0".parse().unwrap()],
        host_name: None,
        limits: a2amx::messaging::Limits::default(),
        bwrap_path: Some(bwrap_dir.into_os_string()),
    })
    .await
    .unwrap();
    assert_ne!(daemon.addrs()[0].port(), 0);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn ids_are_numerically_ordered_and_not_reused_after_kill() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    for _ in 0..11 {
        create(&mut client, &["cat"], vec![]).await;
    }
    let ids = list(&mut client)
        .await
        .into_iter()
        .map(|s| s.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "s1", "s2", "s3", "s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11"
        ]
    );
    assert_eq!(
        client
            .request(Request::Kill {
                session: "s1".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(create(&mut client, &["cat"], vec![]).await, "s12");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn kill_escalates_ignored_hangup_and_reports_the_signal_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut client,
        &["sh", "-c", "trap '' HUP; printf ready; exec sleep 30"],
        vec![],
    )
    .await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut attachment, &mut emu, "ready").await;
    assert_eq!(
        client
            .request(Request::Kill {
                session: id,
                now: true
            })
            .await
            .unwrap(),
        Response::Ok
    );
    loop {
        match next(&mut attachment).await {
            ServerFrame::Exit(code) => {
                assert_eq!(code, 137);
                break;
            }
            ServerFrame::Data(_) => {}
            frame => panic!("{frame:?}"),
        }
    }
    assert_eq!(list(&mut client).await, vec![]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn cwd_environment_and_argument_boundaries_reach_the_child() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let result = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s|%s|%s|%s|%s' \"$VALUE\" \"$1\" \"$TERM\" \"$COLORTERM\" \"$PWD\""
                    .into(),
                "argv0".into(),
                "literal ; $text".into(),
            ],
            cols: 100,
            rows: 5,
            cwd: Some(dir.path().to_str().unwrap().into()),
            env: vec![
                ("VALUE".into(), "supplied".into()),
                ("TERM".into(), "wrong".into()),
                ("COLORTERM".into(), "wrong".into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        })
        .await
        .unwrap();
    let Response::Created { session: id } = result else {
        panic!("{result:?}");
    };
    eventually_sessions(&mut client, |s| s[0].exit_code == Some(0)).await;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 100, 5)
        .await
        .unwrap();
    let mut emu = Emulator::new(Size { cols: 100, rows: 5 });
    match next(&mut attachment).await {
        ServerFrame::Data(bytes) => {
            emu.feed(&bytes);
        }
        f => panic!("{f:?}"),
    }
    assert!(text(&emu).contains("supplied|literal ; $text|xterm-256color|truecolor|"));
    assert!(text(&emu).contains(dir.path().to_str().unwrap()));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn malformed_stream_frame_disconnects_without_killing_the_session() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let token = std::fs::read_to_string(dir.path().join("admin.token")).unwrap();
    let mut socket = TcpStream::connect(daemon.addrs()[0]).await.unwrap();
    socket
        .write_all(&encode_frame(&serde_json::to_vec(&Request::Hello { token }).unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Ok);
    socket
        .write_all(
            &encode_frame(
                &serde_json::to_vec(&Request::Attach {
                    session: id,
                    force: false,
                    cols: 40,
                    rows: 5,
                    status: false,
                    top_rows: 0,
                })
                .unwrap(),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(raw_response(&mut socket).await, Response::Attached);
    let length = socket.read_u32().await.unwrap();
    let mut initial = vec![0; length as usize];
    socket.read_exact(&mut initial).await.unwrap();
    socket
        .write_all(&encode_frame(&[0xff]).unwrap())
        .await
        .unwrap();
    assert_eq!(
        socket.read_u8().await.unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    eventually_sessions(&mut client, |s| !s[0].attached && s[0].exit_code.is_none()).await;
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn failed_attach_leaves_the_control_connection_usable() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let _attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    assert!(matches!(
        client
            .request(Request::Attach {
                session: id,
                force: false,
                cols: 40,
                rows: 5,
                status: false,
                top_rows: 0,
            })
            .await
            .unwrap(),
        Response::Error { .. }
    ));
    assert_eq!(list(&mut client).await[0].argv, ["cat"]);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn input_backpressure_preserves_bytes_and_keeps_control_requests_responsive() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let out = dir.path().join("input");
    let start = dir.path().join("start");
    let id = create(&mut client,
        &["sh", "-c", "stty raw -echo; printf ready; while [ ! -e \"$START\" ]; do :; done; head -c 8388608 > \"$OUT\""],
        vec![("OUT".into(), out.to_str().unwrap().into()), ("START".into(), start.to_str().unwrap().into())]).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut attached, &mut emulator, "ready").await;
    let sending = tokio::spawn(async move {
        attached
            .send(ClientFrame::Input(vec![0xab; 8 * 1024 * 1024]))
            .await
            .unwrap();
        attached
    });
    let sessions = tokio::time::timeout(Duration::from_secs(2), list(&mut client))
        .await
        .unwrap();
    assert_eq!(sessions[0].exit_code, None);
    std::fs::write(start, b"go").unwrap();
    let mut attached = tokio::time::timeout(Duration::from_secs(10), sending)
        .await
        .unwrap()
        .unwrap();
    loop {
        match next(&mut attached).await {
            ServerFrame::Exit(code) => {
                assert_eq!(code, 0);
                break;
            }
            ServerFrame::Data(_) => {}
            frame => panic!("{frame:?}"),
        }
    }
    let bytes = std::fs::read(out).unwrap();
    assert_eq!(bytes.len(), 8 * 1024 * 1024);
    assert!(bytes.iter().all(|byte| *byte == 0xab));
    daemon.shutdown().await.unwrap();
}

#[test]
fn newly_created_parents_are_private_even_with_a_restrictive_umask() {
    use std::os::unix::process::CommandExt;

    if let Some(path) = std::env::var_os("A2AMX_TEST_PRIVATE_STATE") {
        let path = std::path::PathBuf::from(path);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let bwrap_dir = path.join("bwrap-path");
        let daemon = runtime
            .block_on(Daemon::start(DaemonConfig {
                state_dir: path.clone(),
                listen: vec!["127.0.0.1:0".parse().unwrap()],
                host_name: None,
                limits: a2amx::messaging::Limits::default(),
                bwrap_path: Some(bwrap_dir.clone().into_os_string()),
            }))
            .unwrap();
        std::fs::create_dir_all(&bwrap_dir).unwrap();
        for directory in [path.clone(), path.parent().unwrap().to_owned()] {
            assert_eq!(
                std::fs::metadata(directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        for file in ["admin.token", "addr"] {
            assert_eq!(
                std::fs::metadata(path.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        runtime.block_on(daemon.shutdown()).unwrap();
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "newly_created_parents_are_private_even_with_a_restrictive_umask",
        ])
        .env("A2AMX_TEST_PRIVATE_STATE", root.path().join("parent/state"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    // SAFETY: after fork the child invokes only the non-allocating umask syscall
    // before exec; the parallel parent tests' process-global mask is never changed.
    unsafe {
        command.pre_exec(|| {
            rustix::process::umask(rustix::fs::Mode::all());
            Ok(())
        });
    }
    let child = command.spawn().unwrap();
    let pid = rustix::process::Pid::from_raw(child.id() as i32).unwrap();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });
    let result = receiver.recv_timeout(Duration::from_secs(10));
    if result.is_err() {
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
    }
    let output = result
        .expect("isolated daemon check within deadline")
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[tokio::test]
async fn repeated_force_takeovers_close_each_previous_controller_before_registration() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let id = create(&mut client, &["cat"], vec![]).await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    next(&mut first).await;
    let mut second = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 60, 8)
        .await
        .unwrap();
    assert_eq!(
        next(&mut first).await,
        ServerFrame::Detached("taken over by another attachment".into())
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(10), first.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    next(&mut second).await;
    let mut third = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, true, 30, 6)
        .await
        .unwrap();
    assert_eq!(
        next(&mut second).await,
        ServerFrame::Detached("taken over by another attachment".into())
    );
    assert!(
        tokio::time::timeout(Duration::from_secs(10), second.recv())
            .await
            .unwrap()
            .unwrap()
            .is_none()
    );
    next(&mut third).await;
    third
        .send(ClientFrame::Input(b"last-controller\n".to_vec()))
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size { cols: 30, rows: 6 });
    wait_text(&mut third, &mut emulator, "last-controller").await;
    let sessions = list(&mut client).await;
    assert!(sessions[0].attached);
    assert_eq!((sessions[0].cols, sessions[0].rows), (30, 6));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn large_final_snapshot_is_split_at_the_data_limit_and_precedes_exit() {
    let (dir, daemon) = common::start_daemon().await;
    let mut client = Client::connect(dir.path()).await.unwrap();
    let Response::Created { session: id } = client
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "head -c 131072 /dev/zero | tr '\\0' x".into(),
            ],
            cols: 512,
            rows: 256,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        })
        .await
        .unwrap()
    else {
        panic!("created session");
    };
    eventually_sessions(&mut client, |sessions| sessions[0].exit_code == Some(0)).await;
    let mut attached = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 512, 256)
        .await
        .unwrap();
    let mut emulator = Emulator::new(Size {
        cols: 512,
        rows: 256,
    });
    let mut total = 0;
    loop {
        match next(&mut attached).await {
            ServerFrame::Data(bytes) => {
                assert!(bytes.len() <= 65536);
                total += bytes.len();
                emulator.feed(&bytes);
            }
            ServerFrame::Exit(code) => {
                assert_eq!(code, 0);
                break;
            }
            frame => panic!("{frame:?}"),
        }
    }
    assert!(total > 65536);
    assert!(emulator.screen().cells.iter().all(|cell| cell.ch == 'x'));
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn takeover_waits_for_received_input_to_enqueue_under_backpressure() {
    let (dir, daemon) = common::start_daemon().await;
    let release = dir.path().join("release-input");
    let mut control = Client::connect(dir.path()).await.unwrap();
    let id = create(
        &mut control,
        &[
            "sh",
            "-c",
            "stty raw -echo; printf ready; while [ ! -f \"$RELEASE\" ]; do :; done; cat >/dev/null",
        ],
        vec![("RELEASE".into(), release.to_str().unwrap().into())],
    )
    .await;
    let mut first = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&id, false, 40, 5)
        .await
        .unwrap();
    let mut screen = Emulator::new(Size { cols: 40, rows: 5 });
    wait_text(&mut first, &mut screen, "ready").await;
    let mut input = Box::pin(first.send(ClientFrame::Input(vec![b'x'; 20_000_000])));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), input.as_mut())
            .await
            .is_err(),
        "input should be backpressured while the child does not read"
    );
    let force_client = Client::connect(dir.path()).await.unwrap();
    let mut forced = Box::pin(force_client.attach(&id, true, 60, 8));
    assert!(
        tokio::time::timeout(Duration::from_millis(200), forced.as_mut())
            .await
            .is_err(),
        "takeover must not discard a received input frame waiting for queue space"
    );
    tokio::task::spawn_blocking(move || std::fs::write(release, b"resume"))
        .await
        .unwrap()
        .unwrap();
    let mut second = tokio::time::timeout(Duration::from_secs(10), forced.as_mut())
        .await
        .expect("takeover completes once the child drains input")
        .unwrap();
    drop(input);
    assert!(matches!(next(&mut second).await, ServerFrame::Data(_)));
    assert!(list(&mut control).await[0].attached);
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn opted_in_attachments_receive_initial_and_changed_status_only() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let Response::Created { session: id } = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "stty raw -echo; cat".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agent-review".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
        })
        .await
        .unwrap()
    else {
        panic!("created session");
    };
    let address = list(&mut admin)
        .await
        .into_iter()
        .find(|s| s.id == id)
        .unwrap()
        .address;
    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&id, false, 40, 10, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let info = next(&mut attachment).await;
    let ServerFrame::Status(info) = info else {
        panic!("initial status frame");
    };
    assert_eq!(info.address, address);
    assert_eq!(info.pending, 0);
    assert_eq!(info.hold, None);
    assert_eq!(info.harness, Some(Harness::Generic));
    assert!(info.activity.is_some());
    assert_eq!(info.quota, None);
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .any(|s| s.id == id && s.hold_reason.as_deref() == Some("human_draft"))
    })
    .await;
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(addr, &token).await.unwrap();
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: address.clone(),
                subject: "Status test".into(),
                message: "hello".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(ServerFrame::Status(info)) = attachment.recv().await.unwrap()
                && info.pending == 1
            {
                assert_eq!(info.address, address);
                assert_eq!(info.pending, 1);
                assert_eq!(info.hold, Some("human_draft".into()));
                assert_eq!(info.harness, Some(Harness::Generic));
                assert!(info.activity.is_some());
                assert_eq!(info.quota, None);
                break;
            }
        }
    })
    .await
    .expect("changed status within five seconds");
    // A stable attachment sends no repeated status, and legacy attachments never opt in.
    let other = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let mut legacy = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&other, false, 40, 10)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_millis(2500), async {
        loop {
            tokio::select! {
                frame = legacy.recv() => assert!(!matches!(frame.unwrap(), Some(ServerFrame::Status(_)))),
                frame = attachment.recv() => assert!(!matches!(frame.unwrap(), Some(ServerFrame::Status(_)))),
            }
        }
    }).await.expect_err("observation ends at deadline");
    daemon.shutdown().await.unwrap();
}

#[tokio::test]
async fn team_attachment_status_lists_sorted_members_and_exited_state() {
    use a2amx::harness::{Deliver, Harness};
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let team = TeamScope {
        name: "demo".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let first = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("zeta"),
        Harness::Generic,
        Deliver::Auto,
        Some(team.clone()),
        &[],
        &[],
    )
    .await
    .0;
    let second = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("alpha"),
        Harness::Generic,
        Deliver::Auto,
        Some(team.clone()),
        &[],
        &[],
    )
    .await
    .0;
    let other_team = TeamScope {
        name: "other".into(),
        private: false,
        allow: Vec::new(),
        agents: Vec::new(),
    };
    let other = common::new_team_agent(
        &mut admin,
        dir.path(),
        Some("other"),
        Harness::Generic,
        Deliver::Auto,
        Some(other_team),
        &[],
        &[],
    )
    .await
    .0;
    let ungrouped = create(&mut admin, &["sh", "-c", "sleep 30"], vec![]).await;
    let exited = match admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "exit 0".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            activity: None,
            heartbeat: None,
            name: Some("dead".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Auto),
            team: Some(team),
            role: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        other => panic!("created exited team session: {other:?}"),
    };
    eventually_sessions(&mut admin, |sessions| {
        sessions
            .iter()
            .find(|session| session.id == exited)
            .is_some_and(|session| session.exit_code.is_some())
    })
    .await;

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&first, false, 40, 8, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut attachment).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut attachment).await else {
        panic!("initial team status frame");
    };
    assert_eq!(
        info.members
            .iter()
            .map(|member| member.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "dead", "zeta"]
    );
    assert!(
        info.members
            .iter()
            .all(|member| member.activity.is_some() || member.exited)
    );
    assert!(
        info.members
            .iter()
            .any(|member| member.id == exited && member.exited)
    );
    assert!(
        info.members
            .iter()
            .any(|member| member.id == second && !member.exited)
    );

    assert!(
        !info
            .members
            .iter()
            .any(|member| member.id == other || member.id == ungrouped)
    );

    let mut plain = Client::connect(dir.path())
        .await
        .unwrap()
        .attach_status(&ungrouped, false, 40, 8, 0)
        .await
        .unwrap();
    assert!(matches!(next(&mut plain).await, ServerFrame::Data(_)));
    let ServerFrame::Status(info) = next(&mut plain).await else {
        panic!("initial ungrouped status frame");
    };
    assert!(info.members.is_empty());
    daemon.shutdown().await.unwrap();
}
