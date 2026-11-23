use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refusal_blocks_auto_start_and_yes_is_one_run_only() {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping auto consent start test: bubblewrap unavailable");
        return;
    }
    let state = tempfile::tempdir().expect("state");
    let bwrap_path = String::from_utf8(
        AsyncCommand::new("which")
            .arg("bwrap")
            .output()
            .await
            .expect("which bwrap")
            .stdout,
    )
    .expect("bwrap path");
    let bwrap_dir = state.path().join("bwrap-path");
    std::fs::create_dir_all(&bwrap_dir).expect("bwrap directory");
    std::os::unix::fs::symlink(bwrap_path.trim(), bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");

    let refused = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach"],
    )
    .expect("refused up");
    assert!(!refused.status.success());
    let refused_stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        refused_stderr.contains(
            "auto confinement would open: cargo, cache; run a2amx team up on a terminal to accept them, or pass --yes for this run"
        ),
        "{refused_stderr}"
    );
    assert!(
        !refused_stderr.contains("auto confinement for session"),
        "{refused_stderr}"
    );
    assert!(!state.path().join("state/auto-grants.toml").exists());
    let listed = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["list"],
    )
    .expect("list after refusal");
    assert!(
        !String::from_utf8_lossy(&listed.stdout).contains("worker"),
        "{}",
        String::from_utf8_lossy(&listed.stdout)
    );
    std::fs::write(state.path().join("state/auto-grants"), "cache\ncargo\n")
        .expect("valid legacy grant");
    let legacy_ignored = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("legacy dry run");
    assert!(legacy_ignored.status.success());
    assert!(String::from_utf8_lossy(&legacy_ignored.stdout).contains("not yet accepted"));

    let accepted_once = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("yes up");
    assert!(
        accepted_once.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&accepted_once.stdout),
        String::from_utf8_lossy(&accepted_once.stderr)
    );
    let yes_stderr = String::from_utf8_lossy(&accepted_once.stderr);
    assert!(
        yes_stderr.contains("cargo: Cargo.toml found")
            && yes_stderr.contains("auto confinement for session"),
        "{yes_stderr}"
    );
    assert!(!state.path().join("state/auto-grants.toml").exists());
    assert_eq!(
        std::fs::read_to_string(state.path().join("state/auto-grants"))
            .expect("read unknown grant"),
        "cache\ncargo\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_yes_persists_accepted_grants_with_private_mode() -> anyhow::Result<()> {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping PTY auto consent test: bubblewrap unavailable");
        return Ok(());
    }
    let state = tempfile::tempdir()?;
    let project = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let _daemon = common::spawn_daemon_process(state.path(), user_home.path(), runtime.path());
    std::fs::write(
        state.path().join("config/team.toml"),
        format!(
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n",
            project.path().to_string_lossy()
        ),
    )?;
    std::fs::write(project.path().join("Cargo.toml"), "")?;
    let args = vec![
        "--home".to_owned(),
        state.path().to_string_lossy().into_owned(),
        "team".to_owned(),
        "up".to_owned(),
        "--detach".to_owned(),
    ];
    let home_env = user_home.path().to_string_lossy().into_owned();
    let runtime_env = runtime.path().to_string_lossy().into_owned();
    let mut pty = PtyHarness::spawn_with_env_in(
        &args,
        state.path(),
        project.path(),
        &[("HOME", &home_env), ("XDG_RUNTIME_DIR", &runtime_env)],
    )?;
    pty.wait_for_text("cargo: Cargo.toml found", Duration::from_secs(10))?;
    pty.wait_for_text(
        "Accept these for auto mode in this project (",
        Duration::from_secs(10),
    )?;
    let screen = pty.screen_text();
    assert!(
        screen.find("cargo:").expect("cargo line")
            < screen
                .find("Accept these for auto mode in this project")
                .expect("question")
    );
    pty.send(b"y\n")?;
    assert_eq!(pty.wait_exit(Duration::from_secs(10))?, 0);
    let listed = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["list"],
    )?;
    assert!(
        String::from_utf8_lossy(&listed.stdout).contains("worker"),
        "{}",
        String::from_utf8_lossy(&listed.stdout)
    );
    let accepted = state.path().join("state/auto-grants.toml");
    assert_eq!(
        std::fs::read_to_string(&accepted)?,
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            project.path().to_string_lossy()
        )
    );
    assert_eq!(
        std::fs::metadata(&accepted)?.permissions().mode() & 0o777,
        0o600
    );
    assert!(user_home.path().join(".cargo").is_dir());
    let project_b = tempfile::tempdir()?;
    std::fs::write(project_b.path().join("Cargo.toml"), "")?;
    std::fs::write(
        state.path().join("config/team.toml"),
        format!(
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker-b\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n",
            project_b.path().to_string_lossy()
        ),
    )?;
    let mut pty_b = PtyHarness::spawn_with_env_in(
        &args,
        state.path(),
        project_b.path(),
        &[("HOME", &home_env), ("XDG_RUNTIME_DIR", &runtime_env)],
    )?;
    pty_b.wait_for_text("cargo: Cargo.toml found", Duration::from_secs(10))?;
    pty_b.wait_for_text(
        "Accept these for auto mode in this project (",
        Duration::from_secs(10),
    )?;
    pty_b.send(b"y\n")?;
    assert_eq!(pty_b.wait_exit(Duration::from_secs(10))?, 0);
    let accepted_text = std::fs::read_to_string(&accepted)?;
    let project_a = project.path().to_string_lossy().into_owned();
    let project_b_key = project_b.path().to_string_lossy().into_owned();
    let expected = if project_a < project_b_key {
        format!(
            "[projects.{project_a:?}]\ngrants = [\"cache\", \"cargo\"]\n\n[projects.{project_b_key:?}]\ngrants = [\"cache\", \"cargo\"]\n"
        )
    } else {
        format!(
            "[projects.{project_b_key:?}]\ngrants = [\"cache\", \"cargo\"]\n\n[projects.{project_a:?}]\ngrants = [\"cache\", \"cargo\"]\n"
        )
    };
    assert_eq!(accepted_text, expected);
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_no_and_eof_refuse_without_starting_or_writing() -> anyhow::Result<()> {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping PTY auto refusal test: bubblewrap unavailable");
        return Ok(());
    }
    async fn run_case(input: &[u8]) -> anyhow::Result<()> {
        let state = tempfile::tempdir()?;
        let project = tempfile::tempdir()?;
        let user_home = tempfile::tempdir()?;
        let runtime = tempfile::tempdir()?;
        let _daemon = common::spawn_daemon_process(state.path(), user_home.path(), runtime.path());
        std::fs::write(
            state.path().join("config/team.toml"),
            format!(
                "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n",
                project.path().to_string_lossy()
            ),
        )?;
        std::fs::write(project.path().join("Cargo.toml"), "")?;
        let args = vec![
            "--home".to_owned(),
            state.path().to_string_lossy().into_owned(),
            "team".to_owned(),
            "up".to_owned(),
            "--detach".to_owned(),
        ];
        let home_env = user_home.path().to_string_lossy().into_owned();
        let runtime_env = runtime.path().to_string_lossy().into_owned();
        let mut pty = PtyHarness::spawn_with_env_in(
            &args,
            state.path(),
            project.path(),
            &[("HOME", &home_env), ("XDG_RUNTIME_DIR", &runtime_env)],
        )?;
        pty.wait_for_text(
            "Accept these for auto mode in this project (",
            Duration::from_secs(10),
        )?;
        pty.send(input)?;
        assert_ne!(pty.wait_exit(Duration::from_secs(10))?, 0);
        assert!(!state.path().join("state/auto-grants.toml").exists());
        let listed = run_binary(
            state.path(),
            project.path(),
            user_home.path(),
            runtime.path(),
            &["list"],
        )?;
        assert!(
            !String::from_utf8_lossy(&listed.stdout).contains("worker"),
            "{}",
            String::from_utf8_lossy(&listed.stdout)
        );
        Ok(())
    }
    run_case(b"n\n").await?;
    run_case(&[4]).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_deduplicates_auto_consent_across_sessions() -> anyhow::Result<()> {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping PTY auto dedupe test: bubblewrap unavailable");
        return Ok(());
    }
    let state = tempfile::tempdir()?;
    let first = tempfile::tempdir()?;
    let second = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let _daemon = common::spawn_daemon_process(state.path(), user_home.path(), runtime.path());
    std::fs::write(
        state.path().join("config/team.toml"),
        format!(
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker-a\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n[[session]]\nname = \"worker-b\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n",
            first.path().to_string_lossy(),
            second.path().to_string_lossy()
        ),
    )?;
    let args = vec![
        "--home".to_owned(),
        state.path().to_string_lossy().into_owned(),
        "team".to_owned(),
        "up".to_owned(),
        "--detach".to_owned(),
    ];
    let home_env = user_home.path().to_string_lossy().into_owned();
    let runtime_env = runtime.path().to_string_lossy().into_owned();
    let mut pty = PtyHarness::spawn_with_env_in(
        &args,
        state.path(),
        first.path(),
        &[("HOME", &home_env), ("XDG_RUNTIME_DIR", &runtime_env)],
    )?;
    pty.wait_for_text(
        "Accept these for auto mode in this project (",
        Duration::from_secs(10),
    )?;
    let screen = pty.screen_text();
    let cache_lines = screen
        .lines()
        .filter(|line| line.trim_start().starts_with("cache:"))
        .count();
    assert_eq!(cache_lines, 1, "{screen}");
    pty.send(b"y\n")?;
    assert_eq!(pty.wait_exit(Duration::from_secs(10))?, 0);
    assert_eq!(
        std::fs::read_to_string(state.path().join("state/auto-grants.toml"))?,
        format!(
            "[projects.{:?}]\ngrants = [\"cache\"]\n",
            first.path().to_string_lossy()
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pty_subtracts_seeded_auto_grants_from_consent() -> anyhow::Result<()> {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping PTY accepted-grant subtraction test: bubblewrap unavailable");
        return Ok(());
    }
    let state = tempfile::tempdir()?;
    let plain = tempfile::tempdir()?;
    let cargo = tempfile::tempdir()?;
    let user_home = tempfile::tempdir()?;
    let runtime = tempfile::tempdir()?;
    let _daemon = common::spawn_daemon_process(state.path(), user_home.path(), runtime.path());
    std::fs::write(
        state.path().join("state/auto-grants.toml"),
        format!(
            "[projects.{:?}]\ngrants = [\"cache\"]\n",
            cargo.path().to_string_lossy()
        ),
    )?;
    std::fs::write(cargo.path().join("Cargo.toml"), "")?;
    std::fs::write(
        state.path().join("config/team.toml"),
        format!(
            "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"plain\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n[[session]]\nname = \"cargo\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\ncwd = {:?}\n",
            plain.path().to_string_lossy(),
            cargo.path().to_string_lossy()
        ),
    )?;
    let args = vec![
        "--home".to_owned(),
        state.path().to_string_lossy().into_owned(),
        "team".to_owned(),
        "up".to_owned(),
        "--detach".to_owned(),
    ];
    let home_env = user_home.path().to_string_lossy().into_owned();
    let runtime_env = runtime.path().to_string_lossy().into_owned();
    let mut pty = PtyHarness::spawn_with_env_in(
        &args,
        state.path(),
        cargo.path(),
        &[("HOME", &home_env), ("XDG_RUNTIME_DIR", &runtime_env)],
    )?;
    pty.wait_for_text(
        "Accept these for auto mode in this project (",
        Duration::from_secs(10),
    )?;
    let screen = pty.screen_text();
    let cargo_lines = screen
        .lines()
        .filter(|line| line.trim_start().starts_with("cargo:"))
        .count();
    let cache_lines = screen
        .lines()
        .filter(|line| line.trim_start().starts_with("cache:"))
        .count();
    assert_eq!(cargo_lines, 1, "{screen}");
    assert_eq!(cache_lines, 0, "{screen}");
    pty.send(b"y\n")?;
    assert_eq!(pty.wait_exit(Duration::from_secs(10))?, 0);
    assert_eq!(
        std::fs::read_to_string(state.path().join("state/auto-grants.toml"))?,
        format!(
            "[projects.{:?}]\ngrants = [\"cache\", \"cargo\"]\n",
            cargo.path().to_string_lossy()
        )
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn escape_notice_precedes_auto_confinement_summary() {
    let state = tempfile::tempdir().expect("state");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let team_file = project.path().join("team.toml");
    std::fs::write(
        &team_file,
        "team = \"t\"\n[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"docker\"]\n",
    )
    .expect("team file");

    let out = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "team",
            "up",
            "--dry-run",
            "--file",
            team_file.to_str().unwrap(),
        ],
    )
    .expect("team up");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "session t-worker: grant docker can escape: a privileged container started through the daemon mounts the host filesystem and runs as root on the host\nauto confinement for session t-worker: grants cargo, cache\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn write_auto_summaries_preserves_session_order() {
    let state = tempfile::tempdir().expect("state");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let team_file = project.path().join("team.toml");
    std::fs::write(
        &team_file,
        "team = \"t\"\n[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"first\"\ncommand = [\"cat\"]\n[[session]]\nname = \"second\"\ncommand = [\"cat\"]\n",
    )
    .expect("team file");

    let out = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "team",
            "up",
            "--dry-run",
            "--file",
            team_file.to_str().unwrap(),
        ],
    )
    .expect("team up");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "auto confinement for session t-first: grants cargo, cache\nauto confinement for session t-second: grants cargo, cache\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_auto_team_skips_loading_accepted_grants() {
    let state = tempfile::tempdir().expect("state");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    std::fs::create_dir_all(state.path().join("state/auto-grants.toml"))
        .expect("auto-grants.toml as directory");
    let project = tempfile::tempdir().expect("project");
    let canonical_project = std::fs::canonicalize(project.path()).expect("canonical project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    let team_file = project.path().join("team.toml");
    std::fs::write(
        &team_file,
        "team = \"plain\"\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("team file");

    let out = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "team",
            "up",
            "--dry-run",
            "--file",
            team_file.to_str().unwrap(),
        ],
    )
    .expect("dry run");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!(
            "would start plain-worker\n  harness: generic\n  cwd: {}\n  team: plain, private, allow: -\n  confinement: off\n  command: cat\n",
            canonical_project.display()
        )
    );
}
