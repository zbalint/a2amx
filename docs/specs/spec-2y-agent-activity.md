# Spec 2y: agent activity in `list` and `list_agents`, and the heartbeat's busy test

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `cb6f831`; gate notes in section 7).
Decisions D1 to D7 come from the owner's idea of 2026-10-04 ("add the agent state, idle or working
or busy, to the `a2amx list` and `list_agents` output") and the owner's approval ("lgtm") of the
three definitions and the 10 second window below. Names and edge rules are the architect's, marked
**(architect)**.

**Baseline:** develop at `cb6f831`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-activity`.
Public test seams: the wire `Request::List` and `Request::ListAgents` against a real daemon
(`tests/broker.rs`, `tests/quota.rs`, `tests/daemon.rs`), the MCP tool through `a2amx mcp`
(`tests/mcp.rs`, `tests/bridge.rs`), the `a2amx list` output (`tests/refs_and_list.rs`,
`tests/attach_cli.rs`), and the heartbeat tests that start a real daemon.

**Scope.** May edit exactly these files and no others.

- `src/wire.rs` (new `Activity` enum; `SessionSummary` and `AgentSummary` gain `activity`)
- `src/daemon.rs` (new `session_activity`; the `Request::List` and `Request::ListAgents` arms;
  the heartbeat task's busy test)
- `src/main.rs` (the `ACTIVITY` column of `a2amx list`: `SESSION_HEADERS`, `DETAIL_HEADERS`,
  `session_value_rows`, `format_session_table`)
- tests: any file under `tests/` that builds or compares a `SessionSummary` or `AgentSummary`,
  asserts the `a2amx list` header line, asserts the `list_agents` JSON, or covers the heartbeat
  (the baseline list is in section 7), and `tests/common/mod.rs` if it needs a helper
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/session.rs`, `src/harness.rs`, `src/delivery.rs`, `src/mcp.rs` (the tool
forwards `agents` as serialized), `src/cli.rs`, the interactive picker (`PICKER_HEADERS`,
`picker_value_rows`), `scripts/`, `extension/`, `docs/specs/` other than this file. No new
dependency, no new `Request`. Do not commit, stage or merge: leave the diff uncommitted for review.

## 1. Why

On 2026-10-04 the architect ran the heartbeat (spec 2w) against a real OMP developer for over 30
minutes, 10 to 20 of them with the developer working, and received no heartbeat. The cause is the
heartbeat's busy test: it calls `harness::ready`, and for OMP that is only
`bracketed_paste && !scrolled` (`src/harness.rs:162`). OMP is a full-screen interface that keeps
bracketed paste enabled while it works, so it always reads as ready, `has_busy_peer` stays false
(`src/daemon.rs:547`) and every tick is skipped. The earlier live test used generic `sh`
sessions, which stop being ready while a command runs, so it could not show this.

The same gap hides from agents and humans: `list` and `list_agents` say `running` for a session
whether it is idle or deep in a task. One daemon-side activity value, based on recent screen
output as well as readiness, fixes the heartbeat and gives peers the fact they lacked.

## 2. Decisions

- **D1. (architect)** New `wire::Activity` enum: `Idle`, `Working`, `Busy`. It derives `Debug,
  Clone, Copy, PartialEq, Eq, Serialize, Deserialize`, uses `#[serde(rename_all = "snake_case")]`,
  and has `as_str()` returning `"idle"`, `"working"`, `"busy"`.
- **D2.** Definition, for a running session, evaluated in this order:
  1. `Busy`: it cannot take a message now: the channel reports a hold
     (`AnyChannel::for_session(..).hold_reason()` is `Some`, which includes `human_draft`) or the
     session is resetting (`Session::resetting()`).
  2. `Working`: not ready (`harness::ready` is false) or the screen changed within the last 10
     seconds (`now - last_change < 10 s`).
  3. `Idle`: otherwise (ready, unheld, not resetting, and quiet for 10 seconds or more).
- **D3.** The 10 second quiet window is one named constant in `src/daemon.rs`
  (`ACTIVITY_QUIET`), not configurable. It carries a `// shortcut:` comment: a tool call that
  prints nothing for over 10 seconds reads as idle; upgrade to a per-session setting or a
  harness-specific working marker if that misleads in practice. The values are as fresh as the
  screen at the moment of the request, not polled.
- **D4. (architect)** An exited session has no activity: the field is `Option<Activity>`, `None`
  for exited sessions. The wire field is
  `#[serde(default, skip_serializing_if = "Option::is_none")] pub activity: Option<Activity>` on
  both summaries, so an old payload deserializes to `None` and a client must treat `None` as
  "unknown or exited", never as `idle`. `list` renders `None` as `-`.
- **D5. (architect)** `a2amx list` gains an `ACTIVITY` column directly after `STATE` in the default
  and the `details` table. The picker is unchanged. `STATE` (`running` or `exited(code)`) is
  unchanged.
- **D6.** `list_agents` gains `activity` as in D4; `state` and every other field are unchanged.
- **D7.** Heartbeat busy test (`heartbeat` in `src/daemon.rs`, lines 480 to 504 and 545 to 549 at
  the baseline): a watched live peer counts as busy when its activity (D2, computed with the same
  function) is `Working` or `Busy`, or it has queued messages. This replaces `!ready ||
  hold.is_some()` in the has-busy-peer test, and `busy_since` tracks the first time the peer was
  seen non-idle, not not-ready. The digest body format, the watcher's own idle gate
  (`watcher_ready`, resetting, hold, `last_change` older than the interval), the duplicate
  suppression and everything else in spec 2w are unchanged.

## 3. `src/wire.rs`

Add `Activity` (D1) beside `QuotaInfo`. `SessionSummary` (line 266 at the baseline) and
`AgentSummary` (line 323) each gain the D4 field after `cwd`, with a doc comment that says it is
`None` for an exited session and that `None` must not be read as idle.

## 4. `src/daemon.rs`

Add, beside `session_quota` (line 1702):

```rust
fn session_activity(session: &Arc<Session>) -> Option<Activity>
```

returning `None` when `session.exit_code().is_some()`, otherwise the D2 result. It takes the
screen lock once for readiness and `last_change`, and calls `hold_reason` and `resetting` outside
that lock, as the existing code does. The `Request::List` arm fills `activity:
session_activity(session)` and the `Request::ListAgents` arm the same. The heartbeat task
replaces its readiness read (lines 481 to 502) with `session_activity(&peer)`: `busy_for` is
`Some(..)` while the activity is `Working` or `Busy`, using `busy_since` as today, and the
`ready` element of the observed tuple becomes the idle test. The existing separate `hold` and
`unchanged_for` reads stay for the digest.

## 5. `src/main.rs`

`SESSION_HEADERS` (line 1358) and `DETAIL_HEADERS` (line 1361) gain `"ACTIVITY"` after `"STATE"`;
`session_value_rows` returns ten cells, with `session.activity.map_or("-", Activity::as_str)`
after the state cell; `format_session_table` destructures and rebuilds the rows with the extra
cell. The picker's use of `session_value_rows` (if it shares it) must keep its own columns
unchanged: read `picker_value_rows` first and do not change the picker's output.

## 6. Docs

`README.md` (the `list_agents` bullet, line 58, names `activity`), `docs/architecture.md` (the
`list_agents()` return shape at line 135, the `a2amx list` columns at line 416, and the
heartbeat's busy test wherever the architecture or delivery text describes it as "ready").
`docs/backlog.md`: add a Closed row for spec 2y.

## 7. Tests (failing first) and acceptance

Tests, each through a public seam with literal expected values:

1. `tests/wire.rs`: `activity` serializes as `"idle"`, `"working"`, `"busy"`; absent when `None`;
   a payload without it deserializes to `None`, for both summaries.
2. A real-daemon test: a generic `sh` session sitting at its prompt for longer than 10 seconds
   reports `idle`; one running `sleep 30` reports `working` (not ready); a session whose screen
   just changed reports `working` until the window passes. Where a literal wait is needed, keep it
   under 15 seconds total per test.
3. A held session (the existing human-draft fixture in `tests/delivery.rs`) reports `busy`.
4. An exited session reports no `activity` in `list_agents`.
5. `a2amx list`: the header line becomes
   `ID  NAME  HARNESS  STATE  ACTIVITY  ATTACHED  PENDING  HELD  QUOTA  SIZE` (column
   widths follow the data), with `-` for an exited session; update every existing header assertion.
6. Heartbeat: a regression test where the watched peer is a session that stays ready
   (bracketed paste on) but keeps printing output (a generic `sh` loop writing a line every
   second after enabling bracketed paste) produces a heartbeat for an idle watcher; and the
   existing spec 2w "no heartbeat when every peer is idle" case still passes, after waiting out the
   quiet window.

Run these. All must pass with no warnings:

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

Expected values are literals, never recomputed with the logic under test. Existing heartbeat tests
that assert a skip for a ready peer may need to wait up to 10 seconds for the window to pass; that
is allowed. A test that cannot be made to pass without changing the D2 or D7 rules is a
`BLOCKED — SPEC ADJUDICATION REQUIRED`, not a license to relax the assertion.

**Out of scope:** a per-session or configurable quiet window; an OMP-specific working marker
(parsing the footer, spinner or `Working` text); changing the watcher's idle gate or the digest
body; activity in the picker; `activity` input filters on `list_agents`; waking or acting on a
peer; the OMP draft check that rejected `reset_session` with `draft_present` on an empty composer
(a separate suspected bug, not investigated here).

**Gate notes at lock time (baseline `cb6f831`):** `SessionSummary` is built in `src/daemon.rs`
only (the `List` arm) and `AgentSummary` only in the `ListAgents` arm; test literals live in
`tests/wire.rs` (lines 139, 362, 395, 490) and `tests/team.rs` (line 16), and
`tests/delivery.rs:181`, `tests/quota.rs:255` and `tests/broker.rs:131` read them. `a2amx list`
header assertions are at `tests/refs_and_list.rs:169` and `tests/attach_cli.rs` lines 475, 486,
635. Files mentioning `heartbeat` under `tests/`: `codex.rs`, `daemon_cli.rs`, `team.rs`,
`attach_cli.rs`, `broker.rs`, `claude_channel.rs`, `common/mod.rs`, `daemon.rs`, `delivery.rs`,
`hook.rs`, `quota.rs`, `wire.rs`; the developer reruns `rg heartbeat tests` and `rg
"SessionSummary \{|AgentSummary \{|HARNESS  STATE" tests` as the first step and edits only files
those return. `Hold::HumanDraft` maps to hold reason `human_draft` (`src/delivery.rs:394`), so D2
step 1 covers a human draft where the channel reports it. `git status` was clean at lock time.
The acceptance commands require the new code and run after implementation.
