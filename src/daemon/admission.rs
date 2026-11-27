use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use super::views::session_quota;
use super::{Role, Runtime, failed, lookup, parse_message_id, session_id};
use crate::delivery::{AnyChannel, Channel};
use crate::harness::Deliver;
use crate::messaging::{self, code};
use crate::session::Session;
use crate::store::{CancelResult, InsertError, Message, NewMessage};
use crate::wire::{HumanOutboundMessage, Response};

pub(super) struct Outgoing {
    pub(super) to: String,
    pub(super) subject: String,
    pub(super) body: String,
}

fn insert_outcome(
    result: Result<Result<i64, &'static str>, InsertError>,
) -> Result<i64, (&'static str, &'static str)> {
    match result {
        Ok(Ok(seq)) => Ok(seq),
        Ok(Err(message)) => Err((code::RATE_LIMITED, message)),
        Err(InsertError::QueueFull) => Err((code::QUEUE_FULL, "message queue is full")),
        Err(InsertError::Internal(error)) => {
            tracing::error!(%error, "message acceptance failed");
            Err((code::INTERNAL, "message storage failed"))
        }
    }
}

fn owner_present_message(cause: messaging::PresenceCause, away_secs: u64) -> String {
    match cause {
        messaging::PresenceCause::Attached => {
            "the owner is attached to this session in the terminal (viewer mode does not count); write to them there instead".to_owned()
        }
        messaging::PresenceCause::Grace => format!(
            "the owner left this session less than {away_secs} seconds ago; try again later or write to them in the terminal"
        ),
    }
}

impl Runtime {
    fn recipient(&self, to: &str) -> Option<Arc<Session>> {
        let local = messaging::local_part(to, &self.host_name)?;
        self.sessions
            .snapshot()
            .iter()
            .find(|session| session.name().unwrap_or(&session.id().0) == local)
            .cloned()
    }

    async fn send_to_human(&self, sender: &Session, subject: String, body: String) -> Response {
        if let Some((error_code, message)) =
            self.remote_ineligible(sender, Instant::now(), messaging::Direction::ToHuman)
        {
            return failed(error_code, message);
        }
        let sender_id = &sender.id().0;
        if let Some((cause, away_secs)) = self.owner_cause(sender_id, Instant::now()) {
            return failed(
                code::OWNER_PRESENT,
                &owner_present_message(cause, away_secs),
            );
        }
        let new = NewMessage {
            boot: self.boot.clone(),
            sender_session: sender_id.into(),
            sender_address: messaging::address(sender.name(), sender_id, &self.host_name),
            recipient_session: messaging::HUMAN_SESSION.into(),
            recipient_address: messaging::HUMAN_SESSION.into(),
            subject,
            body,
        };
        let windows = [
            (
                sender_id.as_str(),
                self.limits.rate_per_minute,
                "sender rate limit reached",
            ),
            (
                "human:out",
                self.limits.human_outbound_per_minute,
                "human outbound rate limit reached",
            ),
        ];
        match insert_outcome(self.insert_message_rate_limited(&windows, new).await) {
            Ok(seq) => Response::Accepted {
                id: format!("m_{seq}"),
                recipient_hold: None,
                recipient_quota: None,
            },
            Err((error_code, message)) => failed(error_code, message),
        }
    }

    fn routable_recipient(
        &self,
        sender: &Session,
        to: &str,
    ) -> Result<Arc<Session>, (&'static str, &'static str)> {
        let Some(recipient) = self.recipient(to) else {
            return Err((code::UNKNOWN_RECIPIENT, "unknown recipient"));
        };
        if !messaging::visible(sender.party(), recipient.party()) {
            return Err((code::UNKNOWN_RECIPIENT, "unknown recipient"));
        }
        if recipient.id().0 == sender.id().0 {
            return Err((code::UNKNOWN_RECIPIENT, "a session cannot message itself"));
        }
        if recipient.exit_code().is_some() {
            return Err((code::RECIPIENT_EXITED, "recipient has exited"));
        }
        Ok(recipient)
    }

    fn agent_accepted(&self, recipient: &Arc<Session>, seq: i64) -> Response {
        recipient.message_notify().notify_one();
        Response::Accepted {
            id: format!("m_{seq}"),
            recipient_hold: if recipient.deliver() == Deliver::Hold {
                Some("deliver_hold".into())
            } else {
                AnyChannel::for_session(recipient.clone())
                    .hold_reason()
                    .map(str::to_owned)
            },
            recipient_quota: session_quota(recipient, self.omp_usage.current().as_ref())
                .and_then(|quota| quota.exhausted())
                .map(|window| format!("{window} quota exhausted")),
        }
    }

    pub(super) async fn send_message(&self, role: &Role, outgoing: Outgoing) -> Response {
        let Outgoing { to, subject, body } = outgoing;
        let sender_id = match session_id(role, "send_message") {
            Ok(id) => id,
            Err(message) => return Response::Error { message },
        };
        if let Err(error) = messaging::validate_message(&subject, &body) {
            return failed(error.code, &error.message);
        }
        let Some((_, sender)) = lookup(self, sender_id) else {
            return failed(code::INTERNAL, "sender session is unavailable");
        };
        if to == messaging::HUMAN_SESSION {
            return self.send_to_human(&sender, subject, body).await;
        }
        let recipient = match self.routable_recipient(&sender, &to) {
            Ok(recipient) => recipient,
            Err((error_code, message)) => return failed(error_code, message),
        };
        let new = NewMessage {
            boot: self.boot.clone(),
            sender_session: sender_id.into(),
            sender_address: messaging::address(sender.name(), sender_id, &self.host_name),
            recipient_session: recipient.id().0.clone(),
            recipient_address: messaging::address(
                recipient.name(),
                &recipient.id().0,
                &self.host_name,
            ),
            subject,
            body,
        };
        let windows = [(
            sender_id,
            self.limits.rate_per_minute,
            "sender rate limit reached",
        )];
        match insert_outcome(self.insert_message_rate_limited(&windows, new).await) {
            Ok(seq) => self.agent_accepted(&recipient, seq),
            Err((error_code, message)) => failed(error_code, message),
        }
    }

    async fn insert_message_rate_limited(
        &self,
        windows: &[(&str, u32, &'static str)],
        new: NewMessage,
    ) -> std::result::Result<std::result::Result<i64, &'static str>, InsertError> {
        let mut rate = self.rate.lock().await;
        for &(key, limit, refusal) in windows {
            let timestamps = rate.entry(key.into()).or_default();
            while timestamps
                .front()
                .is_some_and(|time| time.elapsed() > Duration::from_secs(60))
            {
                timestamps.pop_front();
            }
            if timestamps.len() >= limit as usize {
                return Ok(Err(refusal));
            }
        }
        let sequence = self.store.insert_message(new).await?;
        let now = Instant::now();
        for &(key, _, _) in windows {
            rate.entry(key.into()).or_default().push_back(now);
        }
        Ok(Ok(sequence))
    }

    pub(super) async fn human_inbound(&self, to: String, body: String) -> Response {
        if let Err(error) = messaging::validate_message(messaging::HUMAN_SUBJECT, &body) {
            return failed(error.code, &error.message);
        }
        if body.len() > messaging::MAX_HUMAN_BODY_BYTES {
            return failed(
                code::TOO_LARGE,
                "message exceeds the byte limit for a human message",
            );
        }
        let Some(recipient) = self.recipient(&to) else {
            return failed(code::UNKNOWN_RECIPIENT, "unknown recipient");
        };
        if let Some((error_code, message)) =
            self.remote_ineligible(&recipient, Instant::now(), messaging::Direction::ToAgent)
        {
            return failed(error_code, message);
        }
        let new = NewMessage {
            boot: self.boot.clone(),
            sender_session: messaging::HUMAN_SESSION.into(),
            sender_address: messaging::HUMAN_SESSION.into(),
            recipient_session: recipient.id().0.clone(),
            recipient_address: messaging::address(
                recipient.name(),
                &recipient.id().0,
                &self.host_name,
            ),
            subject: messaging::HUMAN_SUBJECT.into(),
            body,
        };
        let windows = [(
            "human:in",
            self.limits.human_inbound_per_minute,
            "human inbound rate limit reached",
        )];
        match insert_outcome(self.insert_message_rate_limited(&windows, new).await) {
            Ok(seq) => {
                recipient.message_notify().notify_one();
                Response::Accepted {
                    id: format!("m_{seq}"),
                    recipient_hold: None,
                    recipient_quota: None,
                }
            }
            Err((error_code, message)) => failed(error_code, message),
        }
    }

    async fn fail_human_bound(&self, seq: i64, detail: &'static str) -> anyhow::Result<()> {
        if let Some(attempt) = self.store.begin_attempt(seq).await? {
            self.store
                .finish_attempt(seq, attempt, "failed", Some(detail))
                .await?;
        }
        Ok(())
    }

    async fn next_eligible_human(&self) -> anyhow::Result<Option<(Message, i64)>> {
        for _ in 0..self.limits.max_pending_per_recipient {
            let Some(message) = self
                .store
                .next_pending(&self.boot, messaging::HUMAN_SESSION)
                .await?
            else {
                return Ok(None);
            };
            let eligible = lookup(self, &message.sender_session)
                .map(|(_, sender)| {
                    self.remote_ineligible(&sender, Instant::now(), messaging::Direction::ToHuman)
                        .is_none()
                })
                .unwrap_or(false);
            if !eligible {
                self.fail_human_bound(
                    message.seq,
                    "sender is no longer eligible for remote messaging",
                )
                .await?;
                continue;
            }
            let Some(attempt) = self.store.begin_attempt(message.seq).await? else {
                continue;
            };
            return Ok(Some((message, attempt)));
        }
        Ok(None)
    }

    pub(super) async fn human_outbound_next(&self) -> Response {
        match self.next_eligible_human().await {
            Ok(Some((message, attempt))) => {
                self.human_attempts
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(message.seq, attempt);
                Response::HumanOutbound {
                    message: Some(HumanOutboundMessage {
                        id: format!("m_{}", message.seq),
                        from: message.sender_address,
                        subject: message.subject,
                        body: message.body,
                    }),
                }
            }
            Ok(None) => Response::HumanOutbound { message: None },
            Err(error) => {
                tracing::error!(%error, "human outbound lookup failed");
                Response::Error {
                    message: "human outbound lookup failed".into(),
                }
            }
        }
    }

    pub(super) async fn human_outbound_done(&self, id: String, sent: bool) -> Response {
        let Some(seq) = parse_message_id(&id) else {
            return Response::Error {
                message: "unknown message".into(),
            };
        };
        let Some(attempt) = self
            .human_attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&seq)
        else {
            return Response::Error {
                message: "unknown message".into(),
            };
        };
        let outcome = if sent { "submitted" } else { "failed" };
        let detail = (!sent).then_some("the gateway could not send it");
        match self
            .store
            .finish_attempt(seq, attempt, outcome, detail)
            .await
        {
            Ok(()) => Response::Ok,
            Err(error) => {
                self.human_attempts
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(seq, attempt);
                tracing::error!(%error, "human outbound result failed");
                Response::Error {
                    message: "human outbound result failed".into(),
                }
            }
        }
    }
}

pub(super) async fn cancel_message(runtime: &Runtime, id: &str) -> anyhow::Result<Response> {
    Ok(match parse_message_id(id) {
        Some(seq) => match runtime.store.cancel(seq).await? {
            CancelResult::Cancelled => Response::Ok,
            CancelResult::NotCancellable(state) => Response::Error {
                message: format!("message {id} is {} and cannot be cancelled", state.as_str()),
            },
            CancelResult::Unknown => Response::Error {
                message: format!("unknown message {id}"),
            },
        },
        None => Response::Error {
            message: format!("unknown message {id}"),
        },
    })
}
