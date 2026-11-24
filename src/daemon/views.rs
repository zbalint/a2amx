use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use super::{ACTIVITY_QUIET, Role, Runtime, failed, lookup, parse_message_id};
use crate::delivery::{AnyChannel, Channel};
use crate::harness::{self, Deliver, Harness};
use crate::messaging::{self, MessageState, code};
use crate::quota;
use crate::session::Session;
use crate::store::{self, Message};
use crate::wire::{
    Activity, AgentSummary, ExplainInfo, HoldInfo, MessageInfo, QuotaInfo, ResetVerdict, Response,
    SessionSummary,
};

pub(super) async fn list_sessions(runtime: &Runtime) -> anyhow::Result<Response> {
    let counts = runtime.store.open_counts(&runtime.boot).await?;
    let omp_usage = runtime.omp_usage.current();
    let sessions = runtime.sessions.snapshot();
    let sessions = sessions
        .iter()
        .map(|session| {
            let activity = session_activity(session);
            let activity_secs =
                activity.map(|current| session.note_activity(current, Instant::now()).as_secs());
            let hold_reason = AnyChannel::for_session(session.clone())
                .hold_reason()
                .or(session.needs_input());
            let quota = session_quota(session, omp_usage.as_ref());
            let starting = session.starting();
            let state = session.lock();
            let (uptime_secs, activity_secs) = if state.exit_code.is_none() {
                (Some(session.started().elapsed().as_secs()), activity_secs)
            } else {
                (None, None)
            };
            SessionSummary {
                id: session.id().0.clone(),
                argv: session.argv.clone(),
                cols: state.size.cols,
                rows: state.size.rows,
                attached: state.attachment.is_some(),
                viewer: state.attachment.as_ref().is_some_and(|slot| slot.viewer),
                exit_code: state.exit_code,
                name: session.name().map(str::to_owned),
                team: session.team().cloned(),
                address: messaging::address(session.name(), &session.id().0, &runtime.host_name),
                pending: counts.get(&session.id().0).copied().unwrap_or(0),
                held: hold_reason.is_some(),
                hold_reason: hold_reason.map(str::to_owned),
                quota,
                harness: session.harness(),
                confine: session.confine_info(),
                cwd: session
                    .cwd()
                    .map(|path| path.to_string_lossy().into_owned()),
                activity,
                uptime_secs,
                activity_secs,
                starting,
            }
        })
        .collect();
    Ok(Response::Sessions { sessions })
}

pub(super) fn list_agents(runtime: &Runtime, role: &Role) -> Response {
    let admin = matches!(role, Role::Admin);
    let viewer_id = match role {
        Role::Admin => None,
        Role::Gateway => None,
        Role::Session(id) => Some(id.as_str()),
    };
    let sessions = runtime.sessions.snapshot();
    let viewer = viewer_id.and_then(|id| {
        sessions
            .iter()
            .find(|session| session.id().0 == id)
            .cloned()
    });
    if !admin && viewer.is_none() {
        return Response::Error {
            message: "sender session is unavailable".into(),
        };
    }
    let owner = viewer
        .as_ref()
        .filter(|viewer| viewer.remote().is_some())
        .map(|viewer| {
            if runtime
                .owner_cause(&viewer.id().0, Instant::now())
                .is_some()
            {
                "present".to_owned()
            } else {
                "away".to_owned()
            }
        });
    let omp_usage = runtime.omp_usage.current();
    let agents = sessions
        .iter()
        .filter(|session| {
            admin
                || viewer.as_ref().is_some_and(|viewer| {
                    session.id() == viewer.id()
                        || messaging::visible(viewer.party(), session.party())
                })
        })
        .map(|session| {
            let activity = session_activity(session);
            let slot = session.attachment_viewer();
            AgentSummary {
                address: messaging::address(session.name(), &session.id().0, &runtime.host_name),
                state: if session.exit_code().is_some() {
                    "exited"
                } else {
                    "running"
                }
                .into(),
                attached: slot.is_some(),
                viewer: slot == Some(true),
                team: session.team().map(|scope| scope.name.clone()),
                you: viewer
                    .as_ref()
                    .is_some_and(|viewer| session.id() == viewer.id()),
                quota: session_quota(session, omp_usage.as_ref()),
                harness: session.harness(),
                confine: session.confine_info(),
                cwd: session
                    .cwd()
                    .map(|path| path.to_string_lossy().into_owned()),
                activity,
            }
        })
        .collect();
    Response::Agents { agents, owner }
}
pub(super) fn screen_of(runtime: &Runtime, id: &str) -> Response {
    match lookup(runtime, id) {
        Some((_, session)) => {
            let screen = {
                let state = session.lock();
                state.emulator.screen()
            };
            Response::Screen {
                lines: screen.text_lines(),
            }
        }
        None => Response::Error {
            message: format!("unknown session {id}"),
        },
    }
}

pub(super) async fn explain_of(runtime: &Runtime, id: &str) -> Response {
    let counts = match runtime.store.open_counts(&runtime.boot).await {
        Ok(counts) => counts,
        Err(error) => {
            return Response::Error {
                message: error.to_string(),
            };
        }
    };
    let Some((_, session)) = lookup(runtime, id) else {
        return Response::Error {
            message: format!("unknown session {id}"),
        };
    };
    let exit_code = session.exit_code();
    let pending = if exit_code.is_some() {
        0
    } else {
        counts.get(&session.id().0).copied().unwrap_or(0)
    };
    let (ready, starting, activity, activity_cause, needs_input, hold, reset) =
        if exit_code.is_some() {
            (
                None,
                false,
                None,
                None,
                None,
                None,
                ResetVerdict {
                    allowed: false,
                    code: None,
                    reason: None,
                },
            )
        } else {
            let activity_cause = session_activity_cause(&session);
            let activity = activity_cause.as_ref().map(|(activity, _)| *activity);
            let cause = activity_cause.map(|(_, cause)| cause);
            let needs_input = session.needs_input().map(str::to_owned);
            let hold = if session.deliver() == Deliver::Hold {
                let reason = "deliver_hold";
                Some(HoldInfo {
                    explanation: messaging::hold_explanation(reason).map(str::to_owned),
                    reason: reason.to_owned(),
                })
            } else if let Some(reason) = AnyChannel::for_session(session.clone()).hold_reason() {
                Some(HoldInfo {
                    explanation: messaging::hold_explanation(reason).map(str::to_owned),
                    reason: reason.to_owned(),
                })
            } else {
                session.lock().hold.map(|hold| HoldInfo {
                    reason: crate::delivery::hold_name(hold).to_owned(),
                    explanation: None,
                })
            };
            let starting = session.starting();
            let ready = {
                let state = session.lock();
                Some(harness::ready(
                    session.harness(),
                    &state.emulator.screen(),
                    state.emulator.is_scrolled(),
                ))
            };
            let reset = session.reset_gate_preview().map_or_else(
                || ResetVerdict {
                    allowed: true,
                    code: None,
                    reason: None,
                },
                |(code, reason)| ResetVerdict {
                    allowed: false,
                    code: Some(code.to_owned()),
                    reason: Some(reason),
                },
            );
            (ready, starting, activity, cause, needs_input, hold, reset)
        };
    Response::Explain {
        info: ExplainInfo {
            id: session.id().0.clone(),
            address: messaging::address(session.name(), &session.id().0, &runtime.host_name),
            harness: session.harness(),
            exit_code,
            ready,
            starting,
            activity,
            activity_cause,
            needs_input,
            hold,
            pending,
            reset,
        },
    }
}
pub(super) async fn list_messages(
    runtime: &Runtime,
    session: Option<String>,
    state: Option<String>,
) -> anyhow::Result<Response> {
    let filter = state.as_deref().map(MessageState::parse);
    if matches!(filter, Some(None)) {
        return Ok(Response::Error {
            message: "unknown message state".into(),
        });
    }
    let rows = runtime
        .store
        .list(&runtime.boot, session.as_deref(), filter.flatten())
        .await?;
    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        messages.push(runtime.message_info(row).await?);
    }
    Ok(Response::Messages { messages })
}
/// The quota a running Claude, Codex or OMP session reports.
pub(super) fn session_quota(
    session: &Session,
    omp_usage: Option<&BTreeMap<String, QuotaInfo>>,
) -> Option<QuotaInfo> {
    let harness = session.harness();
    if session.exit_code().is_some()
        || !matches!(
            harness,
            Harness::Claude | Harness::Codex | Harness::Omp | Harness::Agy
        )
    {
        return None;
    }
    let screen = quota::read(&session.lock().emulator.screen(), harness);
    if matches!(harness, Harness::Claude | Harness::Agy) {
        let (five_hour, weekly) = session.quota_report(store::now());
        let mut quota = screen.unwrap_or_default();
        if let Some(window) = five_hour {
            quota.five_hour_left = Some(window.left);
        }
        if let Some(window) = weekly {
            quota.weekly_left = Some(window.left);
        }
        return (quota.five_hour_left.is_some() || quota.weekly_left.is_some()).then_some(quota);
    }
    if harness != Harness::Omp {
        return screen;
    }
    let limit_reached = screen.is_some_and(|quota| quota.limit_reached);
    let omp_quota = session
        .native_provider()
        .and_then(|provider| omp_usage.and_then(|usage| usage.get(&provider).cloned()));
    match (omp_quota, limit_reached) {
        (Some(quota), true) => Some(QuotaInfo {
            limit_reached: true,
            ..quota
        }),
        (Some(quota), false) => Some(quota),
        (None, true) => Some(QuotaInfo {
            limit_reached: true,
            ..Default::default()
        }),
        (None, false) => None,
    }
}
fn background_job_waiting(session: &Arc<Session>) -> bool {
    if session.harness() != Harness::Agy {
        return false;
    }
    let state = session.lock();
    harness::agy_background_job_running(&state.emulator.screen())
}

pub(super) fn session_working(session: &Arc<Session>) -> Option<bool> {
    if session.exit_code().is_some() {
        return None;
    }
    let now = Instant::now();
    let (ready, last_change) = {
        let state = session.lock();
        (session.observe_ready(&state), state.last_change)
    };
    Some(
        !ready
            || now.saturating_duration_since(last_change) < ACTIVITY_QUIET
            || background_job_waiting(session),
    )
}

fn session_activity_cause(session: &Arc<Session>) -> Option<(Activity, String)> {
    let now = Instant::now();
    let (ready, last_change) = {
        let state = session.lock();
        if state.exit_code.is_some() {
            return None;
        }
        (session.observe_ready(&state), state.last_change)
    };
    if let Some(reason) = AnyChannel::for_session(session.clone()).hold_reason() {
        Some((Activity::Busy, format!("held: {reason}")))
    } else if session.resetting() {
        Some((Activity::Busy, "reset running".to_owned()))
    } else if !ready {
        Some((Activity::Working, "screen not ready".to_owned()))
    } else {
        let unchanged = now.saturating_duration_since(last_change);
        if unchanged < ACTIVITY_QUIET {
            Some((
                Activity::Working,
                format!("screen changed {}s ago", unchanged.as_secs()),
            ))
        } else if background_job_waiting(session) {
            Some((Activity::Waiting, "background job running".to_owned()))
        } else {
            Some((
                Activity::Idle,
                format!("screen quiet for {}s", unchanged.as_secs()),
            ))
        }
    }
}

pub(super) fn session_activity(session: &Arc<Session>) -> Option<Activity> {
    session_activity_cause(session).map(|(activity, _)| activity)
}

impl Runtime {
    pub(super) async fn message_status(&self, role: &Role, id: &str) -> anyhow::Result<Response> {
        let Some(seq) = parse_message_id(id) else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        let Some(message) = self.store.get(seq).await? else {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        };
        if let Role::Session(sender) = role
            && (message.boot != self.boot || message.sender_session != *sender)
        {
            return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message"));
        }
        Ok(Response::Status {
            message: self.message_info(message).await?,
        })
    }

    async fn message_info(&self, message: Message) -> anyhow::Result<MessageInfo> {
        let recipient = if message.boot == self.boot && message.state == MessageState::Pending {
            lookup(self, &message.recipient_session).map(|(_, session)| session)
        } else {
            None
        };
        let hold_reason = if let Some(session) = recipient {
            let queued = self
                .store
                .has_earlier_open(&self.boot, &message.recipient_session, message.seq)
                .await?;
            let channel = AnyChannel::for_session(session.clone());
            if session.exit_code().is_some() {
                None
            } else if session.deliver() == Deliver::Hold {
                Some("deliver_hold")
            } else if let Some(reason) = channel.hold_reason() {
                Some(reason)
            } else if queued {
                Some("queued")
            } else {
                channel.unready_reason()
            }
        } else {
            None
        };
        Ok(MessageInfo {
            id: format!("m_{}", message.seq),
            from: message.sender_address,
            to: message.recipient_address,
            subject: message.subject,
            state: message.state.as_str().into(),
            detail: message.detail,
            hold_reason: hold_reason.map(str::to_owned),
            hold_explanation: hold_reason
                .and_then(messaging::hold_explanation)
                .map(str::to_owned),
            accepted_at: Some(message.accepted_at),
            updated_at: Some(message.updated_at),
            evidence: if message.state == MessageState::Submitted {
                Some(
                    if message.observed {
                        // shortcut: a removed session reports submission_observed for a
                        // native receipt; persist the channel with the receipt if that matters.
                        if lookup(self, &message.recipient_session).is_some_and(|(_, session)| {
                            matches!(session.harness(), Harness::Omp | Harness::Codex)
                                || session.channel()
                        }) {
                            "native_receipt"
                        } else {
                            "submission_observed"
                        }
                    } else {
                        "write_complete"
                    }
                    .into(),
                )
            } else {
                None
            },
        })
    }
}
