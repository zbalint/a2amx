#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::daemon::{Daemon, DaemonConfig};
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{HUMAN_SESSION, HUMAN_SUBJECT, Limits, RemoteMode, TeamScope, code};
use a2amx::wire::{Request, Response};
use common::{
    create_confined, create_confined_with_team, create_confined_with_token, error_message,
    failed_code, failed_message, gateway, issue_and_redeem, issue_code, limits_with,
    new_session_request, new_session_request_with_remote, start_confined_daemon,
    start_confined_daemon_with_limits, team_scope,
};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use std::path::Path;
use std::process::{Command, Output};
mod gateway_capability;
mod inbound_admission;
mod outbound;
mod rate_limits;
mod remote_mode;
