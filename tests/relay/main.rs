#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::messaging::RemoteMode;
use a2amx::relay::{Io, pair_only, run};
use a2amx::relay_files::{self, Loaded};
use a2amx::wire::{Request, Response};
use common::eventually;
use common::fake_discord::FakeDiscord;
use common::start_daemon;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tempfile::tempdir;

// A relay mutation can leave the run task in a state where it never settles; awaiting
// it directly would hang instead of failing. Every direct await of the relay task is
// wrapped in this helper so the suite reports a failure in bounded time.
async fn settled(task: tokio::task::JoinHandle<anyhow::Result<()>>) -> anyhow::Result<()> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), task)
        .await
        .expect("relay task must settle");
    result.expect("relay task finished without panic")
}
async fn kill_and_wait(child: std::process::Child) -> std::process::Output {
    tokio::task::spawn_blocking(move || {
        let mut child = child;
        child.kill().unwrap();
        child.wait_with_output().unwrap()
    })
    .await
    .unwrap()
}

fn write_file(path: &std::path::Path, text: &str, mode: u32) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn valid_gateway(home: &std::path::Path) {
    let layout = a2amx::layout::Layout::new(home.to_owned());
    layout.ensure().unwrap();
    write_file(&layout.discord_token(), "bot-token\n", 0o600);
    write_file(&layout.gateway_config(), "guild_id = \"123\"\n", 0o600);
}
async fn wait_for_member_checks(discord: &FakeDiscord, baseline: u64) {
    eventually(|| async { (discord.member_count_requests() >= baseline + 2).then_some(()) }).await;
}
mod config_loading;
mod gateway_token_and_client;
mod outbound_and_reconnect;
mod pairing;
mod sync_and_members;
