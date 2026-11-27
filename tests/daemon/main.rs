// Shared test helpers are not themselves annotated with #[test].
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "../common/mod.rs"]
mod common;

use std::os::unix::fs::{PermissionsExt, symlink};

use a2amx::daemon::{Daemon, DaemonConfig};
use std::time::Duration;

use a2amx::client::{Attachment, Client};
use a2amx::emulator::{Emulator, Scroll, Size};
use a2amx::messaging::{Limits, TeamScope};
use a2amx::wire::{ClientFrame, Request, Response, ServerFrame, SessionSummary, encode_frame};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

async fn create(client: &mut Client, argv: &[&str], env: Vec<(String, String)>) -> String {
    match client
        .request(Request::NewSession {
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            cols: 40,
            rows: 5,
            cwd: None,
            env,
            reset: vec![],
            control_from: vec![],
            watch: vec![],
            name: None,
            harness: a2amx::harness::Harness::Generic,
            deliver: None,
            heartbeat: None,
            activity: None,
            team: None,
            role: None,
            remote: None,
        })
        .await
        .unwrap()
    {
        Response::Created { session } => session,
        response => panic!("unexpected creation response: {response:?}"),
    }
}

async fn list(client: &mut Client) -> Vec<SessionSummary> {
    match client.request(Request::List).await.unwrap() {
        Response::Sessions { sessions } => sessions,
        response => panic!("unexpected list response: {response:?}"),
    }
}

async fn eventually_sessions(
    client: &mut Client,
    predicate: impl Fn(&[SessionSummary]) -> bool,
) -> Vec<SessionSummary> {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let sessions = list(client).await;
            if predicate(&sessions) {
                return sessions;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("session condition within deadline")
}
fn text(emulator: &Emulator) -> String {
    emulator.screen().cells.iter().map(|cell| cell.ch).collect()
}

async fn next(attachment: &mut Attachment) -> ServerFrame {
    tokio::time::timeout(Duration::from_secs(10), attachment.recv())
        .await
        .expect("stream deadline")
        .unwrap()
        .expect("stream frame")
}

async fn wait_text(attachment: &mut Attachment, emulator: &mut Emulator, expected: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !text(emulator).contains(expected) {
            match attachment.recv().await.unwrap().expect("data before close") {
                ServerFrame::Data(bytes) => {
                    emulator.feed(&bytes);
                }
                frame => panic!("expected text before {frame:?}"),
            }
        }
    })
    .await
    .expect("screen text within deadline");
}

async fn raw_response(stream: &mut TcpStream) -> Response {
    let size = stream.read_u32().await.unwrap() as usize;
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

mod attach_streams;
mod confine;
mod daemon_lifecycle;
mod serve_dispatch_pins;
mod serve_protocol;
mod session_creation;
mod status_reporting;
