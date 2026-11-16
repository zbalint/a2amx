//! Team formats and lifecycle through the public module and real daemon.

#[path = "../common/mod.rs"]
mod common;

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use common::pty::PtyHarness;

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::RemoteMode;
use a2amx::team::{self, TeamSession};
use a2amx::wire::{Request, Response, SessionSummary};

fn run_binary(home: &Path, args: &[&str]) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(home)
        .stdin(Stdio::null())
        .output()
}

fn run_binary_in_with_home_and_runtime(
    home: &Path,
    cwd: &Path,
    user_home: &Path,
    runtime_dir: &Path,
    args: &[&str],
) -> std::io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("--home")
        .arg(home)
        .args(args)
        .current_dir(cwd)
        .env("HOME", user_home)
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .stdin(Stdio::null())
        .output()
}

mod dry_run;
mod lifecycle;
mod parse_and_guards;
mod parse_sessions;
mod plan_and_status;
mod reset_and_validation;
mod team_config;
mod team_file_lookup;
