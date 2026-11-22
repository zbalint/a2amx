# Spec 3gn: `sgr_mouse_report_len` moves to `mouse` and loses its repeated digit loops (code health)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file (HEAD before it: ba57b48). Context `a2amx-main-code-health-2026-11-22`. Public test seams: `a2amx::mouse::parse_report` through `tests/mouse.rs`, and `a2amx::messaging::input_is_typing` through `tests/messaging/limits_and_envelopes.rs`, which sees the inner function's result unmasked.

Scope (edit): `src/mouse.rs`, `src/messaging.rs`, `tests/mouse.rs`, `tests/messaging/limits_and_envelopes.rs`.

Does not touch: every other file, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates nothing else (the `mouse` and `messaging` rows of `AGENTS.md` already describe the result).

## 1. Why

`cs check` (CodeScene CLI, 2026-11-22) scores `src/messaging.rs` 5.99, the lowest source file, with file-level Low Cohesion and String Heavy Function Arguments, and flags `sgr_mouse_report_len` (`src/messaging.rs:709`) as Complex Method (cc = 13). The function parses a terminal mouse report: the `mouse` module's job. Today `mouse.rs` imports it from `messaging` (`src/mouse.rs:3`), the wrong way round, and `messaging` keeps it only for `input_is_typing`.

The function is three copies of "skip digits, require at least one, then require a separator". A private helper in `mouse.rs` that does that once leaves the function a short chain.

Rejected: moving `input_is_typing` too (it is `pub`, called by `src/session.rs:340` and imported by `tests/messaging/main.rs:7`: a rename of a used path for no extra score gain here); changing what the function accepts (X10, urxvt and prefix handling stay as they are; the spec 3ar claims on it stay true).

## 2. Changes

1. Characterization tests first (this is a behavior-preserving refactor: they pass on today's code and must stay green after each step; there is no red phase). `tests/mouse.rs` gains `sgr_report_end_*` tests through `parse_report(bytes, 0).map(|r| r.end)` (hand-written literals): `\x1b[<0;10;20M` gives `Some(11)`; the release form `\x1b[<0;10;20m` gives `Some(11)`; `\x1b[<0;1;2Mxyz` gives `Some(9)`; and `None` for a bad final byte (`\x1b[<0;1;2X`) and a missing `;` after the first number (`\x1b[<0M`). The truncated case (`\x1b[<0;5;2`, `tests/mouse.rs:66`) and trailing bytes not consumed (`tests/mouse.rs:75-76`) already exist in `malformed_and_partial_mouse_reports_are_rejected`: do not repeat them. Because `parse_report` re-checks the number fields itself (`src/mouse.rs:35-42`), it masks mutants that make the inner function accept an empty number or a missing second `;`. `tests/messaging/limits_and_envelopes.rs` therefore gains `input_is_typing` cases that each expect `true`, one assertion per input: `\x1b[<;1;2M`, `\x1b[<0;;2M`, `\x1b[<0;1;M`, `\x1b[<0:1;2M`, `\x1b[<0;1:2M` (the existing cases at `:137-155` do not kill those mutants).
2. `src/mouse.rs`: add `pub(crate) fn sgr_mouse_report_len(bytes: &[u8], start: usize) -> Option<usize>` with the same contract: `start` is at `ESC`, the digits begin at `start + 3`, the result is the index one past the final `M` or `m`. Implement with one private helper that takes the index and returns the index after one or more ASCII digits, or `None` when there is none, so the body is three helper calls with `;` checks and the final-byte match. Remove the `use crate::messaging::sgr_mouse_report_len;` line.
3. `src/messaging.rs`: delete `sgr_mouse_report_len`; `input_is_typing` calls `crate::mouse::sgr_mouse_report_len`, and the call site uses that path. No other line in `messaging.rs` changes.

## 3. Behaviour

None: every input yields the same `Option<usize>`. Existing tests pass unchanged; only new tests are added.

## 4. Out of scope

`input_is_typing`; the X10 branch; urxvt and the prefix check; any other `messaging.rs` finding (`validate_message`, `render_envelope`, the `:690` conditional): follow-ups.

## 5. Acceptance

- `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `rg -n 'fn sgr_mouse_report_len' src` prints one line, in `src/mouse.rs`.
- `git status --short` lists only the four scope files (the spec is committed before assignment).
- `CS_DISABLE_VERSION_CHECK=1 cs check src/mouse.rs src/messaging.rs` (tester, reported, advisory): no `sgr_mouse_report_len` Complex Method finding; scores before: messaging 5.99; the tester records mouse.rs's before and after.
- Mutation check (tester, one change at a time, restore by checksum), naming the killing test: make the helper accept zero digits; drop the first `;` check; drop the second `;` check; change `M | m` to `M`; start at `start + 2`. The first three are killed only through `input_is_typing` (the `tests/messaging` cases); the last two also through `tests/mouse.rs`. Survivors reported.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| `sgr_mouse_report_len` is at `src/messaging.rs:709`, cc = 13 | `cs check` output, file | read |
| its users are `src/mouse.rs:32` (import at `:3`) and `src/messaging.rs:753` only | `rg -n sgr_mouse_report_len` | probed |
| `tests/` never names it directly | `rg -n 'sgr_mouse_report_len\|input_is_typing' tests` | probed |
| `input_is_typing` is used by `src/session.rs:340` and tests | same | probed |
| spec 3ar says it "stays where it is" (scope of that spec only) | `docs/specs/spec-3ar-attach-top-rows.md:31` | read |
| `mouse.rs` has no I/O and does not otherwise import `messaging` | `src/mouse.rs` | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (its report names its own model); developer antigravity Gemini 4 Hydrogen (Medium); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft | architect | the expected `end` values in the first draft were miscounted (12 and 8, correct 11 and 9) | corrected before review |
| draft review | consultant | `parse_report` re-checks the number fields, so three of the five listed mutants could not be killed through `tests/mouse.rs` | `tests/messaging/limits_and_envelopes.rs` added to scope with five `input_is_typing` cases |
| draft review | consultant | the spec said both "private" and `pub(crate)`; "red first" was wrong for a behavior-preserving refactor; two cases already existed; two mutants named code that would not exist; the status check counted three files | all five corrected in the lock |
| tester pass | tester | a non-`M`/`m` final byte followed by `ESC [ I` survived (`_ => None` mutated to `Some(after_row)`); `parse_report` masks it | architect added the case `\x1b[<0;1;2\x1b[I` to `malformed_sgr_mouse_reports_count_as_typing` |
