# Spec 3du: `list` and `list_agents` show a viewer attach as a viewer

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-live-test-fixes-2026-11-10`.

Scope (files that may be edited): `src/wire.rs` (two summary structs), `src/session.rs` (`AttachmentSlot` and one accessor), `src/daemon.rs` (the slot's creation, the `Mode` arm, the two summary builders), `src/main.rs` (the ATTACHED cell text in `session_value_rows`, which `list` and the picker share), `src/mcp.rs` (the `list_agents` tool description), `tests/wire.rs`, `tests/attach_cli.rs`, `tests/team.rs` (only the struct literals the new field forces), `tests/mcp.rs` (only the literal that asserts the `list_agents` description, line 225), `tests/presence.rs` (the new test), `docs/cli.md`, `docs/architecture.md`.

Does not touch: presence rules and the away notice, the attach client, `team reset` and its skip rule, `CHANGELOG.md`.

Public interface change: a new optional field in the control protocol (`sessions` and `agents` responses) and in the `list_agents` MCP tool result; old clients ignore it, old daemons omit it. The changelog line (architect) says so.

Public test seam: a real daemon (`tests/common`, `common::start_daemon`: the new test needs no confinement, so it must not use `start_confined_daemon_with_limits`, which returns `None` and lets a test silently skip when `bwrap` is missing), a raw attach connection that sends `Request::Attach` with `viewer: true` and `ClientFrame::Mode`, and `Request::List` / `Request::ListAgents` responses (as `tests/presence.rs` already drives viewer attaches).

## 1. Why

Today a viewer attach shows as plain `attached` in `a2amx list` and in `list_agents`: the daemon reports `attached` as "the attachment slot is occupied" (`src/daemon.rs:2994`, `src/daemon.rs:3071`, `src/session.rs:1183`), and a viewer holds the slot (a second client needs `--force` to take over). The owner noticed during the first live Discord test that `list` still says attached after switching to viewer mode. The viewer flag lives only in the local `AttachmentGuard` of `attach()`, so the list cannot tell a viewer from someone typing, and an agent reading `attached: true` may conclude the owner is at the keyboard (presence is a separate signal: `owner` in `list_agents`, shown only to remote-enabled sessions).

Alternatives rejected: reporting a viewer as not attached (it still holds the slot; hiding that would make `--force` surprises and `team reset` behaviour inexplicable); a separate `viewers` count (there is one slot).

## 2. Mechanical changes

2.1. `src/session.rs:204` `AttachmentSlot`: add `pub viewer: bool`. Add `Session::attachment_viewer(&self) -> Option<bool>` next to `attached()` (line 1183): `None` when there is no attachment, `Some(viewer)` otherwise, read under one lock so `attached` and `viewer` cannot disagree.

2.2. `src/daemon.rs`, where the slot is created (`state.attachment = Some(AttachmentSlot { .. })`, line 3665): set `viewer` from the attach request's `viewer`. In the `ClientFrame::Mode { viewer }` arm (line 3778): after the existing presence update, set the slot's flag when the slot still belongs to this connection (same `generation` as the guard), under the session lock (taken in its own statement, never while the presence lock is held), for both values of `viewer`. Do not wrap it in the `guard.viewer != viewer` test (the guard is already updated by then, so the condition would always be false). The generation check is defensive: after a `--force` takeover the old connection ends (`src/daemon.rs:3718`), so it cannot normally send `Mode` against the new slot.

2.3. `src/wire.rs`: `SessionSummary` (line 470) and `AgentSummary` (line 543) each get `viewer: bool` with `#[serde(default, skip_serializing_if = "is_default")]` as `you` already has, placed right after `attached`. `viewer` is true only while `attached` is true.

2.4. `src/daemon.rs` summary builders (lines 2989 and 3063): `viewer: state.attachment.as_ref().is_some_and(|slot| slot.viewer)` for the session summary (the lock is already held there) and, for the agent summary, one call `let slot = session.attachment_viewer();` then `attached: slot.is_some()` and `viewer: slot == Some(true)`. In `list_agents` the local variable `viewer` already names the calling session (`src/daemon.rs:3040-3070`): write the field as `viewer: slot == Some(true)`, never the shorthand `viewer`.

2.5. `src/main.rs` `session_value_rows` (the ATTACHED cell, lines 3389-3394; the three header arrays keep the column name `ATTACHED`): print `viewer` instead of `yes` when `session.viewer` is true; `no` and `yes` stay. The picker (`picker_value_rows`, line 3407, used at line 4120) builds on the same rows, so it shows `viewer` too, deliberately: it is the same column with the same meaning. Column headers, widths and everything else are unchanged. Nothing else in `main.rs` changes (`team reset` still treats a viewer as attached, deliberately: it holds the slot).

2.6. `src/mcp.rs:473`: extend the `list_agents` description: `"attached"` stays "a human client is attached"; add `"viewer"` is true when that client is in read-only viewer mode (it holds the session but does not type, so it does not mean the owner is at the keyboard; the `owner` field is the presence signal for remote-enabled sessions).

2.7. Struct literals: adding a field breaks every literal of the two structs. The literals are `src/daemon.rs:2989`, `src/daemon.rs:3063`, `tests/wire.rs:457, 491, 520, 789, 827, 928`, `tests/attach_cli.rs:86`, `tests/team.rs:455`. Add `viewer: false` to each; the exact-JSON comparisons in `tests/wire.rs` stay unchanged because the default is skipped when serializing. The developer greps `SessionSummary {` and `AgentSummary {` across `src` and `tests` again before editing.

## 3. Documentation

3.1. `docs/cli.md:226`: after "ATTACHED means a human client is attached, not that the session is reachable", add that `viewer` in that column means the client is in viewer mode.
3.2. `docs/architecture.md:140`: add `viewer` to the `list_agents` entry fields (present only when true, only with `attached`).

## 4. Out of scope

Changing `team reset`'s skip rule for viewers; a viewer marker in the status line (the picker shows it through the shared cell, section 2.5); the 60-second grace period; `a2amx list --json`-style output if it exists elsewhere (grep shows only the two structs); `CHANGELOG.md`.

## 5. Acceptance

1. New test in `tests/presence.rs` (red first), on `common::start_daemon`: a viewer attach (a running `cat` session) shows `attached: true, viewer: true` in `Request::List` and in the agents list; `ClientFrame::Mode { viewer: false }` then `Redraw`, then wait for the redraw data (`receive_data`, `tests/presence.rs:161`; frames on one connection are handled in order, so the data proves `Mode` was handled) and the same two show `attached: true, viewer: false`; `Mode { viewer: true }` the same way shows `viewer: true`; after the attach ends the guard drops asynchronously, so poll (the 100 x 20 ms loops already in the file) until both show `attached: false, viewer: false`. No `sleep`. Command: `cargo test --test presence`.
2. `tests/wire.rs`: a `SessionSummary` and an `AgentSummary` with `viewer: true` round-trip and serialize with `"viewer": true`; one without the field decodes as `false` (contract tests for the second part). Command: `cargo test --test wire`.
3. Real daemon, a viewer attach through `Client::attach_viewer` (as `tests/presence.rs:305` does), then `run_binary` `list` and `common::list_cells` (as `tests/attach_cli.rs:1252-1266`): the ATTACHED cell (found by its header name, not by index) is `viewer`; with an interactive attach it is `yes`. No PTY and no CLI flag is involved (no flag starts a viewer attach).
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`; `git diff` only in the files of section 0.
5. Mutation check (developer, assigned checkout, restore by checksum, `timeout 120` each): the `Mode` arm stops updating the slot (test 1 fails); the slot is created with `viewer: false` always (test 1); the agent summary hardcodes `viewer: false` (test 1); the session summary hardcodes `viewer: false` (tests 1 and 3); the slot update alone is wrapped in `if guard.viewer != viewer` after `guard.viewer = viewer` has run (test 1 fails: the condition is then always false and real transitions are skipped); the whole `Mode` arm body is wrapped in `if guard.viewer != viewer` (an equivalent mutant: report it as such, because a call with an unchanged value only rewrites the flag the slot already holds); `list` prints `yes` for a viewer (test 3). Name the failing test for each.
6. Trust boundary: none crossed (read-only daemon state to the same callers that already see `attached`).

## 6. Claims table

| Claim | Where | Verified |
| --- | --- | --- |
| `attached` is `state.attachment.is_some()` | `src/daemon.rs:2994`, `src/session.rs:1183-1184` | read |
| The viewer flag is only in `AttachmentGuard` and is set by the `Mode` arm | `src/daemon.rs:3491-3498, 3778-3787` | read |
| `Mode` changes the guard only when `guard.counted` | `src/daemon.rs:3779` | read |
| Two struct-literal sites in `src/daemon.rs` and eight in tests (`tests/wire.rs:457, 491, 520, 789, 827, 928`, `tests/attach_cli.rs:86`, `tests/team.rs:455`); no `Default` derive, no macros | grep `SessionSummary {` / `AgentSummary {` in src and tests | grep (consultant confirmed) |
| An attach to an exited session ends right after the snapshot and `Exit`, before the frame loop | `src/daemon.rs:3688-3706` | read |
| `session_value_rows` feeds `list` (line 3529) and the picker (`picker_value_rows`, line 4120) | `src/main.rs:3377, 3407, 3529, 4120` | read |
| `tests/mcp.rs:225` asserts the `list_agents` description text | `tests/mcp.rs:225` | read |
| `you` uses `skip_serializing_if = "is_default"` | `src/wire.rs:549-550` | read |
| The ATTACHED column prints `yes`/`no` | `src/main.rs:3389-3394` | read |

## Amendment 1

Section 5 item 5, the wrapped-update mutant, said the slot update wrapped in `if guard.viewer != viewer` is equivalent. It is not: the guard is assigned just before, so the wrapped update never runs on a real transition and test 1 fails. Item 5 now names two mutants: the slot update alone wrapped (killed by test 1) and the whole `Mode` arm body wrapped (equivalent). Found by the tester and the developer; raised by the tester as BLOCKED, confirmed by the architect against `src/daemon.rs`.
