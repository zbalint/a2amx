//! Session references and list output through the real CLI and daemon.

mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{Request, Response};
use common::pty::PtyHarness;

fn cli_args(home: &Path, command: &[&str]) -> Vec<String> {
    let mut args = vec!["--home".to_owned(), home.to_string_lossy().into_owned()];
    args.extend(command.iter().map(|arg| (*arg).to_owned()));
    args
}

fn run_binary(home: &Path, command: &[&str]) -> anyhow::Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(cli_args(home, command))
        .current_dir(home)
        .stdin(Stdio::null())
        .output()?)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_by_name_removes_the_named_session() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");

    let killed = run_binary(dir.path(), &["kill", "agent-plan", "--yes"])?;
    assert!(
        killed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&killed.stderr)
    );
    let listed = run_binary(dir.path(), &["list"])?;
    assert!(listed.status.success());
    assert!(!String::from_utf8(listed.stdout)?.contains("agent-plan"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_exited_removes_only_exited_sessions_and_frees_names() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    for (name, command) in [
        ("agent-plan", "exit 0"),
        ("agent-review", "exit 7"),
        ("agent-live", "sleep 30"),
    ] {
        let created = run_binary(
            dir.path(),
            &["new", "--detach", "--name", name, "--", "sh", "-c", command],
        )?;
        assert!(
            created.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&created.stderr)
        );
    }
    common::eventually(|| async {
        let listed = run_binary(dir.path(), &["list"]).ok()?;
        let listing = String::from_utf8(listed.stdout).ok()?;
        (listing
            .lines()
            .filter(|line| line.contains("exited("))
            .count()
            == 2)
            .then_some(())
    })
    .await;

    let removed = run_binary(dir.path(), &["kill", "--exited"])?;
    assert!(
        removed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&removed.stderr)
    );
    assert!(removed.stderr.is_empty());
    let output = String::from_utf8(removed.stdout)?;
    let mut lines: Vec<_> = output.lines().collect();
    lines.sort_unstable();
    assert_eq!(lines, ["removed agent-plan", "removed agent-review"]);

    let listed = run_binary(dir.path(), &["list"])?;
    assert!(listed.status.success());
    let listing = String::from_utf8(listed.stdout)?;
    let rows: Vec<Vec<_>> = listing
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().take(4).collect())
        .collect();
    assert_eq!(rows, [["s3", "agent-live", "generic", "running"]]);
    let reused = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    )?;
    assert!(
        reused.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&reused.stderr)
    );
    assert_eq!(reused.stdout, b"s4\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kill_exited_reports_ids_and_handles_empty_selection() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(dir.path(), &["new", "--detach", "--", "sh", "-c", "exit 0"])?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");
    common::eventually(|| async {
        let listed = run_binary(dir.path(), &["list"]).ok()?;
        let listing = String::from_utf8(listed.stdout).ok()?;
        listing
            .lines()
            .any(|line| line.starts_with("s1") && line.contains("exited(0)"))
            .then_some(())
    })
    .await;

    let removed = run_binary(dir.path(), &["kill", "--exited"])?;
    assert!(removed.status.success());
    assert_eq!(removed.stdout, b"removed s1\n");
    assert!(removed.stderr.is_empty());

    let running = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-live",
            "--",
            "sh",
            "-c",
            "sleep 30",
        ],
    )?;
    assert!(running.status.success());
    let empty = run_binary(dir.path(), &["kill", "--exited"])?;
    assert!(empty.status.success());
    assert_eq!(empty.stdout, b"no exited sessions\n");
    assert!(empty.stderr.is_empty());
    Ok(())
}

#[test]
fn kill_exited_selector_validation() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    for args in [
        &["kill"][..],
        &["kill", "agent-plan", "--exited"][..],
        &["kill", "--exited", "--yes"][..],
        &["kill", "--exited", "--now"][..],
    ] {
        let output = run_binary(dir.path(), args)?;
        assert!(!output.status.success(), "accepted invalid args: {args:?}");
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn attach_by_name_uses_the_session_id_for_detachment() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let created = run_binary(
        dir.path(),
        &[
            "new",
            "--detach",
            "--name",
            "agent-plan",
            "--",
            "sh",
            "-c",
            "printf ready; cat",
        ],
    )?;
    assert!(created.status.success());
    assert_eq!(created.stdout, b"s1\n");
    for reference in ["agent-plan", "s1"] {
        let mut attached =
            PtyHarness::spawn(&cli_args(dir.path(), &["attach", reference]), dir.path())?;
        attached.wait_for_text("ready", Duration::from_secs(10))?;
        attached.send(&[2, b'd'])?;
        attached.wait_for_text("[detached from s1]", Duration::from_secs(10))?;
        assert_eq!(attached.wait_exit(Duration::from_secs(10))?, 0);
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_by_name_match_id_and_retained_messages_survive_kill() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await?;
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-send"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (recipient, _, _) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let mut sender = Client::connect_addr(addr, &token).await?;
    assert!(matches!(
        sender
            .request(Request::SendMessage {
                to: "agent-plan@host-a".into(),
                subject: "Parser issue".into(),
                message: "A retained message.".into(),
            })
            .await?,
        Response::Accepted { .. }
    ));
    let by_id = run_binary(dir.path(), &["messages", "--session", &recipient])?;
    let by_name = run_binary(dir.path(), &["messages", "--session", "agent-plan"])?;
    assert!(by_id.status.success() && by_name.status.success());
    assert!(String::from_utf8_lossy(&by_id.stdout).contains("Parser issue"));
    assert_eq!(by_name.stdout, by_id.stdout);
    assert!(
        run_binary(dir.path(), &["kill", "agent-plan", "--yes"])?
            .status
            .success()
    );
    let retained = run_binary(dir.path(), &["messages", "--session", &recipient])?;
    assert!(retained.status.success());
    assert!(String::from_utf8(retained.stdout)?.contains("Parser issue"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unknown_reference_keeps_the_daemon_error() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let output = run_binary(dir.path(), &["kill", "nobody"])?;
    assert!(!output.status.success());
    assert!(String::from_utf8(output.stderr)?.contains("unknown session nobody"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn default_list_is_lean_and_details_show_cwd_and_command() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    assert!(
        run_binary(
            dir.path(),
            &[
                "new",
                "--detach",
                "--name",
                "agent-plan",
                "--",
                "sh",
                "-c",
                "sleep 30"
            ],
        )?
        .status
        .success()
    );
    let plain = run_binary(dir.path(), &["list"])?;
    assert!(plain.status.success());
    let plain = String::from_utf8(plain.stdout)?;
    assert_eq!(
        plain.lines().next(),
        Some("ID  NAME        HARNESS  STATE    ACTIVITY  ATTACHED  PENDING  HELD  QUOTA  SIZE"),
    );
    assert!(plain.contains(
        "s1  agent-plan  generic  running  working   no        0        -     -      80x24\n"
    ));
    assert!(!plain.contains("sleep 30"));
    assert!(!plain.contains(&dir.path().to_string_lossy().into_owned()));
    let details = run_binary(dir.path(), &["list", "--details"])?;
    assert!(details.status.success());
    let details = String::from_utf8(details.stdout)?;
    assert!(
        details
            .lines()
            .next()
            .is_some_and(|line| line.contains("CWD") && line.ends_with("COMMAND"))
    );
    assert!(details.contains(&format!("{}  sh -c sleep 30\n", dir.path().display())));
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_reports_explicit_omp_and_unnamed_generic_harnesses() -> anyhow::Result<()> {
    let (dir, _daemon) = common::start_daemon().await;
    let omp = run_binary(
        dir.path(),
        &["new", "--detach", "--harness", "omp", "--", "sh"],
    )?;
    assert!(omp.status.success());
    let generic = run_binary(dir.path(), &["new", "--detach", "--", "sh"])?;
    assert!(generic.status.success());
    let output = run_binary(dir.path(), &["list"])?;
    assert!(output.status.success());
    let listing = String::from_utf8(output.stdout)?;
    let cells: Vec<Vec<&str>> = listing
        .lines()
        .skip(1)
        .map(|line| line.split_whitespace().collect())
        .collect();
    assert_eq!(&cells[0][..3], ["s1", "-", "omp"]);
    assert_eq!(&cells[1][..3], ["s2", "-", "generic"]);
    Ok(())
}
