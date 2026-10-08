#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use a2amx::client::Client;
use a2amx::daemon::Daemon;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::Limits;
use a2amx::wire::{MessageInfo, Request, Response, SessionSummary};
use common::fake_codex::{FakeCodex, TurnStart};
use serde_json::{Value, json};
use tempfile::TempDir;

const BODY: &str = "I found the regression in parser.py.";
const ENVELOPE_TEXT: &str = "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>";

// Stands in for `codex`: records its arguments and credentials by role, then idles.
const FAKE_CODEX: &str = "#!/bin/sh
case \"$1\" in app-server) role=server;; *) role=tui;; esac
printf '%s\\n' \"$@\" > \"$ARGS_DIR/$role.args\"
printf '%s\\n' \"${A2AMX_TOKEN:-unset}\" > \"$ARGS_DIR/$role.token\"
printf '%s\\n' \"${A2AMX_ROLE:-unset}\" > \"$ARGS_DIR/$role.role\"
# Like Codex, answer on a short real socket and link the requested long path to it.
if [ \"$role\" = server ] && [ -n \"$FAKE_SOCKET\" ]; then ln -sf \"$FAKE_SOCKET\" \"${3#unix://}\"; fi
echo $$ > \"$ARGS_DIR/$role.pid\"
pwd -P > \"$ARGS_DIR/$role.cwd\"
exec sleep 60
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
                        team: None,
                        role: None,
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
        let Response::Created { .. } = created.await.unwrap() else {
            panic!("codex session creation failed");
        };
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
