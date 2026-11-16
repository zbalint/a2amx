# Spec 3fc: split `tests/daemon.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at `d5170e7` (spec 3fb accepted) or later.
Shared task `context_id`: `a2amx-codescene-refactor-batch3-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/daemon.rs`: deleted (its content moves, see section 2).
- `tests/daemon/main.rs` and `tests/daemon/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test daemon`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/daemon.rs` is 2829 lines with 47 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fb applied one method to seven test files (one directory test target, byte-for-byte moves, mechanical equivalence checks, accepted with 0 BLOCKED cycles); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, and the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`).

Differences from 3fb: lines 1-2 are an ordinary comment followed by an inner `#![allow(clippy::unwrap_used, clippy::expect_used)]`; both stay at the top of `main.rs` (the inner attribute covers the whole test binary, modules included, so no module carries it). Eleven indented `use` lines sit inside tests (lines 20, 21, 22, 217, 878, 916, 1305, 1355, 2319, 2574, 2692), so the normalization of acceptance 2 keeps them on both sides. The file has no `env!` use, no `#[ignore]` and no `#[cfg]`. The helpers used by several modules stay in `main.rs`: `create`, `list`, `eventually_sessions`, `text`, `next`, `wait_text` and `raw_response`; the helpers used by one module move there (`confine_new_session_request` and `confined_reset_request` to `confine`, `create_claude` and `create_claude_team` to `status_reporting`).

Alternatives rejected, as in 3ev: several `tests/daemon_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step). A first cut with one module for session creation and status together was 1,076 lines and was split in two.

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/daemon.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2829 is in exactly one of them (the architect checked this with a script that also lists helper use sites). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed. Two seams have no blank line between items in the old file (lines 80/81 and 754/755), so a range that ends there is followed directly by the next range's first line: a missing blank line at a seam stays missing (adding one is allowed; acceptance 2 ignores blank lines and `rustfmt` adds none between items).

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-17, 81-108, 732-754, 1151-1184, then the 6 `mod` lines in alphabetical order (`attach_streams`, `confine`, `daemon_lifecycle`, `serve_protocol`, `session_creation`, `status_reporting`) | the preamble (comment, inner `#![allow]`, `mod common;`, imports) and the helpers used by several modules: `create`, `list`, `eventually_sessions`, `text`, `next`, `wait_text`, `raw_response` | ~110 |
| `confine` | 109-351 | 2 tests, `nested_confine_is_rejected_without_closing_daemon_connection`, `session_tokens_cannot_confine_and_admin_confine_rejects_invalid_targets`; helpers `confine_new_session_request`, `confined_reset_request` (used only here) | ~245 |
| `session_creation` | 466-675, 1629-1694, 2060-2091, 2134-2191 | 4 tests, `new_session_validation_messages_and_precedence`, `failed_spawns_and_zero_sizes_leave_no_sessions`, `ids_are_numerically_ordered_and_not_reused_after_kill`, `cwd_environment_and_argument_boundaries_reach_the_child` | ~368 |
| `status_reporting` | 18-64, 676-731, 755-1150, 2572-2829 | 12 tests, `activity_age_preserves_same_state_and_resets_on_transition` .. `team_attachment_status_lists_sorted_members_and_exited_state`; helpers `create_claude`, `create_claude_team` (used only here) | ~759 |
| `serve_protocol` | 1185-1466, 1534-1628, 2192-2267 | 10 tests, `serve_authentication_errors_preserve_connection_lifecycle` .. `failed_attach_leaves_the_control_connection_usable` | ~455 |
| `attach_streams` | 352-465, 1695-1925, 1961-1990, 2268-2316, 2397-2571 | 11 tests, `attach_top_rows_acknowledges_and_resets_origin` .. `takeover_waits_for_received_input_to_enqueue_under_backpressure` | ~601 |
| `daemon_lifecycle` | 65-80, 1467-1533, 1926-1960, 1991-2059, 2092-2133, 2317-2396 | 8 tests, `shutdown_wire_defaults_graceful_and_omits_false_now` .. `newly_created_parents_are_private_even_with_a_restrictive_umask` | ~311 |

Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least `Client`, `Request` or `Response` from the preamble).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments, `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/daemon.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fc` first; delete it before reporting. Before editing, save `cargo test --test daemon -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fc/before.txt` (48 lines expected). After, the same command with `sed -E 's/^(attach_streams|confine|daemon_lifecycle|serve_protocol|session_creation|status_reporting)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/daemon.rs`, `new` = `cat tests/daemon/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path` (an indented `use` inside a function is kept on both sides, so losing one fails the check), then `sort`; `diff` exit 0. For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The eleven indented `use` lines inside tests are untouched.
3. `wc -l tests/daemon/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/daemon.rs` under `target/scratch/3fc/` before editing and run `cs check` on that copy); every new score at least the old one.
4. Before editing, run `cargo test --test daemon` once and record its passed/ignored counts (48 passed, 0 ignored expected on a quiet machine; it needs no other test run in parallel). After the split `cargo test --test daemon` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/daemon.rs`, the new `tests/daemon/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: F37, which named `exited_claude_session_never_reports_needs_input`, is closed; F30 is a test-hardening idea, not a flake). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 47 tests; eleven indented `use` lines at 20, 21, 22, 217, 878, 916, 1305, 1355, 2319, 2574, 2692; no `env!`, `#[ignore]` or `#[cfg]`; lines 1-2 are a comment and `#![allow(clippy::unwrap_used, clippy::expect_used)]` | `rg -c '^#\[(tokio::)?test' tests/daemon.rs`, `rg -n '^\s+use '`, `rg -n 'env!'`, `rg -n '#\[ignore\|#\[cfg'` | grep |
| Helper use sites by module and the partition of all 2829 lines | `plan.py` run by the architect over the file (every line covered once; `create`, `list`, `eventually_sessions`, `text`, `next`, `wait_text`, `raw_response` each used by at least two modules; `confine_new_session_request` and `confined_reset_request` only by `confine`; `create_claude` and `create_claude_team` only by `status_reporting`) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | name counts over each module's ranges (`Client`, `Request`, `Response`, `ServerFrame` or `Daemon`) | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by each of the 47 test names: F37 (Closed section) and F30 (idea) only | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths (52 uses) resolve through `super::*` | specs 3ev to 3fb accepted (latest d5170e7) | built |
| Baseline test list | `cargo test --test daemon -- --list` on HEAD d5170e7, 48 lines (47 tests plus the `common::fake_discord` line) | run |
