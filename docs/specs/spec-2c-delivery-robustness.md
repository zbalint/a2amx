# Spec 2c: delivery robustness and sender visibility

## 0. Status

**LOCKED.** Decisions D1 to D6 in section 2 were settled with the owner.

**Scope.** May edit exactly these files and no others:

- `src/harness.rs`, `src/messaging.rs`, `src/daemon.rs`, `src/store.rs`, `src/wire.rs`,
  `src/mcp.rs`, `src/main.rs` (the `list` table only)
- `tests/messaging.rs`, `tests/delivery.rs`, `tests/mcp.rs`, `tests/wire.rs`,
  `tests/attach_cli.rs`, `tests/broker.rs`
- `docs/delivery.md`, `docs/architecture.md`, `README.md`, `docs/validation-plan.md`

Does not touch: `src/emulator.rs`, `src/prefix.rs`, `src/hook.rs`, `src/delivery.rs`,
`src/session.rs`, `src/client.rs`, `src/cli.rs`, `Cargo.toml`, `Cargo.lock`,
`tests/hook.rs`, every other `tests/` file, the locked spec documents under
`docs/specs/`, the Codex and OMP profiles. No new dependency. Do not commit and do not
merge.

## 1. Why

Spec 2b shipped the hook, receipts and the corrupted-submission verdict. The first real
multi-agent runs found what spec 2b could not see:

1. **A message that can never match.** Claude Code rewrites text it pastes: each tab
   becomes four spaces (handled in `harness::paste_view`), and each literal occurrence of
   its own paste-wrapper tag gets a backslash (not handled). A reply whose body discussed
   the wrapper tag was blocked three times and held its recipient, and the explanatory
   retry sent to fix it hit the same rule and held a second session.
2. **One bad message holds the whole session.** Three corrupted submissions in a row set
   the `corrupted_submissions` hold. Only a human keypress (prefix, then `r`) clears it,
   and it blocks every later message to that session, not only the unmatchable one.
3. **The sender is not told.** `send_message` returns `accepted`; later the message sits at
   `pending` with a `hold_reason`. The sending agent did not notice a block until a human
   relayed it and never called `message_status` on the failed message.
4. **Tool descriptions mislead.** `list_agents` reports `attached`, which means a human
   client is attached, not that the session is reachable; one agent read it the other way.
   `send_message` says acceptance is not read or acted on, which agents read as delivered.

## 2. Decisions settled with the owner

- **D1.** A message rejected as corrupted twice is `undeliverable` (detail
  `unmatchable_submission`). One bad message can therefore no longer hold the session: it
  adds at most two to the session's count, and the session hold at three corrupted
  submissions in a row stays as a backstop for repeated bad messages. The second rejection
  is the limit because a human-caused corruption needs typing during delivery, which
  already sets the `human_draft` hold and stops delivery; two rejections of one message
  point at the message. The limit also catches harness rewrites no profile models yet.
- **D2.** Fix the matcher only. No rejection of wrapper-tag text at send time.
- **D3.** The sender learns through new fields: `hold_explanation` in status and
  `recipient_hold` in the send response. No notice is pushed into a sender's composer.
- **D4.** Status carries `accepted_at` and `updated_at`.
- **D5.** `a2amx list` shows the hold reason in its HELD column.
- **D6.** Out of scope: a persisted hold log, auto-clearing `human_draft`, envelope trust
  wording, a native-channel adapter. (Messages to an exited session are already failed as
  `undeliverable` with detail `recipient_exited` by `src/delivery.rs`; nothing to do.)

## 3. `src/harness.rs`: apply Claude Code's wrapper-tag escaping

`paste_view` (line 154) already models the tab rewrite in its `Harness::Claude` arm. Apply
the tag rewrite to the result of the tab replacement. Rule, from the probe below:

- Wherever the text contains `<` immediately followed by `pasted_content`, compared
  ASCII-case-insensitively, insert `\` after the `<`.
- Wherever the text contains `</` immediately followed by `pasted_content`, compared
  ASCII-case-insensitively, insert `\` between the `<` and the `/`.
- Nothing else changes. `<\pasted_content` does not match the rule and is left alone.
- `Harness::Generic` is unchanged.
- Use a small scan over the text, no new dependency. Comment the rule with the probed
  Claude Code version (2.1.286) in the style of the existing tab comment. Corners not
  probed (runs like `<<pasted_content`, Unicode case folding, other versions) get a
  `// shortcut:` comment saying so.

**Probe record (Claude Code 2.1.286, blocking prompt hook, bracketed paste).** Expected
values in tests are these literals; do not compute them with the new code:

| Pasted text | Hook received |
| --- | --- |
| `x\n<pasted_content id="abc">\ny` | `x\n<\pasted_content id="abc">\ny` |
| `x\n</pasted_content id="abc">\ny` | `x\n<\/pasted_content id="abc">\ny` |
| `x\n<pasted_content>\ny` | `x\n<\pasted_content>\ny` |
| `x\n<PASTED_CONTENT id="a">\ny` | `x\n<\PASTED_CONTENT id="a">\ny` |
| `x\n<pasted_content\ny` | `x\n<\pasted_content\ny` |
| `x <pasted_content id="abc"> y\nz` | `x <\pasted_content id="abc"> y\nz` |
| `x\npasted_content bare\ny` | unchanged |
| `x\n<a2amx-message id="m_1" from="a@b">\ny` | unchanged |
| `x\n</a2amx-message>\ny` | unchanged |
| `x\n<channel source="s">\ny` | unchanged |
| `x\n<system-reminder>\ny` | unchanged |
| `x\n<command-name>\ny` | unchanged |
| `x\n<\pasted_content already\ny` | unchanged |

Claude Code adds its own wrapper after escaping and does not escape that wrapper, so
`unwrap_pastes` needs no change: an escaped tag in a body never matches its open string
(`<pasted_content id="`) and never ends an enclosing wrapper.

Tests. In `tests/messaging.rs`, beside `claude_paste_view_expands_each_tab_to_four_spaces`:
one assertion per table row through `paste_view(Harness::Claude, ...)`, and every input
through `Harness::Generic` unchanged. In `tests/delivery.rs`, beside
`a_tab_in_the_body_matches_after_claude_expands_it`: send a body that contains the opening
tag text and the closing tag text, report the prompt as Claude Code delivers it (wrapper
around the envelope with the escaped body), and expect `allow()` and evidence
`submission_observed`.

## 4. Unmatchable-message policy (D1)

New constants in `src/messaging.rs`, beside `MAX_CORRUPTED_SUBMISSIONS`:

```rust
pub const MAX_MESSAGE_REJECTIONS: u32 = 2;
pub const UNMATCHABLE_SUBMISSION_REASON: &str = "A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.";
```

`src/store.rs`: `reject_attempt(connection, seq)` (line 607) and the `Store` wrapper (line
251) take a `limit: u32` and return a new `pub(crate) enum RejectOutcome { NoAttempt,
Requeued, Undeliverable }` instead of `bool` (`NoAttempt` is the old `false`). Inside the
existing transaction, after marking the attempt `rejected`, count the message's attempts
with outcome `rejected` and detail `corrupted_submission`. If the count is at least
`limit`, set the message to state `undeliverable` with detail `unmatchable_submission`
instead of back to `pending`, and return `Undeliverable`; otherwise behave as today and
return `Requeued`. The count is total, not only consecutive: a message with any receipt is
finished and never reaches this path, so the two are equivalent for a message still in
play. `undeliverable` is not an open state (`has_earlier_open` counts only `pending` and
`delivering`), so later messages are delivered.

`src/session.rs` is not changed: every rejection still counts toward the session's
corrupted count, a clean submit still resets it, and three in a row still set the hold.
A single message cannot reach three on its own, because the second rejection makes it
`undeliverable`.

`src/daemon.rs`, the corrupted branch (lines 306 to 330): the store rejections already run
after `note_corrupted`; keep that order. Pass `MAX_MESSAGE_REJECTIONS` to each
`reject_attempt`, collect whether any returned `Undeliverable`, and use
`UNMATCHABLE_SUBMISSION_REASON` as the verdict reason when one did, otherwise
`CORRUPTED_SUBMISSION_REASON` as today. The notify call after the loop stays.

Worked instances that must hold:

1. Unmatchable message M, nothing else pending. Block 1: M back to `pending`, session count
   1, reason `CORRUPTED_SUBMISSION_REASON`. Redelivered. Block 2: M `undeliverable` with
   detail `unmatchable_submission`, session count 2, no hold, reason
   `UNMATCHABLE_SUBMISSION_REASON`. A later message N is delivered with no release, and
   its exact submit resets the count to zero.
2. M as above, then a second unmatchable message M2 with no clean submit in between: block
   1 of M2 makes the count 3 and sets the `corrupted_submissions` hold; M2 is `pending`
   with that hold reason. After a release M2 is redelivered, and block 2 makes it
   `undeliverable`.
3. M blocked once, then submitted exactly: delivered with a receipt; the next message's
   count starts at zero (the existing `note_submit(true)` reset).

Tests in `tests/delivery.rs`, using the existing helpers (`Case::start`, `send`, `report`,
`wait_state`, `wait_rest`, `status`, `recipient_summary`). For a prompt that cannot be
repaired use the existing "text glued after the closing tag" shape from the test that
asserts `case.report(odd).await == block()`, or the `interleaved()` helper the existing
corruption tests use; the escaped-tag body is repaired by section 3 and must not be used
here. One test per instance above; instance 1 also asserts `!recipient_summary().held`
after block 2 and that N reaches `submitted`.

**Existing test that must change.** `three_corruptions_hold_until_release_even_after_human_submit`
(`tests/delivery.rs` line 600) blocks one message three times and expects a hold; under
this section the second block ends that message. Rewrite it as instance 2: keep its
later assertions (still held after a human submit of `hello`, the blocked message stays
`pending`, a release redelivers it) and adjust each `wait_rest` repeat count to the number
of pastes actually made so far. `interleaved_corruption_redelivers_without_a_draft_hold`
(line 587) blocks once and is unaffected. `tests/hook.rs` blocks once and is unaffected.

## 5. Sender visibility (D3, D4)

`src/wire.rs`:
- `MessageInfo` gains `hold_explanation: Option<String>`, `accepted_at: Option<i64>`,
  `updated_at: Option<i64>`, each with
  `#[serde(default, skip_serializing_if = "Option::is_none")]` as `evidence` has.
- `Response::Accepted` gains `recipient_hold: Option<String>` with the same attribute.
- `SessionSummary` gains `hold_reason: Option<String>` with the same attribute; keep
  `held: bool` unchanged (`tests/delivery.rs` and `tests/attach_cli.rs` read it).

`src/messaging.rs`: one function `hold_explanation(reason: &str) -> Option<&'static str>`
with exactly these strings, and `None` for any other reason:

| Reason | Explanation |
| --- | --- |
| `deliver_hold` | The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery. |
| `unsubmitted_envelope` | A message was pasted into the recipient's prompt box and not submitted yet. |
| `corrupted_submissions` | Three submissions in a row were blocked. A person must release the recipient session (prefix, then r). |
| `human_draft` | A person typed in the recipient session. Delivery resumes when they submit or release it (prefix, then r). |
| `queued` | An earlier message to this recipient is still open. |
| `not_ready` | The recipient's prompt box is not ready, for example a dialog is open. |
| `cooldown` | A message was submitted less than a second ago. |

`src/store.rs`: `Message` (line 40) gains `accepted_at: i64` and `updated_at: i64`. The four
`SELECT` statements that feed `message_from_row` (lines 491, 701, 721, 737) append
`accepted_at, updated_at` after the existing columns, and `message_from_row` (line 822)
reads them at the next two indexes. `next_pending` appends them after its `EXISTS` column;
check each query's column order before editing.

`src/daemon.rs`:
- `message_info` (line 355) fills `hold_explanation` from `messaging::hold_explanation` of
  the computed `hold_reason`, and `accepted_at`, `updated_at` from the message.
- The `SessionSummary` construction (line 816) fills `hold_reason` from `state.hold`:
  `Hold::HumanDraft` gives `human_draft`, `Hold::UnsubmittedEnvelope` gives
  `unsubmitted_envelope`, `Hold::CorruptedSubmissions` gives `corrupted_submissions`,
  none gives `None`. Factor the three names into one place shared with `message_info`'s
  existing comparisons if that does not enlarge the diff; otherwise leave the existing
  chain alone.
- `send_message` (line 195), on success: `recipient_hold` is `Some("deliver_hold")` when
  the recipient's `deliver()` is `Deliver::Hold`, else the session-level hold name from
  the mapping above, else `None`. It never reports `queued`, `not_ready` or `cooldown`:
  those clear without a person and their value depends on timing.

`src/mcp.rs`:
- The `Response::Accepted` arm (line 191) returns `{"id", "status": "accepted"}` and adds a
  `"recipient_hold"` key only when it is `Some`.
- The status payload (lines 197 to 206) adds `"hold_explanation"`, `"accepted_at"`,
  `"updated_at"` after `"evidence"`; absent values are `null`, as `hold_reason` is today.

`a2amx messages` (the CLI table) is unchanged.

Existing assertions to update, each with a literal expectation (no recomputation): every
`Response::Accepted { id: ... }` literal in `tests/attach_cli.rs` (lines 573, 583, 591,
648), `tests/broker.rs` (line 73), `tests/wire.rs` (line 302) and `tests/mcp.rs` (line
549) gains `recipient_hold`: `None`, except where the recipient was started with
`--deliver hold`, or the generic harness default, in which case `Some("deliver_hold")`;
read each test's launch arguments to decide. Struct literals of `MessageInfo` and
`SessionSummary` in `tests/wire.rs` (lines 291 and 110) gain the new fields. The status
JSON assertion in `tests/mcp.rs` (lines 299 to 309) gains the three new keys.

New tests: a held message's status carries the matching explanation string for each of
`deliver_hold`, `human_draft` and `corrupted_submissions` (the last two through
`tests/delivery.rs` helpers); `accepted_at` is at most `updated_at` and both are nonzero;
`send` to a `--deliver hold` recipient returns `recipient_hold` of `deliver_hold`.

## 6. `a2amx list` (D5)

`src/main.rs`, the table builder around line 672: the HELD cell is `hold_reason` of the
summary, or `-` when absent. Do not change the header or the other columns.

Update the two literal rows in `tests/attach_cli.rs` that contain the old cell: the one
at line 243 and the `agent-plan` row at lines 400 to 402. Each changes `no` in the HELD
position to `-` and keeps the column width, so
`s1  -     running  no        0        no    80x24  sh -c sleep 30\n` becomes
`s1  -     running  no        0        -     80x24  sh -c sleep 30\n` (the ATTACHED
cell stays `no`), and the `agent-plan` row changes the same way. The header assertions
(lines 241, 250 and 398) stay as they are. Add one case where a session is held by a
human draft and its row shows `human_draft`; derive that row's widths from the table
builder's existing rule that each column is as wide as its longest cell.

## 7. Tool descriptions, `src/mcp.rs`

Exact new descriptions, replacing lines 360, 369 and 392:

- `list_agents`: `List the agent sessions you can message. "attached" means a human client is attached to the session; it does not mean the session is reachable, and a detached session still receives messages.`
- `send_message`: `Send a message to another agent session. Returns a message id once the message is accepted; acceptance does not mean it was delivered, read, or acted on. Check message_status when the reply matters. If recipient_hold is present, the recipient is held and a person may need to act. Plain text; do not write the text of a paste-wrapper tag in a body.`
- `message_status`: `Show the state of a message you sent. "state" and "detail" say whether it was delivered, is waiting, or was given up on; "hold_reason" and "hold_explanation" say what a waiting message needs; "evidence" of submission_observed means the recipient's prompt hook saw it submitted; "accepted_at" and "updated_at" are Unix seconds.`

The descriptions are hardcoded in `tests/mcp.rs` at lines 225, 234 and 257; replace those
strings with the new ones verbatim. `docs/specs/spec-2-messaging.md` keeps the old text
because it is a locked document.

## 8. Docs

- `docs/delivery.md`: describe the per-message limit and `unmatchable_submission` in the
  corrupted-submissions section and in the holds table's surroundings; replace the
  sentence in the observed-behavior bullet saying the matcher does not model the wrapper
  escaping with the opposite, citing 2.1.286; describe `recipient_hold`,
  `hold_explanation` and the timestamps where `evidence` is described.
- `docs/architecture.md`: delete the paragraph that starts "Known gaps found in the first
  real runs"; describe the new fields in the human-controls and messaging parts only
  where the current text would otherwise be wrong.
- `README.md`: in "Using it with agents", rewrite the bullet about the wrapper tag to say
  the text is handled on Claude Code and the sender can still avoid it; update the list
  column sentence for HELD to say it shows the reason.
- `docs/validation-plan.md`: add the escaping rule and the per-message limit to the
  checks, in the file's existing table style, marked verified on 2.1.286.
- Public repository rules apply: fictional names only, no message ids from real runs.

## 9. Out of scope

A persisted hold log; auto-clearing `human_draft` from composer state; changes to
`src/emulator.rs`, `src/prefix.rs`, `src/hook.rs`, `src/delivery.rs`; Codex and OMP
profiles; cross-host routing; envelope wording or per-peer authorization (the operator
line is unchanged); pushing notices into a sender's composer; a send-time rejection of
wrapper-tag text; renaming the envelope tag; any rename or refactor beyond what sections
3 to 7 name.

## 10. Acceptance

All three must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

And, run from the worktree root after implementation:

```sh
git diff --name-only | sort
rg -n "pasted_content" src/harness.rs
rg -l "hold_explanation" src tests | sort
rg -n "UNMATCHABLE_SUBMISSION_REASON|MAX_MESSAGE_REJECTIONS" src tests
git diff --check
```

Expected: the first lists only paths from section 0's scope list; the second prints at
least one line inside `paste_view`; the third lists at least `src/daemon.rs`,
`src/messaging.rs`, `src/wire.rs`, `src/mcp.rs` and one file under `tests/`; the fourth
shows both names defined in `src/messaging.rs` and used in `src/daemon.rs`; the last
prints nothing. Leave the diff uncommitted and unmerged.
