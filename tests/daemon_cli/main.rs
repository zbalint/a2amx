#![allow(clippy::expect_used, clippy::unwrap_used)]
#[path = "../common/mod.rs"]
mod common;
use common::pty::PtyHarness;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_a2amx");

fn a2amx(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(BIN);
    command.arg("--home").arg(home).args(args);
    command
}

fn run(home: &Path, args: &[&str]) -> Output {
    a2amx(home, args)
        .stdin(Stdio::null())
        .output()
        .expect("run a2amx")
}

fn run_owned(home: &Path, args: &[String]) -> Output {
    a2amx(home, &[])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("run a2amx")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn screen_contains_until(home: &Path, args: &[&str], expected: &str) -> Output {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let output = run(home, args);
        if !output.status.success()
            || stdout(&output).contains(expected)
            || Instant::now() >= deadline
        {
            return output;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Ends any daemon left in the state dir, so a failing test leaves no process.
struct Cleanup<'a>(&'a Path);

impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = run(self.0, &["daemon", "stop", "--yes"]);
    }
}

mod daemon_lifecycle;
mod heartbeat_and_activity;
mod kill_and_down;
mod peer_exit_watch;
mod reset;
mod screen;
mod state_layout;
