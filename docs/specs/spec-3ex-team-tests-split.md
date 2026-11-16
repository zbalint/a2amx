# Spec 3ex: split `tests/team.rs` into themed modules of one test binary (refactor, third file)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts spec 3ew or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/team.rs`: deleted (its content moves, see section 2).
- `tests/team/main.rs` and `tests/team/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`,
`CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test team`. No test is added, removed or changed; the equivalence checks in
section 5 replace a mutation check.

## 1. Why

`tests/team.rs` is 3,838 lines (CodeScene 4.25, 37 findings) with 117 tests and 8 helpers. The owner's goal is no `.rs` file over
1,000 lines, tests included. Specs 3ev and 3ew applied one method to `tests/confine.rs` and `tests/broker.rs` (one directory test target,
byte-for-byte moves, two mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: the preamble stays whole in
`main.rs`; a helper used by more than one module stays in `main.rs`; the test list of a binary with `mod common;` has one extra line,
`common::fake_discord::tests::push_message_survives_gateway_reconnect`, so 118 lines.

Differences from 3ev: the file starts with an inner doc comment `//!` (line 1), which stays as line 1 of `main.rs`; there is no
`#![allow]`; one indented `use team::{ResetState, ResetTarget};` sits inside the test at line 567-568, so the normalization of
acceptance 2 deletes it on both sides and it stays untouched inside its function.

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/team.rs` (1-based, both ends included),
preceded by `use super::*;` and a blank line in every module and followed, in `main.rs`, by the `mod` lines. The ranges partition the file exactly:
every line 1-3838 is in exactly one of them (the architect checked this with a script). Ranges start on a test attribute or a helper's
`fn`/`const` line and end on the line before the next one's start; the carve-outs for helpers are explicit because "up to the next test attribute"
would swallow a helper that sits between tests. No visibility changes: no `pub(crate)`, no other `use` line.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-16, 916-925, 2271-2288, then the 8 `mod` lines | the `//!` comment, `mod common;` (with `#[path = "../common/mod.rs"]` added above it), imports, and the two helpers used by several modules: `run_binary` (used in `plan_and_status`, `dry_run`, `lifecycle`, `reset_and_validation`) and `run_binary_in_with_home_and_runtime` (used in `parse_and_guards` and `dry_run`) | ~55 |
| `parse_and_guards` | 17-540 | `global_team_name_is_normalized_valid_and_bounded` .. `parse_characterizes_session_allow_without_team` | ~525 |
| `plan_and_status` | 541-565, 566-624, 1567-2025 | `summary` (used only here), `reset_targets_preserve_order_and_protect_attached_sessions`, the `plan_*`, `inspect_*` and `status_*` tests, `start_fake_claude` | ~545 |
| `team_file_lookup` | 625-899, 900-915, 926-935 | the `team_file_*`, `global_team_*` and `explicit_team_file_*` tests, then `run_binary_in_with_home` and `run_binary_in` (used only here) | ~300 |
| `lifecycle` | 936-944, 2424-2890 | `LIVE_TEAM` (used only here), then `status_notes_messages_lost_in_a_daemon_restart_only_for_wanted_names` .. `terminal_attach_honors_file_selection_flag_first_and_detach_override` | ~475 |
| `parse_sessions` | 945-1566 | `parse_preserves_commands_cwd_order_and_attach_selection` .. `confined_claude_prompt_file_after_separator_is_ignored` | ~620 |
| `dry_run` | 2026-2270, 2289-2423, 3175-3264 | the `dry_run_*` tests; 3175-3264 is the named move of `dry_run_prints_exact_all_optional_details_in_order`, `dry_run_prints_working_only_activity_detail` and `dry_run_prints_idle_only_activity_detail` | ~470 |
| `reset_and_validation` | 2891-3174 | `configured_reset_and_controller_are_accepted_by_team_parser` .. `parse_characterizes_check_precedence`, with `assert_validation_error` (used only here) | ~285 |
| `team_config` | 3265-3838 | `heartbeat_team_entry_requires_watch_and_validates_interval` .. `confine_table_covers_disabled_and_merged_limit_rules` | ~575 |

Module names are fixed; test and helper names do not change. A helper moved into a module keeps its body and sits where its range puts it
(after the module's first range in the cases above). The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table.

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs`
  and the one `use super::*;` line in each module.
- The `CARGO_BIN_EXE_a2amx` and `CARGO_MANIFEST_DIR` uses do not change (neither depends on the file's location; `tests/fixtures/fake_claude.sh` is read through `CARGO_MANIFEST_DIR`).
- `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged,
  split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; correcting misplaced tests beyond the named move
(they go to the follow-up row F94 in `docs/backlog.md`).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Before editing, save `cargo test --test team -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ex/before.txt`
   (118 lines expected). After, the same command with `sed -E 's/^(parse_and_guards|team_file_lookup|parse_sessions|plan_and_status|dry_run|lifecycle|reset_and_validation|team_config)::(.*)$/\2/'`
   applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (create `target/scratch/3ex` first, delete it before reporting) (`diff` exit 0). Judge: `diff`; the architect and the tester rerun it.
2. **Same code.** `old` = `git show HEAD:tests/team.rs`, `new` = `cat tests/team/*.rs`. Delete blank lines and lines starting (after indentation) with
   `mod `, `use ` or `#[path`, then `sort`; `diff` exit 0.
3. `wc -l tests/team/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores; every score at least the old 4.25.
4. `cargo test --test team` has the same passed/ignored counts as the baseline recorded before editing; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/team.rs`, the new `tests/team/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none recorded for this file. A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 117 tests (`rg -c '^#\[(tokio::)?test' tests/team.rs`); one indented `use` inside a function (line 568); `env!` uses are `CARGO_BIN_EXE_a2amx` and `CARGO_MANIFEST_DIR` (1874) | `rg -c`, `rg -n '^\s+use '`, `rg -n 'env!'` | grep |
| Helper use sites by module: `summary` 571-577 and 1571-1685 (`plan_and_status`); `run_binary_in` 636-845 and `run_binary_in_with_home` 781-888 (`team_file_lookup`); `LIVE_TEAM` 2484-2582 (`lifecycle`); `run_binary` 1744-3248 (four modules); `run_binary_in_with_home_and_runtime` 187 and 2118-2256 (two modules); `start_fake_claude` 1914; `assert_validation_error` 3033-3171 | `rg -n` per helper | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works | spec 3ev accepted (aad8a9a): same shape built and run | built |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0. No consultant design read of the diff (the method was reviewed on spec 3ev; consultant quota was low).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | four of the six helpers the draft kept in `main.rs` have one user module each (`summary`, `run_binary_in`, `run_binary_in_with_home`, `LIVE_TEAM`) | moved into their module in the locked table |
| Draft review | consultant | the rule "up to the next test attribute" would have swallowed helpers that sit between tests, and `LIVE_TEAM` was off by one | the table gives explicit line ranges that partition the file; the architect checked the partition with a script |
| Draft review | consultant | no scratch directory step in acceptance 1; a status test in `lifecycle` and a reset test in `plan_and_status` sit in another theme | scratch step added; the two tests left for backlog F94 |
| Tester pass | tester | section 2 says each file is the concatenation of its ranges, but seven modules lack the one trailing blank line of their last range, because `rustfmt` removes it (`cargo fmt --check` is part of acceptance) | accepted; no code effect, acceptance 2 ignores blank lines; wording only |
| Implementation | developer | no problems found; acceptance 1-5 pass, scores 7.76 to 10.00 | none |
| Architect review | architect | no problems found; rebuilt each file from the spec's ranges and compared it in order after dropping blank, `mod`, `use` and `#[path` lines (all nine identical); fmt, clippy, `cargo test --test team` (118 passed) | none |
