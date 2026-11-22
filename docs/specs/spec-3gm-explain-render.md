# Spec 3gm: `a2amx explain` renders its state lines in a pure function (S5c)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file (HEAD before it: 7c5ff7b). Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seams: the `a2amx explain` command through `tests/explain.rs` (process tests, unchanged) and the library function `a2amx::table::format_explain` through `tests/table.rs`.

Scope (edit): `src/table.rs`, `src/commands/sessions.rs`, `tests/table.rs`.

Does not touch: every other file, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md` (the `table` row) and the backlog when committing.

## 1. Why

`run_explain` (`src/commands/sessions.rs:182-269`, 88 lines) fetches the session's explain info, builds eleven state lines from it with seven branches (exited or running, activity unknown, three ready states, needs input, hold with and without explanation, reset gate open or refused with unknown code/reason), then asks the daemon for the session list, runs `grant::denial_help` on a blocking thread and appends it. The state lines are pure text over an `ExplainInfo`, but they can only be tested through a real daemon and a spawned binary: `tests/explain.rs` covers most branches (the refused reset with code `not_ready` at `:249`) and the unknown-activity, unknown-reset-code/reason and hold-without-explanation-in-the-running-line branches have no test. Moving the rendering to `src/table.rs` (the module that turns wire views into text, pure) gives them a unit seam and leaves `run_explain` the I/O.

Target shape:

```rust
// src/table.rs
/// The state lines `a2amx explain` prints for a session: one header line, then for a running session activity, readiness, needed input, hold, queue and the reset verdict.
pub fn format_explain(info: &ExplainInfo) -> String;
```

`run_explain` becomes: resolve, connect, `explain`, `let mut output = format_explain(&info);`, the confinement help as today, write.

Rejected: moving the confinement-help block (it needs `HOME`, the filesystem and a blocking thread: I/O, not rendering) and splitting it into its own function in this spec (it is 30 lines of one concern; a follow-up if wanted); a `Display` impl on `ExplainInfo` (it would put CLI wording in the wire module); changing any text.

## 2. Changes

1. `src/table.rs`: add `format_explain` after `format_messages_table`, with the body of `run_explain`'s state-line block moved byte for byte except for exactly two edits, both needed because `info` is now `&ExplainInfo`: `if let Some(hold) = info.hold` becomes `if let Some(hold) = info.hold.as_ref()` and `if let Some(explanation) = hold.explanation` becomes `if let Some(explanation) = hold.explanation.as_deref()` (a move out of a reference is E0507); the other accesses (`exit_code`, `ready`, `activity`, `starting`, `pending` are `Copy`; `needs_input`, `activity_cause` and the reset fields already use `as_deref`) work through the reference unchanged: the exact lines and their order are (exited) `{id} ({address}): {harness}, exited (code {code})`; (running) `{id} ({address}): {harness}, running`, `activity: {activity|unknown} ({cause|unknown})`, `ready: {yes|no (starting)|no|unknown}` (`Some(true)` yes, `Some(false)` with `starting` "no (starting)", `Some(false)` "no", `None` unknown), `needs input: {needs_input|no}`, `hold: {reason}[ - {explanation}]` or `hold: none`, `queued: {pending}`, `reset: gate open` or `reset: refused ({code|unknown}): {reason|unknown}`; every line ends in `\n`. Imports: `ExplainInfo` joins the existing `crate::wire` import; `Activity` is already imported. The `//!` line of `src/table.rs` becomes `//! Turning wire views into CLI text: aligned session and message tables and the explain lines.` (in scope; the architect updates the AGENTS.md row).
2. `src/commands/sessions.rs`: in `run_explain` replace the `let mut output = String::new();` and the whole `if let Some(code) = info.exit_code { .. } else { .. }` block (`:186-236`) with `let mut output = crate::table::format_explain(&info);` (or the existing `use crate::table::{..}` list gains `format_explain`); everything from `let sessions = client.sessions().await?;` stays. The `use crate::wire::Activity;` import (`:10`) goes: its only other use is inside the moved block (`:201`), and clippy `-D warnings` fails on an unused import.
3. `tests/table.rs`: add unit tests first, red before step 1 (compile failure counts): `explain_exited_session_is_one_line`, `explain_running_session_with_unknowns_and_open_gate`, `explain_running_session_with_hold_and_refused_reset`, and `explain_ready_states` (the three `ready:` lines: ready, not ready starting, not ready), each asserting the exact whole string from a literal built by hand, never from the function. Fixed contents (all 12 fields of `ExplainInfo` spelled out, no `Default`; `HoldInfo` has 2 fields, `ResetVerdict` 3; import `ExplainInfo, HoldInfo, ResetVerdict, Activity` from `a2amx::wire`): test 1 `exit_code: Some(3)`; test 2 (`..unknowns_and_open_gate`): `exit_code None, ready None, activity None, activity_cause None, needs_input None, hold None, pending 0, reset allowed true` giving `activity: unknown (unknown)`, `ready: unknown`, `needs input: no`, `hold: none`, `queued: 0`, `reset: gate open`; test 3 (`..hold_and_refused_reset`): `activity Some(Working), activity_cause Some("output"), needs_input Some("trust prompt"), hold Some(HoldInfo{reason: "recipient_busy", explanation: Some("composer has a draft")}), pending 2, reset allowed false, code Some("not_ready"), reason Some("screen not ready")`, plus a second assertion in the same test with `explanation: None` giving `hold: recipient_busy\n` and `reset: refused (unknown): unknown\n` (code and reason None); test 4: the three `ready:` lines. The ` - ` separator is then pinned by test 3. Build `ExplainInfo` literals with fictional ids (`s1`, `agent-plan@host-a`) and `Harness::Generic` (name the harness variant that exists; `rg -n 'Generic' src/harness.rs`).

## 3. Behaviour

No observable change: the same text for every input. `tests/explain.rs` pass unchanged.

## 4. Out of scope

The confinement help block; `run_screen`, `run_messages`, `run_kill`; any new text, error or flag; `ExplainInfo`'s wire shape.

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass; baseline 66 suites, 1418 passed, 3 ignored (7c5ff7b); after: 1422 (four new tests: the developer states the count if different). The new tests fail to compile before step 1: that is the red phase.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'fn format_explain' src` prints one line; `rg -n 'exited \(code' src` prints only `src/table.rs`; `wc -l` of `run_explain` reported (about 38 expected, a report item).
- `git status --short` lists only the three scope files; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Diff check (tester, mechanical): every format string of the removed block appears byte for byte in `format_explain`, in the same order.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the killing test; survivors reported: swap the `starting` condition of `ready:`; `hold: none` text changed; ` - ` separator of the explanation changed; `reset: gate open` and refused swapped; `activity` unknown text changed; the `activity_cause` unknown text changed; `needs input` default `no` changed; exit-code line changes `code` to `exit`; queued line drops its value. Each should be killed by a new table test (some also by `tests/explain.rs`, which the tester names where it does).
- Known flaky tests: those that wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| `run_explain` is `src/commands/sessions.rs:182-269` and the state-line block is `:186-236` | file | read |
| `ExplainInfo`, `HoldInfo`, `ResetVerdict` fields and types (`exit_code: Option<i32>`, `ready: Option<bool>`, `starting: bool`, `activity: Option<Activity>`, `pending: u32`, `reset.code/reason: Option<String>`) | `src/wire.rs:393-430` | read |
| `Activity` is `Copy` and has `as_str`; `src/table.rs` already imports it | `src/wire.rs:431-440`, `src/table.rs:6` | read |
| `src/table.rs` is pure (no I/O) and `tests/table.rs` imports the table functions and wire types | files | read |
| `tests/explain.rs` covers the ready, hold, not-ready, starting, needs-input, exited-confined and exited branches by process tests, and `reset: refused (not_ready): screen not ready` at `:249`; nothing asserts `activity: unknown` or the `unknown` fallbacks of the reset verdict | `rg -n 'reset: refused\|activity: unknown' tests` | probed |
| the string `exited (code` occurs once in `src` (`commands/sessions.rs:189`), and `run_explain` is called only from `commands.rs:185` | `rg -n 'exited \(code\|run_explain' src` | probed |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer antigravity Gemini 4 Hydrogen (Medium) (the senior developer); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft | architect | the claim "no test asserts `reset: refused`" was false (`tests/explain.rs:249`) | claims row and Why corrected before review |
| draft review | consultant | the borrow edits were listed wrongly: `if let Some(explanation) = hold.explanation` moves out of a reference (E0507) and was not named | the two edits stated exactly |
| draft review | consultant | the block range was `:186-232`, it is `:186-236`; length estimate 45 (38) | corrected |
| draft review | consultant | the new tests left the unknown-fallback gaps open (their field values were not fixed) | literal contents fixed in the spec |
| draft review | consultant | the `Activity` import and the `//!` line of `table.rs` were left open | import removal stated as fact; `//!` line in scope |
| implementation | developer | `push_str(&explanation)` after `as_deref()` fails clippy `needless_borrow`; the spec said exactly two edits | third edit made, behaviour identical; recorded (tester reproduced the lint) |
