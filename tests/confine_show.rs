#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::process::{Command, Output};

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

fn link_real_bwrap(state: &Path) -> bool {
    let output = Command::new("which")
        .arg("bwrap")
        .output()
        .expect("find bwrap");
    if !output.status.success() {
        return false;
    }
    let path = String::from_utf8(output.stdout).expect("bwrap path");
    std::os::unix::fs::symlink(path.trim(), state.join("bwrap-path/bwrap")).expect("link bwrap");
    true
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_reports_off_mode_by_name_and_id() {
    let (state, _daemon) = common::start_daemon().await;
    let cwd = tempfile::tempdir().expect("cwd");
    let user_home = tempfile::tempdir().expect("home");
    let runtime = tempfile::tempdir().expect("runtime");
    let created = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &[
            "new", "--detach", "--name", "worker", "--", "sh", "-c", "sleep 30",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    assert_eq!(created.stdout, b"s1\n");

    let by_name = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show", "worker"],
    );
    assert!(
        by_name.status.success(),
        "{}",
        String::from_utf8_lossy(&by_name.stderr)
    );
    let output = String::from_utf8(by_name.stdout).expect("utf8 output");
    assert!(
        output.starts_with("worker s1 generic confine off\n"),
        "{output}"
    );
    assert!(!output.contains("GRANT  SOURCE"));

    let by_id = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show", "s1"],
    );
    assert!(
        by_id.status.success(),
        "{}",
        String::from_utf8_lossy(&by_id.stderr)
    );
    assert!(
        String::from_utf8(by_id.stdout)
            .expect("utf8 output")
            .starts_with("worker s1 generic confine off\n")
    );

    let killed = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["kill", "--now", "--yes", "worker"],
    );
    assert!(
        killed.status.success(),
        "{}",
        String::from_utf8_lossy(&killed.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_reports_manual_grants_and_writes() {
    let (state, _daemon) = common::start_daemon().await;
    if !link_real_bwrap(state.path()) {
        eprintln!("skipping confined show test: bubblewrap unavailable");
        return;
    }
    let cwd = tempfile::tempdir().expect("cwd");
    let user_home = tempfile::tempdir().expect("home");
    let runtime = tempfile::tempdir().expect("runtime");
    let writable = cwd.path().join("write");
    std::fs::create_dir(&writable).expect("writable directory");
    let writable = writable.to_string_lossy().into_owned();
    let created = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &[
            "new",
            "--detach",
            "--name",
            "worker",
            "--confine",
            "--confine-grant",
            "cargo",
            "--confine-write",
            &writable,
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    );
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let shown = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show", "worker"],
    );
    assert!(
        shown.status.success(),
        "{}",
        String::from_utf8_lossy(&shown.stderr)
    );
    let output = String::from_utf8(shown.stdout).expect("utf8 output");
    assert!(
        output.contains("worker s1 generic confine manual"),
        "{output}"
    );
    assert!(output.contains("GRANT  SOURCE\ncargo  set\n"), "{output}");
    assert!(output.contains(&format!("write: {writable}\n")), "{output}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_reports_auto_reasons_and_explicit_grants() {
    let (state, _daemon) = common::start_daemon().await;
    if !link_real_bwrap(state.path()) {
        eprintln!("skipping auto confined show test: bubblewrap unavailable");
        return;
    }
    let project = tempfile::tempdir().expect("project");
    let user_home = tempfile::tempdir().expect("home");
    let runtime = tempfile::tempdir().expect("runtime");
    std::fs::create_dir(project.path().join(".git")).expect("git directory");
    std::fs::write(
        project.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\n",
    )
    .expect("cargo marker");
    std::fs::write(
        project.path().join(".git/config"),
        "[remote \"origin\"]\nurl = git@host-a:team/demo.git\n",
    )
    .expect("git config");
    std::fs::write(
        state.path().join("team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"sh\", \"-c\", \"sleep 30\"]\n[session.confine]\ngrants = [\"cargo\"]\n",
    )
    .expect("team file");
    let started = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "up", "--detach", "--yes"],
    );
    assert!(
        started.status.success(),
        "{}",
        String::from_utf8_lossy(&started.stderr)
    );
    let session_name = String::from_utf8_lossy(&started.stdout)
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .expect("started session name")
        .to_owned();
    let shown = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show", session_name.as_str()],
    );
    assert!(
        shown.status.success(),
        "started stdout: {}\nstarted stderr: {}\nshow stderr: {}",
        String::from_utf8_lossy(&started.stdout),
        String::from_utf8_lossy(&started.stderr),
        String::from_utf8_lossy(&shown.stderr)
    );
    let output = String::from_utf8(shown.stdout).expect("utf8 output");
    assert!(
        output.contains(&format!("{session_name} s1 generic confine auto")),
        "{output}"
    );
    assert!(output.contains("cargo  set\n"), "{output}");
    assert!(
        output.contains("cache  auto: always added by auto\n"),
        "{output}"
    );
    assert!(
        output.contains("ssh  auto: git remote uses ssh\n"),
        "{output}"
    );
    assert!(!output.contains("cargo  auto:"), "{output}");
    let _ = run_binary(
        state.path(),
        project.path(),
        user_home.path(),
        runtime.path(),
        &["team", "down", "--now"],
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn show_reports_no_sessions_and_unknown_reference() {
    let (state, _daemon) = common::start_daemon().await;
    let cwd = tempfile::tempdir().expect("cwd");
    let user_home = tempfile::tempdir().expect("home");
    let runtime = tempfile::tempdir().expect("runtime");

    let empty = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show"],
    );
    assert!(
        empty.status.success(),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    assert_eq!(empty.stdout, b"no sessions\n");

    let unknown = run_binary(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &["confine", "show", "missing"],
    );
    assert!(!unknown.status.success());
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unknown session missing"));
}
