# Spec 3ez: split `tests/messaging.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts the previous refactor spec or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/messaging.rs`: deleted (its content moves, see section 2).
- `tests/messaging/main.rs` and `tests/messaging/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test messaging`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/messaging.rs` is 2088 lines (CodeScene 4.01, 25 findings) with 43 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev, 3ew and 3ex applied one method to `tests/confine.rs`, `tests/broker.rs` and `tests/team.rs` (one directory test target, byte-for-byte moves, mechanical equivalence checks, accepted with 0 BLOCKED cycles); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`.

Differences from 3ev: this file has no `mod common;`, no `#![allow]` and no inner doc comment, so the test list has no extra line and `main.rs` starts with the `use` lines of the old file. Six `use a2amx::messaging::...` lines sit inside tests (lines 1606, 1641, 1673, 1700, 1707, 1910); the normalization of acceptance 2 keeps them on both sides and they stay untouched inside their functions. The top-level consts `WORKED_*` (lines 12-17) are used only by `limits_and_envelopes` and move there, so every `use` of the preamble is still imported by `main.rs` and reaches the modules through `use super::*;`.

Alternatives rejected, as in 3ev: several `tests/messaging_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/messaging.rs` (1-based, both ends included), preceded in every module except `paste_and_teams` by `use super::*;` and a blank line (`paste_and_teams` uses nothing from the preamble, its tests import their own items, so a glob there would be an unused import and `clippy -D warnings` would fail; its file starts with its first range), and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2088 is in exactly one of them (the architect checked this with a script that also lists helper use sites). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-11, then the 5 `mod` lines | the imports (lines 1-11); there are no helpers shared by several modules, so `main.rs` holds only the imports and the `mod` lines | ~17 |
| `limits_and_envelopes` | 12-200 | 6 tests, `limits_and_message_states_have_locked_values` .. `names_hosts_and_addresses_follow_reserved_namespace_rules`; helpers `WORKED_ID`, `WORKED_FROM`, `WORKED_SUBJECT`, `WORKED_BODY`, `WORKED_ENVELOPE` (used only here) | ~189 |
| `harness_wiring` | 201-706, 1919-2088 | 11 tests, `harness_defaults_and_claude_argv_wiring_are_exact` .. `claude_status_line_invalid_files_and_shapes_are_fail_open` | ~676 |
| `agy_readiness` | 712-1119 | 4 tests, `agy_readiness_accepts_composer_shapes_and_pasted_content` .. `agy_turn_in_flight_detects_spinner_and_queued_rows_above_composer`; helpers `agy_screen_from`, `agy_screen_from_size`, `agy_composer_with_rows`, `agy_composer_bytes` (used only here) | ~408 |
| `claude_readiness` | 707-711, 1120-1603 | 16 tests, `claude_readiness_accepts_idle_and_mid_turn_composers` .. `generic_readiness_only_uses_bracketed_paste_and_scroll_state`; helpers `screen_from`, `f1_bytes` (used only here) | ~489 |
| `paste_and_teams` | 1604-1918 | 6 tests, `paste_wrappers_are_unwrapped_without_changing_malformed_text` .. `role_validation_accepts_architect_and_rejects_invalid_shapes` | ~315 |

Module names are fixed; test and helper names do not change. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is.

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module except `paste_and_teams`.
- The `env!` uses (none (`rg -n 'env!' tests/messaging.rs` finds nothing)) do not change; none depends on the file's location.
- Comments, `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3ez` first; delete it before reporting. Before editing, save `cargo test --test messaging -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ez/before.txt` (43 lines expected). After, the same command with `sed -E 's/^(limits_and_envelopes|harness_wiring|agy_readiness|claude_readiness|paste_and_teams)::(.*)$/\2/'` applied, then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/messaging.rs`, `new` = `cat tests/messaging/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path` (an indented `use` inside a function is kept on both sides, so losing one fails the check), then `sort`; `diff` exit 0. For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The six indented `use` lines inside tests are untouched.
3. `wc -l tests/messaging/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores; every score at least the old 4.01.
4. `cargo test --test messaging` has the same passed/ignored counts as the baseline recorded before editing; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/messaging.rs`, the new `tests/messaging/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none recorded for this file. A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 43 tests; six indented `use` lines inside tests (1606, 1641, 1673, 1700, 1707, 1910); `env!` uses: none (`rg -n 'env!' tests/messaging.rs` finds nothing) | `rg -c '^#\[(tokio::)?test' tests/messaging.rs`, `rg -n '^\s+use '`, `rg -n 'env!'` | grep |
| Helper use sites by module and the partition of all 2088 lines | `plan.py` run by the architect over the file (every line covered once) | script |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works | specs 3ev, 3ew, 3ex accepted (aad8a9a, 81d9a1a, 2cf2d65) | built |
