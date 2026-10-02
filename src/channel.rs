//! Relay daemon deliveries into Claude Code channel notifications.

use std::net::SocketAddr;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use crate::client::Client;
use crate::wire::{BRIDGE_PROTOCOL, BridgeDown, BridgeUp};

pub(crate) struct Delivery {
    pub notification: Value,
    pub written: oneshot::Sender<bool>,
}

pub(crate) async fn run(
    address: SocketAddr,
    token: String,
    client_version: Option<String>,
    out: mpsc::UnboundedSender<Delivery>,
) -> Result<()> {
    let mut link = Client::connect_addr(address, &token)
        .await?
        .bridge()
        .await?;
    // shortcut: omp_version predates this use; rename it with the next protocol bump.
    link.send(BridgeUp::Hello {
        protocol: BRIDGE_PROTOCOL,
        omp_version: format!(
            "claude-code {}",
            client_version.as_deref().unwrap_or("unknown")
        ),
        missing: vec![],
    })
    .await?;
    link.send(BridgeUp::State {
        idle: true,
        pending: false,
        draft: false,
    })
    .await?;

    // shortcut: no reconnect; a lost link leaves channel_down until restart. Add
    // bounded reconnect if daemon-link loss occurs in practice (sessions die with it today).
    while let Some(frame) = link.recv().await? {
        match frame {
            BridgeDown::Ready => {}
            BridgeDown::Refused { reason } => {
                eprintln!("a2amx: channel refused: {reason}");
                return Ok(());
            }
            BridgeDown::Deliver { id, envelope } => {
                let (written, result) = oneshot::channel();
                let sent = out.send(Delivery {
                    notification: json!({"jsonrpc":"2.0","method":"notifications/claude/channel","params":{"content":envelope}}),
                    written,
                }).is_ok();
                if sent && result.await.unwrap_or(false) {
                    link.send(BridgeUp::Ack { id }).await?;
                    link.send(BridgeUp::State {
                        idle: true,
                        pending: false,
                        draft: false,
                    })
                    .await?;
                } else {
                    link.send(BridgeUp::Nack {
                        id,
                        reason: "write_failed".into(),
                    })
                    .await?;
                }
            }
        }
    }
    Ok(())
}
