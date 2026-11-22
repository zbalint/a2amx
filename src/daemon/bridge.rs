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
async fn bridge(
    connection: &mut Framed,
    runtime: &Arc<Runtime>,
    session: Arc<Session>,
    mut guard: NativeGuard,
) -> anyhow::Result<()> {
    let first = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let Ok(Ok(Some(bytes))) = first else {
        return Ok(());
    };
    let Ok(BridgeUp::Hello {
        protocol,
        omp_version,
        missing,
    }) = serde_json::from_slice(&bytes)
    else {
        return Ok(());
    };
    let refusal = if protocol != BRIDGE_PROTOCOL {
        Some("protocol_mismatch".to_owned())
    } else if !missing.is_empty() {
        Some(format!("missing_apis: {}", missing.join(", ")))
    } else {
        None
    };
    if let Some(reason) = refusal {
        session.native_refuse();
        connection
            .send(&serde_json::to_vec(&BridgeDown::Refused { reason })?)
            .await?;
        return Ok(());
    }
    session.native_accept();
    connection
        .send(&serde_json::to_vec(&BridgeDown::Ready)?)
        .await?;
    tracing::info!(%omp_version, session = %session.id().0, "native bridge accepted");
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            bytes = connection.recv() => {
                let Some(bytes) = bytes? else {
                    return Ok(());
                };
                let Ok(frame) = serde_json::from_slice::<BridgeUp>(&bytes) else {
                    return Ok(());
                };
                match frame {
                    BridgeUp::Hello { .. } => return Ok(()),
                    BridgeUp::State {
                        idle,
                        draft,
                        pending,
                        queue,
                        host_pending,
                        provider,
                    } => {
                        // OMP holds next-turn reminders while idle; an empty extension queue means a2amx has nothing in flight.
                        let waiting_for_turn =
                            idle && queue == Some(0) && host_pending == Some(true);
                        session.native_state(
                            idle,
                            draft,
                            pending && !waiting_for_turn,
                            provider,
                        );
                        session.native_diagnostics(queue, host_pending);
                    }
                    BridgeUp::Ack { id } => session.native_ack(&id, Ok(())),
                    BridgeUp::Nack { id, reason } => session.native_ack(&id, Err(reason)),
                    BridgeUp::Receipt { id } => {
                        if record_receipt(&runtime.store, &runtime.boot, &session, &id).await? {
                            session.native_received();
                        }
                    }
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
