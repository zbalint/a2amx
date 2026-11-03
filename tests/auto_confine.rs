#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Automatic grant detection through the public team and CLI seams.

mod common;

use common::start_daemon;
use std::path::Path;
use std::process::{Command, Output, Stdio};
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
        &["team", "up", "--detach"],
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
        &["team", "up", "--detach"],
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
        &["team", "up", "--detach"],
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
