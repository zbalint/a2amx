#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{BridgeDown, BridgeUp, ClientFrame, Request, Response};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn run_binary(home: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(home)
        .stdin(Stdio::null())
        .output()
}

fn run_binary_with_context(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime)
        .stdin(Stdio::null())
        .output()
}

fn bwrap_available() -> bool {
    matches!(
        Command::new("bwrap").arg("--version").output(),
        Ok(output) if output.status.success()
    )
}

fn start_process_daemon() -> (TempDir, TempDir, TempDir, common::DaemonProcess) {
    let state = tempfile::tempdir().expect("state directory");
    let user_home = tempfile::tempdir().expect("user home");
    let runtime = tempfile::tempdir().expect("runtime directory");
    let daemon = common::spawn_daemon_process(state.path(), user_home.path(), runtime.path());
    (state, user_home, runtime, daemon)
}

fn start_command(
    home: &Path,
    name: &str,
    harness: Option<&str>,
    deliver: Option<&str>,
    command: &[&str],
) {
    let mut args = vec!["new", "--detach", "--name", name];
    if let Some(harness) = harness {
        args.extend(["--harness", harness]);
    }
    if let Some(deliver) = deliver {
        args.extend(["--deliver", deliver]);
    }
    args.push("--");
    args.extend(command.iter().copied());
    let output = run_binary(home, &args).expect("start session");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn start_session(
    home: &Path,
    name: &str,
    harness: Option<&str>,
    deliver: Option<&str>,
    command: &str,
) {
    start_command(home, name, harness, deliver, &["sh", "-c", command]);
}

fn start_generic(home: &Path, name: &str, deliver: Option<&str>, command: &str) {
    start_session(home, name, Some("generic"), deliver, command);
}

fn start_inferred_claude(home: &Path, name: &str) -> PathBuf {
    let command = home.join("claude");
    std::fs::write(&command, "#!/bin/sh\nexec sleep 30\n").expect("write Claude stub");
    let mut permissions = std::fs::metadata(&command)
        .expect("Claude stub metadata")
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&command, permissions).expect("make Claude stub executable");
    let command_arg = command.to_string_lossy().into_owned();
    start_command(home, name, None, None, &[&command_arg]);
    command
}

fn explain_until_named<F>(home: &Path, name: &str, mut ready: F) -> String
where
    F: FnMut(&str) -> bool,
{
    // Above the daemon's ten second NEEDS_INPUT_AFTER delay: the `input?` label only appears after
    // the dialog signal has lasted that long, so a ten second deadline raced it.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let output = run_binary(home, &["explain", name]).expect("run explain");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() && ready(&stdout) {
            return stdout;
        }
        assert!(
            Instant::now() < deadline,
            "explain did not reach the expected state: {}{}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn explain_until<F>(home: &Path, ready: F) -> String
where
    F: FnMut(&str) -> bool,
{
    explain_until_named(home, "ready", ready)
}

fn bracketed_cat() -> &'static str {
    r"printf '\033[?2004h'; exec cat"
}

#[test]
fn ready_generic_explain_reports_state_and_open_reset_gate() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_generic(state.path(), "ready", Some("auto"), bracketed_cat());

    let output = explain_until(state.path(), |text| {
        text.contains("ready: yes\n") && text.contains("reset: gate open\n")
    });
    assert!(output.lines().any(|line| line.starts_with("activity: ")));
    assert!(output.contains("needs input: no\n"));
    assert!(output.contains("hold: none\n"));
    assert!(output.contains("queued: 0\n"));
}

#[test]
fn deliver_hold_explain_reports_hold_explanation_without_gating_reset() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_generic(state.path(), "ready", Some("hold"), bracketed_cat());

    let output = explain_until(state.path(), |text| text.contains("ready: yes\n"));
    assert!(output.contains(
        "hold: deliver_hold - The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery.\n"
    ));
    assert!(output.contains("reset: gate open\n"));
}

#[test]
fn non_ready_generic_explain_reports_screen_readiness_gate() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_generic(state.path(), "ready", None, "exec cat");

    let output = explain_until(state.path(), |text| {
        text.contains("ready: no\n")
            && text.contains("reset: refused (not_ready): screen not ready\n")
    });
    assert!(output.contains("activity: working (screen not ready)\n"));
}

#[test]
fn non_ready_claude_explain_reports_starting_latch() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    let _command = start_inferred_claude(state.path(), "starting");

    let output = explain_until_named(state.path(), "starting", |text| {
        text.contains("ready: no (starting)\n")
    });
    assert!(output.contains("starting@"));
}

#[test]
fn claude_explain_reports_needs_input_reason() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_session(
        state.path(),
        "input",
        Some("claude"),
        Some("auto"),
        r#"printf '\033[2J\033[H Enter to confirm · Esc to cancel'; exec sleep 30"#,
    );

    let output = explain_until_named(state.path(), "input", |text| {
        text.contains("needs input: input?\n")
    });
    assert!(output.contains("needs input: input?\n"));
}

#[test]
fn exited_confined_explain_reports_denial_help() {
    if !bwrap_available() {
        eprintln!("skipping confined explain test: bubblewrap unavailable");
        return;
    }
    let (state, user_home, runtime, _daemon) = start_process_daemon();
    let cwd = tempfile::tempdir().expect("working directory");
    let created = run_binary_with_context(
        state.path(),
        cwd.path(),
        user_home.path(),
        runtime.path(),
        &[
            "new",
            "--detach",
            "--name",
            "confined",
            "--confine",
            "--",
            "sh",
            "-c",
            "exit 7",
        ],
    )
    .expect("start confined session");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run_binary_with_context(
            state.path(),
            cwd.path(),
            user_home.path(),
            runtime.path(),
            &["explain", "confined"],
        )
        .expect("run explain");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if output.status.success() && stdout.contains("exited (code 7)") {
            let mut lines = stdout.lines();
            assert!(
                lines
                    .next()
                    .is_some_and(|line| line.contains("exited (code 7)"))
            );
            assert!(
                lines
                    .next()
                    .is_some_and(|line| line.starts_with("confinement: manual; grants:"))
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "explain did not report exited confined session: {}{}",
            stdout,
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn exited_explain_reports_only_the_exited_state_line() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_generic(state.path(), "ready", None, "exit 0");

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run_binary(state.path(), &["explain", "ready"]).expect("run explain");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        if output.status.success() && stdout.contains("generic, exited (code 0)") {
            assert_eq!(stdout.lines().count(), 1, "{stdout}");
            break;
        }
        assert!(Instant::now() < deadline, "{stdout}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn unknown_explain_reference_is_rejected_by_the_daemon() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    let output = run_binary(state.path(), &["explain", "nobody"]).expect("run explain");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("unknown session nobody"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explain_does_not_change_stable_list_state() {
    let (state, _user_home, _runtime, _daemon) = start_process_daemon();
    start_generic(state.path(), "ready", Some("auto"), bracketed_cat());
    let _ = explain_until(state.path(), |text| text.contains("ready: yes\n"));

    let before = run_binary(state.path(), &["list"]).expect("list before");
    let _ = run_binary(state.path(), &["explain", "ready"]).expect("explain");
    let after = run_binary(state.path(), &["list"]).expect("list after");
    assert!(before.status.success());
    assert!(after.status.success());
    let before_text = String::from_utf8(before.stdout).unwrap();
    let after_text = String::from_utf8(after.stdout).unwrap();
    let before = common::list_cells(&before_text, "s1");
    let after = common::list_cells(&after_text, "s1");
    for index in [4, 9, 10] {
        assert_eq!(before[index], after[index], "list column {index}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_token_cannot_explain() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_session, token, address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("worker"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut session = Client::connect_addr(address, &token).await.unwrap();
    let response = session
        .request(Request::Explain {
            session: "s1".into(),
        })
        .await
        .unwrap();
    assert_eq!(
        response,
        Response::Error {
            message: "not permitted for a session token".into()
        }
    );
    daemon.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stale_omp_draft_preview_does_not_clear_the_hold() {
    let (dir, daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.unwrap();
    let (_controller, _controller_token, _controller_address) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("controller"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let output = dir.path().join("target.credentials");
    let target = match admin
        .request(Request::NewSession {
            argv: vec![
                "sh".into(),
                "-c".into(),
                "printf '%s\\n%s\\n' \"$A2AMX_TOKEN\" \"$A2AMX_ADDR\" > \"$OUT\"; printf '\\033[?2004h'; exec cat".into(),
            ],
            cols: 40,
            rows: 10,
            cwd: None,
            env: vec![("OUT".into(), output.to_string_lossy().into_owned())],
            reset: vec![],
            control_from: vec!["controller".into()],
            watch: vec![],
            activity: None,
            heartbeat: None,
            name: Some("target".into()),
            harness: Harness::Omp,
            deliver: Some(Deliver::Auto),
            team: None,
            role: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected target response: {response:?}"),
    };
    let (target_token, target_address) = common::eventually(|| async {
        let text = std::fs::read_to_string(&output).ok()?;
        let mut lines = text.lines();
        let token = lines.next()?.to_owned();
        let address = lines.next()?.parse().ok()?;
        (token.len() == 64).then_some((token, address))
    })
    .await;
    let mut bridge = Client::connect_addr(target_address, &target_token)
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
    bridge
        .send(BridgeUp::State {
            idle: true,
            pending: false,
            draft: false,
            queue: None,
            host_pending: None,
            provider: None,
        })
        .await
        .unwrap();

    let mut attachment = Client::connect(dir.path())
        .await
        .unwrap()
        .attach(&target, false, 40, 10)
        .await
        .unwrap();
    attachment
        .send(ClientFrame::Input(b"x".to_vec()))
        .await
        .unwrap();
    let home = dir.path().to_owned();
    let target_id = target.clone();
    common::eventually(|| {
        let home = home.clone();
        let target = target_id.clone();
        async move {
            let mut client = Client::connect(&home).await.ok()?;
            let response = client
                .request(Request::Explain { session: target })
                .await
                .ok()?;
            let Response::Explain { info } = response else {
                return None;
            };
            (info.reset.code.as_deref() == Some("draft_present")
                && info.reset.reason.as_deref() == Some("hold human_draft"))
            .then_some(())
        }
    })
    .await;
    tokio::time::sleep(Duration::from_millis(2200)).await;

    for _ in 0..2 {
        let response = admin
            .request(Request::Explain {
                session: target.clone(),
            })
            .await
            .unwrap();
        let Response::Explain { info } = response else {
            panic!("unexpected explain response: {response:?}");
        };
        assert!(info.reset.allowed, "{info:?}");
    }
    daemon.shutdown().await.unwrap();
}
