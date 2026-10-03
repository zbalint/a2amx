# Spec 2l: report a harness's remaining quota

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 were settled with the owner. Builds on spec 2k
(committed `ce67510`). The work is done directly on `develop` in the main checkout, no
worktree. Shared task `context_id`: `a2amx-quota-report`. Public test seams: `quota::read`
(a pure function over an `Emulator` screen), and the daemon through `Client` requests as
`tests/daemon.rs` and `tests/broker.rs` do.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/quota.rs` (new), `src/lib.rs`, `src/wire.rs`, `src/daemon.rs`, `src/mcp.rs`,
  `src/main.rs`
- `tests/quota.rs` (new), `tests/wire.rs`, `tests/mcp.rs`, `tests/attach_cli.rs`,
  `tests/broker.rs`
- `README.md`, `docs/architecture.md`

Does not touch: every other file under `src/` (in particular `src/emulator.rs`,
`src/session.rs`, `src/delivery.rs`, `src/harness.rs`, `src/cli.rs`), every other file under
`tests/` (`tests/common` included), `scripts/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the
locked spec documents. No new dependency (no `regex`). Do not commit and do not merge: leave
the diff uncommitted in the working tree.

In the existing test files the only permitted changes are the mechanical ones named in
section 7. Do not change any other existing assertion.

## 1. Why

A Claude or Codex session that has used up its 5-hour or weekly quota still accepts messages
but cannot act on them, and a peer has no way to know. Both harnesses draw a status line on the
last rows of the screen that says how much is left, and the daemon owns every screen. The
daemon reads that line and reports it. Report only: delivery is unchanged (owner decision,
option A). Holding delivery while exhausted is a possible later spec once it is known whether
an idle Codex session redraws a stale `0% left` after the quota resets.

## 2. Decisions settled with the owner

- **D1.** Report-only. Delivery, holds, and message states do not change.
- **D2.** Detection reads the screen's last 3 rows only. Conversation text higher on the
  screen, for example a quoted `5h 0% left` or Codex's `Heads up, you have less than 10% of
  your 5h limit left`, never counts.
- **D3.** Two windows are recognised. The 5-hour window is the text `5h <N>% left` in both
  harnesses. The weekly window is `weekly <N>% left` (Codex) or `7d <N>% left` (Claude).
  `<N>` is 1 to 3 digits and at most 100.
- **D4.** Only harnesses `claude` and `codex` are scanned, and only while the session is
  running. Generic and OMP sessions, and exited sessions, report nothing.
- **D5.** Silent when absent: no line found means no field, never an error and never a log.
- **D6.** "Exhausted" means a recognised window with 0% left.
- **D7.** Reported in three places: a `quota` field on each session in `a2amx list` (a new
  `QUOTA` column) and on each agent in `list_agents`; and `recipient_quota` on the
  `send_message` result, present only when the recipient is exhausted.
- **D8.** The daemon never sends input to a session to learn its quota.
- **D9.** The value can be stale: it is whatever the harness last drew. This is documented,
  not corrected.

## 3. `src/quota.rs` (new) and `src/lib.rs`

`src/lib.rs`: add `pub mod quota;` in alphabetical position.

`src/quota.rs` is a pure reader. No I/O.

```rust
use crate::emulator::Screen;
use crate::wire::QuotaInfo;

/// What the harness status line on the screen's last rows says about quota, or `None` when it
/// says nothing.
pub fn read(screen: &Screen) -> Option<QuotaInfo>;
```

Rules:

1. Take the last 3 rows of `screen` (fewer when the screen has fewer). Build each row's text
   from its cells' `ch`, mapping U+00A0 to a space, and trim trailing spaces.
2. Scan rows from the bottom up. In a row, a **window token** is a label immediately followed
   by one space, 1 to 3 ASCII digits whose value is at most 100, and the exact text `% left`.
   Labels: `5h` for the five-hour window; `weekly` or `7d` for the weekly window. The label must
   start at the beginning of the row or after a character that is not alphanumeric (so `x5h 3%
   left` does not match), and `% left` must be followed by the end of the row or a character
   that is not alphanumeric.
3. The first token found for each window, scanning bottom-up and left to right, is its value.
   Later tokens for an already-found window are ignored.
4. Return `None` when neither window was found, otherwise `Some(QuotaInfo { five_hour, weekly
   })` with the found windows as `Some(percent)`.

Worked examples. `rows = 24` unless stated; the text is on row 24 (the last) unless stated.

| # | Row text | Result |
|---|---|---|
| 1 | `gpt-5 · repo · Ready · 40% context · weekly 12% left · 1.2M used · 5h 0% left` | five_hour 0, weekly 12 |
| 2 | `[Sonnet] repo \| main \| $1.20 \| 45k/200k (22%) \| 5h 7% left 7d 63% left` | five_hour 7, weekly 63 |
| 3 | `5h 100% left` | five_hour 100, weekly none |
| 4 | `5h 101% left` | `None` |
| 5 | `5h left`, `5h 3%left`, `5h 3% leftover`, `x5h 3% left`, `5h  3% left` | `None` for each |
| 6 | `5h 0% left` on row 19 (the 6th from the bottom), rows 20 to 24 empty | `None` |
| 7 | `5h 0% left` on row 22, rows 23 and 24 empty | five_hour 0, weekly none |
| 8 | row 24 `5h 40% left`, row 23 `5h 0% left` | five_hour 40 (bottom row wins) |
| 9 | a 2-row screen with `5h 9% left` on row 1 | five_hour 9 |
| 10 | an empty screen | `None` |

Examples 1 and 2 are the real shapes seen on the owner's machine (Codex and Claude status
lines). Build the screens in tests with `Emulator::feed`, as `tests/emulator_roundtrip.rs`
does.

## 4. `src/wire.rs`

Add

```rust
/// Percent left per quota window, as the harness's status line last showed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct QuotaInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weekly: Option<u8>,
}

impl QuotaInfo {
    /// Which windows are at 0%: `"5h"`, `"weekly"`, `"5h and weekly"`, or `None`.
    pub fn exhausted(&self) -> Option<&'static str>;
}
```

`exhausted`: `five_hour == Some(0)` and `weekly == Some(0)` give `"5h and weekly"`; only the
first `"5h"`; only the second `"weekly"`; otherwise `None`.

Three additive fields, each with `#[serde(default, skip_serializing_if = "Option::is_none")]`
so existing JSON is unchanged when absent:

- `SessionSummary.quota: Option<QuotaInfo>` (after `hold_reason`, `src/wire.rs:254`)
- `AgentSummary.quota: Option<QuotaInfo>` (after `attached`, `src/wire.rs:268`)
- `Response::Accepted.recipient_quota: Option<String>` (after `recipient_hold`,
  `src/wire.rs:216`)

## 5. `src/daemon.rs`

Add a private helper next to the other free functions:

```rust
/// The quota a running claude or codex session's status line reports.
fn session_quota(session: &Session) -> Option<QuotaInfo> {
    if session.exit_code().is_some() || !matches!(session.harness(), Harness::Claude | Harness::Codex) {
        return None;
    }
    quota::read(&session.lock().emulator.screen())
}
```

(`session.lock().emulator.screen()` is the pattern at `src/daemon.rs:568`; `Session`'s other
accessors used here, `exit_code` and `harness`, already exist. Do not hold the session lock
across an `await`.)

Use it in three places:

- `Request::List` (`src/daemon.rs:1003`): `quota: session_quota(session)`.
- `Request::ListAgents` (`src/daemon.rs:1086`): `quota: session_quota(session)`.
- Message acceptance (`src/daemon.rs:339`): add
  `recipient_quota: session_quota(&recipient).and_then(|q| q.exhausted()).map(|window| format!("{window} quota exhausted"))`.

It does not touch `hold_reason`, `unready_reason`, `deliver`, or any delivery code. The
`Request::List` hunk takes the session lock once already (`let state = session.lock();`); the
developer may call `session_quota` before taking that lock to avoid locking twice.

## 6. `src/mcp.rs`, `src/main.rs`

`src/mcp.rs`, `ToolCall::SendMessage` result (`src/mcp.rs:252`): the tool result is the JSON
`{"id", "status": "accepted"}` plus `"recipient_hold"` when present (today) plus
`"recipient_quota"` when present. Both optional keys may appear together. The tool
description of `send_message` (`src/mcp.rs:445`) gets one sentence appended after the
`recipient_hold` sentence: ` If recipient_quota is present, the recipient's harness reports that quota is used up and it may not answer until the quota resets.`
`list_agents` needs no code change: `{"agents": agents}` serializes the new field, and its
description is unchanged.

`src/main.rs`: `SESSION_HEADERS` (`src/main.rs:885`) becomes
`["ID", "NAME", "STATE", "ATTACHED", "PENDING", "HELD", "QUOTA", "SIZE", "COMMAND"]`; its
type and `session_value_rows` return `[String; 9]`. The new cell, after the `HELD` cell, is `-`
when `quota` is `None`; otherwise the present windows in order, joined by one space:
`5h <N>%` then `wk <N>%`. Examples: `5h 0% wk 12%`, `5h 7%`, `wk 63%`. The picker
(`PICKER_HEADERS`) does not change.

## 7. Mechanical changes to existing tests

Every other existing assertion stays. These are the only edits, found with
`rg "Response::Accepted|SessionSummary \{|AgentSummary \{|COMMAND\\\\n" tests`:

- `tests/wire.rs`: add `quota: None` to the `SessionSummary` literals (lines 111 and 410) and
  the `AgentSummary` literal (316); add `recipient_quota: None` to the `Response::Accepted`
  literal (307).
- `tests/attach_cli.rs`: the three header assertions (lines 241, 250, 398) gain the `QUOTA`
  column, with the column padding the table renderer produces; add `recipient_quota: None`
  to the four `Response::Accepted` literals (580, 596, 610) and `recipient_quota: None` to the
  one at 671.
- `tests/broker.rs`: the `accepted` helper (line 72) builds `Response::Accepted` with
  `recipient_quota: None`.
- `tests/mcp.rs`: add `recipient_quota: None` to the `Response::Accepted` literals (350, 653);
  the `send_message` description literal (234) gets the sentence of section 6.

Patterns written with `..` (for example `tests/codex.rs:179`) need no edit.

## 8. Tests

Write the failing test first, one behavior at a time.

**`tests/quota.rs` (new).**

- One test for each row of the section 3 table, or a table-driven test with expected values
  written out as literals. They call `quota::read` on screens built with `Emulator::feed`
  (use the cursor-position escape `ESC [ <row> ; 1 H` to place text on a row).
- `QuotaInfo::exhausted` on the four cases of section 4.
- Daemon tests (use `tests/common` as `tests/daemon.rs` does, a real `Daemon`, port 0):
  - a session started with `Request::NewSession { harness: Harness::Claude, argv: ["sh", "-c",
    "printf '\\033[24;1H5h 0%% left 7d 80%% left'; exec sleep 30"], .. }` at 80 columns by 24
    rows. Eventually, `Request::List` reports its `quota` as five_hour 0, weekly 80, and
    `Request::ListAgents` reports the same.
  - the same script as a `Harness::Generic` session reports no quota.
  - a message from a second session to the exhausted one is accepted with `recipient_quota`
    `Some("5h quota exhausted")`; a message to a Claude session whose status line says
    `5h 30% left` has `recipient_quota` `None`. Follow `tests/broker.rs` for creating the
    sender and sending as a session.
  - sending to the exhausted session still ends in the same message state as sending to the
    non-exhausted one (delivery is unchanged); assert `hold_reason` is not changed by quota.
- `tests/mcp.rs` already scripts a daemon reply; its existing test of `recipient_hold` is the
  model for one new test that a `Response::Accepted` carrying `recipient_quota` becomes the
  tool result `{"id":"m_1","status":"accepted","recipient_quota":"5h quota exhausted"}`, and
  one where both fields are present.

## 9. `README.md`, `docs/architecture.md`

`README.md`: in the paragraph that explains the `a2amx list` columns (the one containing `HELD shows the
reason a hold is stopping delivery`), add one sentence after the HELD explanation: `QUOTA shows what a Claude or Codex status line last said about remaining quota (5h and weekly percent left); it can be stale and is empty when no status line is recognised.`

`docs/architecture.md`: one short paragraph under the daemon section, written in the
document's own style, saying that the daemon reads the last three screen rows of running
Claude and Codex sessions for `5h N% left`, `weekly N% left` and `7d N% left`, reports the
result in `list`, `list_agents` and as `recipient_quota` on `send_message`, never changes
delivery because of it, and may report a stale value.

## 10. Out of scope

Holding or retrying delivery when exhausted (option B), alerts in the attach status bar,
configurable or per-harness patterns, parsing the Codex in-conversation warnings, OMP or
generic sessions, a `a2amx screen` command, sending input to learn quota, notifying peers
proactively, and `src/cli.rs`, `src/harness.rs`, `src/session.rs`, `src/emulator.rs`, and the
unrelated flake in `tests/attach_cli.rs` (`status_line_toggles_and_shows_the_session_address`,
about 2 failures in 12 runs on a clean tree, from spec 2i).

## 11. Acceptance

Run all three, no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`cargo test` includes `tests/quota.rs` and the updated existing tests. If `attach_cli`'s
`status_line_toggles_and_shows_the_session_address` fails, rerun it alone up to three times
and report it as the known flake; any other failure is real. These need the new code, so they
run after implementation, not at lock time.

`git diff --stat` shows only the files in section 0; `git diff --check` is clean.
