# Spec 3ew: split `tests/broker.rs` into themed modules of one test binary (refactor, second file)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts spec 3ev or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/broker.rs`: deleted (its content moves, see section 2).
- `tests/broker/main.rs` and `tests/broker/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`,
`CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test broker`. No test is added, removed or changed; the equivalence checks in
section 5 replace a mutation check.

## 1. Why

`tests/broker.rs` is 3,493 lines (CodeScene 3.51, 47 findings) with 47 tests and about 20 helper functions. The owner's goal is
no `.rs` file over 1,000 lines, tests included. Spec 3ev did the same to `tests/confine.rs` with this method (one directory test
target, byte-for-byte moves, two mechanical equivalence checks); this spec applies it unchanged. Lessons carried over from 3ev:
list the shared helpers explicitly (a helper used by two modules stays in `main.rs`), and treat the test list of a binary with
`mod common;` as 47 tests plus `common::fake_discord::tests::push_message_survives_gateway_reconnect`.

Alternatives rejected, as in 3ev: several `tests/broker_<theme>.rs` binaries (disk and link cost), `mod` blocks inside the one file,
shortening long tests (a later step).

## 2. Target shape

`tests/broker/main.rs` is today's lines 1-205 verbatim (line 206 is the attribute of the first test) (inner `#![allow(clippy::expect_used, clippy::unwrap_used)]`, `mod common;` with
`#[path = "../common/mod.rs"]` added above it, imports, `DaemonChild`, `session`, `screen_session`, `error_message`, `failed_code`, `accepted`,
`assert_secure_database`, `send`, `status`, `wait_agents`, `wait_status`), followed by the reset helpers that more than one module uses, in their original order (lines 1598-1917; line 1918 is the next test's attribute):
`omp_reset_target`, `reset_rejection`, `reset_success`, `reset_rejection_admin`, `reset_success_admin`, `wait_claude_ready`, `wait_claude_hold`,
then one `mod <name>;` line per module. A helper after the preamble that only one module uses moves into that module, before its first test and in original order, and stays private:
`claude_reset_target` and `timing_seconds` into `reset_diagnostics`, `agy_reset_target` and `wait_screen_contains` into `agy`, `quota_for` into `quota`.
Preamble helpers with one user (`screen_session`, `assert_secure_database`) stay in `main.rs`; do not move them. The developer confirms every placement with `rg -n`. **No visibility changes**: each module file starts with
exactly `use super::*;` and adds no other `use` line, no `#![allow]` and no `pub(crate)`.

Modules by line range in the current file (a range starts at the first attribute of the first listed function and ends on the line before the
attribute of the next listed one, except for the named move):

| Module | First function | Last function | Lines |
| --- | --- | --- | --- |
| `sessions_and_naming` | `session_tokens_roles_permissions_and_child_identity` (207) | `message_status_visibility_and_hold_reason` (584) | ~460 |
| `limits_and_lifecycle` | `pending_limit_and_content_errors` (665) | `not_ready_and_queued_hold_reasons` (1215), plus the named move | ~690 |
| `reset_basics` | `reset_authorizes_controller_and_rejects_unlisted_or_self` (1259) | `reset_holds_message_delivery_until_sequence_finishes` (1487) without the named move | ~250 |
| `reset_diagnostics` | `reset_diagnostic_busy_names_native_flags` (1919) | `reset_nonempty_claude_composer_stays_held_after_margin` (2339) | ~445 |
| `agy` | `agy_default_delivery_is_auto` (2364) | `reset_non_agy_with_spinner_above_composer_succeeds` (2785) | ~440 |
| `reset_in_flight` | `reset_real_omp_draft_hold_survives_margin_then_clears` (2804) | `reset_rejects_an_in_flight_native_delivery_as_busy` (2873) | ~180 |
| `storage_and_retention` | `retention_zero_and_database_permissions` (2984) | `retention_includes_the_exact_nonzero_expiry_boundary` (3251) | ~340 |
| `quota` | `quota_for` (3320) | `claude_quota_reports_are_token_scoped_and_merge_independent_windows` (3331) | ~175 |

Named move: `graceful_restart_marks_exited_and_continues_ids` (1398) goes to `limits_and_lifecycle`. Module names are fixed; test names do not change.
Estimated sizes after the move: `main.rs` about 425, `limits_and_lifecycle` about 685, `reset_diagnostics` about 485, `agy` about 510, the others 175 to 340 lines; all under 1,000.

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs`
  and the one `use super::*;` line in each module.
- The `CARGO_BIN_EXE_a2amx` and `CARGO_MANIFEST_DIR` uses do not change (neither depends on the file's location; the `frozen_realtime.c`
  fixture is read through `CARGO_MANIFEST_DIR`).
- `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged,
  split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; correcting misplaced tests beyond the named move
(they go to the follow-up row F94 in `docs/backlog.md`).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Before editing, save `cargo test --test broker -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ew/before.txt`
   (48 lines expected). After, the same command with `sed -E 's/^(sessions_and_naming|limits_and_lifecycle|reset_basics|reset_diagnostics|agy|reset_in_flight|storage_and_retention|quota)::(.*)$/\2/'`
   applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Judge: `diff`; the architect and the tester rerun it.
2. **Same code.** `old` = `git show HEAD:tests/broker.rs`, `new` = `cat tests/broker/*.rs`. Delete blank lines and lines starting (after indentation) with
   `mod `, `use ` or `#[path`, then `sort`; `diff` exit 0. The old file has no `use` inside a function (`rg -n '^\s+use ' tests/broker.rs` finds none).
3. `wc -l tests/broker/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores; every score at least the old 3.51.
4. `cargo test --test broker` has the same passed/ignored counts as the baseline recorded before editing; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/broker.rs`, the new `tests/broker/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none recorded for this file. A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| Preamble is lines 1-205; 47 tests; no `use` inside functions; `env!` uses are `CARGO_BIN_EXE_a2amx` (21) and `CARGO_MANIFEST_DIR` (3255) | `rg -c`, `rg -n '^\s+use '`, `rg -n 'env!'` on `tests/broker.rs` | grep |
| Helpers at 1598-1917 are `async fn`/`fn` without a test attribute | function list | read |
| A directory target works with `#[path = "../common/mod.rs"] mod common;` and a glob `use super::*;` | spec 3ev, accepted: same shape built and run (147 tests passed) | built |
