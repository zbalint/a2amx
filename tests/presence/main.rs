#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::client::Client;
use a2amx::confine::ConfineRequest;
use a2amx::emulator::Scroll;
use a2amx::harness::{Deliver, Harness};
use a2amx::messaging::{self, Limits, RemoteMode, code};
use a2amx::wire::{ClientFrame, RemoteLease, RemoteStatus, Request, Response, ServerFrame};
use common::{
    create_confined_with_token, gateway, issue_and_redeem, new_session_request,
    start_confined_daemon, start_confined_daemon_with_limits, start_daemon, team_scope,
};
use std::time::{Duration, Instant};

async fn owner_state(client: &mut Client) -> Option<String> {
    match client.request(Request::ListAgents).await.unwrap() {
        Response::Agents { owner, .. } => owner,
        response => panic!("unexpected list_agents response: {response:?}"),
    }
}

async fn receive_data(attachment: &mut a2amx::client::Attachment) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match attachment.recv().await.unwrap() {
                Some(ServerFrame::Data(_)) => break,
                Some(_) => {}
                None => panic!("viewer attachment closed"),
            }
        }
    })
    .await
    .expect("viewer receives redraw data");
}

mod away_notice;
mod owner_presence;
mod presence_clock;
mod remote_status;
mod viewers;
