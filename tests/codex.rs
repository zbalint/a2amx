#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::confine::{ConfineRequest, Host};
use a2amx::daemon::Daemon;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::Limits;
use a2amx::wire::{MessageInfo, Request, Response, SessionSummary};
use common::fake_codex::{FakeCodex, TurnStart};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

const BODY: &str = "I found the regression in parser.py.";
const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

// Stands in for `codex`: records its arguments and credentials by role, then idles.
const FAKE_CODEX: &str = "#!/bin/sh
case \"$1\" in app-server) role=server;; *) role=tui;; esac
printf '%s\\n' \"$@\" > \"$ARGS_DIR/$role.args\"
printf '%s\\n' \"${A2AMX_TOKEN:-unset}\" > \"$ARGS_DIR/$role.token\"
printf '%s\\n' \"${A2AMX_ROLE:-unset}\" > \"$ARGS_DIR/$role.role\"
printf '%s\\n' \"$(readlink /proc/self/ns/pid)\" > \"$ARGS_DIR/$role.pidns\"
if [ \"$role\" = server ]; then
    target=\"${3#unix://}\"
    if [ -n \"$FAKE_SOCKET\" ]; then
        ln -sf \"$FAKE_SOCKET\" \"$target\"
    elif [ -n \"$FAKE_LINK\" ]; then
        ln -sf \"$FAKE_LINK\" \"$target\"
    elif [ -n \"$FAKE_HASH_LINK\" ]; then
        ln -sf \"${target%/*}/evil\" \"$target\"
        ln -sf \"$FAKE_HASH_LINK\" \"${target%/*}/evil\"
    elif [ -n \"$FAKE_CONFINED\" ]; then
        ln -sf \"${target%/*}/fake\" \"$target\"
    fi
fi
if [ -n \"$FAKE_EXIT\" ]; then sleep \"$FAKE_EXIT\"; exit 0; fi
echo $$ > \"$ARGS_DIR/$role.pid\"
pwd -P > \"$ARGS_DIR/$role.cwd\"
trap 'echo gone > \"$ARGS_DIR/$role.gone\"; exit 0' TERM INT HUP
while :; do sleep 1; done
";

struct Case {
    _dir: TempDir,
    state: PathBuf,
    _daemon: Daemon,
    addr: SocketAddr,
    sender_token: String,
    fake: FakeCodex,
    socket: PathBuf,
    _bin: TempDir,
}

impl Case {
    async fn start() -> Self {
        Self::launch(false, &[], None, |_| {}).await
    }

    async fn start_with(configure: impl FnOnce(&FakeCodex)) -> Self {
        Self::launch(false, &[], None, configure).await
    }

    /// A state dir so deep that its socket path exceeds the unix `SUN_LEN` limit.
    async fn start_deep() -> Self {
        Self::launch(true, &[], None, |fake| {
            fake.set_threads(&[("thread-1", idle())]);
            fake.auto_enter(true);
        })
        .await
    }

    async fn start_env(extra: &[(&str, &str)]) -> Self {
        Self::launch(false, extra, None, |_| {}).await
    }

    async fn start_in(cwd: &std::path::Path) -> Self {
        Self::launch(false, &[], Some(cwd.to_string_lossy().into_owned()), |_| {}).await
    }

    async fn launch(
        deep: bool,
        extra: &[(&str, &str)],
        cwd: Option<String>,
        configure: impl FnOnce(&FakeCodex),
    ) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let mut state = dir.path().to_path_buf();
        if deep {
            for _ in 0..3 {
                state.push("d".repeat(40));
            }
            std::fs::create_dir_all(&state).unwrap();
        }
        let daemon = common::start_daemon_in(&state, Some("host-a"), Limits::default()).await;
        let mut admin = Client::connect(&state).await.unwrap();
        let (_, sender_token, addr) = common::new_agent(
            &mut admin,
            dir.path(),
            Some("agent-plan"),
            Harness::Generic,
            Deliver::Hold,
        )
        .await;
        // Not under the state dir's own `codex/`, where the daemon keeps its sockets.
        let bin = tempfile::tempdir().unwrap();
        let program = bin.path().join("codex");
        std::fs::write(&program, FAKE_CODEX).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let create = {
            let program = program.to_string_lossy().into_owned();
            let args_dir = state.to_string_lossy().into_owned();
            let path = std::env::var("PATH").unwrap_or_default();
            let short = bin.path().join("s.sock");
            let mut env = vec![("ARGS_DIR".into(), args_dir), ("PATH".into(), path)];
            env.extend(extra.iter().map(|(k, v)| ((*k).into(), (*v).into())));
            if deep {
                env.push(("FAKE_SOCKET".into(), short.to_string_lossy().into_owned()));
            }
            async move {
                admin
                    .request(Request::NewSession {
                        argv: vec![program],
                        cols: 40,
                        rows: 10,
                        cwd,
                        env,
                        reset: vec![],
                        control_from: vec![],
                        watch: vec![],
                        name: Some("agent-impl".into()),
                        harness: Harness::Codex,
                        deliver: None,
                        heartbeat: None,
                        activity: None,
                        team: None,
                        role: None,
                        remote: None,
                    })
                    .await
                    .unwrap()
            }
        };
        let mut created = tokio::spawn(create);
        // The daemon removes a stale socket before it starts the server, so bind after
        // the fake server process has run. A creation error would otherwise just hang.
        let args_file = state.join("server.args");
        let listen = tokio::select! {
            listen = common::eventually(|| async { complete(&args_file) }) => listen,
            early = &mut created => panic!("session creation ended first: {:?}", early.unwrap()),
        };
        let socket = PathBuf::from(
            listen
                .lines()
                .nth(2)
                .and_then(|line| line.strip_prefix("unix://"))
                .expect("--listen unix://PATH"),
        );
        let fake = FakeCodex::bind(&if deep {
            bin.path().join("s.sock")
        } else {
            socket.clone()
        });
        configure(&fake);
        let response = created.await.unwrap();
        if !matches!(response, Response::Created { .. }) {
            panic!("codex session creation failed: {response:?}");
        }
        Self {
            _dir: dir,
            state,
            _daemon: daemon,
            addr,
            sender_token,
            fake,
            socket,
            _bin: bin,
        }
    }

    async fn recorded(&self, file: &str) -> Vec<String> {
        let path = self.state.join(file);
        let text = common::eventually(|| async { complete(&path) }).await;
        text.lines().map(str::to_owned).collect()
    }

    async fn send(&self) -> String {
        let mut sender = Client::connect_addr(self.addr, &self.sender_token)
            .await
            .unwrap();
        let response = sender
            .request(Request::SendMessage {
                to: "agent-impl@host-a".into(),
                subject: "Parser issue".into(),
                message: BODY.into(),
            })
            .await
            .unwrap();
        let Response::Accepted { id, .. } = response else {
            panic!("message was not accepted");
        };
        id
    }

    async fn status(&self, id: &str) -> MessageInfo {
        let mut admin = Client::connect(&self.state).await.unwrap();
        let Response::Status { message } = admin
            .request(Request::MessageStatus { id: id.into() })
            .await
            .unwrap()
        else {
            panic!("message status missing");
        };
        message
    }

    async fn wait_state(&self, id: &str, wanted: &str) -> MessageInfo {
        common::eventually(|| async {
            let message = self.status(id).await;
            (message.state == wanted).then_some(message)
        })
        .await
    }

    async fn summary(&self) -> SessionSummary {
        let mut admin = Client::connect(&self.state).await.unwrap();
        let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
            panic!("session list missing");
        };
        sessions
            .into_iter()
            .find(|session| session.harness == Harness::Codex)
            .expect("Codex session summary")
    }

    async fn wait_reason(&self, id: &str, wanted: &str) -> MessageInfo {
        common::eventually(|| async {
            let message = self.status(id).await;
            (message.hold_reason.as_deref() == Some(wanted)).then_some(message)
        })
        .await
    }
}

// The script writes its records with redirection, so a file can be read half-written.
fn complete(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .filter(|text| text.ends_with('\n'))
}

fn idle() -> Value {
    json!({"type": "idle"})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_uses_private_socket_dir_and_delivers() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let (_, sender_token, sender_addr) = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let cwd = work.path().join("workspace");
    std::fs::create_dir_all(&cwd).unwrap();
    let program = cwd.join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let request = Request::Confine {
        confine: ConfineRequest::default(),
        request: Box::new(Request::NewSession {
            argv: vec![program.to_string_lossy().into_owned()],
            cols: 40,
            rows: 10,
            cwd: Some(cwd.to_string_lossy().into_owned()),
            env: vec![
                ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                ("FAKE_CONFINED".into(), "1".into()),
                ("A2AMX_NO_AUTHORIZE_PEERS".into(), "1".into()),
            ],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("agent-impl".into()),
            harness: Harness::Codex,
            deliver: None,
            team: None,
            role: None,
            remote: None,
        }),
    };
    let socket_dir = state_dir.path().join("codex/s2");
    std::fs::create_dir_all(&socket_dir).unwrap();
    symlink("/tmp", socket_dir.join("stale")).unwrap();
    let created = tokio::spawn(async move { admin.request(request).await.unwrap() });
    let app_link = socket_dir.join("app.sock");
    common::eventually(|| async {
        std::fs::symlink_metadata(&app_link)
            .ok()
            .filter(|metadata| metadata.file_type().is_symlink())
            .map(|_| ())
    })
    .await;
    assert!(std::fs::symlink_metadata(socket_dir.join("stale")).is_err());
    assert_eq!(
        std::fs::symlink_metadata(&socket_dir)
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let fake = FakeCodex::bind(&socket_dir.join("fake"));
    fake.set_threads(&[("thread-1", idle())]);
    fake.auto_enter(true);
    let response = created.await.unwrap();
    assert!(matches!(response, Response::Created { .. }), "{response:?}");
    let server_args = common::eventually(|| async {
        complete(&cwd.join("server.args"))
            .map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
    })
    .await;
    let instructions: Vec<_> = server_args
        .iter()
        .filter(|argument| argument.starts_with("developer_instructions="))
        .collect();
    assert_eq!(instructions.len(), 1);
    let notice: String = serde_json::from_str(
        instructions[0]
            .strip_prefix("developer_instructions=")
            .expect("developer instructions prefix"),
    )
    .expect("JSON notice");
    assert!(notice.starts_with("This session is confined.\n"));
    assert!(!notice.contains(a2amx::harness::PEER_AUTHORIZATION_PROMPT));
    let expected_notice = a2amx::confine::notice(
        &a2amx::confine::plan(
            &ConfineRequest::default(),
            Harness::Codex,
            &cwd,
            &Host {
                home: PathBuf::from(std::env::var_os("HOME").expect("HOME")),
                state_dir: state_dir.path().to_path_buf(),
                runtime_dir: PathBuf::from(
                    std::env::var_os("XDG_RUNTIME_DIR").expect("XDG_RUNTIME_DIR"),
                ),
                empty_file: state_dir.path().join("confine-empty"),
            },
        )
        .expect("expected Codex plan"),
    );
    assert_eq!(notice, expected_notice);
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("server.cwd")) })
            .await
            .trim(),
        cwd.to_string_lossy()
    );
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("tui.cwd")) })
            .await
            .trim(),
        cwd.to_string_lossy()
    );
    let guest_socket = format!(
        "unix:///tmp/codex-daemon-{}/app.sock",
        rustix::process::geteuid().as_raw()
    );
    assert_eq!(
        common::eventually(|| async { complete(&cwd.join("tui.args")) }).await,
        format!("--remote\n{guest_socket}\n")
    );
    let host_pidns = std::fs::read_link("/proc/self/ns/pid")
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_ne!(
        common::eventually(|| async { complete(&cwd.join("server.pidns")) })
            .await
            .trim(),
        host_pidns
    );
    assert_ne!(
        common::eventually(|| async { complete(&cwd.join("tui.pidns")) })
            .await
            .trim(),
        host_pidns
    );
    let mut sender = Client::connect_addr(sender_addr, &sender_token)
        .await
        .unwrap();
    let Response::Accepted { id, .. } = sender
        .request(Request::SendMessage {
            to: "agent-impl@host-a".into(),
            subject: "Parser issue".into(),
            message: BODY.into(),
        })
        .await
        .unwrap()
    else {
        panic!("message was not accepted");
    };
    let submitted = common::eventually(|| async {
        let response = Client::connect(state_dir.path())
            .await
            .unwrap()
            .request(Request::MessageStatus { id: id.clone() })
            .await
            .unwrap();
        match response {
            Response::Status { message } if message.state == "submitted" => Some(message),
            _ => None,
        }
    })
    .await;
    assert_eq!(submitted.state, "submitted");
    common::eventually(|| async {
        fake.turn_starts()
            .first()
            .filter(|turn| turn.text.contains(BODY))
            .map(|_| ())
    })
    .await;
    let server_ns = common::eventually(|| async { complete(&cwd.join("server.pidns")) }).await;
    let tui_ns = common::eventually(|| async { complete(&cwd.join("tui.pidns")) }).await;
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    assert!(matches!(
        admin
            .request(Request::Kill {
                session: "s2".into(),
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    ));
    common::eventually(|| async {
        let response = Client::connect(state_dir.path())
            .await
            .unwrap()
            .request(Request::List)
            .await
            .unwrap();
        match response {
            Response::Sessions { sessions }
                if !sessions
                    .iter()
                    .any(|session| session.harness == Harness::Codex) =>
            {
                Some(())
            }
            _ => None,
        }
    })
    .await;
    common::eventually(|| async {
        let alive = std::fs::read_dir("/proc")
            .ok()
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.bytes().all(|byte| byte.is_ascii_digit()))
            .any(|pid| {
                std::fs::read_link(format!("/proc/{pid}/ns/pid"))
                    .ok()
                    .is_some_and(|namespace| {
                        namespace.to_string_lossy().trim() == server_ns.trim()
                            || namespace.to_string_lossy().trim() == tui_ns.trim()
                    })
            });
        (!alive).then_some(())
    })
    .await;
    drop(fake);
    drop(daemon);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_plan_failure_does_not_start_app_server() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let work = tempfile::tempdir().unwrap();
    let marker = work.path().join("started");
    let fake = work.path().join("codex");
    std::fs::write(&fake, "#!/bin/sh\nprintf started > \"$MARKER\"\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let fake_path = fake.to_string_lossy().into_owned();
    let marker_path = marker.to_string_lossy().into_owned();
    let response = admin
        .request(Request::Confine {
            confine: ConfineRequest {
                write: vec!["relative".into()],
                ..ConfineRequest::default()
            },
            request: Box::new(Request::NewSession {
                argv: vec![fake_path],
                cols: 40,
                rows: 10,
                cwd: Some(work.path().to_string_lossy().into_owned()),
                env: vec![("MARKER".into(), marker_path)],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some("plan-refused".into()),
                harness: Harness::Codex,
                deliver: Some(Deliver::Hold),
                team: None,
                role: None,
                remote: None,
            }),
        })
        .await;
    assert!(response.is_ok(), "daemon request failed: {response:?}");
    let response = response.unwrap();
    let Response::Error { message } = response else {
        panic!("expected confinement plan refusal: {response:?}");
    };
    assert!(message.contains("relative"), "{message}");
    assert!(
        !marker.exists(),
        "Codex app-server launch marker was created"
    );
    assert!(
        !state_dir.path().join("codex").exists(),
        "Codex app-server state was created despite plan refusal"
    );
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn confined_codex_rejects_hostile_socket_links() {
    let Some((state_dir, daemon)) = common::start_confined_daemon().await else {
        assert!(
            std::env::var_os("A2AMX_REQUIRE_BWRAP").is_none(),
            "bubblewrap is required for this test"
        );
        return;
    };
    let mut admin = Client::connect(state_dir.path()).await.unwrap();
    let _ = common::new_agent(
        &mut admin,
        state_dir.path(),
        Some("sender"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let work = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let outside_socket = outside.path().join("outside.sock");
    let listener = std::os::unix::net::UnixListener::bind(&outside_socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let guest_prefix = format!("/tmp/codex-daemon-{}", rustix::process::geteuid().as_raw());
    for (index, (key, value)) in [
        ("FAKE_LINK", format!("{guest_prefix}/../../outside")),
        (
            "FAKE_HASH_LINK",
            outside_socket.to_string_lossy().into_owned(),
        ),
        ("FAKE_LINK", format!("{guest_prefix}/{}", "x".repeat(81))),
        ("FAKE_LINK", "/tmp/elsewhere/fake".into()),
    ]
    .into_iter()
    .enumerate()
    {
        let cwd = work.path().join(format!("case-{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        let program = cwd.join("codex");
        std::fs::write(&program, FAKE_CODEX).unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let request = Request::Confine {
            confine: ConfineRequest::default(),
            request: Box::new(Request::NewSession {
                argv: vec![program.to_string_lossy().into_owned()],
                cols: 40,
                rows: 10,
                cwd: Some(cwd.to_string_lossy().into_owned()),
                env: vec![
                    ("ARGS_DIR".into(), cwd.to_string_lossy().into_owned()),
                    ("PATH".into(), std::env::var("PATH").unwrap_or_default()),
                    (key.into(), value),
                    (
                        "FAKE_EXIT".into(),
                        if index == 2 { "5".into() } else { "1".into() },
                    ),
                ],
                reset: vec![],
                control_from: vec![],
                watch: vec![],
                activity: None,
                heartbeat: None,
                name: Some(format!("hostile-{index}")),
                harness: Harness::Codex,
                deliver: None,
                team: None,
                role: None,
                remote: None,
            }),
        };
        let mut request_client = Client::connect(state_dir.path()).await.unwrap();
        let mut fake_server: Option<FakeCodex> = None;
        let response = if index == 2 {
            let existing_dirs = std::fs::read_dir(state_dir.path().join("codex"))
                .into_iter()
                .flatten()
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            let pending = tokio::spawn(async move { request_client.request(request).await });
            let socket_dir = common::eventually(|| async {
                std::fs::read_dir(state_dir.path().join("codex"))
                    .ok()?
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        if existing_dirs.contains(&path) {
                            return None;
                        }
                        let app_link = path.join("app.sock");
                        std::fs::symlink_metadata(&app_link)
                            .ok()
                            .filter(|metadata| metadata.file_type().is_symlink())
                            .map(|_| path)
                    })
                    .max()
            })
            .await;
            fake_server = Some(FakeCodex::bind(&socket_dir.join("x".repeat(81))));
            tokio::time::timeout(Duration::from_secs(8), pending)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        } else {
            tokio::time::timeout(Duration::from_secs(8), request_client.request(request))
                .await
                .unwrap()
                .unwrap()
        };
        let Response::Error { message } = response else {
            panic!("hostile socket link was accepted: {response:?}");
        };
        assert!(message.contains("unexpected socket link"), "{message}");
        if index == 2 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            assert_eq!(
                fake_server
                    .as_ref()
                    .expect("long-name fake listener")
                    .connections(),
                0
            );
        }
    }
    assert!(listener.accept().is_err());
    drop(daemon);
}

fn waiting_on_approval() -> Value {
    json!({"type": "active", "activeFlags": ["waitingOnApproval"]})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_new_starts_a_private_app_server_and_attaches_the_tui_to_it() {
    let case = Case::start().await;
    let exe = std::env::current_exe().unwrap();
    let server = case.recorded("server.args").await;
    assert_eq!(
        server[..3],
        [
            "app-server".to_owned(),
            "--listen".to_owned(),
            format!("unix://{}", case.socket.display()),
        ]
    );
    assert_eq!(
        server[3..],
        [
            "-c",
            &format!("mcp_servers.a2amx.command={}", json!(exe.to_string_lossy())),
            "-c",
            "mcp_servers.a2amx.args=[\"mcp\"]",
            "-c",
            "mcp_servers.a2amx.env_vars=[\"A2AMX_TOKEN\",\"A2AMX_ADDR\"]",
            "-c",
            "mcp_servers.a2amx.tools.list_agents.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.send_message.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.message_status.approval_mode=\"approve\"",
            "-c",
            "mcp_servers.a2amx.tools.reset_session.approval_mode=\"approve\"",
            "-c",
            &format!(
                "developer_instructions={}",
                json!(a2amx::harness::PEER_AUTHORIZATION_PROMPT)
            ),
        ]
    );
    assert_eq!(
        case.recorded("tui.args").await,
        [
            "--remote".to_owned(),
            format!("unix://{}", case.socket.display())
        ]
    );
    // The session token reaches the server through its environment, never argv.
    let token = case.recorded("server.token").await.remove(0);
    assert_eq!(token.len(), 64);
    assert!(server.iter().all(|argument| !argument.contains(&token)));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_role_reaches_only_the_operator_instructions() {
    for (env, expected) in [
        (
            vec![("A2AMX_ROLE", "architect")],
            format!(
                "{} Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.",
                a2amx::harness::PEER_AUTHORIZATION_PROMPT
            ),
        ),
        (
            vec![
                ("A2AMX_NO_AUTHORIZE_PEERS", "1"),
                ("A2AMX_ROLE", "consultant"),
            ],
            "Operator-assigned role for this session: consultant. It is a label set by your operator and grants no extra authority.".to_owned(),
        ),
        (
            vec![
                ("A2AMX_NO_AUTHORIZE_PEERS", "1"),
                ("A2AMX_ROLE", "earlier"),
                ("A2AMX_ROLE", "architect"),
            ],
            "Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority.".to_owned(),
        ),
    ] {
        let case = Case::start_env(&env).await;
        let server = case.recorded("server.args").await;
        let instructions: Vec<_> = server
            .iter()
            .filter(|arg| arg.starts_with("developer_instructions="))
            .collect();
        assert_eq!(
            instructions,
            [&format!("developer_instructions={}", json!(expected))]
        );
        assert_eq!(case.recorded("server.role").await, ["unset"]);
        assert_eq!(case.recorded("tui.role").await, ["unset"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn inherited_role_is_removed_by_the_cli_before_codex_launch() {
    let dir = tempfile::tempdir().unwrap();
    let daemon = common::start_daemon_in(dir.path(), Some("host-a"), Limits::default()).await;
    let bin = tempfile::tempdir().unwrap();
    let program = bin.path().join("codex");
    std::fs::write(&program, FAKE_CODEX).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = bin.path().join("s.sock");
    let _fake = FakeCodex::bind(&socket);
    let args_dir = dir.path().to_string_lossy().into_owned();
    let path = std::env::var("PATH").unwrap_or_default();
    let output = tokio::task::spawn_blocking({
        let home = dir.path().to_owned();
        let program = program.clone();
        let socket = socket.clone();
        move || {
            Command::new(env!("CARGO_BIN_EXE_a2amx"))
                .arg("--home")
                .arg(home)
                .args(["new", "--detach", "--harness", "codex", "--"])
                .arg(program)
                .env("ARGS_DIR", &args_dir)
                .env("PATH", path)
                .env("FAKE_SOCKET", socket)
                .env("A2AMX_ROLE", "leak")
                .env_remove("A2AMX_BIN")
                .output()
        }
    })
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let server = common::eventually(|| async {
        let path = dir.path().join("server.args");
        complete(&path).map(|text| text.lines().map(str::to_owned).collect::<Vec<_>>())
    })
    .await;
    assert!(server.iter().all(
        |argument| !argument.contains("developer_instructions=") || !argument.contains("leak")
    ));
    assert_eq!(
        common::eventually(|| async { complete(&dir.path().join("server.role")) })
            .await
            .trim(),
        "unset"
    );
    assert_eq!(
        common::eventually(|| async { complete(&dir.path().join("tui.role")) })
            .await
            .trim(),
        "unset"
    );
    drop(daemon);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_message_reaches_the_codex_thread_and_the_thread_item_becomes_the_receipt() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    let info = case.wait_state(&id, "submitted").await;
    assert_eq!(
        common::eventually(|| async {
            let info = case.status(&id).await;
            (info.evidence.as_deref() == Some("native_receipt")).then_some(info.evidence)
        })
        .await,
        Some("native_receipt".to_owned())
    );
    assert_eq!(info.state, "submitted");
    assert_eq!(
        case.fake.turn_starts(),
        [TurnStart {
            thread: "thread-1".into(),
            client_id: "m_1".into(),
            text: ENVELOPE_TEXT.into(),
        }]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_loaded_thread_the_message_waits() {
    let case = Case::start().await;
    let id = case.send().await;
    let info = case.wait_reason(&id, "no_thread").await;
    assert_eq!(info.state, "pending");
    assert!(case.fake.turn_starts().is_empty());
    tokio::time::sleep(std::time::Duration::from_millis(10_200)).await;
    let held = common::eventually(|| async {
        let summary = case.summary().await;
        (summary.hold_reason.as_deref() == Some("needs_input") && summary.held).then_some(summary)
    })
    .await;
    assert_eq!(held.hold_reason.as_deref(), Some("needs_input"));
    case.fake.set_threads(&[("thread-1", idle())]);
    case.fake.auto_enter(true);
    common::eventually(|| async {
        let summary = case.summary().await;
        (summary.hold_reason.is_none() && !summary.held).then_some(())
    })
    .await;
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_open_approval_dialog_holds_delivery_until_it_closes() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", waiting_on_approval())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    let info = case.wait_reason(&id, "waiting_on_approval").await;
    assert_eq!(info.state, "pending");
    assert!(case.fake.turn_starts().is_empty());
    case.fake.set_status("thread-1", idle());
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_most_recently_updated_thread_receives_the_message() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("old-thread", idle()), ("new-thread", idle())]);
        fake.auto_enter(true);
    })
    .await;
    let id = case.send().await;
    case.wait_state(&id, "submitted").await;
    assert_eq!(case.fake.turn_starts()[0].thread, "new-thread");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_second_message_waits_until_the_first_shows_up_in_the_thread() {
    let case = Case::start_with(|fake| fake.set_threads(&[("thread-1", idle())])).await;
    let first = case.send().await;
    case.wait_state(&first, "submitted").await;
    let second = case.send().await;
    let info = case.wait_reason(&second, "in_flight").await;
    assert_eq!(info.state, "pending");
    assert_eq!(case.fake.turn_starts().len(), 1);
    case.fake.enter("thread-1", "m_1", ENVELOPE_TEXT);
    case.wait_state(&second, "submitted").await;
    let starts = case.fake.turn_starts();
    assert_eq!(starts.len(), 2);
    assert_eq!(starts[1].client_id, "m_2");
    assert_eq!(
        case.status(&first).await.evidence.as_deref(),
        Some("native_receipt")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_thread_item_with_different_text_is_no_receipt() {
    let case = Case::start_with(|fake| fake.set_threads(&[("thread-1", idle())])).await;
    let first = case.send().await;
    case.wait_state(&first, "submitted").await;
    let second = case.send().await;
    case.fake.enter("thread-1", "m_1", "tampered text");
    case.wait_state(&second, "submitted").await;
    assert_eq!(
        case.status(&first).await.evidence.as_deref(),
        Some("write_complete")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_turn_makes_the_message_undeliverable() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.reject_turns("thread is shutting down");
    })
    .await;
    let id = case.send().await;
    let info = case.wait_state(&id, "undeliverable").await;
    assert_eq!(info.detail.as_deref(), Some("thread is shutting down"));
    assert_eq!(case.fake.turn_starts().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_socket_path_beyond_sun_len_is_reached_through_the_link_the_server_makes() {
    let case = Case::start_deep().await;
    assert!(case.socket.as_os_str().len() > 108);
    let id = case.send().await;
    case.wait_state(&id, "submitted").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_authorize_peers_leaves_the_operator_line_out() {
    let case = Case::start_env(&[("A2AMX_NO_AUTHORIZE_PEERS", "1")]).await;
    let server = case.recorded("server.args").await;
    assert!(
        server
            .iter()
            .all(|a| !a.starts_with("developer_instructions="))
    );
    assert!(
        server
            .iter()
            .any(|a| a.starts_with("mcp_servers.a2amx.command="))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dead_app_server_makes_the_message_wait() {
    let case = Case::start_with(|fake| {
        fake.set_threads(&[("thread-1", idle())]);
        fake.auto_enter(true);
    })
    .await;
    case.fake.stop();
    let id = case.send().await;
    let info = case.wait_reason(&id, "app_server_down").await;
    assert_eq!(info.state, "pending");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn killing_the_session_stops_its_app_server() {
    let case = Case::start().await;
    let pid = case.recorded("server.pid").await.remove(0);
    let alive = || std::path::Path::new(&format!("/proc/{pid}")).exists();
    assert!(alive());
    let mut admin = Client::connect(&case.state).await.unwrap();
    let Response::Ok = admin
        .request(Request::Kill {
            session: "s2".into(),
            now: true,
        })
        .await
        .unwrap()
    else {
        panic!("kill failed");
    };
    common::eventually(|| async { (!alive()).then_some(()) }).await;
}

#[tokio::test]
async fn app_server_and_tui_start_in_the_requested_directory() {
    let work = tempfile::tempdir().unwrap();
    let expected = work
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let case = Case::start_in(work.path()).await;
    assert_eq!(case.recorded("server.cwd").await, vec![expected.clone()]);
    assert_eq!(case.recorded("tui.cwd").await, vec![expected]);
}
