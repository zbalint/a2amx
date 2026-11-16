# Spec 3fa: split `tests/daemon_cli.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts the previous refactor spec or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/daemon_cli.rs`: deleted (its content moves, see section 2).
- `tests/daemon_cli/main.rs` and `tests/daemon_cli/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test daemon_cli`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/daemon_cli.rs` is 2946 lines (CodeScene 4.54, 54 findings) with 72 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev, 3ew and 3ex applied one method to `tests/confine.rs`, `tests/broker.rs` and `tests/team.rs` (one directory test target, byte-for-byte moves, mechanical equivalence checks, accepted with 0 BLOCKED cycles); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, and the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`).

Differences from 3ex: line 1 is an inner `#![allow(clippy::expect_used, clippy::unwrap_used)]`, which stays line 1 of `main.rs`; the doc comments (`///`) above `Cleanup`-style helpers move with their item; five indented `use` lines sit inside tests (2134-2136, 2610-2611), so the normalization of acceptance 2 keeps them on both sides. `ExitFixture` (struct plus its two impl blocks, 1317-1446) is used only by the kill tests and moves to `kill_and_down`.

Alternatives rejected, as in 3ev: several `tests/daemon_cli_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/daemon_cli.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2946 is in exactly one of them (the architect checked this with a script that also lists helper use sites). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-39, 51-64, 77-85, then the 6 `mod` lines | the preamble (inner `#![allow]`, `mod common;`, imports, `BIN`) and the helpers used by several modules: `BIN`, `a2amx`, `run`, `run_owned`, `stdout`, `stderr`, `screen_contains_until`, `Cleanup` | ~69 |
| `daemon_lifecycle` | 86-219, 1212-1265, 1307-1316, 1607-1636, 2763-2946 | 18 tests, `background_start_detaches_and_logs` .. `config_show_reads_owner_away_environment`; helper `wait_exit` (used only here) | ~412 |
| `reset` | 65-76, 228-825, 840-957 | 13 tests, `reset_uses_default_clear_sequence` .. `reset_rejects_a_concurrent_request_as_busy`; helpers `session_diagnostics`, `fake_reset_composer`, `fake_timeout_composer` (used only here) | ~728 |
| `screen` | 40-50, 220-227, 826-839, 958-1211 | 10 tests, `harness_is_inferred_from_the_command` .. `screen_wide_character_is_not_duplicated`; helpers `screen_until`, `fake_claude` (used only here) | ~287 |
| `kill_and_down` | 1266-1306, 1317-1606 | 13 tests, `kill_running_session_requires_confirmation_without_a_terminal` .. `team_down_now_skips_ctrl_d`; helper `ExitFixture` (used only here) | ~331 |
| `peer_exit_watch` | 1637-2025 | 7 tests, `watching_session_receives_a_peer_exit_message` .. `team_down_peer_does_not_trigger_an_exit_message` | ~389 |
| `heartbeat_and_activity` | 2026-2762 | 11 tests, `idle_watcher_receives_heartbeat_for_busy_peer` .. `not_ready_watcher_does_not_receive_heartbeat` | ~737 |

Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is.

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- The `env!` uses (`CARGO_BIN_EXE_a2amx` at line 9, `CARGO_PKG_VERSION` at line 1377) do not change; none depends on the file's location.
- Comments, `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fa` first; delete it before reporting. Before editing, save `cargo test --test daemon_cli -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fa/before.txt` (73 lines expected). After, the same command with `sed -E 's/^(daemon_lifecycle|reset|screen|kill_and_down|peer_exit_watch|heartbeat_and_activity)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/daemon_cli.rs`, `new` = `cat tests/daemon_cli/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path` (an indented `use` inside a function is kept on both sides, so losing one fails the check), then `sort`; `diff` exit 0. For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The five indented `use` lines inside tests are untouched.
3. `wc -l tests/daemon_cli/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores; every score at least the old 4.54.
4. `cargo test --test daemon_cli` has the same passed/ignored counts as the baseline recorded before editing; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/daemon_cli.rs`, the new `tests/daemon_cli/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none recorded for this file. A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 72 tests; five indented `use` lines inside tests (2134-2136, 2610-2611); `env!` uses: `CARGO_BIN_EXE_a2amx` at line 9, `CARGO_PKG_VERSION` at line 1377 | `rg -c '^#\[(tokio::)?test' tests/daemon_cli.rs`, `rg -n '^\s+use '`, `rg -n 'env!'` | grep |
| Helper use sites by module and the partition of all 2946 lines | `plan.py` run by the architect over the file (every line covered once) | script |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works | specs 3ev, 3ew, 3ex accepted (aad8a9a, 81d9a1a, 2cf2d65) | built |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (combined pre-lock review of 3ez and 3fa, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0. No consultant design read of the diff (method reviewed on earlier specs; consultant quota low).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | §1 said the five indented `use` lines are deleted by the normalization, contradicting the column-0 form of acceptance 2 | wording changed to "kept on both sides" |
| Draft review | consultant | `new_rejects_the_daemon_sender_name` is a validation test of `new`, not a peer-exit test | moved to `daemon_lifecycle` |
| Implementation | developer | no problems found; acceptance 1-5 pass, scores 5.97 to 10.00 | none |
| Tester pass | tester | no problems found: 162 items byte-identical and placed as the table says, 73 passed | none |
| Architect review | architect | no problems found; rebuilt each file from the spec's ranges (all seven identical), sorted-line diff, fmt, clippy, `cargo test --test daemon_cli` (73 passed) | none |
| After the push (found in the 3fb pre-lock review) | consultant, then architect | section 5 said no known flaky tests, but backlog row I18 names `reset_runs_configured_steps_in_order` and `reset_times_out_when_second_step_stays_not_ready` of this file | not a code defect; backlog I18 now names `tests/daemon_cli/reset.rs`; this spec's line stays as locked; no other split spec's file has a flake row (checked in `docs/backlog.md`) |
