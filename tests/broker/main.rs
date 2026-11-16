#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use std::net::SocketAddr;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use a2amx::client::{BridgeLink, Client};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::Limits;
use a2amx::wire::{BridgeDown, BridgeUp, ClientFrame, QuotaWindow, Request, Response};
use tempfile::NamedTempFile;

struct DaemonChild(Option<Child>);

impl DaemonChild {
    fn start(dir: &Path, clock: Option<&Path>) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_a2amx"));
        command
            .arg("--home")
            .arg(dir)
            .args([
                "daemon",
                "start",
                "--foreground",
                "--host-name",
                "host-a",
                "--listen",
                "127.0.0.1:0",
            ])
            .stdout(std::process::Stdio::null());
        if let Some(clock) = clock {
            command.env("LD_PRELOAD", clock);
        }
        Self(Some(command.spawn().expect("daemon child")))
    }

    fn kill(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        self.kill();
    }
}

async fn session(
    admin: &mut Client,
    dir: &Path,
    name: Option<&str>,
    harness: Harness,
    deliver: Deliver,
) -> (String, String, Client) {
    let (id, token, address) = common::new_agent(admin, dir, name, harness, deliver).await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    (id, token, client)
}

async fn screen_session(admin: &mut Client, dir: &Path) -> (String, Client) {
    let output = NamedTempFile::new_in(dir).expect("credential capture path");
    let response = admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; printf '\\n\\n\\n\\n\\n\\n\\n5h 11%% left\\n'; sleep 30".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.path().to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: Some("agent-screen".into()),
            harness: Harness::Claude,
            deliver: Some(Deliver::Hold),
            team: None,
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .expect("create screen session");
    let Response::Created { session } = response else {
        panic!("screen session creation failed");
    };
    let (token, address) = common::eventually(|| async {
        let text = std::fs::read_to_string(output.path()).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let address = lines.next()?.parse::<SocketAddr>().ok()?;
        (token.len() == 64).then_some((token, address))
    })
    .await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    (session, client)
}

fn error_message(response: Response) -> String {
    match response {
        Response::Error { message } => message,
        other => panic!("expected error, got {other:?}"),
    }
}

fn failed_code(response: Response) -> String {
    match response {
        Response::Failed { code, .. } => code,
        other => panic!("expected failed response, got {other:?}"),
    }
}

fn accepted(response: Response, id: &str, recipient_hold: Option<&str>) {
    assert_eq!(
        response,
        Response::Accepted {
            id: id.to_owned(),
            recipient_hold: recipient_hold.map(str::to_owned),
            recipient_quota: None,
        }
    );
}

fn assert_secure_database(dir: &Path) {
    assert_eq!(
        std::fs::metadata(dir.join("state/messages.db"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    for suffix in ["-wal", "-shm"] {
        let path = dir.join(format!("messages.db{suffix}"));
        if path.exists() {
            assert_eq!(
                std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}

async fn send(client: &mut Client, to: &str, subject: &str, message: &str) -> Response {
    client
        .request(Request::SendMessage {
            to: to.to_owned(),
            subject: subject.to_owned(),
            message: message.to_owned(),
        })
        .await
        .unwrap()
}

async fn status(client: &mut Client, id: &str) -> Response {
    client
        .request(Request::MessageStatus { id: id.to_owned() })
        .await
        .unwrap()
}

async fn wait_agents(
    client: &mut Client,
    predicate: impl Fn(&[a2amx::wire::AgentSummary]) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match client.request(Request::ListAgents).await.unwrap() {
                Response::Agents { agents, .. } if predicate(&agents) => return,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("agent state becomes observable");
}

async fn wait_status(
    client: &mut Client,
    id: &str,
    predicate: impl Fn(&a2amx::wire::MessageInfo) -> bool,
) -> a2amx::wire::MessageInfo {
    let id = id.to_owned();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match status(client, &id).await {
                Response::Status { message } if predicate(&message) => return message,
                _ => tokio::task::yield_now().await,
            }
        }
    })
    .await
    .expect("message state becomes observable")
}

async fn omp_reset_target(
    admin: &mut Client,
    dir: &Path,
    daemon_address: SocketAddr,
) -> (String, BridgeLink, SocketAddr, String, String) {
    let (_, controller_token, controller_address) = common::new_agent(
        admin,
        dir,
        Some("controller"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let output = tempfile::NamedTempFile::new_in(dir).unwrap();
    let target = match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; printf '\\033[?2004h'; cat".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.path().to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec!["controller".into()],
            watch: vec![],
            name: Some("target".into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Auto),
            heartbeat: None,
                activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected target response: {response:?}"),
    };
    let target_token = common::eventually(|| async {
        let text = std::fs::read_to_string(output.path()).ok()?;
        let token = text.lines().next()?.to_owned();
        (token.len() == 64).then_some(token)
    })
    .await;
    let mut bridge = Client::connect_addr(daemon_address, &target_token)
        .await
        .unwrap()
        .bridge()
        .await
        .unwrap();
    bridge
        .send(BridgeUp::Hello {
            protocol: 1,
            omp_version: "18.4.5".into(),
            missing: vec![],
        })
        .await
        .unwrap();
    assert!(matches!(
        bridge.recv().await.unwrap(),
        Some(BridgeDown::Ready)
    ));
    (
        target,
        bridge,
        controller_address,
        controller_token,
        target_token,
    )
}

async fn reset_rejection(
    address: SocketAddr,
    token: &str,
    target: &str,
    expected_code: &str,
    source: &str,
) -> String {
    common::eventually(|| async {
        let mut controller = Client::connect_addr(address, token).await.unwrap();
        let response = controller
            .request(Request::Reset {
                session: target.to_owned(),
            })
            .await
            .unwrap();
        match response {
            Response::Failed { code, message }
                if code == expected_code && message.contains(source) =>
            {
                Some(message)
            }
            Response::Failed { .. } => None,
            response => panic!("unexpected reset response: {response:?}"),
        }
    })
    .await
}
async fn reset_success(address: SocketAddr, token: &str, target: &str) -> u32 {
    common::eventually(|| async {
        let mut controller = Client::connect_addr(address, token).await.unwrap();
        match controller
            .request(Request::Reset {
                session: target.to_owned(),
            })
            .await
            .unwrap()
        {
            Response::Reset { steps } => Some(steps),
            Response::Failed { .. } => None,
            response => panic!("unexpected reset response: {response:?}"),
        }
    })
    .await
}

async fn reset_rejection_admin(
    home: &Path,
    target: &str,
    expected_code: &str,
    source: &str,
) -> String {
    let home = home.to_owned();
    let target = target.to_owned();
    let expected_code = expected_code.to_owned();
    let source = source.to_owned();
    common::eventually(|| {
        let home = home.clone();
        let target = target.clone();
        let expected_code = expected_code.clone();
        let source = source.clone();
        async move {
            let mut admin = Client::connect(&home).await.unwrap();
            let response = admin
                .request(Request::Reset { session: target })
                .await
                .unwrap();
            match response {
                Response::Failed { code, message }
                    if code == expected_code && message.contains(&source) =>
                {
                    Some(message)
                }
                Response::Failed { .. } => None,
                response => panic!("unexpected reset response: {response:?}"),
            }
        }
    })
    .await
}

async fn reset_success_admin(home: &Path, target: &str) -> u32 {
    let home = home.to_owned();
    let target = target.to_owned();
    common::eventually(|| {
        let home = home.clone();
        let target = target.clone();
        async move {
            let mut admin = Client::connect(&home).await.unwrap();
            match admin
                .request(Request::Reset { session: target })
                .await
                .unwrap()
            {
                Response::Reset { steps } => Some(steps),
                Response::Failed { .. } => None,
                response => panic!("unexpected reset response: {response:?}"),
            }
        }
    })
    .await
}

async fn wait_claude_ready(home: &Path, target: &str) {
    common::eventually(|| async {
        let mut controller = Client::connect(home).await.ok()?;
        let response = controller
            .request(Request::Explain {
                session: target.to_owned(),
            })
            .await
            .ok()?;
        let Response::Explain { info } = response else {
            return None;
        };
        (info.ready == Some(true)).then_some(())
    })
    .await;
}
async fn wait_claude_hold(home: &Path, target: &str) {
    common::eventually(|| async {
        let mut controller = Client::connect(home).await.ok()?;
        let response = controller
            .request(Request::Explain {
                session: target.to_owned(),
            })
            .await
            .ok()?;
        let Response::Explain { info } = response else {
            return None;
        };
        (info.reset.reason.as_deref() == Some("hold human_draft")).then_some(())
    })
    .await;
}

mod agy;
mod limits_and_lifecycle;
mod quota;
mod reset_basics;
mod reset_diagnostics;
mod reset_in_flight;
mod sessions_and_naming;
mod storage_and_retention;
