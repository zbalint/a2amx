use std::sync::Arc;

use super::{Role, Runtime, lookup, parse_message_id, session_id};
use crate::harness::{self, Harness};
use crate::messaging::{self, MessageState};
use crate::session::Session;
use crate::store::{self, Message, RejectOutcome, Store};
use crate::wire::{QuotaWindow, Response};

/// Stores a native receipt when `id` is an open message of this daemon run addressed
/// to `session` and no receipt exists yet; reports whether one was stored.
pub(crate) async fn record_receipt(
    store: &Store,
    boot: &str,
    session: &Session,
    id: &str,
) -> anyhow::Result<bool> {
    let Some(seq) = parse_message_id(id) else {
        return Ok(false);
    };
    let Some(message) = store.get(seq).await? else {
        return Ok(false);
    };
    if !awaiting_receipt(&message, boot, &session.id().0) {
        return Ok(false);
    }
    store.record_receipt(seq).await?;
    Ok(true)
}

fn awaiting_receipt(message: &Message, boot: &str, session_id: &str) -> bool {
    message.boot == boot
        && message.recipient_session == session_id
        && matches!(
            message.state,
            MessageState::Delivering | MessageState::Submitted | MessageState::Unsubmitted
        )
        && !message.observed
}

impl Runtime {
    pub(super) async fn report_prompt(
        &self,
        role: &Role,
        prompt: String,
    ) -> anyhow::Result<Response> {
        let session_id = match session_id(role, "report_prompt") {
            Ok(id) => id,
            Err(message) => return Ok(Response::Error { message }),
        };
        let (_, session) =
            lookup(self, session_id).ok_or_else(|| anyhow::anyhow!("session is unavailable"))?;
        if session.channel()
            && let Some(inner) = messaging::unwrap_channel(&prompt)
        {
            self.channel_prompt_verdict(&session, session_id, &inner)
                .await?;
            return Ok(Response::PromptVerdict {
                verdict: "allow".into(),
                reason: None,
            });
        }
        let text = messaging::unwrap_pastes(&prompt);
        let known = self.known_envelopes(&session, session_id, &text).await?;
        if known.is_empty() {
            session.note_submit(false);
        } else if known.len() == 1 && text == known[0].1 {
            self.store.record_receipt(known[0].0).await?;
            session.note_submit(true);
        } else {
            return self.block_corrupted_submission(&session, text, known).await;
        }
        Ok(Response::PromptVerdict {
            verdict: "allow".into(),
            reason: None,
        })
    }

    pub(super) async fn report_quota(
        &self,
        role: &Role,
        five_hour: Option<QuotaWindow>,
        weekly: Option<QuotaWindow>,
    ) -> anyhow::Result<Response> {
        let session_id = match session_id(role, "report_quota") {
            Ok(id) => id,
            Err(message) => return Ok(Response::Error { message }),
        };
        let now = store::now();
        let latest = now.saturating_add(8 * 24 * 60 * 60);
        for window in [five_hour, weekly].into_iter().flatten() {
            if window.left > 100 {
                return Ok(Response::Error {
                    message: "quota left must be at most 100".into(),
                });
            }
            if window.resets_at > latest {
                return Ok(Response::Error {
                    message: "quota reset is too far in the future".into(),
                });
            }
        }
        let Some((_, session)) = lookup(self, session_id) else {
            return Ok(Response::Error {
                message: "session is unavailable".into(),
            });
        };
        if matches!(session.harness(), Harness::Claude | Harness::Agy) {
            session.set_quota_report(five_hour, weekly);
        }
        Ok(Response::Ok)
    }

    async fn channel_prompt_verdict(
        &self,
        session: &Arc<Session>,
        session_id: &str,
        inner: &str,
    ) -> anyhow::Result<()> {
        let ids = messaging::envelope_ids(inner);
        let [seq] = ids.as_slice() else {
            return Ok(());
        };
        let Some(message) = self.store.get(*seq).await? else {
            return Ok(());
        };
        if awaiting_receipt(&message, &self.boot, session_id)
            && inner == messaging::channel_view(&message.envelope())
        {
            self.store.record_receipt(*seq).await?;
            session.native_received();
        }
        Ok(())
    }

    async fn known_envelopes(
        &self,
        session: &Arc<Session>,
        session_id: &str,
        text: &str,
    ) -> anyhow::Result<Vec<(i64, String)>> {
        let mut known = Vec::new();
        // shortcut: a session-token holder can forge prompts for that session;
        // stronger attestation requires a different same-user trust contract.
        for seq in messaging::envelope_ids(text) {
            if let Some(message) = self.store.get(seq).await?
                && awaiting_receipt(&message, &self.boot, session_id)
            {
                // The hook sees the envelope as the harness transformed it on paste.
                let envelope = harness::paste_view(session.harness(), &message.envelope());
                known.push((seq, envelope));
            }
        }
        Ok(known)
    }

    async fn block_corrupted_submission(
        &self,
        session: &Arc<Session>,
        text: String,
        known: Vec<(i64, String)>,
    ) -> anyhow::Result<Response> {
        // shortcut: interleaved envelopes cannot be separated safely; their
        // human text stays in the transcript until a harness can rewrite it.
        let draft = strip_envelopes(text, &known).and_then(|rest| recoverable_draft(&rest));
        session.note_corrupted(draft);
        let undeliverable = self.reject_known(known).await?;
        session.message_notify().notify_one();
        Ok(Response::PromptVerdict {
            verdict: "block".into(),
            reason: Some(
                if undeliverable {
                    messaging::UNMATCHABLE_SUBMISSION_REASON
                } else {
                    messaging::CORRUPTED_SUBMISSION_REASON
                }
                .into(),
            ),
        })
    }

    async fn reject_known(&self, known: Vec<(i64, String)>) -> anyhow::Result<bool> {
        let mut undeliverable = false;
        for (seq, _) in known {
            if self
                .store
                .reject_attempt(seq, messaging::MAX_MESSAGE_REJECTIONS)
                .await?
                == RejectOutcome::Undeliverable
            {
                undeliverable = true;
            }
        }
        Ok(undeliverable)
    }
}

fn strip_envelopes(mut text: String, known: &[(i64, String)]) -> Option<String> {
    for (_, envelope) in known {
        let start = text.find(envelope)?;
        text.replace_range(start..start + envelope.len(), "");
    }
    Some(text)
}

fn recoverable_draft(remainder: &str) -> Option<String> {
    // A leftover wrapper tag means unwrapping failed; pasting it back would put
    // harness markup in the human's composer.
    if remainder.contains("<pasted_content") || remainder.contains("</pasted_content") {
        return None;
    }
    Some(messaging::sanitize_draft(remainder.trim()))
}
