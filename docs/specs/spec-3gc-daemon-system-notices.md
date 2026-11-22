# Spec 3gc: the system-notice producers become one module with one posting helper (daemon S3)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing heartbeat, activity, peer-exit and presence tests (`tests/daemon_cli/heartbeat_and_activity.rs`, `tests/daemon_cli/peer_exit_watch.rs`, `tests/presence/`, `tests/human_inbound/`); the helpers are private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/notices.rs`.

Does not touch: every other file under `src/`, every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `daemon` row of `AGENTS.md` and the backlog when committing.

## 1. Why

`src/daemon.rs` is 3,745 lines. Five producers of system messages sit in it (presence "owner away", peer exit, activity, heartbeat, and the digest builder) as about 560 lines of `Runtime` methods and free functions spread between `:150` and `:1230`. Four of them copy the same block: build a `NewMessage` from the system sender (`SYSTEM_SESSION`, `SYSTEM_SENDER`, the recipient's address), `insert_message`, notify the recipient, and log `QueueFull` at `warn` and `Internal` at `error`. Three places (presence, activity, and the heartbeat digest) also copy the "is an unanswered notice of this subject already open" lookup with its error log. The copies differ only in the subject, the body, the log wording and what happens after a successful insert (activity acknowledges its trackers). The wording differs only by a prefix word, so a reader cannot tell whether the differences are intended. The consultant's design read (m_8377, backlog F94) names this the S3 redesign.

Target shape: a new module `src/daemon/notices.rs` holding these producers, and two private `Runtime` helpers in it:

```rust
/// Insert one system message for `recipient`, notify it, and log a refusal. True when accepted.
async fn post_system(&self, recipient: &Arc<Session>, kind: &str, subject: &str, body: String) -> bool;
/// True when a notice of `subject` is still open for `recipient`, or the lookup failed (the lookup failure is logged).
async fn system_notice_blocked(&self, recipient: &Arc<Session>, kind: &str, subject: &str) -> bool;
```

`kind` is the word that prefixes the log text (`presence`, `exit event`, `activity`, `heartbeat`): `post_system` logs `"{kind} message queue is full"` at `warn` and `"{kind} message acceptance failed"` at `error`; `system_notice_blocked` logs `"{kind} state lookup failed"` at `error`, each with the existing `recipient` field (and `%error` on the error lines). The four existing texts are reproduced exactly.

Rejected: a `Notice` struct with a builder (two helpers cover every caller; one function, no state); a trait for the producers (no second implementation); moving `start_session_tasks` and the `tokio::spawn` sites (they belong with the launch code in S6); also moving `owner_cause` (it serves `list_agents` and `send_message`, not only presence; it moves with the views).

## 2. Mechanical changes

### 2.1 New `src/daemon/notices.rs`

Move, in this order and byte for byte except as 2.2 says:

- the types `HeartbeatPeer` and `ObservedPeer` and the functions `observe_peers`, `heartbeat_body`, `activity_body` (`src/daemon.rs:126-222`);
- inside `impl Runtime`: `poll_transitions`, `poll_presence`, `send_presence_notices`, `observe_exit`, `heartbeat`, `activity`, `send_activity`, `live_watched_peers`, `watcher_may_receive`, `heartbeat_digest`, `send_heartbeat` (`:769-1230` minus `sample_activity` at `:924-954`, which stays: it posts no message, it feeds `note_needs_input` and `note_activity`; it and `ACTIVITY_POLL` move with the launch and task code in S6; one `impl Runtime` block);
- the unit test `heartbeat_body_matches_digest_example` with its module (`:3707-3745`; the only test of the digest text: no daemon-backed test pins it, `rg 'you have been idle for' tests` prints nothing). It keeps its place beside `heartbeat_body`; it is the one test of a private function, kept because it is the only pin of that text and the function is pure.

Constant `HEARTBEAT_POLL` (`:44`) moves: only the moved code uses it (`:791, :957, :988`). `ACTIVITY_POLL` (`:46`, used by `sample_activity` at `:927`), `ACTIVITY_QUIET` (`:3315`, `:3335`) and `NEEDS_INPUT_AFTER` (`session.rs:311`) stay. Visibility: methods that `daemon.rs` still calls are `pub(super)`: `observe_exit`, `heartbeat` and `activity` (spawned by `start_session_tasks` at `:747, :751, :755`) and `poll_presence` (spawned by `Daemon::start` at `:2345`); everything else stays private to the child, including the two helpers. The moved code calls three private items of the parent that stay (`session_activity` at `:3349`, `session_working` at `:3306`, `Runtime::remote_ineligible` at `:1588`); a child module sees them, so it needs `use super::{Runtime, session_activity, session_working};` and no visibility change. `notices.rs` imports what the moved code uses: `crate::activity::{Thresholds, Tracker}`, `crate::delivery::AnyChannel`, `crate::harness::{self, Harness}`, `crate::messaging::{self, display_duration}`, `crate::session::Session`, `crate::store::{InsertError, NewMessage}`, `HashMap`, `HashSet`, `Arc`, `Duration`, `Instant` and `tokio::sync::watch` (the compiler names any miss).

### 2.2 The two helpers and their four call sites

`post_system` and `system_notice_blocked` live in `notices.rs`'s `impl Runtime`. Each of the four copied blocks becomes a call:

- `send_presence_notices`: `if self.system_notice_blocked(&session, "presence", messaging::PRESENCE_SUBJECT).await { continue; }`, then `self.post_system(&session, "presence", messaging::PRESENCE_SUBJECT, messaging::OWNER_AWAY_BODY.into()).await;` (result ignored).
- `observe_exit`: for each watcher `self.post_system(&watcher, "exit event", &subject, body.clone()).await;` (result ignored; no open check there, as today).
- `send_activity`: `if self.system_notice_blocked(session, "activity", messaging::ACTIVITY_SUBJECT).await { return; }`, then `if self.post_system(session, "activity", messaging::ACTIVITY_SUBJECT, activity_body(&changes)).await { for (id, _, state) in changes { if let Some(tracker) = trackers.get_mut(&id) { tracker.announced(state); } } }`.
- `heartbeat_digest`: its opening `open_system_message` match (`:1131-1144`, with the `"heartbeat state lookup failed"` log) becomes `if self.system_notice_blocked(session, "heartbeat", messaging::HEARTBEAT_SUBJECT).await { return None; }`; the rest of the function is unchanged (its `open_counts` failure log stays as written).
- `send_heartbeat`: `self.post_system(session, "heartbeat", messaging::HEARTBEAT_SUBJECT, heartbeat_body(interval, &peers)).await;` (no open check).

`post_system` builds the `NewMessage` exactly as the four copies do (`boot`, `SYSTEM_SESSION`, the system sender address from `messaging::address(Some(SYSTEM_SENDER), SYSTEM_SENDER, &self.host_name)`, the recipient's session id and address from `messaging::address(recipient.name(), &recipient.id().0, &self.host_name)`, the subject and body), calls `self.store.insert_message(message).await`, and on `Ok(_)` calls `recipient.message_notify().notify_one()` and returns true; on `Err(InsertError::QueueFull)` it logs and returns false; on `Err(InsertError::Internal(error))` it logs and returns false.

### 2.3 `src/daemon.rs`

Declare `mod notices;`. Delete the moved items. Remove from `daemon.rs`'s `use` lines the names only the moved code used (`Thresholds` and `Tracker` at `:20`, `display_duration` at `:27`; `-D warnings` fails on any other leftover, the compiler says which); `InsertError`, `NewMessage`, `AnyChannel`, `HashSet`, `watch`, `harness` and `Harness` are still used there and stay. Nothing else changes. `src/daemon.rs` shrinks by about 530 lines.

## 3. Behaviour

No observable change: the same messages, the same notify, the same log levels, fields and field order, and texts. The three helper log calls assemble the text with an inline format argument instead of the literal each of the eleven former call sites had; the printed text is identical. One order change, invisible: in `send_activity` the notify now happens inside `post_system` before the trackers are acknowledged (it was after); the trackers are local to the activity task and no await lies between the two, so no other task can tell. `observe_exit` builds the system sender address once per watcher instead of once per call; same value. Tracing targets of the moved events change to `a2amx::daemon::notices` (the module path is the default; as in spec 3fp and 3ga, nothing filters on it).

## 4. Out of scope

- Any behaviour change: the unpinned digest-text path, `heartbeat` ordering, the activity ack, the shortcut comments (they move unchanged).
- `owner_cause`, `start_session_tasks`, `remote_ineligible` (S4), `send_message` and the admission code (S6).
- A daemon-backed test of the digest text (a follow-up; the unit test stays).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `a46e46d` (architect, 2026-11-22): 66 suites, 1415 passed, 0 failed, 3 ignored; after: the same count (no test added or removed; the unit test moves); the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'SYSTEM_SESSION|insert_message\(' src/daemon/notices.rs` prints exactly one `SYSTEM_SESSION` line and one `insert_message(` line (both in `post_system`); `rg -n 'SYSTEM_SESSION' src/daemon.rs` prints nothing and `rg -n 'insert_message\(' src/daemon.rs` prints exactly one line (the `send_message` path); `rg -n 'queue is full|acceptance failed|state lookup failed' src/daemon/notices.rs` prints exactly three lines (the two in `post_system`, the one in `system_notice_blocked`); `rg -n 'open_system_message' src/daemon/notices.rs` prints exactly one line (in `system_notice_blocked`);
- `rg -n 'fn (observe_peers|heartbeat_body|activity_body|poll_transitions|poll_presence|send_presence_notices|observe_exit|heartbeat|activity|send_activity|live_watched_peers|watcher_may_receive|heartbeat_digest|send_heartbeat)\b' src/daemon.rs` prints nothing.
- `git status --short` lists only `src/daemon.rs` and `src/daemon/notices.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Mutation check (tester, in the assigned checkout, restoring by checksum), naming the repo test that fails on each: `post_system` returns false always (activity trackers never acknowledged, so a settled state is re-reported: `activity_notices_report_peer_transitions_once_in_order`, which asserts one notice per transition); `post_system` skips `notify_one` (expected survivor: the delivery loop also wakes on a 1 s interval, so delivery is only late; report); `system_notice_blocked` returns false always (expected killers: `held_activity_delivery_keeps_at_most_one_open_message` for activity and `held_delivery_keeps_at_most_one_heartbeat_open` for the heartbeat, both in `tests/daemon_cli/heartbeat_and_activity.rs`; presence may survive, `tests/presence/away_notice.rs::away_notice_is_fixed_and_sent_once_to_remote_sessions_only` is the candidate; report each of the three call sites separately); the `observe_exit` watcher filter loses `messaging::visible`: `tests/teams/team_visibility.rs::mutual_agent_allow_controls_exit_notifications`; the heartbeat body loses the `queued` field: the unit test `heartbeat_body_matches_digest_example`; the exit subject text changed: `peer_exit_watch::two_watchers_each_receive_one_peer_exit_message` (see the Review record: the first-named test never asserts the text). Survivors are reported, not fixed.
- Known flaky tests: the heartbeat, activity and peer-exit tests wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| Four copies of the insert-notify-log block | `src/daemon.rs` `send_presence_notices` (`:804-866`), `observe_exit` (`:867-923`), `send_activity` (`:1024-1085`), `send_heartbeat` (`:1194-1230`) | read |
| Presence, activity and the heartbeat digest do the open-notice lookup; peer exit and `send_heartbeat` do not | `rg -n 'open_system_message' src/daemon.rs`: `:815`, `:1032`, `:1133` | probed |
| The log texts are `{kind} message queue is full`, `{kind} message acceptance failed`, `{kind} state lookup failed` with kinds `presence`, `exit event`, `activity`, `heartbeat`; the heartbeat digest's lookup log is `"heartbeat state lookup failed"` (`:1140`) | the four blocks | read: presence `"presence message queue is full"`, exit `"exit event message queue is full"` and `"exit event message acceptance failed"`, activity and heartbeat likewise, lookup lines `"presence state lookup failed"` and `"activity state lookup failed"` |
| Only activity acts on success | `send_activity`'s `Ok(_)` arm (tracker `announced`) | read |
| `heartbeat_body` is pinned by the unit test only | `rg -n 'you have been idle for\|Heartbeat: ' tests` prints nothing | probed |
| `sample_activity` posts no message | `src/daemon.rs:924-954` | read (consultant m_8436) |
| `NEEDS_INPUT_AFTER` is used outside the daemon | `src/session.rs:311` (consultant m_8404/m_8377) | read |
| `owner_cause` is used by `list_agents` and the `send_message` path | `src/daemon.rs:1232`; callers | grep before lock |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer OMP openai-codex/gpt-5.6-luna; tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | "Nothing else changes" in `daemon.rs` contradicted the use-line removals clippy forces; the child's `use super::{...}` for parent private items was unnamed | section 2.1 and 2.3 name both |
| draft review | consultant | wrong anchors (test module `:3707`, `send_heartbeat` ends `:1230`); `poll_presence` said to be spawned by `start_session_tasks` | corrected |
| draft review | consultant | `sample_activity` posts no message and does not belong in a notices module | stays in `daemon.rs` with `ACTIVITY_POLL` until S6 |
| draft review | consultant | the heartbeat digest is a third open-notice lookup; the `daemon.rs` greps were vague; the activity ack reorder (notify before ack) went unstated | third call site added, greps pinned, reorder stated as invisible |
| draft review | consultant | mutation killers misnamed (activity-ack, exit-visibility), notify mutant expected to survive | named the real tests; survivor stated up front |
| tester pass | tester | the spec's exit-subject killer `watching_session_receives_a_peer_exit_message` passes on the mutant (its helper returns at the deadline and the test never asserts the text) | spec corrected to `two_watchers_each_receive_one_peer_exit_message`; the weak test recorded in backlog F94 as a follow-up |
| tester pass | tester | `post_system` without `notify_one` survives every test (delivery loop also wakes each second) | accepted: the notify is a latency optimisation; recorded |
| implementation | developer | first `cargo check` failed on a missing `delivery::Channel` trait import (`hold_reason`) and an unused `Harness` import | fixed before the report |
