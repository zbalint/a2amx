use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

use crate::client::Framed;
use crate::emulator::Size;
use crate::wire::{Request, Response};

use super::admission::cancel_message;
use super::attach::attach;
use super::bridge::bridge_attach;
use super::launch::kill_session;
use super::trust::request_permitted;
use super::views::{explain_of, list_agents, list_messages, list_sessions, screen_of};
use super::{Role, Runtime, lookup};

struct GatewayConnectionGuard<'a> {
    connections: &'a AtomicUsize,
}

impl Drop for GatewayConnectionGuard<'_> {
    fn drop(&mut self) {
        self.connections.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) async fn response(connection: &mut Framed, response: Response) -> anyhow::Result<()> {
    connection.send(&serde_json::to_vec(&response)?).await
}

pub(super) async fn serve(socket: TcpStream, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let mut connection = Framed::new(socket);
    let Some(role) = authenticate_hello(&mut connection, &runtime).await? else {
        return Ok(());
    };
    let _gateway_guard = if matches!(role, Role::Gateway) {
        runtime.gateway_connections.fetch_add(1, Ordering::Relaxed);
        Some(GatewayConnectionGuard {
            connections: &runtime.gateway_connections,
        })
    } else {
        None
    };
    response(&mut connection, Response::Ok).await?;
    while let Some(bytes) = connection.recv().await? {
        if session_has_exited(&runtime, &role) {
            response(
                &mut connection,
                Response::Error {
                    message: "session has exited".into(),
                },
            )
            .await?;
            connection.stream.shutdown().await?;
            return Ok(());
        }
        let request = match serde_json::from_slice::<Request>(&bytes) {
            Ok(request) => request,
            Err(_) => {
                response(
                    &mut connection,
                    Response::Error {
                        message: "invalid control request".into(),
                    },
                )
                .await?;
                continue;
            }
        };
        if !request_permitted(&role, &request) {
            let message = if matches!(&role, Role::Session(_)) {
                "not permitted for a session token"
            } else {
                "not permitted for this token"
            };
            response(
                &mut connection,
                Response::Error {
                    message: message.into(),
                },
            )
            .await?;
            continue;
        }
        let result = match request {
            Request::Hello { .. } => Response::Error {
                message: "already authenticated".into(),
            },
            Request::List => list_sessions(&runtime).await?,
            Request::Version => Response::Version {
                version: crate::VERSION.to_owned(),
            },
            request @ Request::Confine { .. } => runtime.create_session(request).await?,
            request @ Request::NewSession { .. } => runtime.create_session(request).await?,
            Request::Shutdown { now } => {
                // Answer first: shutdown closes every connection, this one included.
                runtime.stop_now.store(now, Ordering::Relaxed);
                let sent = response(&mut connection, Response::Ok).await;
                runtime.stop_requested.notify_one();
                sent?;
                continue;
            }
            Request::Kill { session: id, now } => kill_session(&runtime, &id, now).await,
            Request::Reset { session } => runtime.reset_session(&role, session).await,
            Request::Screen { session: id } => screen_of(&runtime, &id),
            Request::Explain { session: id } => explain_of(&runtime, &id).await,
            Request::Attach {
                session: id,
                force,
                cols,
                rows,
                status,
                top_rows,
                viewer,
            } => {
                if cols == 0 || rows == 0 {
                    Response::Error {
                        message: "attachment dimensions must be nonzero".into(),
                    }
                } else if let Some((_, session)) = lookup(&runtime, &id) {
                    if attach(
                        &mut connection,
                        session,
                        force,
                        Size { cols, rows },
                        top_rows,
                        viewer,
                        status,
                        runtime.clone(),
                    )
                    .await?
                    {
                        return Ok(());
                    }
                    continue;
                } else {
                    Response::Error {
                        message: format!("unknown session {id}"),
                    }
                }
            }
            Request::ListAgents => list_agents(&runtime, &role),
            Request::SendMessage {
                to,
                subject,
                message,
            } => runtime.send_message(&role, to, subject, message).await,
            Request::ReportPrompt { prompt } => runtime.report_prompt(&role, prompt).await?,
            Request::ReportQuota { five_hour, weekly } => {
                runtime.report_quota(&role, five_hour, weekly).await?
            }
            Request::BridgeAttach => {
                if let Some(result) = bridge_attach(&mut connection, &runtime, &role).await? {
                    result
                } else {
                    return Ok(());
                }
            }
            Request::MessageStatus { id } => runtime.message_status(&role, &id).await?,
            Request::HumanInbound { to, body } => runtime.human_inbound(to, body).await,
            Request::GatewayAgents => runtime.gateway_agents(),
            Request::HumanOutboundNext => runtime.human_outbound_next().await,
            Request::HumanOutboundDone { id, sent } => runtime.human_outbound_done(id, sent).await,
            Request::IssueLeaseCode { team } => runtime.issue_lease_code(team).await?,
            Request::RedeemLeaseCode { code } => runtime.redeem_lease_code(code),
            Request::ListMessages { session, state } => {
                list_messages(&runtime, session, state).await?
            }
            Request::CancelMessage { id } => cancel_message(&runtime, &id).await?,
        };
        response(&mut connection, result).await?;
    }
    Ok(())
}

fn session_has_exited(runtime: &Runtime, role: &Role) -> bool {
    matches!(role, Role::Session(id) if !lookup(runtime, id).is_some_and(|(_, session)| session.exit_code().is_none()))
}

async fn authenticate_hello(
    connection: &mut Framed,
    runtime: &Runtime,
) -> anyhow::Result<Option<Role>> {
    let hello = tokio::time::timeout(Duration::from_secs(5), connection.recv()).await;
    let role = match hello {
        Ok(Ok(Some(bytes))) => match serde_json::from_slice::<Request>(&bytes) {
            Ok(Request::Hello { token }) => runtime.authenticate(&token),
            _ => None,
        },
        _ => None,
    };
    let Some(role) = role else {
        response(
            connection,
            Response::Error {
                message: "authentication required; check admin.token".into(),
            },
        )
        .await?;
        connection.stream.shutdown().await?;
        return Ok(None);
    };
    Ok(Some(role))
}
