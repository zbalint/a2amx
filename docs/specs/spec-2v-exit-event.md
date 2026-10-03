# Spec 2v: exit event, the daemon tells watching sessions when a peer exits

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `59cb3a6` and re-read against `d80c399`; greps in section 9).
Decisions D1 to D9 come from the owner's design of 2026-10-03 (backlog item F8, "no timer, covers
crashes only") with the open points (who watches whom, the sender, the body) settled by the
architect on the owner's instruction to decide while away; they are marked **(architect)**.

**Baseline:** develop at `d80c399`, the commit that implements spec 2u (`Session::kill(graceful)`,
`--now`); this spec adds the daemon-ended flag at the top of that `kill`. **Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
Shared task `context_id`: `a2amx-exit-event`. Public test seam: the `a2amx` CLI against a real
daemon in a temp state dir (`tests/daemon_cli.rs`, `tests/common`); the new message is observed
with `a2amx messages`.

**Scope.** May edit exactly these files and no others.

- `src/wire.rs` (`Request::NewSession.watch`)
- `src/messaging.rs` (the system sender constant, one reserved name in `validate_name`)
- `src/team.rs` (`TeamSession.watch`, validated)
- `src/cli.rs`, `src/main.rs` (`new --watch`, `team up` forwards it, the `NewOptions` and
  `NewSession` literals)
- `src/session.rs` (`Session.watch`, a daemon-ended flag, `wait_exit` made `pub(crate)`)
- `src/daemon.rs` (store the list, the exit observer task, the message insert)
- tests: `tests/daemon_cli.rs`, `tests/team.rs`, `tests/wire.rs`, and the mechanical
  `watch: vec![]` additions to every existing `NewSession`, `SessionSpec`, `NewOptions` and
  `TeamSession` literal in `tests/*.rs` and `src/*.rs`
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `a2amx.toml.example`, `docs/backlog.md`

Does not touch: `src/store.rs`, `src/delivery.rs`, `src/harness.rs`, `src/mcp.rs`,
`src/codex.rs`, `scripts/`, `extension/`. No new dependency, no new `Request`. Do not commit,
stage or merge: leave the diff uncommitted for review.

## 1. Why

An architect session that dispatched work and sits idle only wakes on messages. If its developer
crashes or its harness quits, nothing wakes the architect; the owner finds out hours later. The
daemon already observes every child exit (the monitor thread in `Session::spawn`). It can queue
one message to the sessions that asked to be told, through the normal delivery path and gates.
This covers exits and crashes only. A peer that loops forever is the heartbeat's job (backlog F9).

## 2. Decisions

- **D1. (architect)** A session declares whom it watches: `watch = ["developer"]` in its team
  file entry, `a2amx new --watch NAME` (repeatable) for the flag form. Entries are session names
  validated by `validate_name`. A name may be absent at declaration time (the peer can start
  later); a session may not list its own name (team file and `new` both reject it). The list is
  stored on the watcher's `Session` and sent as `NewSession.watch` (`#[serde(default,
  skip_serializing_if = "Vec::is_empty")]`, like `control_from`). Default: watches nobody.
- **D2.** When a named session's child exits, the daemon queues exactly one message for every
  live session whose `watch` contains that name. Unnamed sessions cannot be watched. A watcher
  that has itself exited gets nothing. No timer, no retry, no re-send.
- **D3. (architect)** The sender is a fixed system identity: `sender_session` `daemon`,
  `sender_address` `a2amx-daemon@<host name>` (`messaging::SYSTEM_SENDER`, built with
  `messaging::address(Some(SYSTEM_SENDER), ...)`). `validate_name` rejects `a2amx-daemon` as a
  session name (next to the existing `s[0-9]+` rule, message `reserved for the daemon`), so no
  session can impersonate it; this covers `new --name`, `team up` and `create_session`, all of
  which already call `validate_name`.
- **D4. (architect)** Subject `peer exited: <name>`. Body, one line per fact, plain text:
  `<name> (<address>) exited with code <N>.` where `<N>` is `Session`'s exit code (128 plus the
  signal for a signal death, as stored), then `Run a2amx screen <name> to see its last screen.`
  Nothing else (no screen content, no cwd, no argv).
- **D5. (architect)** No message when the daemon itself ended the session: `Session::kill` (any
  path: `a2amx kill`, `team down`, `daemon stop`, and spec 2u's graceful sequence) sets a
  `daemon_ended: AtomicBool` before it signals or types anything. The observer reads it after
  the exit and skips the event when it is set. A human typing `/exit` inside the session is an
  exit and does produce the event.
- **D6.** The observer runs in the daemon, one tokio task per session, spawned in
  `create_session` after the session is registered: `session.wait_exit().await`, then D5, then
  one `store.insert_message` per watcher with the `NewMessage` fields of `send_message`
  (`boot`, the D3 sender, the watcher as recipient) and `recipient.message_notify().notify_one()`
  as `send_message` does. It bypasses the per-sender rate limiter (the system sender has no
  session) but not the store's queue limit: a `QueueFull` is logged with `tracing::warn!` and
  dropped, never retried. Other insert errors are logged with `tracing::error!`.
- **D7.** A daemon that is shutting down sends nothing: `Daemon::shutdown` already aborts the
  deliveries first; it also aborts the observer tasks (store their `JoinHandle`s with the
  existing delivery handles or one more `Vec`, the developer's choice inside `src/daemon.rs`) so
  no event is inserted while sessions are being killed.
- **D8.** `watch` is validated twice, as `control_from` is: in `team::validate_sessions` and in
  `create_session` (the wire is the trust boundary). A bad entry fails the request with the same
  message style as `control_from entry {name:?}: {error}`.
- **D9.** Delivery of the event is the ordinary one: the message waits when the watcher is busy,
  held or has a draft, and appears in `a2amx messages` with its state. Spec 2s's reset hold and
  every delivery gate apply unchanged.

## 3. `src/wire.rs`, `src/messaging.rs`

`Request::NewSession` (the variant carrying `control_from`) gains `watch: Vec<String>` per D1.
`messaging.rs` gains `pub const SYSTEM_SENDER: &str = "a2amx-daemon";` and the reserved-name
check of D3 in `validate_name`.

## 4. `src/team.rs`, `src/cli.rs`, `src/main.rs`

- `TeamSession.watch: Vec<String>` (`#[serde(default)]`, `deny_unknown_fields` stays), validated
  in `validate_sessions` with `validate_name`, own name rejected (`session X watches itself`).
  `flag_sessions` literals get `watch: Vec::new()`.
- `Command::New` gains `--watch` (repeatable, doc: `Session names whose exit this session is
  told about`). `NewOptions` and the `NewSession` request carry it; `team up` forwards the team
  entry's list.

## 5. `src/session.rs`, `src/daemon.rs`

- `Session.watch: Vec<String>` from `SessionSpec`; `daemon_ended: AtomicBool` set first thing in
  `kill`; `wait_exit` becomes `pub(crate)`; a `watch()` accessor.
- `create_session`: validate and store `watch` (D8), spawn the observer (D6). The observer needs
  the runtime (`Arc` as the other tasks use) and the session; it looks watchers up by `name()`
  at exit time, so a watcher started after the peer still counts only if it is live then.
- `Daemon::shutdown`: D7.

## 6. Tests (failing first, one behavior at a time, through the CLI)

Existing `NewSession` literals and friends get `watch: vec![]` and nothing else changes. New:

Tests must not assert on `daemon.log` text (it is colored unless `NO_COLOR` is set); observe
everything through `a2amx messages` and `a2amx list`. Fixtures are Generic `sh -c` sessions.

1. `new --watch peer` on a watcher; `peer` is `sh -c "exit 3"`. Within a bounded wait,
   `a2amx messages --session <watcher>` lists one message from `a2amx-daemon@<host>` with
   subject `peer exited: peer` and a body containing `exited with code 3`. A session that does
   not watch `peer` has no such message.
2. Two watchers of the same peer each get exactly one message.
3. `a2amx kill peer --yes` (and `team down`) produces no message (D5); a peer that exits by
   itself after the watcher exists does.
4. A name equal to `a2amx-daemon` is rejected by `new --name` and by a team file.
5. `team up` with `watch = ["developer"]` on one session: the entry is forwarded (observable
   as in test 1); `watch = ["self"]` on the entry named `self` is rejected, as is a bad name.
6. Wire serialization: `NewSession` with an empty `watch` has no `watch` key (`tests/wire.rs`).
7. The watcher itself exited before the peer: no message and no error in the log.

## 7. Docs

`README.md` (`new --watch`, the team file field), `docs/architecture.md` (the observer and
D5), `docs/delivery.md` (system sender, same gates), `a2amx.toml.example` (a commented `watch`
example on the architect entry), `docs/backlog.md` (delete row F8).

## 8. Out of scope

Heartbeat and any timer (F9), notifying on a hang, a `kill --exited` flow, restarting a crashed
peer, an exit event for sessions nobody names, per-event configuration (only exit exists), and
any change to `reset` or to graceful exit.

## 9. Acceptance and gate notes

```sh
env -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass; a missed struct literal fails to compile. `git status` shows only files from
section 0. Gate notes at lock time: `control_from` appears in 19 files (the same fanout applies
to `watch`; the mechanical literals are in scope above), `validate_name` is called by `new`,
`team` and `create_session` so D3's reserved name needs no other call site, and the store has no
foreign key on `sender_session`, so a synthetic sender inserts as an ordinary row.
