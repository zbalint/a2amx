# Spec 3ee: the owner-presence refusal names its cause (F90, first part)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
Shared task `context_id`: `a2amx-owner-presence-reason-2026-11-13`.

**Scope.** May be edited: `src/messaging.rs`, `src/daemon.rs` (the refusal at lines 1327-1337 only),
`tests/presence.rs`, `docs/remote-messaging.md`.
**Does not touch:** `CHANGELOG.md` and `docs/backlog.md` (the architect edits them), `src/mcp.rs`,
`src/wire.rs`, `src/main.rs` (so `a2amx list` output and the `list_agents` shape do not change),
`tests/attach_cli.rs`, every other file.

**Public test seams.** The daemon through `Request::SendMessage` to `human:owner` with a remote
session and a real `Request::Attach`, as `tests/presence.rs:425-445` does; and the public
`messaging::Presence` with explicit `Instant`s, as `tests/presence.rs:22-105` already does. No test
calls a private function.

## 1. Why

On 2026-11-13 the owner tested remote messaging with a confined Codex session. The session's window
was in viewer mode, more than 60 seconds had passed, and the agent's `send_message(to="human:owner")`
was still refused with "the owner is at the computer; write to them in the terminal instead". The
owner took it for a bug. The cause was another window: the architect session was attached
interactively and the owner was typing in it. Presence is one flag for the whole daemon, counting
interactive (non-viewer) attachments in any session of any team, and the 60-second grace starts only
after the last one ends or turns to viewer (`Presence`, `src/messaging.rs:174-233`). The refusal
text gives no cause, so neither the agent nor the owner reading its report can tell what to change.

Chosen shape: the text distinguishes the two causes without naming any session. A refusal that
named the attached session would tell an agent in one team about sessions of another team, which
the visibility rules (`messaging::visible`) keep apart. The owner already sees which session is
attached in the ATTACHED column of `a2amx list`; the docs will say to look there.

Alternatives rejected: a footer line in `a2amx list` (changes the exact-output tests at
`tests/attach_cli.rs:944,975,1123` and the CLI for one message); a new `owner_reason` field in
`list_agents` (wire and MCP shape change, for an agent that gets the reason in the refusal anyway);
per-session presence (a behaviour change, not a clarification). This spec narrows the owner's
backlog wish (F90): a refusal that names the session, a `list_agents` field and an `a2amx list`
indication of which attachment makes the owner present are **not** delivered here (the first for
the visibility reason above, the others because the agent gets the cause in the refusal and the
owner has the ATTACHED column). F90 stays open for them and the owner may confirm dropping them. The second half of F90, an
attention signal for a detached stuck agent while the owner is present, needs the owner's design
decision and stays in the backlog (with F82).

## 2. `src/messaging.rs`

1. Add `pub enum PresenceCause { Attached, Grace }` (derive `Debug, Clone, Copy, PartialEq, Eq`).
2. Add `pub fn cause(&self, now: Instant) -> Option<PresenceCause>`: `Some(Attached)` when
   `interactive > 0`; else `Some(Grace)` when `last_interactive` is within `away_secs` of `now`;
   else `None`. `present` stays as it is and equals `cause(now).is_some()`; do not duplicate the
   comparison, implement `present` through `cause`.
3. Add `pub fn away_secs(&self) -> u64`.

## 3. `src/daemon.rs`

In the refusal (`src/daemon.rs:1327-1337`), take the cause once under the lock and build the text
from it instead of the fixed string:

- `Attached`: `the owner is at the computer (a terminal is attached interactively to some session; viewer mode does not count); write to them in the terminal instead`
- `Grace`: `the owner was at the computer less than {away_secs} seconds ago; try again later or write to them in the terminal`

The code stays `owner_present`. A `None` cause proceeds exactly as before. Shape: one
`let cause = self.presence.lock().unwrap_or_else(PoisonError::into_inner).cause(Instant::now());`
and an `if let Some(cause) = cause`; the send path no longer calls `present` (it stays for
`list_agents`, `src/daemon.rs:3101`), so there is one read under one lock.

## 4. `docs/remote-messaging.md`

In "When you are at the computer" add two sentences: presence is one flag for the whole daemon, so
an interactive attachment to any session of any team counts and a window in viewer mode does not;
and the refusal says whether a terminal is attached now or was attached in the last grace period,
and `a2amx list` (ATTACHED column: `yes` is interactive, `viewer` is not) shows which session.

## 5. Tests

Update `tests/presence.rs:438` to the `Attached` text. Add, in the same file and style:

- A grace test at the daemon seam, deterministic: start the daemon with `away_secs` 30 (the test
  never waits out the grace, so the wide window removes any timing margin), attach interactively,
  send `ClientFrame::Mode { viewer: true }`, then `Redraw` and `receive_data` (the pattern at
  `tests/presence.rs:660-666`), then send to `human:owner`. Do not drop the attachment and send:
  presence is updated by the attachment guard's `Drop` when the daemon's connection task ends
  (`src/daemon.rs:3571-3574`), asynchronously to the sender. The failure is `owner_present` with
  the literal `the owner was at the computer less than 30 seconds ago; try again later or write to them in the terminal`.
  This also covers the owner's real case: a window turned to viewer.
- The Attached test is the updated assertion at `tests/presence.rs:438` (interactive attachment).
- A viewer-only test (new contract test, no existing one at the daemon seam): attach with
  `viewer: true` from the start and never interactively; sending to `human:owner` is accepted.
- A `cause()` unit test at the `Presence` level, in the style of `tests/presence.rs:45`, with
  explicit `Instant`s: `Some(Attached)` with one interactive attach, `Some(Grace)` just inside
  `away_secs` after it ends, `None` at exactly `last + away_secs`, `None` before any attach.

## 6. Out of scope

The attention signal for detached sessions (F90 second part, F82); per-session presence;
changing `list_agents` or `a2amx list`; the grace period default.

## 7. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `scripts/prepush-fast.sh` pass (in a confined architect session only the documented I24 pair
   may fail). Judge: the architect's fresh run.
2. `cargo test --test presence` passes; the grace test is red on the old code (it fails on the text)
   and green after. The viewer test is a contract test if it already passes on the old code.
3. Mutation check (the developer breaks each in production code, names the failing repo test,
   restores by checksum): (a) swap the two texts; (b) make `cause` return `Grace` while
   `interactive > 0`; (c) make `cause` ignore `away_secs` (always `Grace` after an attach ends; killed by the
   `cause()` unit test, and at the daemon seam only because `present` is implemented through
   `cause`, §2.2); (d) print a fixed `60` instead of `away_secs()` (killed by the grace test's
   30).
4. Trust boundary: the refusal text contains no session name, id or address; a test asserts the
   text for both causes with a literal and no session name (judge: the architect reads the test).

## 8. Claims verified against the tree

| Claim | Where | How |
| --- | --- | --- |
| Presence counts interactive attachments daemon-wide; viewer attachments do not | `src/messaging.rs:174-233` | read |
| The refusal is built once, with a fixed text | `src/daemon.rs:1327-1337` | read |
| Dropping an attachment updates presence asynchronously | `src/daemon.rs:3571-3574` (guard `Drop`) | read (consultant) |
| `tests/mcp.rs:245` pins the tool description, not the refusal text, and stays true | `src/mcp.rs:542` | read (consultant) |
| One test asserts the refusal text | `tests/presence.rs:438` (`grep -rn "at the computer; write" tests` shows that line and nothing else) | grep |
| `list_agents` owner field is read separately at `src/daemon.rs:3101` and is unchanged | `src/daemon.rs:3090-3105` | read |
| `a2amx list` output is pinned exactly by tests | `tests/attach_cli.rs:944,975,1123` | read |
| The owner's refused message in the real run was caused by the interactive architect attachment | `a2amx list` on 2026-11-13: s1 `yes`, s6 `viewer` | observed |
