# Spec 3ab: split `Runtime::heartbeat` below the Sonar cognitive-complexity limit

## 0. Status

**DRAFT** (2026-10-14; consultant-reviewed and amended, not yet LOCKED: at lock, fill in the baseline commit, re-read the line anchors and re-run the pre-lock gate against the then-current tree). Architect-directed, backlog I5 part 3, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Sequencing:** this spec is assigned only after spec 3aa (same file) is accepted and committed; the developer re-reads where `heartbeat` starts at the new baseline (it is near line 659 at `0ce2ee4` and moves after 3aa), so two writers never share `src/daemon.rs`.

**Baseline:** develop at `<fill at commit>`; the developer records `cargo test --test daemon_cli heartbeat` and the unit test `heartbeat_body_matches_digest_example` before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the daemon through the CLI (`tests/daemon_cli.rs`: `idle_watcher_receives_heartbeat_for_busy_peer`,
`idle_watcher_skips_heartbeat_for_ready_peer`, `held_delivery_keeps_at_most_one_heartbeat_open`,
`not_ready_watcher_does_not_receive_heartbeat`). New helpers are private to `src/daemon.rs`.

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: `heartbeat` and the new private items. Placement: `struct ObservedPeer` and the free function `observe_peers` at module level next to `struct HeartbeatPeer` and `heartbeat_body` (near line 87-130); the `&self` helpers inside `impl Runtime` directly after `heartbeat`
- `tests/daemon_cli.rs`: only characterization tests of section 4

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `Runtime::heartbeat` (35) against 30 (backlog I5). One loop body finds the watched peers, tracks how long
each has been busy, decides whether the watcher may receive a digest, queries the store, builds the digest and inserts the
message.

## 2. Rule: behavior is unchanged

Same message body and subject, same skip conditions in the same order, same store calls in the same order, same log lines
and levels, same one-second poll. Keep the `// shortcut:` comment above the ticker. No `pub` change.

## 3. Changes in `src/daemon.rs`

Keep `heartbeat`'s signature and the `interval(HEARTBEAT_POLL)` ticker and `busy_since` map in the function. The per-tick
order today, which must not change: (1) `tick`, (2) return if the watcher has exited, (3) `now`, (4) collect live visible watched
peers (not exited, name matches, `messaging::visible`), (5) for each peer compute idle, unchanged_for, hold and `busy_for`,
updating `busy_since` (this runs BEFORE the skip checks, so the map keeps updating on skipped ticks), (6) skip (continue) if the
watcher is not ready, is resetting, is held, or changed less than `interval` ago, (7) skip if a system heartbeat is already open
(store error: log and skip), (8) `open_counts` (store error: log and skip), (9) per peer: queued count, `has_busy_peer`,
`last_message_age` (store error: log, use `None`), build `HeartbeatPeer`, (10) skip if no busy peer or no peers, (11) return if the
watcher has exited, (12) build and insert the message, notify on success, log on `QueueFull`/`Internal`.

Two details to keep verbatim: (i) in step 9 `last_message_age` is awaited for EVERY peer, even after `has_busy_peer` is already true; no early exit and no reordering inside that row loop (the store calls must not change); (ii) `idle` is `!activity.is_some_and(|value| value != Activity::Idle)` (a `None` activity counts as idle), `busy_for` uses the same predicate un-negated, and `has_busy_peer` is `!idle || queued > 0`; copy these expressions unchanged.

Extract:

1. `fn live_watched_peers(&self, session: &Arc<Session>) -> Vec<Arc<Session>>`: step 4.
2. `struct ObservedPeer { peer, idle, busy_for, unchanged_for, hold }` and `fn observe_peers(live: Vec<Arc<Session>>, busy_since:
   &mut HashMap<String, Instant>, now: Instant) -> Vec<ObservedPeer>`: step 5, same arithmetic and the same
   `entry(id).or_insert(now)` / `remove(&id)` calls.
3. `fn watcher_may_receive(&self, session: &Arc<Session>, interval: Duration, now: Instant) -> bool`: step 6 (ready, not
   resetting, no hold, idle at least `interval`).
4. `async fn heartbeat_digest(&self, session: &Arc<Session>, observed: Vec<ObservedPeer>) -> Option<Vec<HeartbeatPeer>>`: steps 7 to
   10 (open heartbeat check, counts, per-peer digest rows). `None` means "skip this tick": an open system heartbeat exists, the open-heartbeat lookup failed (log `heartbeat state lookup failed` WITH the `recipient` field), `open_counts` failed (log `heartbeat queue lookup failed`, NO `recipient` field), or no busy peer / no peers. A `last_message_age` lookup error is NOT a skip: log it and use `None` for that row. Keep every `tracing` call's fields exactly.
5. `async fn send_heartbeat(&self, session: &Arc<Session>, interval: Duration, peers: Vec<HeartbeatPeer>)`: step 12 (not step 11).
   Step 11, the exit re-check, stays in `heartbeat` between the digest and the send, with `return`.

## 4. Safety net

Existing tests in `tests/daemon_cli.rs`: `idle_watcher_receives_heartbeat_for_busy_peer` (busy through activity, step 12 Ok path),
`idle_watcher_skips_heartbeat_for_ready_peer` (step 10 skip), `held_delivery_keeps_at_most_one_heartbeat_open` (step 7
`open == true`; it does not cover the watcher-hold branch of step 6 because a heartbeat is delivered there),
`not_ready_watcher_does_not_receive_heartbeat` (step 6, readiness only). `new_heartbeat_validates_interval_and_requires_watch`
is CLI validation and does not run the loop. Untested: step 6 resetting, watcher hold and interval age; the empty-digest skip;
both store-error skips and the `last_message_age` error; the step-11 re-check; `QueueFull` and `Internal`.

Add exactly one test, passing on the unchanged code, for the only `has_busy_peer` branch with no test (`queued > 0` with an idle
peer): a ready, idle watched peer started with `--deliver hold` that has one message sent to it must still produce a heartbeat to
the idle watcher. Use the same wait style as the existing tests. If the setup cannot be made deterministic with the existing helpers,
do not force it: report that, and rely on the diff review. The store-error paths and the step-11 race have no cheap deterministic
setup; they rely on reading the diff, which the report must say.

## 5. Out of scope

Everything else in `daemon.rs`, `HeartbeatPeer`, `heartbeat_body`, `session_activity`, the poll constant.

## 6. Acceptance

```sh
cargo test --test daemon_cli heartbeat
cargo test --lib heartbeat_body   # the unit test in src/daemon.rs `mod tests`
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/daemon.rs and, if added, tests/daemon_cli.rs
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
and `attach_does_not_capture_session_already_in_drag_mode` (backlog I7): rerun once, record as pre-existing. Final proof of the
complexity fix is the Sonar rescan after the owner pushes.
