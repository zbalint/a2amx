#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::client::{BridgeLink, Client};
use a2amx::emulator::{Emulator, Size};
use a2amx::harness::{Deliver, Harness};
use a2amx::quota;
use a2amx::wire::{BridgeDown, BridgeUp, QuotaInfo, Request, Response, SessionSummary};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tempfile::tempdir;

async fn summary(admin: &mut Client, id: &str) -> SessionSummary {
    let Response::Sessions { sessions } = admin.request(Request::List).await.unwrap() else {
        panic!("list failed");
    };
    sessions.into_iter().find(|s| s.id == id).unwrap()
}

/// Waits until the session's status line has been drawn and read.
async fn quota_of(admin: &mut Client, id: &str) -> QuotaInfo {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(quota) = summary(admin, id).await.quota {
                return quota;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("status line is read")
}

mod agy_status;
mod claude_rate_limits;
mod daemon_limits;
mod omp_usage_daemon;
mod omp_usage_parsing;
mod quota_info;
mod screen_reading;
