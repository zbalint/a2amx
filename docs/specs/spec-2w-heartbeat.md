# Spec 2w: heartbeat, a digest of the watched peers for an idle orchestrator

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `7b62858` and re-read against `ce9d8e1`; the gate notes are in section 10). Decisions D1 to D11 come from the owner's design
of 2026-10-03 (backlog item F9) with the open points settled by the architect on the owner's
instruction to decide while away; those are marked **(architect)**.

**Baseline:** develop at `ce9d8e1`, the commit that implements spec 2v (the `watch` list and the system
sender `a2amx-daemon@<host>` this spec reuses). **Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared
task `context_id`: `a2amx-heartbeat`. Public test seam: the `a2amx` CLI against a real daemon in a
temp state dir (`tests/daemon_cli.rs`, `tests/common`); heartbeats are observed with
`a2amx messages`. Tests never assert on `daemon.log` text and never depend on `NO_COLOR`.

**Scope.** May edit exactly these files and no others.

- `src/wire.rs` (`Request::NewSession.heartbeat`)
- `src/messaging.rs` (the interval parser, the heartbeat subject constant)
- `src/team.rs` (`TeamSession.heartbeat`, validated)
- `src/cli.rs`, `src/main.rs` (`new --heartbeat`, `team up` forwards it, literals)
- `src/session.rs` (`Session.heartbeat`, `last_change`, accessors)
- `src/store.rs` (one read method, D6)
- `src/daemon.rs` (the heartbeat task per session, its abort at shutdown, the digest)
- tests: `tests/daemon_cli.rs`, `tests/team.rs`, `tests/wire.rs`, `tests/store.rs` if it exists,
  and the mechanical `heartbeat: None` additions to every existing `NewSession`, `SessionSpec`,
  `NewOptions` and `TeamSession` literal in `tests/*.rs` and `src/*.rs`
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `a2amx.toml.example`, `docs/backlog.md`

Does not touch: `src/delivery.rs`, `src/harness.rs`, `src/mcp.rs`, `src/codex.rs`, `scripts/`,
`extension/`. No new dependency, no new `Request`. Do not commit, stage or merge: leave the diff
uncommitted for review.

## 1. Why

An architect session that dispatched work and went idle only wakes on a message. Spec 2v covers
a peer that exits or crashes; nothing covers a peer that is alive but stuck or looping. The
daemon cannot tell long legitimate work from a loop, so it does not judge: after the architect
has been idle for a configured interval it sends one message with evidence about the watched
peers, and the architect (an agent that can run `a2amx screen <peer>`) judges.

## 2. Decisions

- **D1.** Per session, configured on the receiving session: `heartbeat = "30m"` in its team file
  entry, `a2amx new --heartbeat 30m` for the flag form. The value is a positive integer followed
  by `s`, `m` or `h` (`90s`, `30m`, `2h`), between 1 s and 24 h. One parser in `messaging.rs`
  (`parse_interval(&str) -> anyhow::Result<Duration>`), used by the team validator, the `new`
  option and `create_session`. The wire carries whole seconds: `NewSession.heartbeat: Option<u64>`
  (`#[serde(default, skip_serializing_if = "Option::is_none")]`). Default: no heartbeat.
- **D2. (architect)** A heartbeat needs something to report on: it is rejected when the same
  session's `watch` list (spec 2v) is empty (`session X: heartbeat needs a non-empty watch`), in
  the team validator and in `create_session`. The digest covers exactly the sessions named in
  `watch`.
- **D3.** One tokio task per session that has a heartbeat, spawned in `create_session` next to
  spec 2v's exit observer and aborted at `Daemon::shutdown` the same way. It wakes every
  `HEARTBEAT_POLL` (1 s) and ends when its session exits.
- **D4.** Idle means: the session's screen satisfies `harness::ready` (at its prompt), it is not
  held (`hold_reason` is none) and not `resetting`, and `now - last_change >= interval`.
  `Session.last_change: Instant` (set to the creation instant, and refreshed wherever the output
  path sets `state.dirty = true` after PTY output; the `resize_locked` use of `dirty` does not
  count) is the single activity clock, so any output, including the echo of an incoming message,
  resets it. Delivery of a heartbeat therefore restarts the timer without extra code.
- **D5.** At most one undelivered heartbeat: no heartbeat is created while a message from the
  system sender with subject `heartbeat` to this session is still `pending` or `delivering`
  (the store read of D6).
- **D6.** Store: one new read method, `open_system_heartbeat(boot, session) -> Result<bool>`
  and a second one, `last_message_age(boot, session) -> Result<Option<u64>>` (seconds since the
  newest `updated_at` over messages where the peer is `sender_session` or `recipient_session`;
  `None` when there is none). Parameterized queries only; both next to `open_counts`.
- **D7. (architect)** Skip rule: when every watched peer is, at this tick, **idle with an empty
  queue**, send nothing. A peer is idle with an empty queue when it exists, has not exited,
  satisfies `harness::ready`, has no hold, and has no `pending` or `delivering` messages
  (`open_counts`). A peer that has exited or does not exist is not "busy" and does not prevent
  the skip (spec 2v reports exits); if no watched peer is live, send nothing. Otherwise (at least
  one live peer that is busy, held or has a queue) a heartbeat is created.
- **D8.** Message: sender is spec 2v's system sender; subject `heartbeat` (constant
  `messaging::HEARTBEAT_SUBJECT`); recipient is the session. It goes through `insert_message`
  and `message_notify` exactly like spec 2v's exit event, bypasses the per-sender rate limiter,
  and then the ordinary delivery gates (busy, held, draft, reset hold) apply. A `QueueFull` is
  logged with `tracing::warn!` and dropped. The sender is not a session, so a reply to it fails
  with `unknown recipient`; nothing special is needed.
- **D9. (architect)** Body: one line per live watched peer, a header and a hint, plain text:

  ```
  Heartbeat: you have been idle for <interval>. Watched peers:
  - developer (developer@host-a): busy for 12m, screen unchanged for 11m, last message 18m ago, 0 queued
  - tester (tester@host-a): ready, held: human_draft, last message none, 2 queued
  Run a2amx screen <peer> to look before messaging a busy peer.
  ```

  Fields per peer, in this order and wording: `busy for <d>` or `ready`; `screen unchanged for
  <d>` (from the peer's `last_change`; omitted when it is under 1 minute); `held: <reason>` only
  when held (`hold_reason`); `last message <d> ago` or `last message none` (D6); `<N> queued`.
  `<d>` is the largest whole unit of `s`, `m`, `h` that is at least 1, for example `45s`,
  `12m`, `3h`. "Busy for" is the time since the daemon's heartbeat task last saw the peer ready,
  kept in the task's own map (a peer first seen busy counts from that first sight);
  `// shortcut:` comment: poll resolution of 1 s and a lower bound for peers busy before the
  task started, add a per-session ready-since clock if exact times are ever needed.
- **D10.** The task never sends Ctrl-C or any input itself; it only inserts a message. It never
  touches a peer session.
- **D11.** Nothing changes for sessions without `heartbeat`, and no existing wire, list or
  table output changes.

## 3. `src/wire.rs`, `src/messaging.rs`

`NewSession.heartbeat` per D1. `messaging.rs`: `parse_interval` (D1; error text `invalid
heartbeat {value:?}: expected a number followed by s, m or h, from 1s to 24h`) and
`HEARTBEAT_SUBJECT`.

## 4. `src/team.rs`, `src/cli.rs`, `src/main.rs`

`TeamSession.heartbeat: Option<String>` (`deny_unknown_fields` stays), validated in
`validate_sessions` with `parse_interval` and D2. `Command::New` gains `--heartbeat` (doc: `Send
this session a digest of its watched peers when it has been idle this long, for example 30m`),
`NewOptions` and the request carry it as seconds; `team up` forwards the entry's value.
`flag_sessions` literals get `heartbeat: None`.

## 5. `src/session.rs`, `src/store.rs`, `src/daemon.rs`

- `Session.heartbeat: Option<Duration>` from `SessionSpec`, a `heartbeat()` accessor,
  `last_change` per D4 (an accessor for the daemon).
- `store.rs`: D6, next to `open_counts`, same `self.run(move |connection, _| ...)` pattern.
- `daemon.rs`: in `create_session`, validate and store; spawn the task of D3 to D9. The digest
  builder is a private function in `daemon.rs` taking plain values (peer name, address, busy
  duration, unchanged duration, hold, last-message age, queued count) and returning the body, so
  its wording is unit-tested without a daemon.

## 6. Tests (failing first, one behavior at a time)

Existing literals get `heartbeat: None`; nothing else changes. Fixtures are Generic `sh -c`
sessions: a **ready** fixture turns bracketed paste on (`printf '\033[?2004h'`) and then `sleep
60`; a **busy** fixture never turns it on (`sleep 60`), so `harness::ready` is false for it.

1. Unit (daemon.rs `#[cfg(test)]`): the digest builder reproduces the D9 example from literals,
   including `ready`, `held: human_draft`, `last message none`, and the omission of `screen
   unchanged` under a minute.
2. Unit (`messaging.rs`): `parse_interval` accepts `90s`, `30m`, `2h`, `24h`, rejects `0s`,
   `25h`, `30`, `m`, `1d`, `-5m`.
3. CLI, positive: a ready watcher `arch` with `--watch dev --heartbeat 2s`, a busy `dev`: within a
   bounded wait `a2amx messages --session arch` lists a message from `a2amx-daemon@<host>` with
   subject `heartbeat`.
4. CLI, skip rule: same, but `dev` is a ready fixture: after waiting several intervals there is
   no heartbeat.
5. CLI, at most one: the watcher also has `--deliver hold` (messages stay pending) and `dev` is
   busy; after waiting at least four intervals exactly one heartbeat is listed.
6. CLI, validation: `new --heartbeat 30m` without `--watch` fails with the D2 message;
   `--heartbeat 0s` and `--heartbeat 25h` fail; a team file entry with `heartbeat` and `watch`
   is accepted and one without `watch` is rejected (`tests/team.rs`).
7. Wire: `NewSession` without `heartbeat` has no `heartbeat` key (`tests/wire.rs`).
8. A watcher that is not ready (a busy fixture with `--heartbeat 2s`) never gets one while
   it is not at its prompt.

## 7. Docs

`README.md` (`new --heartbeat`, the team file field), `docs/architecture.md` (the heartbeat
task, the single activity clock, the skip rule), `docs/delivery.md` (system sender, same
gates, at most one undelivered), `a2amx.toml.example` (a commented `heartbeat` and `watch`
example on the architect entry), `docs/backlog.md` (delete row F9).

## 8. Out of scope

Quota handling, a heartbeat for sessions without a watch list, per-peer intervals, replying to
a heartbeat, any action on a peer (no nudge, no kill), a different clock per harness, changing
spec 2v's exit event, and `list_agents` fields (spec 2x).

## 9. Acceptance

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass. A missed struct literal fails to compile. `git status` shows only files from
section 0.

## 10. Gate notes at lock time

`control_from` appears in 19 files, so `heartbeat` has the same literal fanout; `open_counts`
already counts `pending` and `delivering` per recipient; the store has no foreign key on
`sender_session`; `a2amx messages` prints FROM, TO, STATE, DETAIL and SUBJECT, so tests
observe the subject, not the body; `harness::ready` for the Generic harness is bracketed paste
on and not scrolled, which the two fixtures rely on; the output path that sets `dirty` is the
PTY reader in `Session::spawn` (line 222 at `7b62858`). Spec 2v's `watch`, system sender and exit observer exist at `ce9d8e1`.
