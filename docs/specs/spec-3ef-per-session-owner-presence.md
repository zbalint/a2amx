# Spec 3ef: owner presence is judged per sending session (F90)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from `86f55ae` (the commit that accepts spec 3em). Spec 7 of 7 of the owner's configuration redesign (rewritten on the new config; `away_secs` now comes from `daemon.toml`).
Shared task `context_id`: `a2amx-config-layout-2026-11-13` (the earlier presence context was `a2amx-owner-presence-reason-2026-11-13`). Supersedes spec 3ee, which was
locked at `54129ef` but never implemented (the owner rejected its text-only approach).

**Scope.** May be edited: `src/messaging.rs`, `src/daemon.rs` (the presence field and its
initialisation, the poll and notice code, the refusal, the `list_agents` owner field, the
attachment guard and attach flow), `tests/presence.rs`, `docs/remote-messaging.md`,
`docs/using-with-agents.md`, `docs/roadmap.md` and `README.md` (wording only, if a sentence stops being true), `src/mcp.rs` (the one sentence of the `send_message` tool description, line 542) and `tests/mcp.rs` (its pinned copy, line 245).
**Does not touch:** `CHANGELOG.md` and `docs/backlog.md` (the architect edits them), `src/wire.rs`, `src/main.rs`, `src/session.rs`, `docs/architecture.md`, `tests/attach_cli.rs`, every other
file.

**Public test seams.** The daemon through `Request::SendMessage` to `human:owner`,
`Request::Attach` with `ClientFrame::Mode`, `Redraw` and `receive_data`, and `Request::ListAgents`,
as `tests/presence.rs` already does; and the public `messaging::Presence` with explicit `Instant`s
(`tests/presence.rs:22-105`). No test calls a private function.

## 1. Why

Owner request (2026-11-13, backlog F90), after testing remote messaging with a confined Codex
session: "A user runs 10 agents, 9 detached and one attached that they work in actively. If any of
the 9 needs input, hits a blocker or has a question, the user will not know. Presence detection
should be from the agent's perspective, not the daemon's. I sit in front of my computer doing
something else, one agent session is open and the rest are detached."

Today presence is one flag for the whole daemon (`Presence`, `src/messaging.rs:175-232`): any
interactive attachment to any session of any team makes the owner present, and every agent's
`send_message(to="human:owner")` is refused with `owner_present`. The refusal exists so a message is
not pushed to the phone when the owner can read the agent's terminal. That reason holds only for the
session the owner is attached to; a detached agent's terminal is not being read.

Alternatives rejected: a text-only change that names the cause (spec 3ee: leaves a stuck detached
agent silent); per team (an agent in the team the owner works in would still be silent); a notice in
the owner's attached window (a separate design, out of scope); a rate limit on owner messages (see
section 7, existing limits apply).

## 2. The rule

The owner is **present for a session** when that session has an interactive (non-viewer)
attachment, or had one that ended less than `away_secs` ago (default 60 s; `owner_away` in `daemon.toml`, `--owner-away-secs`, `A2AMX_OWNER_AWAY_SECS`; unchanged by this spec).
An attachment to any other session does not make the owner present for this one. A session's
`send_message(to="human:owner")` is refused with `owner_present` only when the owner is present for
that session. The viewer rule is unchanged, applied per session (a viewer attachment does not count;
turning an interactive attachment to viewer starts the grace). A session has at most one attachment
at a time (a second needs `--force`), so the per-session counter is 0 or 1. Delivery of an allowed
message is unchanged (Discord through the gateway). Fewer refusals result; the change is compatible,
not breaking.

## 3. `src/messaging.rs`

1. Add `pub enum PresenceCause { Attached, Grace }` (derive `Debug, Clone, Copy, PartialEq, Eq`).
2. Add `pub fn cause(&self, now: Instant) -> Option<PresenceCause>`: `Some(Attached)` when
   `interactive > 0`; else `Some(Grace)` when `last_interactive` is within `away_secs` of `now`;
   else `None`. Implement `present` through `cause` (`present == cause(now).is_some()`); do not keep a
   second comparison.
3. Add `pub fn away_secs(&self) -> u64`.

## 4. `src/daemon.rs`

Anchors re-verified at `86f55ae` (spec 3em landed; `src/daemon.rs` and `src/messaging.rs` unchanged since spec 3eg).

1. The field `presence: Arc<Mutex<messaging::Presence>>` (line 97, initialised at 2425) becomes a map
   `Arc<Mutex<HashMap<String, messaging::Presence>>>` keyed by `session.id().0`, one lock for the
   whole map, an entry created on the session's first attach (`Presence::new(owner_away_secs)`). A
   missing entry means away. Add one helper that returns the cause and `away_secs` for a session id
   (`fn owner_cause(&self, session_id: &str, now: Instant) -> Option<(PresenceCause, u64)>` or
   equivalent); the refusal and `list_agents` use it, and only the attachment guard and the poll
   loop touch the map otherwise.
2. `AttachmentGuard` (3508-3541) and the attach flow (3705, 3796) call `attach_started`,
   `mode_changed` and `attach_ended` on their own session's entry.
3. The refusal (1325-1337; the old text is at line 1333) reads the **sender's** entry with one lock read and no second call to
   `present`. Texts (no session or team name appears; the agent learns only about its own session):
   - `Attached`: `the owner is attached to this session in the terminal (viewer mode does not count); write to them there instead`
   - `Grace`: `the owner left this session less than {away_secs} seconds ago; try again later or write to them in the terminal`
   The code stays `owner_present`.
4. `list_agents` (3053-3067; the owner block is 3055-3065): `owner` reports present or away for the **caller's own session**, still
   only for remote-enabled callers.
5. `poll_presence` and `send_presence_notices` (840-928; 930-935 is `observe_exit`, untouched): each session's entry is polled; when a
   session's own presence turns to away, only that session gets the existing away notice (same
   `remote_ineligible` check, one open notice, same body). A session that was never attached has no
   entry, never transitions and gets no notice (as today). Entries of exited sessions are removed.
   `HashMap` iteration must not hold the presence lock across the `.await` of the notice send.
6. `list_agents` and the refusal for a session id with no entry report away and allow the send. An
   admin or gateway caller has no session id: its `owner` stays absent, and the helper is never called
   with an empty id (unchanged, `tests/presence.rs:753`).
7. **Entry lifecycle.** Only `attach_started` creates an entry, and only when the session has not
   exited (the existing condition at 3703). `mode_changed` and `attach_ended` (the guard's `Drop` and
   the Mode frame) use `get_mut` and do nothing when the entry is missing; they never create one. An
   attachment can outlive its session's exit (the Exit frame is sent, then `Drop`), so this keeps a
   removed entry from coming back. The poll tick removes entries whose session is gone from
   `runtime.sessions` or has an exit code. Removal is memory hygiene only: session ids are monotonic
   (`s{number}`, line 558), so a stale entry can never reach a new session, and no notice is lost
   because `remote_ineligible` refuses an exited recipient (line 1645).
8. **Lock order.** `list_agents` already takes the sessions lock and then the presence lock (3040,
   then 3059). Nothing may take them in the opposite order. The poll tick takes a snapshot of the live,
   non-exited session ids under the sessions lock, releases it, and only then locks presence to prune
   and read. The presence lock is never held across an `.await`.

## 5. Docs

`docs/remote-messaging.md`, "When you are at the computer": rewrite the first paragraph for the
per-session rule (present for a session while you are attached to it interactively and for the grace
period after; other sessions, detached ones included, can message you; viewer mode does not count;
`a2amx list` ATTACHED shows `yes` or `viewer` per session); the sentence on the away notice (line 19) says the notice goes to a session when the owner leaves that session. Change the one sentence of the tool description in `src/mcp.rs:542` from `Sending to human:owner is refused with owner_present while the owner is at the computer.` to `Sending to human:owner is refused with owner_present while the owner is attached to your session.`, and the same literal in `tests/mcp.rs:245`. Check `docs/using-with-agents.md:31`,
`docs/roadmap.md:63` and `README.md:15` and `README.md:65` ("keep remote traffic away while you are at the computer") and change only wording that stops being true: after this spec presence is per session, so say that the owner's attached session is kept quiet, not all traffic.

## 6. Tests

Update, and name in the report, every existing test in `tests/presence.rs` that assumes one
daemon-wide flag. Named by the pre-lock review (the developer finds any others): `:393` (new `Attached`
literal; text at 438); `:910` `away_notice_body_matches_the_fixed_operator_message` (observer B is never
attached, so its owner state is now `away`: red, update it); `:969`
`exited_session_attach_does_not_make_owner_present` and `:1045`
`exited_attach_does_not_drop_another_interactive_count` (they attach an exited, non-remote session and
check another session's state; they become vacuous, so rewrite them to assert that an exited attach
creates no entry and leaves the sender's own state `away` / its count unchanged, and show a mutant that
counts an exited attach red); `:729` (see the remote-only notice test below); `:682` (force takeover)
stays valid. Add at the daemon seam, with two remote-enabled sessions A and B of
one team:

- **The owner's scenario:** attach A interactively; B (never attached) sends to `human:owner` and is
  accepted; A's own send is refused with the `Attached` literal.
- **Per-session grace, deterministic:** `away_secs` 30; attach A, send `ClientFrame::Mode { viewer:
  true }`, `Redraw` and `receive_data` (the pattern at `tests/presence.rs:660-666`); A's send is
  refused with `the owner left this session less than 30 seconds ago; try again later or write to them in the terminal`;
  B's is accepted; A's `list_agents` owner is `present`, B's `away`. Do not drop the attachment and
  send: presence is updated by the attachment guard's `Drop` when the connection task ends
  (`src/daemon.rs:3532-3536`), asynchronously to the sender.
- **Remote-only notice kept:** a non-remote session is attached interactively and detached (tiny
  `away_secs`); it gets no away notice (this keeps the `remote_ineligible` check in the notice path
  protected now that a never-attached session gets none anyway).
- **Viewer-only:** a session attached only as a viewer is accepted (new contract test).
- **Away notice per session:** attach A and B interactively, turn A to viewer with a tiny
  `away_secs`, wait for A's notice, then wait at least one more poll period (`HEARTBEAT_POLL`, 1 s, `src/daemon.rs:43`) plus margin, then assert B has none (a mutant that notifies every session inserts the notices in one pass, so B's would otherwise arrive after the first check); sync the Mode change with `Redraw` and `receive_data` as above.
- **`cause()` unit test** at the `Presence` level in the style of `tests/presence.rs:45`:
  `Some(Attached)` with one interactive attach, `Some(Grace)` just inside `away_secs` after it ends,
  `None` at exactly `last + away_secs`, `None` before any attach.

## 7. Out of scope

A notice in the owner's attached window; an attention signal for stuck agents (backlog F82);
per-team presence; changing `a2amx list` or the `list_agents` shape; the grace default; the message
rate limits (an allowed detached agent is subject to the same per-sender limits as any agent
sending to `human:owner` today; the spec adds none).

## 8. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `scripts/prepush-fast.sh` pass; in a confined architect session only the documented I24 pair may
   fail. Judge: the architect's fresh run.
2. `cargo test --test presence` passes; the owner's-scenario test is red on the old code (B is
   refused) and green after. The viewer-only test is a contract test if it already passes on the old
   code.
3. Mutants, each with the failing repo test named (the developer breaks it in production code,
   restores by checksum, and its tree must equal its own diff afterwards): (a) swap the two texts;
   (b) `cause` returns `Grace` while `interactive > 0`; (c) `cause` ignores `away_secs`; (d) fixed
   `60` in the Grace text; (e) the refusal reads another session's entry instead of the sender's;
   (f) `list_agents` owner reads another session's entry; (g) the poll loop notifies every session on
   any session's transition; (h) the notice path drops the `remote_ineligible` check.
4. Trust boundary: neither refusal text nor the `list_agents` owner field carries information about
   any session but the caller's. The tests assert the literals; the architect reads them. The new
   exposure is that a detached agent can now message the owner while the owner is at the computer:
   this is the requested behaviour, bounded by the existing remote eligibility checks
   (`remote_ineligible`) and message limits, which stay unchanged.

## 9. Claims verified against the tree (re-verified at lock time)

| Claim | Where | How |
| --- | --- | --- |
| Presence is one daemon-wide `Presence` counting interactive attachments | `src/messaging.rs:175-232`, `src/daemon.rs:97,2425` | read |
| The refusal is built once with a fixed text | `src/daemon.rs:1325-1337` | read |
| `list_agents` reads presence separately for the owner field | `src/daemon.rs:3053-3067` | read |
| The away notice is sent to every eligible session on the daemon-wide transition | `src/daemon.rs:840-935` | read |
| Dropping an attachment updates presence asynchronously (guard `Drop`) | `src/daemon.rs:3532-3536` | read |
| Only `src/daemon.rs:1333` and `tests/presence.rs:438` pin the old refusal text | grep of src, tests, docs, README, CHANGELOG | grep (consultant) |
| `src/mcp.rs:542` and `tests/mcp.rs:245` pin the tool description sentence `refused with owner_present while the owner is at the computer`, which stops being true (section 5 changes both) | `rg "while the owner is at the computer" src tests docs README.md` matched only those two | grep |
| `list_agents` takes the sessions lock before the presence lock | `src/daemon.rs:3040,3059` | read (consultant) |
| Session ids are monotonic `s{number}` | `src/daemon.rs:558` | read (consultant) |
| A session has one attachment; a second needs `--force` | `src/daemon.rs` attach flow ("is attached elsewhere") | read |
| The owner's refusal on 2026-11-13 came from an interactive window on another session | `a2amx list`: s1 `yes`, s6 `viewer` | observed |
