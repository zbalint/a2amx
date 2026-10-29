#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::env;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_a2amx");
const REFUSAL: &str = "refusing to run as root";
const CHILD_TIMEOUT: Duration = Duration::from_secs(10);

static USERNS_AVAILABLE: OnceLock<bool> = OnceLock::new();

fn path_env() -> Option<std::ffi::OsString> {
    env::var_os("PATH")
}

fn userns_available() -> bool {
    *USERNS_AVAILABLE.get_or_init(|| {
        let output = run_command("unshare", &["-Ur", "true"]);
        match output {
            Ok(output) if output.status.success() => true,
            Ok(output) => {
                if env::var_os("A2AMX_REQUIRE_USERNS").as_deref() == Some("1".as_ref()) {
                    panic!("unshare -Ur true exited with {}", output.status);
                }
                eprintln!(
                    "privilege tests: skipped user-namespace cases (unshare -Ur true exited with {})",
                    output.status
                );
                false
            }
            Err(error) => {
                if env::var_os("A2AMX_REQUIRE_USERNS").as_deref() == Some("1".as_ref()) {
                    panic!("unshare -Ur true could not run: {error}");
                }
                eprintln!(
                    "privilege tests: skipped user-namespace cases (unshare -Ur true failed: {error})"
                );
                false
            }
        }
    })
}
fn run_command(binary: &str, args: &[&str]) -> std::io::Result<Output> {
    let mut command = Command::new(binary);
    command
        .args(args)
        .env_clear()
        .envs(path_env().into_iter().map(|path| ("PATH", path)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let deadline = Instant::now() + CHILD_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("poll child") {
            let output = child.wait_with_output().expect("collect command output");
            assert_eq!(output.status, status);
            return Ok(output);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("command timed out after {CHILD_TIMEOUT:?}: {binary} {args:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_root(home: &Path, args: &[&str]) -> Output {
    let home = home.to_string_lossy();
    let mut unshare_args = vec!["-Ur", BIN, "--home", &home];
    unshare_args.extend(args);
    run_command("unshare", &unshare_args).expect("run root command")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn root_commands_are_refused() {
    if !userns_available() {
        return;
    }

    let cases = [
        ("list", ["list"].as_slice(), true),
        ("daemon status", ["daemon", "status"].as_slice(), true),
        ("daemon start", ["daemon", "start"].as_slice(), true),
        ("mcp", ["mcp"].as_slice(), false),
        ("hook", ["hook"].as_slice(), false),
    ];
    let mut failures = Vec::new();
    for (name, args, assert_empty) in cases {
        let directory = tempfile::tempdir().expect("temporary home");
        let output = run_root(directory.path(), args);
        if name == "daemon start" && output.status.success() {
            let cleanup = run_root(directory.path(), &["daemon", "stop", "--yes"]);
            if !cleanup.status.success() {
                failures.push(format!("daemon start cleanup failed: {}", stderr(&cleanup)));
            }
        }
        if output.status.code() != Some(1) {
            failures.push(format!(
                "{name}: expected exit 1, got {:?}",
                output.status.code()
            ));
        }
        if !stderr(&output).contains(REFUSAL) {
            failures.push(format!("{name}: stderr was {:?}", stderr(&output)));
        }
        if assert_empty {
            let entries = fs::read_dir(directory.path())
                .expect("read temporary home")
                .collect::<Result<Vec<_>, _>>()
                .expect("read temporary home entries");
            if !entries.is_empty() {
                failures.push(format!("{name}: home was not empty: {entries:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "root refusal failures: {failures:#?}");
}

#[test]
fn root_version_remains_available() {
    if !userns_available() {
        return;
    }
    let directory = tempfile::tempdir().expect("temporary home");
    let output = run_root(directory.path(), &["--version"]);
    assert!(output.status.success(), "stderr: {}", stderr(&output));
    assert!(!output.stdout.is_empty());
}

#[test]
fn ordinary_user_list_reports_unreachable_daemon_without_root_refusal() {
    let directory = tempfile::tempdir().expect("temporary home");
    let output = run_command(
        BIN,
        &[
            "--home",
            directory.path().to_str().expect("temporary home path"),
            "list",
        ],
    )
    .expect("run ordinary-user command");
    let error = stderr(&output);
    assert!(!output.status.success());
    assert!(
        error.contains("cannot reach the a2amx daemon"),
        "stderr: {error}"
    );
    assert!(!error.contains(REFUSAL), "stderr: {error}");
}

#[test]
fn effective_uid_zero_is_rejected() {
    let error = a2amx::privilege::ensure_not_root_for(0).expect_err("root must be refused");
    assert!(error.to_string().contains(REFUSAL));
    assert!(a2amx::privilege::ensure_not_root_for(1000).is_ok());
    assert!(a2amx::privilege::ensure_not_root_for(1).is_ok());
}
