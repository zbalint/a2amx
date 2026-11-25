#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
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
// Stands in for `codex`: records its arguments and credentials by role, then idles.
const FAKE_CODEX: &str = "#!/bin/sh
case \"$1\" in app-server) role=server;; *) role=tui;; esac
printf '%s\\n' \"$@\" > \"$ARGS_DIR/$role.args\"
printf '%s\\n' \"${A2AMX_TOKEN:-unset}\" > \"$ARGS_DIR/$role.token\"
printf '%s\\n' \"${A2AMX_ROLE:-unset}\" > \"$ARGS_DIR/$role.role\"
printf '%s\\n' \"${A2AMX_SESSION:-unset}\" > \"$ARGS_DIR/$role.session\"
printf '%s\\n' \"$(readlink /proc/self/ns/pid)\" > \"$ARGS_DIR/$role.pidns\"
if [ \"$role\" = server ]; then
    if [ -n \"$FAKE_GUEST_MUTATION\" ]; then
        if [ -z \"$FAKE_SESSION_DIR\" ] || [ ! -d \"$FAKE_SESSION_DIR\" ]; then
            echo \"invalid-session-dir\" > \"$ARGS_DIR/guest-mutation\"
        else
            {
                echo \"parent:\"
                find \"${FAKE_SESSION_DIR%/*}\" -mindepth 1 -maxdepth 1 -printf '%f\\n' | sort
                echo \"codex:\"
                find \"${FAKE_SESSION_DIR%/*}\" -mindepth 1 -maxdepth 1 -printf '%f\\n' | sort
                if mv \"$FAKE_SESSION_DIR/sock\" \"$FAKE_SESSION_DIR/sock2\"; then echo \"mv=ok\"; else echo \"mv=fail\"; fi
                if ln -s /etc \"$FAKE_SESSION_DIR/x\"; then echo \"ln=ok\"; else echo \"ln=fail\"; fi
                if touch \"$FAKE_SESSION_DIR/y\"; then echo \"touch=ok\"; else echo \"touch=fail\"; fi
            } > \"$ARGS_DIR/guest-mutation\"
        fi
    fi
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
            let role = extra
                .iter()
                .rev()
                .find_map(|(k, v)| (*k == "A2AMX_ROLE").then(|| (*v).to_owned()));
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
                        role,
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

mod app_server_launch;
mod confined_socket;
mod delivery;
