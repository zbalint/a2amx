# Spec 3ff: split `tests/quota.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `78ed06d`, spec 3fe accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch5-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/quota.rs`: deleted (its content moves, see section 2).
- `tests/quota/main.rs` and `tests/quota/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test quota`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/quota.rs` is 1564 lines with 48 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fe applied one method to ten test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names.

Differences from 3fe: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-17 (attribute, `mod common;` and the imports) stay at the top of `main.rs`, and the inner attribute covers the whole test binary, so no module carries it. Two helpers are used by two modules (`summary` by `daemon_limits` and `omp_usage_daemon`, and by `quota_of`; `quota_of` by both modules) and stay in `main.rs`; every other helper is used by one module and moves there (`screen_with`, `read_with`, `read` and `info` to `screen_reading`; the four script constants, `status_line_session` and `status_line_session_sized` to `daemon_limits`; the two `OMP_USAGE_*` fixtures, `fake_executable`, `bridged_fake_executable`, `bridged_omp_session` and `direct_omp_session` to `omp_usage_daemon`; `claude_rate_limits_payload` to `claude_rate_limits`). The file has no indented `use` line, no `#[ignore]`, no `#[cfg]` and no `env!`. Four consecutive items are one-line or two-line constants with no blank line between them (lines 216-221); they stay together in `daemon_limits`.

Alternatives rejected, as in 3ev: several `tests/quota_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/quota.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1564 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `claude_rate_limits`, `daemon_limits`, `omp_usage_daemon`, `omp_usage_parsing`, `quota_info` and `screen_reading` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. No range boundary falls on a seam without a blank line (the only such seams in the old file, 216/217, 217/218, 219/220 and 1120/1121, lie inside one range).

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-17, 266-286, then the 7 `mod` lines in alphabetical order (`agy_status`, `claude_rate_limits`, `daemon_limits`, `omp_usage_daemon`, `omp_usage_parsing`, `quota_info`, `screen_reading`) | the preamble (inner attribute, `mod common;`, imports) and the helpers used by several modules: `summary`, `quota_of` | ~48 |
| `screen_reading` | 18-174 | 16 tests, `reads_omp_usage_limit_error` .. `short_and_empty_screens_are_handled` (`quota::read` over a drawn screen); helpers `screen_with`, `read_with`, `read`, `info` (used only here) | ~159 |
| `quota_info` | 175-215, 647-665 | 2 tests, `exhausted_names_the_windows_at_zero`, `cell_preserves_screen_limit_with_windows` | ~62 |
| `daemon_limits` | 216-265, 428-646 | 5 tests, `list_and_list_agents_report_a_claude_session_quota`, `list_and_list_agents_report_an_omp_limit`, `generic_sessions_report_no_quota`, `send_message_reports_an_exhausted_recipient_and_delivery_is_unchanged`, `send_message_reports_omp_limit_without_changing_delivery`; helpers `EXHAUSTED_SCRIPT`, `HEALTHY_SCRIPT`, `OMP_EXHAUSTED_SCRIPT`, `OMP_HEALTHY_SCRIPT`, `status_line_session`, `status_line_session_sized` (used only here) | ~271 |
| `omp_usage_parsing` | 666-840 | 7 tests, `parses_omp_usage_windows` .. `omp_usage_ignores_identity_fields` (`quota::parse_omp_usage`) | ~177 |
| `omp_usage_daemon` | 287-427, 841-1147 | 7 tests, `list_and_list_agents_report_omp_usage_percentages` .. `omp_wrapper_named_bun_is_not_polled`; helpers `OMP_USAGE_FIXTURE`, `OMP_USAGE_MULTI_FIXTURE`, `fake_executable`, `bridged_fake_executable`, `bridged_omp_session`, `direct_omp_session` (used only here) | ~450 |
| `claude_rate_limits` | 1148-1288 | 3 tests, `claude_rate_limits_convert_used_percentage_to_independent_windows` .. `claude_rate_limits_windows_are_independent_and_empty_is_none` (`quota::from_claude_rate_limits`); helper `claude_rate_limits_payload` (used only here) | ~143 |
| `agy_status` | 1289-1564 | 8 tests, `agy_status_selects_model_group_windows` .. `agy_status_rejects_invalid_buckets_and_rounds_remaining_fraction` (`quota::from_agy_status`) | ~278 |

The 48 tests in the seven modules are exactly the 48 test functions of the old file (16 + 2 + 5 + 7 + 7 + 3 + 8). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table (the word `read` appears in a string inside `quota_of`; `read` the function is not used by `main.rs`). If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name: `quota`, `QuotaInfo`, `Client`, `Emulator`, `BTreeMap` or `Harness`).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments (including the `///` line above `quota_of`) and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/quota.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3ff` first; delete it before reporting. Before editing, save `cargo test --test quota -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ff/before.txt` (49 lines expected). After, the same command with `sed -E 's/^(agy_status|claude_rate_limits|daemon_limits|omp_usage_daemon|omp_usage_parsing|quota_info|screen_reading)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/quota.rs`, `new` = `cat tests/quota/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/quota/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/quota.rs` under `target/scratch/3ff/` before editing and run `cs check` on that copy); every new score at least the old one.
5. Before editing, run `cargo test --test quota` once and record its passed/ignored counts (49 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test quota` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/quota.rs`, the new `tests/quota/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/quota.rs` prints nothing before editing, and the same on `tests/quota` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names it; the hits for `quota.rs` are `src/quota.rs` in F80 and the F94 list, and the word `read` in two rows is not a test reference). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 48 tests (`#[test]` 36 and `#[tokio::test` 12 by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `env!`, `CARGO_MANIFEST_DIR`, `include_*` | `rg -c '^#\[test\]'`, `rg -c '^#\[tokio::test'`, `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1564 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `summary` used by `daemon_limits`, `omp_usage_daemon` and `quota_of`; `quota_of` by both modules; every other helper by one module) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path (`tests/quota`) and by each of the 48 test names: no row names this file or a test of it (F80 names `src/quota.rs`; `read` is an ordinary word elsewhere) | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths (16 uses) resolve through `super::*` | specs 3ev to 3fe accepted (latest 78ed06d) | built |
| Baseline | on 78ed06d: `cargo test --test quota -- --list` 49 `: test` lines (48 tests plus the `common::fake_discord` line); `cargo test --test quota` 49 passed, 0 ignored, 29.6 s | run |
| No script, workflow or doc names `tests/quota.rs` or `--test quota` | `rg -n 'tests/quota\|--test quota\|quota\.rs' --glob '!target' --glob '!docs/specs/*'` hits are `src/quota.rs` in backlog F80 and the F94 row (its list of files still over 1,000 lines) only | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review and design read, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | no problem that would cause a BLOCKED; partition, helper use sites, counts and groupings all confirmed | none |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; the old file's `cs check` score read 5.47 on its copy against 5.63 reported by the developer (unexplained, every new score is at least either) | none |
| Design read | consultant | `daemon_limits` does not say its quota comes from the screen, next to `omp_usage_daemon` which covers the polled `omp usage` source (optional naming nit) | module names are locked; left in backlog F94 |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all eight files, normalized diff empty, fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
