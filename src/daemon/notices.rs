use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::activity::{Thresholds, Tracker};
use crate::delivery::{AnyChannel, Channel};
use crate::harness;
use crate::messaging::{self, display_duration};
use crate::session::Session;
use crate::store::{InsertError, NewMessage};

use super::Runtime;
use super::views::{session_activity, session_working};
use crate::wire::Activity;

const HEARTBEAT_POLL: Duration = Duration::from_secs(1);

struct HeartbeatPeer {
    name: String,
    address: String,
    busy_for: Option<Duration>,
    unchanged_for: Option<Duration>,
    hold: Option<String>,
    last_message_age: Option<Duration>,
    queued: u32,
}

struct ObservedPeer {
    peer: Arc<Session>,
    idle: bool,
    busy_for: Option<Duration>,
    unchanged_for: Duration,
    hold: Option<String>,
}

fn observe_peers(
    live: Vec<Arc<Session>>,
    busy_since: &mut HashMap<String, Instant>,
    now: Instant,
) -> Vec<ObservedPeer> {
    let mut observed = Vec::with_capacity(live.len());
    for peer in live {
        let activity = session_activity(&peer);
        let (idle, unchanged_for) = {
            let state = peer.lock();
            (
                !activity.is_some_and(|value| value != Activity::Idle),
                now.saturating_duration_since(state.last_change),
            )
        };
        let hold = AnyChannel::for_session(peer.clone())
            .hold_reason()
            .map(str::to_owned);
        let id = peer.id().0.clone();
        let busy_for = if activity.is_some_and(|value| value != Activity::Idle) {
            let since = *busy_since.entry(id).or_insert(now);
            Some(now.saturating_duration_since(since))
        } else {
            busy_since.remove(&id);
            None
        };
        observed.push(ObservedPeer {
            peer,
            idle,
            busy_for,
            unchanged_for,
            hold,
        });
    }
    observed
}

fn heartbeat_body(interval: Duration, peers: &[HeartbeatPeer]) -> String {
    let mut body = format!(
        "Heartbeat: you have been idle for {}. Watched peers:\n",
        display_duration(interval)
    );
    for peer in peers {
        body.push_str(&format!("- {} ({}): ", peer.name, peer.address));
        if let Some(busy_for) = peer.busy_for {
            body.push_str(&format!("busy for {}", display_duration(busy_for)));
        } else {
            body.push_str("ready");
        }
        if let Some(unchanged_for) = peer.unchanged_for.filter(|age| age.as_secs() >= 60) {
            body.push_str(&format!(
                ", screen unchanged for {}",
                display_duration(unchanged_for)
            ));
        }
        if let Some(hold) = peer.hold.as_deref() {
            body.push_str(&format!(", held: {hold}"));
        }
        if let Some(age) = peer.last_message_age {
            body.push_str(&format!(", last message {} ago", display_duration(age)));
        } else {
            body.push_str(", last message none");
        }
        body.push_str(&format!(", {} queued\n", peer.queued));
    }
    body.push_str("Run a2amx screen <peer> to look before messaging a busy peer.");
    body
}

fn activity_body(changes: &[(String, String, bool)]) -> String {
    let mut body = String::from("peer activity:\n");
    for (_, name, state) in changes {
        body.push_str(&format!(
            "- {name}: {} (was {})\n",
            if *state { "working" } else { "idle" },
            if *state { "idle" } else { "working" },
        ));
    }
    body
}

impl Runtime {
    /// Insert one system message for `recipient`, notify it, and log a refusal. True when accepted.
    async fn post_system(
        &self,
        recipient: &Arc<Session>,
        kind: &str,
        subject: &str,
        body: String,
    ) -> bool {
        let message = NewMessage {
            boot: self.boot.clone(),
            sender_session: messaging::SYSTEM_SESSION.into(),
            sender_address: messaging::address(
                Some(messaging::SYSTEM_SENDER),
                messaging::SYSTEM_SENDER,
                &self.host_name,
            ),
            recipient_session: recipient.id().0.clone(),
            recipient_address: messaging::address(
                recipient.name(),
                &recipient.id().0,
                &self.host_name,
            ),
            subject: subject.into(),
            body,
        };
        match self.store.insert_message(message).await {
            Ok(_) => {
                recipient.message_notify().notify_one();
                true
            }
            Err(InsertError::QueueFull) => {
                tracing::warn!(
                    recipient = %recipient.id().0,
                    "{kind} message queue is full"
                );
                false
            }
            Err(InsertError::Internal(error)) => {
                tracing::error!(
                    %error,
                    recipient = %recipient.id().0,
                    "{kind} message acceptance failed"
                );
                false
            }
        }
    }

    /// True when a notice of `subject` is still open for `recipient`, or the lookup failed (the lookup failure is logged).
    async fn system_notice_blocked(
        &self,
        recipient: &Arc<Session>,
        kind: &str,
        subject: &str,
    ) -> bool {
        match self
            .store
            .open_system_message(&self.boot, &recipient.id().0, subject)
            .await
        {
            Ok(open) => open,
            Err(error) => {
                tracing::error!(
                    %error,
                    recipient = %recipient.id().0,
                    "{kind} state lookup failed"
                );
                true
            }
        }
    }

    fn poll_transitions(&self) -> Vec<Arc<Session>> {
        let mut presence = self.presence.lock().unwrap_or_else(PoisonError::into_inner);
        // Hold presence before taking the leaf session snapshot so new attachments are retained.
        let sessions = self.sessions.snapshot();
        let live_ids = sessions
            .iter()
            .filter(|session| session.exit_code().is_none())
            .map(|session| session.id().0.clone())
            .collect::<HashSet<_>>();
        presence.retain(|id, _| live_ids.contains(id));
        sessions
            .iter()
            .filter(|session| session.exit_code().is_none())
            .filter_map(|session| {
                presence
                    .get_mut(&session.id().0)
                    .and_then(|state| state.poll(Instant::now()).then_some(session.clone()))
            })
            .collect()
    }

    pub(super) async fn poll_presence(self: Arc<Self>, mut stopping: watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(HEARTBEAT_POLL);
        ticker.tick().await;
        loop {
            tokio::select! {
                _ = stopping.changed() => return,
                _ = ticker.tick() => {
                    let transitioned = self.poll_transitions();
                    self.send_presence_notices(transitioned).await;
                }
            }
        }
    }

    async fn send_presence_notices(&self, sessions: Vec<Arc<Session>>) {
        let now = Instant::now();
        for session in sessions {
            if self
                .remote_ineligible(&session, now, messaging::Direction::ToHuman)
                .is_some()
            {
                continue;
            }
            if self
                .system_notice_blocked(&session, "presence", messaging::PRESENCE_SUBJECT)
                .await
            {
                continue;
            }
            self.post_system(
                &session,
                "presence",
                messaging::PRESENCE_SUBJECT,
                messaging::OWNER_AWAY_BODY.into(),
            )
            .await;
        }
    }

    pub(super) async fn observe_exit(self: Arc<Self>, session: Arc<Session>) {
        session.wait_exit().await;
        if session.daemon_ended() {
            return;
        }
        let Some(name) = session.name() else {
            return;
        };
        let Some(code) = session.exit_code() else {
            return;
        };
        let watchers = self
            .sessions
            .snapshot()
            .iter()
            .filter(|watcher| {
                watcher.exit_code().is_none()
                    && watcher.watch().iter().any(|watched| watched == name)
                    && messaging::visible(watcher.party(), session.party())
            })
            .cloned()
            .collect::<Vec<_>>();
        let peer_address = messaging::address(session.name(), &session.id().0, &self.host_name);
        let subject = format!("peer exited: {name} (code {code})");
        let body = format!(
            "{name} ({peer_address}) exited with code {code}.\nRun a2amx screen {name} to see its last screen."
        );
        for watcher in watchers {
            self.post_system(&watcher, "exit event", &subject, body.clone())
                .await;
        }
    }

    pub(super) async fn heartbeat(self: Arc<Self>, session: Arc<Session>, interval: Duration) {
        // shortcut: poll resolution is one second; peers busy before this task starts are bounded from the first sight, not an exact ready-since clock.
        let mut ticker = tokio::time::interval(HEARTBEAT_POLL);
        let mut busy_since = HashMap::<String, Instant>::new();
        loop {
            ticker.tick().await;
            if session.exit_code().is_some() {
                return;
            }
            let now = Instant::now();
            let live_peers = self.live_watched_peers(&session);
            let observed = observe_peers(live_peers, &mut busy_since, now);
            if !self.watcher_may_receive(&session, interval, now) {
                continue;
            }
            let Some(peers) = self.heartbeat_digest(&session, observed).await else {
                continue;
            };
            if session.exit_code().is_some() {
                return;
            }
            self.send_heartbeat(&session, interval, peers).await;
        }
    }

    pub(super) async fn activity(self: Arc<Self>, session: Arc<Session>) {
        let Some(config) = session.activity().cloned() else {
            return;
        };
        let thresholds = Thresholds {
            working_after: config.working_after.map(Duration::from_secs),
            idle_after: config.idle_after.map(Duration::from_secs),
        };
        let mut ticker = tokio::time::interval(HEARTBEAT_POLL);
        let mut trackers = HashMap::<String, Tracker>::new();
        loop {
            ticker.tick().await;
            if session.exit_code().is_some() {
                return;
            }
            let now = Instant::now();
            let peers = self.live_watched_peers(&session);
            let live_ids = peers
                .iter()
                .map(|peer| peer.id().0.clone())
                .collect::<HashSet<_>>();
            trackers.retain(|id, _| live_ids.contains(id));
            let mut changes = Vec::new();
            for peer in peers {
                let Some(working) = session_working(&peer) else {
                    continue;
                };
                let id = peer.id().0.clone();
                let name = peer.name().unwrap_or(&peer.id().0).to_owned();
                let tracker = trackers
                    .entry(id.clone())
                    .or_insert_with(|| Tracker::new(thresholds));
                if let Some(state) = tracker.observe(now, working) {
                    changes.push((id, name, state));
                }
            }
            if changes.is_empty() {
                continue;
            }
            changes.sort_by(|left, right| left.1.cmp(&right.1));
            self.send_activity(&session, changes, &mut trackers).await;
        }
    }

    async fn send_activity(
        &self,
        session: &Arc<Session>,
        changes: Vec<(String, String, bool)>,
        trackers: &mut HashMap<String, Tracker>,
    ) {
        if self
            .system_notice_blocked(session, "activity", messaging::ACTIVITY_SUBJECT)
            .await
        {
            return;
        }
        if self
            .post_system(
                session,
                "activity",
                messaging::ACTIVITY_SUBJECT,
                activity_body(&changes),
            )
            .await
        {
            for (id, _, state) in changes {
                if let Some(tracker) = trackers.get_mut(&id) {
                    tracker.announced(state);
                }
            }
        }
    }

    fn live_watched_peers(&self, session: &Arc<Session>) -> Vec<Arc<Session>> {
        let watched = session.watch().to_vec();
        let sessions = self.sessions.snapshot();
        watched
            .iter()
            .filter_map(|name| {
                sessions
                    .iter()
                    .find(|peer| {
                        peer.exit_code().is_none()
                            && peer.name() == Some(name.as_str())
                            && messaging::visible(session.party(), peer.party())
                    })
                    .cloned()
            })
            .collect()
    }

    fn watcher_may_receive(
        &self,
        session: &Arc<Session>,
        interval: Duration,
        now: Instant,
    ) -> bool {
        let watcher_ready = {
            let state = session.lock();
            harness::ready(
                session.harness(),
                &state.emulator.screen(),
                state.emulator.is_scrolled(),
            )
        };
        watcher_ready
            && !session.resetting()
            && AnyChannel::for_session(session.clone())
                .hold_reason()
                .is_none()
            && now.saturating_duration_since(session.last_change()) >= interval
    }

    async fn heartbeat_digest(
        &self,
        session: &Arc<Session>,
        observed: Vec<ObservedPeer>,
    ) -> Option<Vec<HeartbeatPeer>> {
        if self
            .system_notice_blocked(session, "heartbeat", messaging::HEARTBEAT_SUBJECT)
            .await
        {
            return None;
        }
        let counts = match self.store.open_counts(&self.boot).await {
            Ok(counts) => counts,
            Err(error) => {
                tracing::error!(%error, "heartbeat queue lookup failed");
                return None;
            }
        };
        let mut digest_peers = Vec::with_capacity(observed.len());
        let mut has_busy_peer = false;
        for ObservedPeer {
            peer,
            idle,
            busy_for,
            unchanged_for,
            hold,
        } in observed
        {
            let queued = counts.get(&peer.id().0).copied().unwrap_or_default();
            if !idle || queued > 0 {
                has_busy_peer = true;
            }
            let last_message_age = match self.store.last_message_age(&self.boot, &peer.id().0).await
            {
                Ok(age) => age.map(Duration::from_secs),
                Err(error) => {
                    tracing::error!(
                        %error,
                        peer = %peer.id().0,
                        "heartbeat message age lookup failed"
                    );
                    None
                }
            };
            digest_peers.push(HeartbeatPeer {
                name: peer.name().unwrap_or(&peer.id().0).to_owned(),
                address: messaging::address(peer.name(), &peer.id().0, &self.host_name),
                busy_for,
                unchanged_for: Some(unchanged_for),
                hold,
                last_message_age,
                queued,
            });
        }
        if !has_busy_peer || digest_peers.is_empty() {
            return None;
        }
        Some(digest_peers)
    }

    async fn send_heartbeat(
        &self,
        session: &Arc<Session>,
        interval: Duration,
        peers: Vec<HeartbeatPeer>,
    ) {
        self.post_system(
            session,
            "heartbeat",
            messaging::HEARTBEAT_SUBJECT,
            heartbeat_body(interval, &peers),
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::{HeartbeatPeer, heartbeat_body};
    use std::time::Duration;

    #[test]
    fn heartbeat_body_matches_digest_example() {
        let body = heartbeat_body(
            Duration::from_secs(30 * 60),
            &[
                HeartbeatPeer {
                    name: "developer".into(),
                    address: "developer@host-a".into(),
                    busy_for: Some(Duration::from_secs(12 * 60)),
                    unchanged_for: Some(Duration::from_secs(11 * 60)),
                    hold: None,
                    last_message_age: Some(Duration::from_secs(18 * 60)),
                    queued: 0,
                },
                HeartbeatPeer {
                    name: "tester".into(),
                    address: "tester@host-a".into(),
                    busy_for: None,
                    unchanged_for: Some(Duration::from_secs(45)),
                    hold: Some("human_draft".into()),
                    last_message_age: None,
                    queued: 2,
                },
            ],
        );
        assert_eq!(
            body,
            "Heartbeat: you have been idle for 30m. Watched peers:\n\
- developer (developer@host-a): busy for 12m, screen unchanged for 11m, last message 18m ago, 0 queued\n\
- tester (tester@host-a): ready, held: human_draft, last message none, 2 queued\n\
Run a2amx screen <peer> to look before messaging a busy peer."
        );
    }
}
