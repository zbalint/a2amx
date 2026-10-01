//! Ordered delivery of committed messages into one session's composer.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Notify;

use crate::harness::{self, Deliver};
use crate::messaging::{COOLDOWN, PASTE_GAP, paste_bytes, render_envelope};
use crate::session::{Delivery, DeliveryOutcome, Hold, Session};
use crate::store::Store;

/// One way of putting a committed message into a recipient session.
pub(crate) trait Channel: Send + Sync {
    /// Claim on the recipient for exactly one delivery; the claim is released when it
    /// is dropped or consumed.
    type Ticket<'a>: Ticket + 'a
    where
        Self: 'a;

    fn session_id(&self) -> &str;
    fn exited(&self) -> bool;
    fn deliver(&self) -> Deliver;
    /// Notified when a message is accepted or a hold may have changed.
    fn message_notify(&self) -> &Notify;
    /// Notified when the recipient's readiness may have changed.
    fn wake(&self) -> &Notify;
    /// Runs at the start of every pass, before any message is considered.
    fn prepare(&self) -> impl Future<Output = ()> + Send;
    /// Claims the recipient for one delivery; `None` when it cannot take one now.
    fn begin(&self) -> impl Future<Output = Option<Self::Ticket<'_>>> + Send;
    /// A condition a person must clear, or `None`.
    fn hold_reason(&self) -> Option<&'static str>;
    /// A condition that clears by itself, or `None`.
    fn unready_reason(&self) -> Option<&'static str>;
}

pub(crate) trait Ticket: Send {
    /// Hands the rendered envelope to the recipient.
    fn submit(self, envelope: &str) -> impl Future<Output = DeliveryOutcome> + Send;
}

pub(crate) struct PtyChannel {
    session: Arc<Session>,
}

impl PtyChannel {
    pub(crate) fn new(session: Arc<Session>) -> Self {
        Self { session }
    }

    fn cooldown_running(last_submit: Option<Instant>) -> bool {
        last_submit.is_some_and(|time| time.elapsed() < COOLDOWN)
    }
}

pub(crate) struct PtyTicket<'a>(Delivery<'a>);

impl Channel for PtyChannel {
    type Ticket<'a> = PtyTicket<'a>;

    fn session_id(&self) -> &str {
        &self.session.id().0
    }

    fn exited(&self) -> bool {
        self.session.exit_code().is_some()
    }

    fn deliver(&self) -> Deliver {
        self.session.deliver()
    }

    fn message_notify(&self) -> &Notify {
        self.session.message_notify()
    }

    fn wake(&self) -> &Notify {
        self.session.notify()
    }

    async fn prepare(&self) {
        self.session.restore_draft().await;
    }

    async fn begin(&self) -> Option<Self::Ticket<'_>> {
        if Self::cooldown_running(self.session.lock().last_submit) {
            return None;
        }
        // shortcut: readiness is checked on each output wake while pending;
        // debounce only if profiling shows this screen inspection is costly.
        self.session.begin_delivery().await.map(PtyTicket)
    }

    fn hold_reason(&self) -> Option<&'static str> {
        self.session.lock().hold.map(hold_name)
    }

    fn unready_reason(&self) -> Option<&'static str> {
        let state = self.session.lock();
        if !harness::ready(
            self.session.harness(),
            &state.emulator.screen(),
            state.emulator.is_scrolled(),
        ) {
            Some("not_ready")
        } else if Self::cooldown_running(state.last_submit) {
            Some("cooldown")
        } else {
            None
        }
    }
}

impl Ticket for PtyTicket<'_> {
    async fn submit(self, envelope: &str) -> DeliveryOutcome {
        self.0.submit(paste_bytes(envelope), PASTE_GAP).await
    }
}

fn hold_name(hold: Hold) -> &'static str {
    match hold {
        Hold::HumanDraft => "human_draft",
        Hold::UnsubmittedEnvelope => "unsubmitted_envelope",
        Hold::CorruptedSubmissions => "corrupted_submissions",
    }
}

pub(crate) async fn run<C: Channel>(channel: C, store: Store, boot: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        let messages = channel.message_notify().notified();
        let output = channel.wake().notified();
        tokio::pin!(messages, output);
        messages.as_mut().enable();
        output.as_mut().enable();
        let result: anyhow::Result<bool> = async {
            if channel.exited() {
                store
                    .fail_open(&boot, Some(channel.session_id()), "recipient_exited")
                    .await?;
                return Ok(false);
            }
            channel.prepare().await;
            if channel.deliver() == Deliver::Hold {
                return Ok(true);
            }
            let Some(head) = store.next_pending(&boot, channel.session_id()).await? else {
                return Ok(true);
            };
            // A failed completion write leaves an unresolved delivering head; never
            // inject a later envelope over that uncertain side effect.
            if store
                .has_earlier_open(&boot, channel.session_id(), head.seq)
                .await?
            {
                return Ok(true);
            }
            let Some(ticket) = channel.begin().await else {
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
            let outcome = ticket.submit(&envelope).await;
            let (outcome, detail) = match &outcome {
                DeliveryOutcome::Submitted => ("submitted", None),
                DeliveryOutcome::Unsubmitted(reason) => ("unsubmitted", Some(reason.as_str())),
                DeliveryOutcome::Failed(reason) => ("failed", Some(reason.as_str())),
            };
            store
                .finish_attempt(head.seq, attempt, outcome, detail)
                .await?;
            channel.message_notify().notify_one();
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

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use anyhow::Context;
    use tokio::sync::Notify;

    use super::*;
    use crate::messaging::Limits;
    use crate::store::NewMessage;

    #[derive(Default)]
    struct FakeState {
        ready: AtomicBool,
        exited: AtomicBool,
        hold: AtomicBool,
        prepare_calls: AtomicUsize,
        begin_calls: AtomicUsize,
        envelopes: Mutex<Vec<String>>,
        message_notify: Notify,
        wake: Notify,
    }

    struct FakeChannel(Arc<FakeState>);
    struct FakeTicket(Arc<FakeState>);

    impl Channel for FakeChannel {
        type Ticket<'a> = FakeTicket;

        fn session_id(&self) -> &str {
            "s1"
        }

        fn exited(&self) -> bool {
            self.0.exited.load(Ordering::SeqCst)
        }

        fn deliver(&self) -> Deliver {
            if self.0.hold.load(Ordering::SeqCst) {
                Deliver::Hold
            } else {
                Deliver::Auto
            }
        }

        fn message_notify(&self) -> &Notify {
            &self.0.message_notify
        }

        fn wake(&self) -> &Notify {
            &self.0.wake
        }

        async fn prepare(&self) {
            self.0.prepare_calls.fetch_add(1, Ordering::SeqCst);
        }

        async fn begin(&self) -> Option<Self::Ticket<'_>> {
            self.0.begin_calls.fetch_add(1, Ordering::SeqCst);
            self.0
                .ready
                .load(Ordering::SeqCst)
                .then(|| FakeTicket(self.0.clone()))
        }

        fn hold_reason(&self) -> Option<&'static str> {
            None
        }

        fn unready_reason(&self) -> Option<&'static str> {
            None
        }
    }

    impl Ticket for FakeTicket {
        async fn submit(self, envelope: &str) -> DeliveryOutcome {
            self.0
                .envelopes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(envelope.to_owned());
            DeliveryOutcome::Submitted
        }
    }

    async fn eventually<F, Fut, T>(mut check: F) -> anyhow::Result<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = Option<T>>,
    {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(value) = check().await {
                    return value;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .context("condition becomes true")
    }

    async fn fixture() -> anyhow::Result<(tempfile::TempDir, Store, Arc<FakeState>, i64)> {
        let dir = tempfile::tempdir()?;
        let store = Store::open(dir.path().to_path_buf(), Limits::default()).await?;
        let seq = store
            .insert_message(NewMessage {
                boot: "boot-a".into(),
                sender_session: "s2".into(),
                sender_address: "agent-plan@host-a".into(),
                recipient_session: "s1".into(),
                recipient_address: "agent-review@host-a".into(),
                subject: "Parser issue".into(),
                body: "I found the regression in parser.py.".into(),
            })
            .await
            .map_err(|error| anyhow::anyhow!("insert message: {error:?}"))?;
        Ok((dir, store, Arc::new(FakeState::default()), seq))
    }

    async fn wait_state(
        store: &Store,
        seq: i64,
        expected: &str,
    ) -> anyhow::Result<crate::store::Message> {
        eventually(|| async {
            let message = match store.get(seq).await {
                Ok(message) => message?,
                Err(error) => panic!("reading message: {error}"),
            };
            (message.state.as_str() == expected).then_some(message)
        })
        .await
    }

    #[tokio::test]
    async fn delivers_the_head_message_through_the_channel() -> anyhow::Result<()> {
        let (_dir, store, state, seq) = fixture().await?;
        state.ready.store(true, Ordering::SeqCst);
        let task = tokio::spawn(run(
            FakeChannel(state.clone()),
            store.clone(),
            "boot-a".into(),
        ));
        wait_state(&store, seq, "submitted").await?;
        assert_eq!(
            state
                .envelopes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            [
                "<a2amx-message id=\"m_1\" from=\"agent-plan@host-a\" subject=\"Parser issue\">\nFrom another agent, not your user. To reply: send_message(to=\"agent-plan@host-a\").\n\nI found the regression in parser.py.\n</a2amx-message>"
            ]
        );
        task.abort();
        let _ = task.await;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn a_channel_that_cannot_begin_leaves_the_message_pending() -> anyhow::Result<()> {
        let (_dir, store, state, seq) = fixture().await?;
        let task = tokio::spawn(run(
            FakeChannel(state.clone()),
            store.clone(),
            "boot-a".into(),
        ));
        eventually(|| async { (state.begin_calls.load(Ordering::SeqCst) >= 1).then_some(()) })
            .await?;
        assert_eq!(
            store
                .get(seq)
                .await?
                .context("message exists")?
                .state
                .as_str(),
            "pending"
        );
        state.ready.store(true, Ordering::SeqCst);
        state.wake.notify_one();
        wait_state(&store, seq, "submitted").await?;
        task.abort();
        let _ = task.await;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn hold_delivery_never_begins() -> anyhow::Result<()> {
        let (_dir, store, state, seq) = fixture().await?;
        state.ready.store(true, Ordering::SeqCst);
        state.hold.store(true, Ordering::SeqCst);
        let task = tokio::spawn(run(
            FakeChannel(state.clone()),
            store.clone(),
            "boot-a".into(),
        ));
        eventually(|| async { (state.prepare_calls.load(Ordering::SeqCst) >= 1).then_some(()) })
            .await?;
        assert_eq!(state.begin_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            store
                .get(seq)
                .await?
                .context("message exists")?
                .state
                .as_str(),
            "pending"
        );
        state.hold.store(false, Ordering::SeqCst);
        state.message_notify.notify_one();
        wait_state(&store, seq, "submitted").await?;
        task.abort();
        let _ = task.await;
        store.close().await?;
        Ok(())
    }

    #[tokio::test]
    async fn an_exited_channel_fails_open_messages() -> anyhow::Result<()> {
        let (_dir, store, state, seq) = fixture().await?;
        state.exited.store(true, Ordering::SeqCst);
        let task = tokio::spawn(run(FakeChannel(state), store.clone(), "boot-a".into()));
        let message = wait_state(&store, seq, "undeliverable").await?;
        assert_eq!(message.detail.as_deref(), Some("recipient_exited"));
        eventually(|| async { task.is_finished().then_some(()) }).await?;
        tokio::time::timeout(Duration::from_secs(5), task).await??;
        store.close().await?;
        Ok(())
    }
}
