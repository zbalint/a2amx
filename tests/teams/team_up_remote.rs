use super::*;

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
