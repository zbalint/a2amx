#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Automatic grant detection through the public team and CLI seams.

#[path = "../common/mod.rs"]
mod common;

use common::pty::PtyHarness;
use common::start_daemon;
use std::os::unix::ffi::OsStringExt;
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

fn global_auto_file(state: &Path) {
    std::fs::write(
        state.join("config/team.toml"),
        "[confine]\nenabled = true\nauto = true\n[[session]]\nname = \"worker\"\ncommand = [\"cat\"]\n",
    )
    .unwrap();
}
mod consent_prompts;
mod daemon_use;
mod dry_run_reports;
mod git_config_limits;
mod git_detection;
