use std::sync::Arc;
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::wire::BridgeDown;

use super::{NativeSlot, Session};

pub(crate) struct NativeGuard {
    session: Arc<Session>,
    pub(crate) down: mpsc::UnboundedReceiver<BridgeDown>,
}

impl Session {
    pub(crate) fn native_attach(self: &Arc<Self>) -> Option<NativeGuard> {
        let mut state = self.lock();
        if state.native.link.is_some() {
            return None;
        }
        let (sender, receiver) = mpsc::unbounded_channel();
        state.native.link = Some(sender);
        drop(state);
        Some(NativeGuard {
            session: self.clone(),
            down: receiver,
        })
    }

    pub(crate) fn native_accept(&self) {
        let mut state = self.lock();
        state.native.connected = true;
        state.native.refused = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_refuse(&self) {
        let mut state = self.lock();
        state.native.refused = true;
        state.native.connected = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_state(
        &self,
        idle: bool,
        draft: bool,
        pending: bool,
        provider: Option<String>,
    ) {
        let mut state = self.lock();
        let now = Instant::now();
        if state.native.idle_since.is_none() || state.native.idle != idle {
            state.native.idle_since = Some(now);
        }
        state.native.reported_at = Some(now);
        state.native.provider = provider;
        state.native.idle = idle;
        state.native.draft = draft;
        state.native.in_flight = pending;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_provider(&self) -> Option<String> {
        self.lock().native.provider.clone()
    }

    pub(crate) fn native_diagnostics(&self, queue: Option<u32>, host_pending: Option<bool>) {
        let mut state = self.lock();
        state.native.diagnostics = queue.zip(host_pending);
    }

    pub(crate) fn native_ack(&self, id: &str, result: Result<(), String>) {
        let sender = {
            let mut state = self.lock();
            let matches_id = state
                .native
                .waiting
                .as_ref()
                .is_some_and(|(waiting_id, _)| waiting_id == id);
            if !matches_id {
                return;
            }
            let Some((_, sender)) = state.native.waiting.take() else {
                return;
            };
            if result.is_ok() {
                state.native.in_flight = true;
            }
            sender
        };
        let _ = sender.send(result);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_received(&self) {
        let mut state = self.lock();
        state.native.in_flight = false;
        drop(state);
        self.message_notify.notify_one();
    }

    pub(crate) fn native_send(
        &self,
        id: &str,
        envelope: &str,
    ) -> Option<oneshot::Receiver<Result<(), String>>> {
        let mut state = self.lock();
        let native = &mut state.native;
        let link = native.link.as_ref()?;
        let (sender, receiver) = oneshot::channel();
        native.waiting = Some((id.to_owned(), sender));
        if link
            .send(BridgeDown::Deliver {
                id: id.to_owned(),
                envelope: envelope.to_owned(),
            })
            .is_err()
        {
            native.waiting = None;
            return None;
        }
        drop(state);
        Some(receiver)
    }

    pub(crate) fn native_reason(&self) -> Option<&'static str> {
        let state = self.lock();
        if state.native.refused {
            Some("channel_refused")
        } else if !state.native.connected {
            Some("channel_down")
        } else if state.native.draft {
            Some("draft_present")
        } else if state.native.in_flight {
            Some("in_flight")
        } else {
            None
        }
    }
    pub(super) fn native_timing(native: &NativeSlot) -> String {
        let now = Instant::now();
        let idle_for = native
            .idle_since
            .map(|since| format!("{}s", now.saturating_duration_since(since).as_secs()))
            .unwrap_or_else(|| "none".to_owned());
        let reported = native
            .reported_at
            .map(|since| format!("{}s ago", now.saturating_duration_since(since).as_secs()))
            .unwrap_or_else(|| "none".to_owned());
        format!(" idle_for={idle_for} reported={reported}")
    }
}

impl Drop for NativeGuard {
    fn drop(&mut self) {
        let mut state = self.session.lock();
        state.native.connected = false;
        state.native.link = None;
        state.native.waiting = None;
        state.native.draft = false;
        state.native.in_flight = false;
        state.native.diagnostics = None;
        drop(state);
        self.session.message_notify.notify_one();
    }
}
