use std::sync::Arc;
use std::time::Duration;

use super::trust::session_id;
use super::{Role, Runtime, lookup, record_receipt, response};
use crate::client::Framed;
use crate::harness::Harness;
use crate::session::{NativeGuard, Session};
use crate::wire::{BRIDGE_PROTOCOL, BridgeDown, BridgeUp, Response};

pub(super) async fn bridge_attach(
    connection: &mut Framed,
    runtime: &Arc<Runtime>,
    role: &Role,
) -> anyhow::Result<Option<Response>> {
    let id = match session_id(role, "bridge_attach") {
        Ok(id) => id,
        Err(message) => return Ok(Some(Response::Error { message })),
    };
    let Some((_, session)) = lookup(runtime, id) else {
        anyhow::bail!("bridge session disappeared");
    };
    if session.harness() != Harness::Omp && !session.channel() {
        Ok(Some(Response::Error {
            message: "session is not an omp session".into(),
        }))
    } else if let Some(guard) = session.native_attach() {
        response(connection, Response::Attached).await?;
        bridge(connection, runtime, session, guard).await?;
        Ok(None)
    } else {
        Ok(Some(Response::Error {
            message: "a bridge is already attached to this session".into(),
        }))
    }
}

fn hello_refusal(protocol: u32, missing: &[String]) -> Option<String> {
    if protocol != BRIDGE_PROTOCOL {
        Some("protocol_mismatch".to_owned())
    } else if !missing.is_empty() {
        Some(format!("missing_apis: {}", missing.join(", ")))
    } else {
        None
    }
}

async fn bridge_handshake(
    connection: &mut Framed,
    session: &Session,
) -> anyhow::Result<Option<String>> {
    let first = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let Ok(Ok(Some(bytes))) = first else {
        return Ok(None);
    };
    let Ok(BridgeUp::Hello {
        protocol,
        omp_version,
        missing,
    }) = serde_json::from_slice(&bytes)
    else {
        return Ok(None);
    };
    if let Some(reason) = hello_refusal(protocol, &missing) {
        session.native_refuse();
        connection
            .send(&serde_json::to_vec(&BridgeDown::Refused { reason })?)
            .await?;
        return Ok(None);
    }
    session.native_accept();
    connection
        .send(&serde_json::to_vec(&BridgeDown::Ready)?)
        .await?;
    Ok(Some(omp_version))
}

async fn bridge_frame(
    runtime: &Arc<Runtime>,
    session: &Session,
    bytes: &[u8],
) -> anyhow::Result<bool> {
    let Ok(frame) = serde_json::from_slice::<BridgeUp>(bytes) else {
        return Ok(false);
    };
    match frame {
        BridgeUp::Hello { .. } => return Ok(false),
        BridgeUp::State {
            idle,
            draft,
            pending,
            queue,
            host_pending,
            provider,
        } => {
            // OMP holds next-turn reminders while idle; an empty extension queue means a2amx has nothing in flight.
            let waiting_for_turn = idle && queue == Some(0) && host_pending == Some(true);
            session.native_state(idle, draft, pending && !waiting_for_turn, provider);
            session.native_diagnostics(queue, host_pending);
        }
        BridgeUp::Ack { id } => session.native_ack(&id, Ok(())),
        BridgeUp::Nack { id, reason } => session.native_ack(&id, Err(reason)),
        BridgeUp::Receipt { id } => {
            if record_receipt(&runtime.store, &runtime.boot, session, &id).await? {
                session.native_received();
            }
        }
    }
    Ok(true)
}

async fn bridge(
    connection: &mut Framed,
    runtime: &Arc<Runtime>,
    session: Arc<Session>,
    mut guard: NativeGuard,
) -> anyhow::Result<()> {
    let Some(omp_version) = bridge_handshake(connection, &session).await? else {
        return Ok(());
    };
    tracing::info!(%omp_version, session = %session.id().0, "native bridge accepted");
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            bytes = connection.recv() => {
                let Some(bytes) = bytes? else {
                    return Ok(());
                };
                if !bridge_frame(runtime, &session, &bytes).await? {
                    return Ok(());
                }
            }
            frame = guard.down.recv() => {
                let Some(frame) = frame else {
                    return Ok(());
                };
                connection.send(&serde_json::to_vec(&frame)?).await?;
            }
            _ = interval.tick() => {
                if session.exit_code().is_some() {
                    return Ok(());
                }
            }
        }
    }
}
