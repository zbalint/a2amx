#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Automatic grant detection through the public team and CLI seams.

mod common;

use common::pty::PtyHarness;
use common::start_daemon;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::mpsc as std_mpsc;
use std::time::Duration;
use tokio::process::Command as AsyncCommand;
fn run_binary(
    state: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(state)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .output()
}

fn run_binary_without_home(
    state: &Path,
    cwd: &Path,
    runtime: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(state)
        .args(args)
        .current_dir(cwd)
        .env_remove("HOME")
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .output()
}

fn global_auto_file(state: &Path) {
    std::fs::write(
        state.join("team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
}
fn write_worktree_git_file(project: &Path, config: &str) {
    std::fs::create_dir_all(project.join("meta/worktree")).expect("worktree metadata");
    std::fs::create_dir_all(project.join("common.git")).expect("common git directory");
    std::fs::write(project.join(".git"), "gitdir: meta/worktree\n").expect("worktree git file");
    std::fs::write(
        project.join("meta/worktree/commondir"),
        "../../common.git\n",
    )
    .expect("commondir");
    std::fs::write(project.join("common.git/config"), config).expect("git config");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_worktree_git_file_for_ssh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    write_worktree_git_file(
        project.path(),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    );

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.path().to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_submodule_git_file_for_ssh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(project.path().join("submodule-meta")).expect("submodule git");
    std::fs::write(project.path().join(".git"), "gitdir: submodule-meta\n")
        .expect("submodule git file");
    std::fs::write(
        project.path().join("submodule-meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.path().to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_repository_from_subdirectory() {
    let (state, _daemon) = start_daemon().await;
    let root = tempfile::tempdir().expect("repository");
    let subdirectory = root.path().join("sub");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(root.path().join(".git")).expect("git directory");
    std::fs::create_dir(&subdirectory).expect("subdirectory");
    std::fs::write(
        root.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let dry_run = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(
        dry_stdout.contains("  grants: cache, ssh\n"),
        "{dry_stdout}"
    );

    let suggest = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            subdirectory.to_str().expect("subdirectory path"),
        ],
    )
    .expect("suggest");
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_follows_worktree_git_file_for_https_gh() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::create_dir_all(user_home.path().join(".config/gh")).expect("gh config");
    write_worktree_git_file(
        project.path(),
        "[remote \"origin\"]\nurl = https://host-a/team/demo.git\n",
    );

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry_run.status.success(),
        "{}",
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry_run.stdout);
    assert!(dry_stdout.contains("  grants: cache, gh\n"), "{dry_stdout}");

    let suggest = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.path().to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(suggest.status.success());
    assert_eq!(
        suggest.stdout,
        b"# Suggested grants:\ngrants = [\"gh\", \"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_dry_run_reports_detected_grants_once() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname='demo'\n",
    )
    .expect("cargo marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("grants: cache, cargo, ssh"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("auto confinement for session")
            && stderr.contains("grants ssh, cargo, cache"),
        "{stderr}"
    );
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dry_run_reports_unaccepted_grants_and_respects_seeded_file() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  cargo: Cargo.toml found\n"), "{stdout}");
    assert!(
        stdout.contains("  cache: always added by auto\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("  not yet accepted: cargo, cache\n"),
        "{stdout}"
    );
    assert!(!state.path().join("auto-grants").exists());

    std::fs::write(
        state.path().join("auto-grants"),
        "unknown\n cache \n\ncargo\n",
    )
    .expect("seed accepted grants");
    let seeded = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("seeded dry run");
    assert!(seeded.status.success());
    assert!(
        !String::from_utf8_lossy(&seeded.stdout).contains("not yet accepted"),
        "{}",
        String::from_utf8_lossy(&seeded.stdout)
    );
    std::fs::write(state.path().join("auto-grants"), "x".repeat(4097))
        .expect("oversized accepted grants");
    let oversized = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("oversized dry run");
    assert!(!oversized.status.success());
    assert!(
        String::from_utf8_lossy(&oversized.stderr)
            .contains(&state.path().join("auto-grants").display().to_string())
    );
}

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
    assert!(!state.path().join("auto-grants").exists());
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
    std::fs::write(state.path().join("auto-grants"), "unknown\n").expect("unknown grant");

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
    assert_eq!(
        std::fs::read_to_string(state.path().join("auto-grants")).expect("read unknown grant"),
        "unknown\n"
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
        state.path().join("team.toml"),
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
        "Accept these for auto mode on all projects?",
        Duration::from_secs(10),
    )?;
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
    let accepted = state.path().join("auto-grants");
    assert_eq!(std::fs::read_to_string(&accepted)?, "cache\ncargo\n");
    assert_eq!(
        std::fs::metadata(accepted)?.permissions().mode() & 0o777,
        0o600
    );
    assert!(user_home.path().join(".cargo").is_dir());

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
            state.path().join("team.toml"),
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
            "Accept these for auto mode on all projects?",
            Duration::from_secs(10),
        )?;
        pty.send(input)?;
        assert_ne!(pty.wait_exit(Duration::from_secs(10))?, 0);
        assert!(!state.path().join("auto-grants").exists());
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
        state.path().join("team.toml"),
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
        "Accept these for auto mode on all projects?",
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
        std::fs::read_to_string(state.path().join("auto-grants"))?,
        "cache\n"
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
    std::fs::write(state.path().join("auto-grants"), "cache\n")?;
    std::fs::write(cargo.path().join("Cargo.toml"), "")?;
    std::fs::write(
        state.path().join("team.toml"),
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
        "Accept these for auto mode on all projects?",
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
        std::fs::read_to_string(state.path().join("auto-grants"))?,
        "cache\ncargo\n"
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_merge_preserves_explicit_grant_and_reports_added_reasons() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"cargo\"]\n",
    )
    .expect("global team file");
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("cargo marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  confinement: auto\n"), "{stdout}");
    assert!(stdout.contains("  grants: cache, cargo, ssh\n"), "{stdout}");
    assert!(stdout.contains("  ssh: git remote uses ssh\n"), "{stdout}");
    assert!(
        stdout.contains("  cache: always added by auto\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("  cargo: "), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn auto_mode_stays_true_when_auto_adds_nothing() {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping auto-adds-nothing test: bubblewrap unavailable");
        return;
    }
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project with no markers");
    let bwrap = String::from_utf8(
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
    std::os::unix::fs::symlink(bwrap.trim(), bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    // The session already names every grant auto would add here (cache only: no
    // markers, no git remote), so auto runs and adds nothing; `auto` stays true.
    std::fs::write(
        state.path().join("team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n[session.confine]\ngrants = [\"cache\"]\n",
    )
    .expect("global team file");

    let dry = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(
        dry.status.success(),
        "{}",
        String::from_utf8_lossy(&dry.stderr)
    );
    let dry_stdout = String::from_utf8_lossy(&dry.stdout);
    assert!(dry_stdout.contains("  confinement: auto\n"), "{dry_stdout}");
    assert!(!dry_stdout.contains("always added by auto"), "{dry_stdout}");
    assert!(
        !dry_stdout.contains("  cargo: ") && !dry_stdout.contains("  ssh: "),
        "{dry_stdout}"
    );
    let dry_stderr = String::from_utf8_lossy(&dry.stderr);
    assert!(
        !dry_stderr.contains("auto confinement for session"),
        "{dry_stderr}"
    );

    let real = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("real up");
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stderr)
    );
    let real_stderr = String::from_utf8_lossy(&real.stderr);
    assert!(
        !real_stderr.contains("auto confinement for session"),
        "{real_stderr}"
    );
    let started_line = String::from_utf8_lossy(&real.stdout)
        .lines()
        .find(|line| line.starts_with("started ") && line.contains("worker"))
        .expect("started worker")
        .to_owned();
    let worker_id = started_line
        .split_whitespace()
        .nth(2)
        .expect("started worker id")
        .to_owned();
    let listed = String::from_utf8(
        run_binary(
            state.path(),
            project.path(),
            user_home.path(),
            runtime.path(),
            &["list"],
        )
        .expect("list")
        .stdout,
    )
    .expect("list output");
    let cells = common::list_cells(&listed, &worker_id);
    assert!(cells.contains(&"auto"), "{listed}");
    assert!(cells.contains(&"running"), "{listed}");

    let _ = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "down", "--now"],
    );
}

#[test]
fn suggest_does_not_probe_cwd_for_host_paths_when_home_is_unset() {
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(project.path().join(".config/gh")).expect("project gh marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = https://example.invalid/team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary_without_home(
        state.path(),
        project.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.path().to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("grants = [\"cache\"]"), "{stdout}");
    assert!(!stdout.contains("\"gh\""), "{stdout}");
}

#[test]
fn git_config_size_limit_accepts_65536_and_rejects_65537_bytes() {
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir(project.path().join(".git")).expect("git directory");
    let remote = "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n";
    let config_for_size = |size: usize| {
        let mut config = format!("{}{}", "# padding\n".repeat(32), remote);
        while config.len() + 2 <= size {
            config.push_str("#\n");
        }
        while config.len() < size {
            config.push('#');
        }
        assert_eq!(config.len(), size);
        config
    };

    for (size, expected) in [
        (
            65_536,
            "# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n",
        ),
        (65_537, "# Suggested grants:\ngrants = [\"cache\"]\n"),
    ] {
        std::fs::write(project.path().join(".git/config"), config_for_size(size))
            .expect("git config");
        assert_eq!(
            std::fs::metadata(project.path().join(".git/config"))
                .expect("git config metadata")
                .len(),
            size as u64
        );
        let output = run_binary(
            state.path(),
            project.path(),
            user_home.path(),
            runtime.path(),
            &[
                "confine",
                "suggest",
                project.path().to_str().expect("project path"),
            ],
        )
        .expect("suggest");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, expected.as_bytes());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_missing_worktree_git_target() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join(".git"), "gitdir: missing-meta\n")
        .expect("missing worktree git file");
    std::fs::write(
        project.path().join("config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("fallback git config bait");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_oversized_worktree_git_config() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(&target).expect("worktree metadata");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::fs::write(
        target.join("config"),
        format!(
            "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n{}",
            "x".repeat(65_500)
        ),
    )
    .expect("oversized git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_directory() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(target.join("config")).expect("config directory");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_symlink_to_directory() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(target.join("config-directory")).expect("config directory");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    std::os::unix::fs::symlink("config-directory", target.join("config"))
        .expect("config directory symlink");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_worktree_git_config_fifo_without_hanging() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    let target = project.path().join("meta/worktree");
    std::fs::create_dir_all(&target).expect("worktree metadata");
    std::fs::write(project.path().join(".git"), "gitdir: meta/worktree\n")
        .expect("worktree git file");
    let status = Command::new("mkfifo")
        .arg(target.join("config"))
        .status()
        .expect("mkfifo");
    assert!(status.success(), "mkfifo status: {status}");
    std::fs::write(
        project.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("sibling git config");

    let state_path = state.path().to_owned();
    let project_path = project.path().to_owned();
    let user_home_path = user_home.path().to_owned();
    let runtime_path = runtime.path().to_owned();
    let (sender, receiver) = std_mpsc::channel();
    std::thread::spawn(move || {
        let result = run_binary(
            &state_path,
            &project_path,
            &user_home_path,
            &runtime_path,
            &[
                "confine",
                "suggest",
                project_path.to_str().expect("project path"),
            ],
        );
        let _ = sender.send(result);
    });
    let output = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("auto grant detection timed out")
        .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_rejects_gitdir_at_home() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(
        project.path().join(".git"),
        format!("gitdir: {}\n", user_home.path().display()),
    )
    .expect("home git file");
    std::fs::write(
        user_home.path().join("config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("home git config");

    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("  grants: cache\n"), "{stdout}");
}

#[test]
fn suggest_symlinked_home_repository_is_not_detected() {
    let state = tempfile::tempdir().expect("state");
    let home_parent = tempfile::tempdir().expect("home parent");
    let real_home = home_parent.path().join("real-home");
    let symlinked_home = home_parent.path().join("home-link");
    let project = symlinked_home.join("project");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(real_home.join(".git")).expect("home git directory");
    std::os::unix::fs::symlink(&real_home, &symlinked_home).expect("home symlink");
    std::fs::create_dir_all(&project).expect("project");
    std::fs::write(
        real_home.join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("home git config");

    let output = run_binary(
        state.path(),
        &project,
        &symlinked_home,
        runtime.path(),
        &[
            "confine",
            "suggest",
            project.to_str().expect("project path"),
        ],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}

#[test]
fn suggest_follows_relative_gitdir_from_parent_repository() {
    let state = tempfile::tempdir().expect("state");
    let root = tempfile::tempdir().expect("repository");
    let subdirectory = root.path().join("sub");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(root.path().join("meta")).expect("git metadata");
    std::fs::create_dir(&subdirectory).expect("subdirectory");
    std::fs::write(root.path().join(".git"), "gitdir: meta\n").expect("gitdir marker");
    std::fs::write(
        root.path().join("meta/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        &subdirectory,
        user_home.path(),
        runtime.path(),
        &[
            "confine",
            "suggest",
            subdirectory.to_str().expect("subdirectory path"),
        ],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"ssh\", \"cache\"]\n"
    );
}

#[test]
fn suggest_canonicalizes_relative_directory_before_git_walk() {
    let state = tempfile::tempdir().expect("state");
    let root = tempfile::tempdir().expect("repository parent");
    let repository = root.path().join("repository");
    let sibling = root.path().join("sibling");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir_all(repository.join(".git")).expect("git directory");
    std::fs::create_dir(&sibling).expect("sibling");
    std::fs::write(
        repository.join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");

    let output = run_binary(
        state.path(),
        &repository,
        user_home.path(),
        runtime.path(),
        &["confine", "suggest", "../sibling"],
    )
    .expect("suggest");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        b"# Suggested grants:\ngrants = [\"cache\"]\n"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_auto_ignores_worktree_git_files_and_oversized_configs() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    global_auto_file(state.path());
    std::fs::write(project.path().join(".git"), "gitdir: /other/repo/.git\n")
        .expect("worktree git file");

    let worktree = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("worktree dry run");
    assert!(worktree.status.success());
    let worktree_stdout = String::from_utf8_lossy(&worktree.stdout);
    let grants = worktree_stdout
        .lines()
        .find(|line| line.starts_with("  grants:"))
        .expect("grants line");
    assert_eq!(grants, "  grants: cache");

    std::fs::remove_file(project.path().join(".git")).expect("remove worktree marker");
    std::fs::create_dir_all(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join(".git/config"),
        format!(
            "[remote \"origin\"]\nurl = git@host-a:repo.git\n{}",
            "x".repeat(65_500)
        ),
    )
    .expect("oversized config");
    let oversized = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("oversized config dry run");
    assert!(oversized.status.success());
    let oversized_stdout = String::from_utf8_lossy(&oversized.stdout);
    let grants = oversized_stdout
        .lines()
        .find(|line| line.starts_with("  grants:"))
        .expect("grants line");
    assert_eq!(grants, "  grants: cache");
}

#[test]
fn local_auto_and_file_auto_are_rejected_before_daemon_use() {
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        project.path().join("a2amx.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("local team file");
    let local = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("local auto rejection");
    assert!(!local.status.success());
    assert!(
        String::from_utf8_lossy(&local.stderr)
            .contains("auto is only read from the global team file")
    );

    let global = state.path().join("global.toml");
    std::fs::write(
        &global,
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("explicit global file");
    let global_path = global.to_str().expect("global path");
    let explicit = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run", "--file", global_path],
    )
    .expect("explicit global auto rejection");
    assert!(!explicit.status.success());
    assert!(
        String::from_utf8_lossy(&explicit.stderr)
            .contains("auto is only read from the global team file")
    );
    assert!(!String::from_utf8_lossy(&explicit.stderr).contains("cannot reach the a2amx daemon"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn global_without_auto_has_no_automatic_grants() {
    let (state, _daemon) = start_daemon().await;
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("team.toml"),
        "[confine]\nenabled = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .expect("global team file");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");
    let output = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("  grants:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_up_reuses_auto_grants_and_running_session_is_not_rescanned() {
    let bwrap = AsyncCommand::new("bwrap").arg("--version").output().await;
    if !matches!(bwrap, Ok(output) if output.status.success()) {
        eprintln!("skipping real auto start test: bubblewrap unavailable");
        return;
    }
    let state = tempfile::tempdir().expect("state");
    let project = tempfile::tempdir().expect("project");
    let bwrap = String::from_utf8(
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
    std::os::unix::fs::symlink(bwrap.trim(), bwrap_dir.join("bwrap")).expect("bwrap symlink");
    let _daemon = common::start_daemon_in(
        state.path(),
        Some("host-a"),
        a2amx::messaging::Limits::default(),
    )
    .await;
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::write(
        state.path().join("team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )
    .expect("global team file");
    std::fs::write(project.path().join("Cargo.toml"), "").expect("cargo marker");

    let dry_run = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--dry-run"],
    )
    .expect("dry run");
    assert!(dry_run.status.success());
    assert!(String::from_utf8_lossy(&dry_run.stdout).contains("grants: cache, cargo"));

    let real = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("real up");
    assert!(
        real.status.success(),
        "{}",
        String::from_utf8_lossy(&real.stderr)
    );
    let real_stderr = String::from_utf8_lossy(&real.stderr);
    assert_eq!(
        real_stderr
            .lines()
            .filter(|line| line.contains("auto confinement for session"))
            .count(),
        1
    );

    let second = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    )
    .expect("second up");
    assert!(second.status.success());
    assert!(!String::from_utf8_lossy(&second.stderr).contains("auto confinement for session"));

    let started_line = String::from_utf8_lossy(&real.stdout)
        .lines()
        .find(|line| line.starts_with("started ") && line.contains("worker"))
        .unwrap_or_else(|| {
            panic!(
                "started worker id\nSTDOUT: {}\nSTDERR: {}",
                String::from_utf8_lossy(&real.stdout),
                String::from_utf8_lossy(&real.stderr)
            )
        })
        .to_owned();
    let mut started_fields = started_line.split_whitespace();
    let _ = started_fields.next();
    let worker_name = started_fields
        .next()
        .expect("started worker name")
        .to_owned();
    let worker_id = started_fields.next().expect("started worker id").to_owned();

    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["list"],
            )
            .expect("list")
            .stdout,
        )
        .expect("list output");
        let cells = common::list_cells(&listed, &worker_id);
        // The team cell may itself contain a space ("name (private)"), so assert on
        // the cell set rather than fragile positional indexes.
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"auto"), "{listed}");
        assert!(cells.contains(&"running"), "{listed}");
    }
    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["list", "--details"],
            )
            .expect("details list")
            .stdout,
        )
        .expect("details output");
        let cells = common::list_cells(&listed, &worker_id);
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"auto"), "{listed}");
    }
    {
        let status = String::from_utf8(
            run_binary(
                state.path(),
                project.path(),
                user_home.path(),
                runtime.path(),
                &["team", "status"],
            )
            .expect("team status")
            .stdout,
        )
        .expect("team status output");
        let row = status
            .lines()
            .find(|line| line.starts_with(&worker_name))
            .unwrap_or_else(|| panic!("missing worker row in {status}"));
        assert!(row.contains(" running auto "), "{row}");
        assert!(row.ends_with(&worker_id), "{row}");
    }
    {
        let second_stdout = String::from_utf8_lossy(&second.stdout);
        assert!(
            second_stdout.contains(&format!(
                "already running {worker_name} {worker_id} (confine auto)"
            )),
            "{second_stdout}"
        );
    }

    // A manually confined session must keep `manual`, never `off` or `auto`.
    let manual_project = tempfile::tempdir().expect("manual project");
    std::fs::write(
        manual_project.path().join("manual.toml"),
        "[confine]\nenabled = true\n[[session]]\nname = \"manualworker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n",
    )
    .expect("manual team file");
    let manual_up = run_binary(
        state.path(),
        manual_project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--file", "manual.toml"],
    )
    .expect("manual up");
    assert!(
        manual_up.status.success(),
        "{}",
        String::from_utf8_lossy(&manual_up.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&manual_up.stderr).contains("auto confinement for session"),
        "{}",
        String::from_utf8_lossy(&manual_up.stderr)
    );
    let manual_id = String::from_utf8_lossy(&manual_up.stdout)
        .lines()
        .find(|line| line.starts_with("started manualworker "))
        .and_then(|line| line.split_whitespace().nth(2))
        .expect("started manualworker id")
        .to_owned();
    {
        let listed = String::from_utf8(
            run_binary(
                state.path(),
                manual_project.path(),
                user_home.path(),
                runtime.path(),
                &["list", "--details"],
            )
            .expect("manual details list")
            .stdout,
        )
        .expect("manual details output");
        let cells = common::list_cells(&listed, &manual_id);
        assert!(cells.contains(&"generic"), "{listed}");
        assert!(cells.contains(&"manual"), "{listed}");
        assert!(cells.contains(&"running"), "{listed}");
    }
    {
        let status = String::from_utf8(
            run_binary(
                state.path(),
                manual_project.path(),
                user_home.path(),
                runtime.path(),
                &["team", "status", "--file", "manual.toml"],
            )
            .expect("manual team status")
            .stdout,
        )
        .expect("manual team status output");
        assert!(
            status.contains(&format!("manualworker - running manual {manual_id}")),
            "{status}"
        );
    }
    {
        let dry = run_binary(
            state.path(),
            manual_project.path(),
            user_home.path(),
            runtime.path(),
            &["team", "up", "--dry-run", "--file", "manual.toml"],
        )
        .expect("manual dry run");
        assert!(dry.status.success());
        let stdout = String::from_utf8_lossy(&dry.stdout);
        std::eprintln!("DEBUG manual_id={manual_id:?} stdout={stdout:?}");
        assert!(
            stdout.contains(&format!(
                "already running manualworker {manual_id} (confine manual)"
            )),
            "{stdout}"
        );
    }

    let _ = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "down", "--now"],
    );
}
