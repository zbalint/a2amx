#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::TeamScope;
use a2amx::wire::{Request, Response};
use common::pty::PtyHarness;
use std::path::Path;
use std::process::{Command, Output};
use std::time::{Duration, Instant};

fn team_file(workspace: &Path) -> std::path::PathBuf {
    std::fs::create_dir_all(workspace.join("config")).unwrap();
    workspace.join("config/team.toml")
}
fn team_up_pty_args(home: &Path, team_file: &Path, detach: bool) -> Vec<String> {
    let mut args = vec![
        "--home".to_owned(),
        home.to_string_lossy().into_owned(),
        "team".to_owned(),
        "up".to_owned(),
    ];
    if detach {
        args.push("--detach".to_owned());
    }
    args.extend([
        "--file".to_owned(),
        team_file.to_string_lossy().into_owned(),
    ]);
    args
}

fn team_up_pty(
    home: &Path,
    workspace: &Path,
    user_home: &Path,
    runtime: &Path,
    team_file: &Path,
    detach: bool,
) -> anyhow::Result<PtyHarness> {
    PtyHarness::spawn_with_env_in(
        &team_up_pty_args(home, team_file, detach),
        user_home,
        workspace,
        &[
            (
                "HOME",
                user_home.to_str().expect("home path is valid UTF-8"),
            ),
            (
                "XDG_RUNTIME_DIR",
                runtime.to_str().expect("runtime path is valid UTF-8"),
            ),
        ],
    )
}

async fn assert_waiting_before_enter(pty: &mut PtyHarness, settle: Duration) -> anyhow::Result<()> {
    let deadline = Instant::now() + settle;
    while Instant::now() < deadline {
        pty.expect_screen_lacks("SESSION-MARK-9f3a")?;
        assert!(
            pty.wait_exit(Duration::from_millis(1)).is_err(),
            "team up exited before Enter"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

fn scope(name: &str, private: bool, allow: &[&str]) -> TeamScope {
    scope_with_agents(name, private, allow, &[])
}

fn scope_with_agents(name: &str, private: bool, allow: &[&str], agents: &[&str]) -> TeamScope {
    TeamScope {
        name: name.into(),
        private,
        allow: allow.iter().map(|entry| (*entry).to_owned()).collect(),
        agents: agents.iter().map(|entry| (*entry).to_owned()).collect(),
    }
}

fn run_binary(home: &Path, cwd: &Path, user_home: &Path, runtime: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime)
        .output()
        .expect("run a2amx")
}

async fn gateway(dir: &Path, address: std::net::SocketAddr) -> Client {
    let token = std::fs::read_to_string(dir.join("run/gateway.token")).expect("gateway token");
    Client::connect_addr(address, token.trim())
        .await
        .expect("gateway connection")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_validates_remote_before_start_and_carries_it_to_the_session() {
    let (dir, daemon) = common::start_daemon().await;
    let workspace = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();
    std::fs::write(
        team_file(workspace.path()),
        r#"
        [[session]]
        name = "plain"
        command = ["sleep", "30"]
        [[session]]
        name = "remote"
        command = ["sleep", "30"]
        remote = "both"
        "#,
    )
    .unwrap();
    let refused = run_binary(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    );
    assert!(!refused.status.success());
    assert!(
        String::from_utf8_lossy(&refused.stderr)
            .contains("remote messaging needs a confined session")
    );
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("expected session listing");
    };
    assert!(
        sessions.is_empty(),
        "team up started sessions before validation"
    );
    daemon.shutdown().await.unwrap();

    let (dir, daemon) = common::start_daemon().await;
    let Some(bwrap) = a2amx::confine::resolve_bwrap(None).ok() else {
        daemon.shutdown().await.unwrap();
        return;
    };
    std::os::unix::fs::symlink(bwrap, dir.path().join("bwrap-path/bwrap")).unwrap();
    std::fs::write(
        team_file(workspace.path()),
        r#"
        team = "team-a"
        [[session]]
        name = "architect"
        command = ["sleep", "30"]
        cwd = "."
        remote = "both"
        [session.confine]
        enabled = true
        [[session]]
        name = "developer"
        command = ["sleep", "30"]
        cwd = "."
        remote = "send"
        [session.confine]
        enabled = true
        "#,
    )
    .unwrap();
    let started = run_binary(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    );
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let stdout = String::from_utf8_lossy(&started.stdout);
    assert!(
        stdout.contains("remote messaging for: team-a-architect (both), team-a-developer (send)")
    );
    let code = stdout
        .lines()
        .find_map(|line| line.strip_prefix("post this code in the control channel within "))
        .and_then(|line| line.rsplit_once(": "))
        .map(|(_, code)| code.to_owned())
        .expect("team up prints lease code");
    let mut gateway = gateway(dir.path(), daemon.addrs()[0]).await;
    assert!(matches!(
        gateway
            .request(Request::RedeemLeaseCode { code })
            .await
            .unwrap(),
        Response::Ok
    ));
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "team-a-architect@host-a".into(),
                body: "message".into(),
            })
            .await
            .unwrap(),
        Response::Accepted { .. }
    ));
    assert!(matches!(
        gateway
            .request(Request::HumanInbound {
                to: "team-a-developer@host-a".into(),
                body: "message".into(),
            })
            .await
            .unwrap(),
        Response::Failed { message, .. }
            if message == "remote messages to this session are not enabled"
    ));
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_remote_lease_output_uses_derived_team_and_not_dry_run() {
    let (dir, daemon) = common::start_daemon().await;
    let Some(bwrap) = a2amx::confine::resolve_bwrap(None).ok() else {
        daemon.shutdown().await.unwrap();
        return;
    };
    std::os::unix::fs::symlink(bwrap, dir.path().join("bwrap-path/bwrap")).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let user_home = tempfile::tempdir().unwrap();
    let runtime = tempfile::tempdir().unwrap();

    std::fs::write(
        team_file(workspace.path()),
        r#"
        team = "quiet"
        [[session]]
        name = "plain"
        command = ["sleep", "30"]
        "#,
    )
    .unwrap();
    let plain = run_binary(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    );
    assert!(plain.status.success());
    let plain_output = format!(
        "{}{}",
        String::from_utf8_lossy(&plain.stdout),
        String::from_utf8_lossy(&plain.stderr)
    );
    assert!(!plain_output.contains("remote messaging"));

    std::fs::write(
        team_file(workspace.path()),
        r#"
        team = "dry-run"
        [[session]]
        name = "remote"
        command = ["sleep", "30"]
        remote = "both"
        [session.confine]
        enabled = true
        "#,
    )
    .unwrap();
    let dry = run_binary(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run", "--file", "config/team.toml"],
    );
    assert!(dry.status.success());
    let dry_output = format!(
        "{}{}",
        String::from_utf8_lossy(&dry.stdout),
        String::from_utf8_lossy(&dry.stderr)
    );
    assert!(!dry_output.contains("post this code in the control channel"));

    std::fs::write(
        team_file(workspace.path()),
        r#"
        [[session]]
        name = "unscoped-remote"
        command = ["sleep", "30"]
        remote = "both"
        [session.confine]
        enabled = true
        "#,
    )
    .unwrap();
    let unscoped = run_binary(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "config/team.toml"],
    );
    let unscoped_output = format!(
        "{}{}",
        String::from_utf8_lossy(&unscoped.stdout),
        String::from_utf8_lossy(&unscoped.stderr)
    );
    let prefix = "post this code in the control channel within 10m: ";
    let lease_lines: Vec<_> = unscoped_output
        .lines()
        .filter(|line| line.starts_with(prefix))
        .collect();
    assert_eq!(lease_lines.len(), 1, "{unscoped_output}");
    let code = &lease_lines[0][prefix.len()..];
    assert_eq!(code.len(), 12, "{unscoped_output}");
    assert!(
        code.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{unscoped_output}"
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_remote_waits_before_attaching() -> anyhow::Result<()> {
    let Some((dir, daemon)) = common::start_confined_daemon().await else {
        return Ok(());
    };
    let workspace = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let team_file = team_file(workspace.path());
    std::fs::write(
        &team_file,
        r#"
team = "team-a"
[[session]]
name = "remote"
command = ["sh", "-c", "printf SESSION-MARK-9f3a; sleep 30"]
attach = true
remote = "send"
[session.confine]
enabled = true
"#,
    )?;

    let mut pty = team_up_pty(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &team_file,
        false,
    )?;
    let wait = Duration::from_secs(10);
    pty.wait_for_text("press Enter to attach to the session", wait)?;
    assert!(
        pty.screen_text()
            .contains("post this code in the control channel within")
    );
    assert_waiting_before_enter(&mut pty, Duration::from_millis(1500)).await?;
    pty.send(b"\r")?;
    pty.wait_for_text("SESSION-MARK-9f3a", wait)?;
    pty.send(&[2, b'd'])?;
    pty.wait_for_text("[detached from", wait)?;
    assert_eq!(pty.wait_exit(wait)?, 0);
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_remote_without_team_name_still_waits_before_attaching() -> anyhow::Result<()> {
    let Some((dir, daemon)) = common::start_confined_daemon().await else {
        return Ok(());
    };
    let workspace = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let team_file = team_file(workspace.path());
    std::fs::write(
        &team_file,
        r#"
[[session]]
name = "remote"
command = ["sh", "-c", "printf SESSION-MARK-9f3a; sleep 30"]
attach = true
remote = "send"
[session.confine]
enabled = true
"#,
    )?;

    let mut pty = team_up_pty(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &team_file,
        false,
    )?;
    let wait = Duration::from_secs(10);
    pty.wait_for_text("press Enter to attach to the session", wait)?;
    assert!(
        pty.screen_text()
            .contains("post this code in the control channel within")
    );
    assert_waiting_before_enter(&mut pty, Duration::from_millis(1500)).await?;
    pty.send(b"\r")?;
    pty.wait_for_text("SESSION-MARK-9f3a", wait)?;
    pty.send(&[2, b'd'])?;
    pty.wait_for_text("[detached from", wait)?;
    assert_eq!(pty.wait_exit(wait)?, 0);
    daemon.shutdown().await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn team_up_remote_detach_does_not_wait_for_enter() -> anyhow::Result<()> {
    let Some((dir, daemon)) = common::start_confined_daemon().await else {
        return Ok(());
    };
    let workspace = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let team_file = team_file(workspace.path());
    std::fs::write(
        &team_file,
        r#"
team = "team-a"
[[session]]
name = "remote"
command = ["sh", "-c", "printf SESSION-MARK-9f3a; sleep 30"]
attach = true
remote = "send"
[session.confine]
enabled = true
"#,
    )?;

    let mut pty = team_up_pty(
        dir.path(),
        workspace.path(),
        user_home.path(),
        runtime.path(),
        &team_file,
        true,
    )?;
    let wait = Duration::from_secs(10);
    assert_eq!(pty.wait_exit(wait)?, 0);
    let screen = pty.screen_text();
    assert!(screen.contains("post this code in the control channel within"));
    assert!(!screen.contains("press Enter to attach to the session"));
    daemon.shutdown().await?;
    Ok(())
}

async fn session(
    admin: &mut Client,
    dir: &std::path::Path,
    name: &str,
    team: Option<TeamScope>,
    watch: &[&str],
    control_from: &[&str],
) -> (String, Client) {
    let (_, token, address) = common::new_team_agent(
        admin,
        dir,
        Some(name),
        Harness::Generic,
        Deliver::Hold,
        team,
        watch,
        control_from,
    )
    .await;
    let client = Client::connect_addr(address, &token).await.unwrap();
    let id = loop {
        let response = admin.request(Request::List).await.unwrap();
        if let Response::Sessions { sessions } = response
            && let Some(session) = sessions
                .into_iter()
                .find(|session| session.name.as_deref() == Some(name))
        {
            break session.id;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    (id, client)
}

async fn exit_session(admin: &mut Client, name: &str, team: TeamScope) -> String {
    let response = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "exit 0".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some(name.into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(team),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    let Response::Created { session } = response else {
        panic!("expected created session: {response:?}");
    };
    session
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_team_visibility_hides_other_private_teams_but_not_ungrouped() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_plain_id, _plain) = session(&mut admin, dir.path(), "plain", None, &[], &[]).await;

    let Response::Agents { agents, .. } = a.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a@") && agent.you)
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("plain@"))
    );
    assert!(!agents.iter().any(|agent| agent.address.starts_with("b@")));

    let denied = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "hidden".into(),
            message: "hidden".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        denied,
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into()
        }
    );
    let Response::Agents { agents, .. } = admin.request(Request::ListAgents).await.unwrap() else {
        panic!("expected admin agent list");
    };
    assert_eq!(agents.len(), 3);
    assert!(agents.iter().all(|agent| !agent.you));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_allow_is_required_for_private_team_message_visibility() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &["b"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, mut b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &["a"])),
        &[],
        &[],
    )
    .await;
    let sent = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "visible".into(),
            message: "visible".into(),
        })
        .await
        .unwrap();
    assert!(matches!(sent, Response::Accepted { .. }), "{sent:?}");
    let Response::Agents { agents, .. } = b.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(agents.iter().any(|agent| agent.address.starts_with("a@")));

    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &["b"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let denied = a
        .request(Request::SendMessage {
            to: "b".into(),
            subject: "hidden".into(),
            message: "hidden".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        denied,
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reset_visibility_fails_before_control_from() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b",
        Some(scope("b", true, &[])),
        &[],
        &["a"],
    )
    .await;
    let response = a
        .request(Request::Reset {
            session: "b".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        response,
        Response::Failed {
            code: "unknown_session".into(),
            message: "unknown session".into()
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invisible_exit_notifications_are_not_queued() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (same_id, _same) = session(
        &mut admin,
        dir.path(),
        "same",
        Some(scope("a", true, &[])),
        &["same-peer"],
        &[],
    )
    .await;
    let (other_id, _other) = session(
        &mut admin,
        dir.path(),
        "other",
        Some(scope("a", true, &[])),
        &["other-peer"],
        &[],
    )
    .await;
    let same_peer = exit_session(&mut admin, "same-peer", scope("a", true, &[])).await;
    let other_peer = exit_session(&mut admin, "other-peer", scope("b", true, &[])).await;
    let same_message = loop {
        let response = admin
            .request(Request::ListMessages {
                session: Some(same_id.clone()),
                state: Some("pending".into()),
            })
            .await
            .unwrap();
        if let Response::Messages { messages } = response
            && let Some(message) = messages
                .into_iter()
                .find(|message| message.subject.contains("same-peer"))
        {
            break message;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(same_message.subject.contains("same-peer"));
    let other_messages = admin
        .request(Request::ListMessages {
            session: Some(other_id.clone()),
            state: Some("pending".into()),
        })
        .await
        .unwrap();
    let Response::Messages {
        messages: other_messages,
    } = other_messages
    else {
        panic!("expected messages response");
    };
    assert!(
        !other_messages
            .iter()
            .any(|message| message.subject.contains("other-peer"))
    );
    let _ = (same_peer, other_peer);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_team_settings_are_consistent_until_the_first_member_exits() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (first_id, _first) = session(
        &mut admin,
        dir.path(),
        "first",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let different = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("second".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", false, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    assert_eq!(
        different,
        Response::Error {
            message: "team a is already running with different settings".into()
        }
    );
    let same = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("second".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", true, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    let same_id = match same {
        Response::Created { session } => session,
        other => panic!("expected same-settings session, got {other:?}"),
    };
    assert_eq!(
        admin
            .request(Request::Kill {
                session: first_id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    assert_eq!(
        admin
            .request(Request::Kill {
                session: same_id,
                now: true,
            })
            .await
            .unwrap(),
        Response::Ok
    );
    let after_exit = admin
        .request(Request::NewSession {
            argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
            cols: 40,
            rows: 10,
            cwd: None,
            env: Vec::new(),
            reset: Vec::new(),
            control_from: Vec::new(),
            watch: Vec::new(),
            name: Some("third".into()),
            harness: Harness::Generic,
            deliver: Some(Deliver::Hold),
            team: Some(scope("a", false, &[])),
            role: None,
            remote: None,
            heartbeat: None,
            activity: None,
        })
        .await
        .unwrap();
    assert!(matches!(after_exit, Response::Created { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_agent_allow_limits_private_team_visibility_and_messaging() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_arch_id, mut a_arch) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &[],
        &[],
    )
    .await;
    let (_a_dev_id, mut a_dev) = session(
        &mut admin,
        dir.path(),
        "a-developer",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    let (_b_arch_id, mut b_arch) = session(
        &mut admin,
        dir.path(),
        "b-architect",
        Some(scope_with_agents("b", true, &[], &["a-architect"])),
        &[],
        &[],
    )
    .await;
    let (_b_dev_id, _b_dev) = session(
        &mut admin,
        dir.path(),
        "b-developer",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;

    let Response::Agents { agents, .. } = a_arch.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a-architect@"))
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("a-developer@"))
    );
    assert!(
        agents
            .iter()
            .any(|agent| agent.address.starts_with("b-architect@"))
    );
    assert!(
        !agents
            .iter()
            .any(|agent| agent.address.starts_with("b-developer@"))
    );

    let sent = a_arch
        .request(Request::SendMessage {
            to: "b-architect".into(),
            subject: "hello".into(),
            message: "hello".into(),
        })
        .await
        .unwrap();
    assert!(matches!(sent, Response::Accepted { .. }), "{sent:?}");
    let reply = b_arch
        .request(Request::SendMessage {
            to: "a-architect".into(),
            subject: "reply".into(),
            message: "reply".into(),
        })
        .await
        .unwrap();
    assert!(matches!(reply, Response::Accepted { .. }), "{reply:?}");
    assert_eq!(
        a_arch
            .request(Request::SendMessage {
                to: "b-developer".into(),
                subject: "hidden".into(),
                message: "hidden".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );
    assert_eq!(
        a_dev
            .request(Request::SendMessage {
                to: "b-architect".into(),
                subject: "hidden".into(),
                message: "hidden".into(),
            })
            .await
            .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );

    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_a_id, mut a) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &[],
        &[],
    )
    .await;
    let (_b_id, _b) = session(
        &mut admin,
        dir.path(),
        "b-architect",
        Some(scope("b", true, &[])),
        &[],
        &[],
    )
    .await;
    let Response::Agents { agents, .. } = a.request(Request::ListAgents).await.unwrap() else {
        panic!("expected agent list");
    };
    assert!(
        !agents
            .iter()
            .any(|agent| agent.address.starts_with("b-architect@"))
    );
    assert_eq!(
        a.request(Request::SendMessage {
            to: "b-architect".into(),
            subject: "one-sided".into(),
            message: "one-sided".into(),
        })
        .await
        .unwrap(),
        Response::Failed {
            code: "unknown_recipient".into(),
            message: "unknown recipient".into(),
        }
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mutual_agent_allow_controls_exit_notifications() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (architect_id, _architect) = session(
        &mut admin,
        dir.path(),
        "a-architect",
        Some(scope_with_agents("a", true, &[], &["b-architect"])),
        &["b-architect"],
        &[],
    )
    .await;
    let (developer_id, _developer) = session(
        &mut admin,
        dir.path(),
        "a-developer",
        Some(scope("a", true, &[])),
        &["b-architect"],
        &[],
    )
    .await;
    let _peer_id = exit_session(
        &mut admin,
        "b-architect",
        scope_with_agents("b", true, &[], &["a-architect"]),
    )
    .await;
    let architect_message = loop {
        let response = admin
            .request(Request::ListMessages {
                session: Some(architect_id.clone()),
                state: Some("pending".into()),
            })
            .await
            .unwrap();
        if let Response::Messages { messages } = response
            && let Some(message) = messages
                .into_iter()
                .find(|message| message.subject.contains("b-architect"))
        {
            break message;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert!(architect_message.subject.contains("peer exited"));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let Response::Messages { messages } = admin
        .request(Request::ListMessages {
            session: Some(developer_id),
            state: Some("pending".into()),
        })
        .await
        .unwrap()
    else {
        panic!("expected messages response");
    };
    assert!(
        !messages
            .iter()
            .any(|message| message.subject.contains("b-architect"))
    );
}

fn new_session_request(name: &str, team: TeamScope) -> Request {
    Request::NewSession {
        argv: vec!["sh".into(), "-c".into(), "sleep 30".into()],
        cols: 40,
        rows: 10,
        cwd: None,
        env: Vec::new(),
        reset: Vec::new(),
        control_from: Vec::new(),
        watch: Vec::new(),
        name: Some(name.into()),
        harness: Harness::Generic,
        deliver: Some(Deliver::Hold),
        team: Some(team),
        role: None,
        remote: None,
        heartbeat: None,
        activity: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn daemon_validates_agent_allow_without_affecting_team_scope_consistency() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_first_id, _first) = session(
        &mut admin,
        dir.path(),
        "a-one",
        Some(scope("a", true, &[])),
        &[],
        &[],
    )
    .await;
    for (name, team, expected) in [
        (
            "self",
            scope_with_agents("a", true, &[], &["self"]),
            "must not name itself",
        ),
        (
            "repeat",
            scope_with_agents("a", true, &[], &["other", "other"]),
            "is repeated",
        ),
        (
            "public",
            scope_with_agents("a", false, &[], &["other"]),
            "allow needs a private team",
        ),
    ] {
        let response = admin
            .request(new_session_request(name, team))
            .await
            .unwrap();
        let Response::Error { message } = response else {
            panic!("expected validation error");
        };
        assert!(message.contains(expected), "{message}");
    }
    let too_many = (0..17)
        .map(|index| format!("other-{index}"))
        .collect::<Vec<_>>();
    let response = admin
        .request(new_session_request(
            "too-many",
            TeamScope {
                name: "a".into(),
                private: true,
                allow: Vec::new(),
                agents: too_many,
            },
        ))
        .await
        .unwrap();
    let Response::Error { message } = response else {
        panic!("expected validation error");
    };
    assert!(message.contains("at most 16"), "{message}");

    let accepted = admin
        .request(new_session_request(
            "a-two",
            scope_with_agents("a", true, &[], &["other"]),
        ))
        .await
        .unwrap();
    assert!(matches!(accepted, Response::Created { .. }), "{accepted:?}");
}

#[tokio::test]
async fn new_session_team_validation_messages_and_precedence() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    for (team, role, expected) in [
        (
            scope("Bad_Name", true, &[]),
            None,
            "team: invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (scope("a-", true, &[]), None, "team must not end in '-'"),
        (
            scope("a", false, &["Bad_Name"]),
            None,
            "allow needs a private team",
        ),
        (
            scope("a", true, &["Bad_Name"]),
            None,
            "team allow entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            scope("a", true, &["a"]),
            None,
            "team allow entry must not be the team itself",
        ),
        (
            scope("a", true, &["b", "b"]),
            None,
            "team allow entry \"b\" is repeated",
        ),
        (
            scope_with_agents("a", true, &[], &["Bad_Name"]),
            None,
            "session allow entry \"Bad_Name\": invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
        (
            scope("Bad_Name", true, &[]),
            Some(""),
            "team: invalid name \"Bad_Name\": expected [a-z0-9][a-z0-9-]{0,62}",
        ),
    ] {
        let mut request = new_session_request("worker", team);
        if let Request::NewSession { role: value, .. } = &mut request {
            *value = role.map(str::to_owned);
        }
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
async fn new_session_team_conflict_precedes_in_use_name() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let created = admin
        .request(new_session_request("worker", scope("a", true, &[])))
        .await
        .unwrap();
    assert!(matches!(created, Response::Created { .. }), "{created:?}");
    assert_eq!(
        admin
            .request(new_session_request("worker", scope("a", false, &[])))
            .await
            .unwrap(),
        Response::Error {
            message: "team a is already running with different settings".into()
        }
    );
    daemon.shutdown().await.unwrap();
}
