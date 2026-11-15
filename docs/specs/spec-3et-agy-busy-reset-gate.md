# Spec 3et: `agy` reset refuses while a turn is in flight (backlog G15)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `3436861` or later.
Shared task `context_id`: `a2amx-agy-reset-busy-g15`. Follows spec 3es.

**Scope.** May edit exactly these files and no others.
- `src/harness.rs` (one new public function next to `agy_composer_idle`)
- `src/session.rs` (`reset_gate_decision`: one agy condition)
- `tests/messaging.rs` (unit tests of the new function, next to the existing agy composer tests)
- `tests/broker.rs` (the `agy_reset_target` helper signature and its callers, and the new reset and delivery tests)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md`,
`Cargo.toml`, `Cargo.lock`, `src/messaging.rs`, `src/daemon.rs`, `src/delivery.rs`, `src/emulator.rs`, every other
file and spec. Do not commit, stage or merge.

Public test seams: `a2amx::harness::agy_turn_in_flight` (unit, `tests/messaging.rs`), and `Request::Reset` /
`Request::Explain` / message submission against a real daemon (`tests/broker.rs`).

## 1. Why

Backlog G15. The tester found, live, that `reset` on an `agy` session sent `/clear` while agy was working. agy then
logged `Starting new conversation` and `Pending inputs cancelled (2 regular + 0 shell dropped)`: the in-flight turn
and the queued messages were dropped, while a2amx still reported those messages `submitted` (it means the paste was
written, not that the model processed it). The reset gate did not see it because `agy_composer_idle` looks only at
the composer box, which is empty while agy works.

Owner decision (2026-11-15): `agy` reset refuses whenever a turn is in flight, as OMP already does (`busy` while the
native bridge reports `in_flight`, `src/session.rs`, `reset_native_reason`), not only when queued messages are
visible.

Evidence, from an owner-run capture of a live agy 1.5.9 session (about 450 frames at two frames per second
across: generating, a tool call, queued messages during both, Esc, idle):

1. While a turn runs, one row directly above the composer's top rule starts with a Braille-pattern glyph
   (U+2800..=U+28FF) followed by a space: the spinner row. Its glyph rotates every frame and its text varies
   (`Generating...`, `Running command...`, `Working...`, `Reading file...`, `Calling MCP tool...`, `Loading...`, or a
   sentence summarizing the model's thinking), so only "Braille glyph, then a space" is stable. It was present in
   every generating and tool-call frame and absent in every idle frame.
2. Queued messages render as `▸ <a2amx-message ...> (alt+m to send immediately)` rows between the spinner row and the
   composer's top rule, during plain generation as well as during a tool call, with the composer `>` empty. With
   queued rows the spinner row is directly above the first queued row, so the row directly above the composer's top
   rule is always either the spinner row or a `▸` row.
3. For one frame (about 0.7 s) between two turns, a `▸` row was visible with no spinner row. A `▸` row alone therefore
   also means work is pending and must refuse.
4. After Esc interrupts a turn: no spinner row, empty composer, and the transcript keeps
   `⎿ Interrupted · What should Antigravity CLI do instead?` for the rest of the session. That line is not a marker;
   reset is correct to run after an interrupt.
5. Idle frames have a blank row directly above the composer's top rule.

Alternatives rejected: gating on `▸` rows only (does nothing for a tool call with nothing queued, which `/clear`
also cancels); matching spinner text (varies; thought summaries are free text); gating the delivery path too
(auto delivery while agy works is intended: agy queues the pasted message, spec 3es).

## 2. `src/harness.rs`: new `agy_turn_in_flight`

Add, directly after `agy_composer_idle`:

```rust
/// Returns whether an Agy turn is in flight: the row directly above the composer's top rule is a spinner row
/// (a Braille-pattern glyph, U+2800..=U+28FF, then a space) or a queued-message row (`▸ `).
pub fn agy_turn_in_flight(screen: &Screen) -> bool
```

It finds the composer with the existing private `agy_composer`; with none, or with its top rule on row 0, it
returns `false`. Otherwise it reads the first two cells of row `top - 1` with `cell_char` and returns `true` when
they are a Braille-pattern glyph then `' '`, or `'▸'` then `' '`.

Placement check: `harness.rs` already owns every agy screen predicate; the function reuses `agy_composer` and
`cell_char`, so it belongs here, not in `session.rs` (which must not read cells).

Leave a `// shortcut:` comment: only the row directly above the rule is read, so a spinner shape agy draws
elsewhere is not seen; widen it if a capture shows one (the permission card, narrow and scrolled screens were
not captured). The matcher fails safe: a false positive refuses a reset with `busy`, never types.

## 3. `src/session.rs`: reset gate

In `reset_gate_decision`, in the chain that already ends with the agy `agy_composer_idle` branch returning
`draft_present` / `composer not empty`, add one more branch after it: when `self.harness == Harness::Agy` and
`harness::agy_turn_in_flight(&screen)` (same screen snapshot and `scrolled` handling as the branch above; the
scrolled case already fails `ready`), return `Some((messaging::code::BUSY, "turn in flight".to_owned()))`.

Order matters and is fixed: exited, then hold (which first releases a stale agy draft), then `ready`, then the
composer branch (`draft_present`), then this branch, then the native reason. A composer holding text during a turn
therefore still answers `draft_present`. Read the screen under the same lock as the composer branch.

Consequences to keep (not to route around): `reset_gate_decision` is shared by reset (before each step and in the
wait after each step), graceful `kill`, and `explain`'s reset preview, as in spec 3es. A busy agy therefore also
skips the graceful Ctrl-D exit and falls back to the non-graceful kill, and `explain` shows `reset: refused (busy):
turn in flight`. The wait after `/clear` now needs the spinner row gone as well; agy draws a fresh banner after
`/clear`, and the live check below covers it.

Accepted consequence: the wait after each reset step, the last one included, polls this gate (settle 1.5 s, then
until the 30 s step timeout). An agy session configured with a reset list whose step starts a model turn (for
example `["/clear", "read AGENTS.md"]`) used to pass that step as soon as the composer was empty. It now waits for
the turn to finish and fails with the step-timeout error after 30 s even though the step was submitted. The default
list is `["/clear"]` and this is accepted; do not add an exception for it.

`begin_delivery` is not changed: a delivery to a working agy is typed and queued by agy (spec 3es), and it still
requires only the idle composer.

## 4. Tests

`tests/messaging.rs`, unit tests of `harness::agy_turn_in_flight`, built with the existing `agy_composer_with_rows`
(row indices are zero-based; the composer's top rule is the `top` argument):
1. Spinner row directly above the rule (`top = 3`, `(2, "⣷  Working...")`): `true`.
2. Queued row directly above the rule (`(2, "▸ queued (alt+m to send immediately)")`): `true`. Two queued rows and a
   spinner row above them (`(0, spinner)`, `(1, ▸ row)`, `(2, ▸ row)`): `true`.
3. Blank row above the rule, and a transcript text row above it (`(2, "  some text")`): `false`. A spinner row two
   rows above the rule with a blank row between: `false`.
4. A Braille glyph not followed by a space (`⣷x`), a non-Braille first cell (`*  Working...`), `▸` not followed by a
   space: `false`. The block edges: U+2800 and U+28FF followed by a space: `true`; U+27FF and U+2900: `false`.
5. No composer, built with `agy_composer_bytes(top, body, col, false)` (cursor hidden; `agy_composer_with_rows`
   always leaves the cursor visible), and a composer with its top rule on row 0 (`agy_composer_with_rows(0, ...)`):
   `false`.
These pass only with the new function; there is no earlier behavior to contract-test.

`tests/broker.rs`:
6. Extend `agy_reset_target` with one more parameter, `above: &str`, printed in the helper's `printf` after
   `\033[2J` and before `\033[2;1H` (`\033[1;1H{above}`; before the clear it would be erased); update every caller (today lines 2374, 2403, 2422, 2442, 2475, 2511, 2549, 2582, 2609; grep
   `agy_reset_target(` again) to pass `""`. No second helper.
7. Reset on a target with `above = "⣷  Working..."` and an empty composer, no hold: rejected `busy` with text
   `turn in flight` (`reset_rejection_admin`), and `explain` reports the reset as refused with code `busy`. The same
   with `above = "▸ queued (alt+m to send immediately)"`. The helper's child is `cat`, so "nothing typed" is shown
   by the rejection and by the mutant below.
8. Reset with `above = "⣷  Working..."` and composer text `draft`: rejected `draft_present` (order check).
9. Contract tests, which pass on the old code and must stay green: reset on an empty idle composer succeeds
   (existing tests); a message sent to a target whose `above` is `"⣷  Working..."` is still submitted (queuing
   behind a running turn is kept). Wait past the retry interval before asserting a state that a retry loop could
   change (spec 3es lesson).

Mutation, in the assigned checkout, restored byte for byte afterwards (checksum): delete the `session.rs` branch:
test 7 fails (test 8 still passes, the composer branch answers it); move the new branch before the composer branch:
test 8 fails; make `agy_turn_in_flight` ignore `▸` rows: the queued cases in tests 2 and 7 fail; make it ignore the
glyph check (any first cell then space): the non-Braille case in test 4 and the blank-row case in test 3 fail; check
the row `top` instead of `top - 1`: tests 1 and 2 fail. Name the failing test per mutant.

## 5. Out of scope

`docs/` and `CHANGELOG.md` (architect), the permission-card race (G11), lone Esc (G12), delivery gating, the
`● [..] ... running` background-task row drawn below the composer (agy keeps a shell job running after Esc; whether
`/clear` kills it is not measured), narrow and scrolled screens, `src/emulator.rs`, `input_is_typing`.

## 6. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass (judge: the commands; run
   with the ambient environment, see AGENTS.md).
2. `git status --short` lists exactly the four scoped files (judge: the architect at final review).
3. The mutation check names a failing test per mutant and the files are restored by checksum (judge: the architect
   reruns two mutants on a scratch copy).
4. Live, owner-run after commit, on a throwaway agy session, never the working one. Judge: the owner, with
   `a2amx screen` captures; the confined tester cannot run live agy (its confinement hides agy's credentials).
   Expected, per case: while a `sleep 25` tool call runs, `a2amx reset` is refused `busy` and `a2amx explain` shows
   `reset: refused (busy)`; while a long essay is written, the same; with a message queued behind either, refused
   `busy` when the screen shows a `▸` row above an empty composer, or `draft_present` when it shows the queued text
   inside the composer (an earlier tester run saw that during plain generation); success is a failure in every one of
   these cases. After Esc, and on an idle session, `a2amx reset` succeeds and the conversation is cleared: `a2amx
   screen` shows the fresh banner and none of the earlier transcript. Two extra idle checks, because the matcher
   depends on a blank row above the composer: after a reply whose last line starts with `▸ ` or a Braille glyph,
   `a2amx explain` shows `reset: gate open`; and note whether a `▸` row stays after Esc while a message was queued
   (unmeasured; if it stays, reset is refused until the queue empties).

Baseline: CI passed for `3436861` (run 43754526774) and the Sonar gate on `develop` is OK. The suite was not rerun
locally for this draft; the machine runs one build at a time.
