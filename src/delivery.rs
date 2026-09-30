//! Ordered delivery of committed messages into one session's composer.

use std::sync::Arc;
use std::time::Duration;

use crate::harness::Deliver;
use crate::messaging::{COOLDOWN, PASTE_GAP, paste_bytes, render_envelope};
use crate::session::{DeliveryOutcome, Session};
use crate::store::Store;

pub(crate) async fn run(session: Arc<Session>, store: Store, boot: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        let messages = session.message_notify().notified();
        let output = session.notify().notified();
        tokio::pin!(messages, output);
        messages.as_mut().enable();
        output.as_mut().enable();
        let result: anyhow::Result<bool> = async {
            if session.exit_code().is_some() {
                store
                    .fail_open(&boot, Some(&session.id().0), "recipient_exited")
                    .await?;
                return Ok(false);
            }
            if session.deliver() == Deliver::Hold {
                return Ok(true);
            }
            let Some(head) = store.next_pending(&boot, &session.id().0).await? else {
                return Ok(true);
            };
            // A failed completion write leaves an unresolved delivering head; never
            // inject a later envelope over that uncertain side effect.
            if store
                .has_earlier_open(&boot, &session.id().0, head.seq)
                .await?
            {
                return Ok(true);
            }
            if session
                .lock()
                .last_submit
                .is_some_and(|time| time.elapsed() < COOLDOWN)
            {
                return Ok(true);
            }
            // shortcut: readiness is checked on each output wake while pending;
            // debounce only if profiling shows this screen inspection is costly.
            let Some(delivery) = session.begin_delivery().await else {
                return Ok(true);
            };
            let Some(attempt) = store.begin_attempt(head.seq).await? else {
                return Ok(true);
            };
            let envelope = render_envelope(
                &format!("m_{}", head.seq),
                &head.sender_address,
                &head.subject,
                &head.body,
            );
            let outcome = delivery.submit(paste_bytes(&envelope), PASTE_GAP).await;
            let (outcome, detail) = match &outcome {
                DeliveryOutcome::Submitted => ("submitted", None),
                DeliveryOutcome::Unsubmitted(reason) => ("unsubmitted", Some(reason.as_str())),
                DeliveryOutcome::Failed(reason) => ("failed", Some(reason.as_str())),
            };
            store
                .finish_attempt(head.seq, attempt, outcome, detail)
                .await?;
            session.message_notify().notify_one();
            Ok(true)
        }
        .await;
        match result {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => tracing::error!(%error, "message delivery storage failed"),
        }
        tokio::select! {
            _ = &mut messages => {},
            _ = &mut output => {},
            _ = interval.tick() => {},
        }
    }
}
