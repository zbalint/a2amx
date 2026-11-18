# Spec 3fi: split `tests/teams.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `120ff4c`, spec 3fh accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch6-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `tests/teams.rs`: deleted (its content moves, see section 2).
- `tests/teams/main.rs` and `tests/teams/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test teams`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/teams.rs` is 1253 lines with 15 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fg applied one method to twelve test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names (every test below was read for the types it builds and asserts on, not only its name).

Differences from 3fg: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-13 (attribute, `mod common;` and the imports) stay at the top of `main.rs`, and the inner attribute covers the whole test binary, so no module carries it. Three helpers are used by two modules and stay in `main.rs` (`scope`, `scope_with_agents`, `session`); every other helper is used by one module and moves there (`team_file`, `team_up_pty_args`, `team_up_pty`, `assert_waiting_before_enter`, `run_binary`, `gateway` to `team_up_remote`; `exit_session` to `team_visibility`; `new_session_request` to `team_validation`). The file has no indented `use` line, no `#[ignore]` and no `#[cfg]`; it has one `env!("CARGO_BIN_EXE_a2amx")` use (line 87, in `run_binary`), a compile-time path that is the same in a directory target. All 15 tests are `#[tokio::test]`. `live_team_settings_are_consistent_until_the_first_member_exits` asserts the daemon's `team ... is already running with different settings` error for a second session, so it sits in `team_validation` with the other new-session team checks, not in `team_visibility`.

Alternatives rejected, as in 3ev: several `tests/teams_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/teams.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1253 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `team_up_remote`, `team_visibility` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Every range starts at an item start and ends at an item end.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-13, 73-85, 464-497, then the 3 `mod` lines in alphabetical order (`team_up_remote`, `team_validation`, `team_visibility`) | the preamble (inner attribute, `mod common;`, imports) and the helpers used by several modules: `scope`, `scope_with_agents` (used by `team_visibility` and `team_validation`), `session` (used by `team_visibility` and one test of `team_validation`) | ~64 |
| `team_up_remote` | 14-72, 86-463 | 5 tests, `team_up_validates_remote_before_start_and_carries_it_to_the_session` .. `team_up_remote_detach_does_not_wait_for_enter` (`a2amx team up` with a remote lease: validation before start, the lease output, and the wait before attaching (a real binary and a PTY); helpers `team_file`, `team_up_pty_args`, `team_up_pty`, `assert_waiting_before_enter`, `run_binary`, `gateway` (used only here)) | ~439 |
| `team_validation` | 760-868, 1088-1253 | 4 tests, `live_team_settings_are_consistent_until_the_first_member_exits`, `daemon_validates_agent_allow_without_affecting_team_scope_consistency`, `new_session_team_validation_messages_and_precedence`, `new_session_team_conflict_precedes_in_use_name` (the daemon's new-session team checks: live team settings consistency, agent-allow validation, error messages and precedence; helper `new_session_request` (used only here)) | ~277 |
| `team_visibility` | 498-759, 869-1087 | 6 tests, `private_team_visibility_hides_other_private_teams_but_not_ungrouped` .. `mutual_agent_allow_controls_exit_notifications` (private-team visibility and messaging, mutual allow, the reset check and exit notifications; helper `exit_session` (used only here)) | ~483 |

The 15 tests in the 3 modules are exactly the 15 test functions of the old file (5 + 4 + 6). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/teams.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fi` first; delete it before reporting. Before editing, save `cargo test --test teams -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fi/before.txt` (16 lines expected). After, the same command with `sed -E 's/^(team_up_remote|team_validation|team_visibility)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/teams.rs`, `new` = `cat tests/teams/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/teams/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/teams.rs` under `target/scratch/3fi/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which may report N/A if it holds no function.
5. Before editing, run `cargo test --test teams` once and record its passed/ignored counts (16 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test teams` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/teams.rs`, the new `tests/teams/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/teams.rs` prints nothing before editing, and the same on `tests/teams` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: one historical row names this file: backlog F91 records a hung run of `tests/teams.rs` on 2026-11-12 at a test then called `team_up_remote_lease_output_requires_a_team_and_not_dry_run` (no longer in the file), caused by stale daemons from killed runs, and says the test passes alone; a hang or failure in a `team_up_remote` test is rerun once after checking for stray `a2amx` daemon processes). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 15 tests (`#[tokio::test` 15 and no plain `#[test]` by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `CARGO_MANIFEST_DIR`, `include_*`; `env!` once (line 87); 20 `common::` uses | `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1253 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `scope` by `team_visibility` and `team_validation` (not by `team_up_remote`); `scope_with_agents` by `team_visibility`, `team_validation` and `scope`; `session` by `team_visibility` (six tests) and `daemon_validates_agent_allow_without_affecting_team_scope_consistency` and `live_team_settings_are_consistent_until_the_first_member_exits` (`team_validation`); `exit_session` by two `team_visibility` tests; `new_session_request` by three `team_validation` tests; `team_file`, `team_up_pty_args`, `team_up_pty`, `assert_waiting_before_enter`, `run_binary`, `gateway` only by `team_up_remote`) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths resolve through `super::*` | specs 3ev to 3fg accepted (latest bfeeb5b) | built |
| Baseline | on 120ff4c: `cargo test --test teams -- --list` 16 `: test` lines (15 tests plus the `common::fake_discord` line); `cargo test --test teams` 16 passed, 0 ignored | run |
| No script, workflow or doc names `tests/teams.rs` or `--test teams` | `rg -n 'tests/teams\|--test teams\|teams\.rs' --glob '!target' --glob '!docs/specs/*'` hits are the F91 row (the historical hang, see above) and the F94 row | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed). No consultant design read of the diff (byte-exact move).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | architect (own review of asserted types) | `live_team_settings_are_consistent_until_the_first_member_exits` sat in `team_visibility` by neighbourhood; it asserts the new-session team-settings error | moved to `team_validation` before locking |
| Draft review | consultant | no problems found | none |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; the old file's `cs check` score matched the developer's (6.72) | none |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all four files, fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
