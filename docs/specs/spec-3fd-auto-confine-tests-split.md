# Spec 3fd: split `tests/auto_confine.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at `0559046` (spec 3fc accepted and pushed) or later.
Shared task `context_id`: `a2amx-codescene-refactor-batch4-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/auto_confine.rs`: deleted (its content moves, see section 2).
- `tests/auto_confine/main.rs` and `tests/auto_confine/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test auto_confine`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/auto_confine.rs` is 2057 lines with 32 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fc applied one method to eight test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), and a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table).

Differences from 3fc: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 2 an inner doc comment (`//!`), line 4 `mod common;`; all of lines 1-14 stay at the top of `main.rs` (the inner attribute covers the whole test binary, modules included, so no module carries it). The file has no indented `use` line inside a test, no `#[ignore]` and no `#[cfg]`; two helpers call `env!("CARGO_BIN_EXE_a2amx")` (lines 22 and 39), which is resolved at compile time and works in any module file (`tests/attach_cli/main.rs:45` and `tests/common/pty.rs:56` already use it). Six tests return early with a "bubblewrap unavailable" message when `bwrap` is missing; a skipped test counts as passed, so the pass count does not tell whether bwrap ran. The helpers used by several modules stay in `main.rs`: `run_binary` (all five modules) and `global_auto_file` (four modules); the helpers used by one module move there (`run_binary_without_home` and `write_worktree_git_file` to `git_detection`).

Alternatives rejected, as in 3ev: several `tests/auto_confine_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (the two `Large Method` warnings are a later step: `real_up_reuses_auto_grants_and_running_session_is_not_rescanned` is 263 lines, `local_auto_and_file_auto_work_before_daemon_use` 108).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/auto_confine.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2057 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `git_detection` (1489-1592) and `git_config_limits` (1593-1661) ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Five seams have no blank line between items in the old file (lines 56/57, 323/324, 559/560, 777/778, 1402/1403), so a range that ends there is followed directly by the next range's first line (in `main.rs`, a range ending at 56 is followed directly by the `mod` lines): a missing blank line at a seam stays missing (adding one is allowed; acceptance 2 ignores blank lines and `rustfmt` adds none between items).

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-14, 15-32, 50-56, then the 5 `mod` lines in alphabetical order (`consent_prompts`, `daemon_use`, `dry_run_reports`, `git_config_limits`, `git_detection`) | the preamble (inner attribute and doc comment, `mod common;`, imports) and the helpers used by several modules: `run_binary`, `global_auto_file` | ~45 |
| `git_detection` | 33-49, 57-282, 1185-1218, 1489-1592 | 8 tests, `global_auto_follows_worktree_git_file_for_ssh` .. `global_auto_follows_worktree_git_file_for_https_gh`, `suggest_does_not_probe_cwd_for_host_paths_when_home_is_unset`, `suggest_symlinked_home_repository_is_not_detected`, `suggest_follows_relative_gitdir_from_parent_repository`, `suggest_canonicalizes_relative_directory_before_git_walk`; helpers `run_binary_without_home`, `write_worktree_git_file` (used only here) | ~383 |
| `git_config_limits` | 1219-1488, 1593-1661 | 8 tests, `git_config_size_limit_accepts_65536_and_rejects_65537_bytes`, the six `global_auto_rejects_*` tests, `global_auto_ignores_worktree_git_files_and_oversized_configs` | ~341 |
| `dry_run_reports` | 283-559, 969-1068 | 7 tests, `global_auto_dry_run_reports_detected_grants_once` .. `symlinked_accepted_file_is_read_and_trailing_slash_key_does_not_match`, `auto_merge_preserves_explicit_grant_and_reports_added_reasons`, `explicit_container_grant_reports_escape_notice_in_dry_run` | ~379 |
| `consent_prompts` | 560-968 | 5 tests, `refusal_blocks_auto_start_and_yes_is_one_run_only`, the four `pty_*` tests | ~411 |
| `daemon_use` | 1069-1184, 1662-2057 | 4 tests, `auto_mode_stays_true_when_auto_adds_nothing`, `local_auto_and_file_auto_work_before_daemon_use`, `global_without_auto_has_no_automatic_grants`, `real_up_reuses_auto_grants_and_running_session_is_not_rescanned` | ~514 |

The 32 tests in the five modules are exactly the 32 test functions of the old file (8 + 8 + 7 + 5 + 4). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble import: `AsyncCommand`, `Duration`, `PtyHarness`, `start_daemon`, `Command` or `Output`).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/auto_confine.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fd` first; delete it before reporting. Before editing, save `cargo test --test auto_confine -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fd/before.txt` (33 lines expected). After, the same command with `sed -E 's/^(consent_prompts|daemon_use|dry_run_reports|git_config_limits|git_detection)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/auto_confine.rs`, `new` = `cat tests/auto_confine/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/auto_confine/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/auto_confine.rs` under `target/scratch/3fd/` before editing and run `cs check` on that copy); every new score at least the old one.
5. Before editing, run `cargo test --test auto_confine` once and record its passed/ignored counts (33 passed, 0 ignored on 0559046; it needs no other test run in parallel). After the split `cargo test --test auto_confine` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/auto_confine.rs`, the new `tests/auto_confine/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/auto_confine.rs` prints nothing before editing, and `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/auto_confine` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: F75, which named four PTY tests of this file, is closed by spec 3cl; F94 lists follow-ups, not flakes). A failure is rerun once; a second failure is reported. `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error` (backlog I27) is another binary and not part of this spec.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 32 tests (`#[test]` 5 and `#[tokio::test]` 27 by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`; `env!` only at lines 22 and 39 | `rg -c '^\s*#\[(tokio::)?test' tests/auto_confine.rs` (32), `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use '` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 2057 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `run_binary` used by all five modules, `global_auto_file` by four, `run_binary_without_home` only by `suggest_does_not_probe_cwd_for_host_paths_when_home_is_unset`, `write_worktree_git_file` only by the two `global_auto_follows_worktree_git_file_*` tests) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names: F75 (Closed section) and F94 (follow-ups) only | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, `env!("CARGO_BIN_EXE_a2amx")` resolves in module files, and `common::` paths resolve through `super::*` | specs 3ev to 3fc accepted (latest 402c8a5); `rg -n 'CARGO_BIN_EXE' tests/*/*.rs` | built |
| Baseline | on 0559046: `cargo test --test auto_confine -- --list` 33 `: test` lines (32 tests plus the `common::fake_discord` line); `cargo test --test auto_confine` 33 passed, 0 ignored, 4.58 s | run |
| No script, workflow or doc names `tests/auto_confine.rs` or `--test auto_confine` | `rg -n 'auto_confine' --glob '!target' --glob '!docs/specs/*'` hits are `src/` method names, `tests/status.rs:198` (a test name) and the backlog rows only | grep |
