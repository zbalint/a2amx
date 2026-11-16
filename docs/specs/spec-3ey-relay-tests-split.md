# Spec 3ey: split `tests/relay.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts the previous refactor spec or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/relay.rs`: deleted (its content moves, see section 2).
- `tests/relay/main.rs` and `tests/relay/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test relay`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/relay.rs` is 2181 lines (CodeScene 3.82, 23 findings) with 36 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev, 3ew and 3ex applied one method to `tests/confine.rs`, `tests/broker.rs` and `tests/team.rs` (one directory test target, byte-for-byte moves, mechanical equivalence checks, accepted with 0 BLOCKED cycles); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, and the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`).

Differences from 3ex: line 1 is an inner `#![allow(clippy::expect_used, clippy::unwrap_used)]`, which stays line 1 of `main.rs`; the comment above `settled` (lines 22-24) goes with it; one indented `use std::io::Write as _;` sits inside a test (line 1828), so the normalization of acceptance 2 deletes it on both sides.

Alternatives rejected, as in 3ev: several `tests/relay_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/relay.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2181 is in exactly one of them (the architect checked this with a script that also lists helper use sites). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-54, then the 5 `mod` lines | the preamble (inner `#![allow]`, `mod common;`, imports) and the helpers used by several modules: `settled`, `kill_and_wait`, `write_file`, `valid_gateway`, `wait_for_member_checks` | ~60 |
| `config_loading` | 62-70, 475-633, 1923-1955 | 11 tests, `load_does_not_create_missing_home` .. `load_rejects_symlinked_config_and_pairing` | ~201 |
| `pairing` | 55-61, 71-474, 750-884, 1748-1922 | 10 tests, `pairing_existing_record_warns_replaces_and_announces_after_write` .. `gateway_run_pair_binary_prompts_and_writes_record`; helper `announced_pairing_code` (used only here) | ~721 |
| `gateway_token_and_client` | 677-749 | 3 tests, `gateway_token_cli_reads_stdin_and_writes_trimmed_secret` .. `gateway_client_uses_gateway_token_file` | ~73 |
| `sync_and_members` | 634-676, 885-1168, 1370-1494 | 6 tests, `relay_starts_syncs_control_channel_and_stops` .. `unlisted_agent_channel_is_unbound` | ~452 |
| `outbound_and_reconnect` | 1169-1369, 1495-1747, 1956-2181 | 6 tests, `failed_outbound_post_reports_undeliverable` .. `relay_end_to_end_confined_both_and_send_agents` | ~680 |

Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. `announced_pairing_code` (55-61) is used only by `pairing`, so it moves there.

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- The `env!` uses (`CARGO_BIN_EXE_a2amx` at lines 680, 714, 1690, 1767, 1859) do not change; none depends on the file's location.
- Comments, `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3ey` first; delete it before reporting. Before editing, save `cargo test --test relay -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ey/before.txt` (37 lines expected). After, the same command with `sed -E 's/^(config_loading|pairing|gateway_token_and_client|sync_and_members|outbound_and_reconnect)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/relay.rs`, `new` = `cat tests/relay/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path` (an indented `use` inside a function is kept on both sides, so losing one fails the check), then `sort`; `diff` exit 0. For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The one indented `use` (line 1828) is inside a test and is untouched.
3. `wc -l tests/relay/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores; every score at least the old 3.82.
4. `cargo test --test relay` has the same passed/ignored counts as the baseline recorded before editing; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/relay.rs`, the new `tests/relay/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests: none recorded for this file. A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 36 tests; one indented `use` inside a function (line 1828); `env!` uses: `CARGO_BIN_EXE_a2amx` at lines 680, 714, 1690, 1767, 1859 | `rg -c '^#\[(tokio::)?test' tests/relay.rs`, `rg -n '^\s+use '`, `rg -n 'env!'` | grep |
| Helper use sites by module and the partition of all 2181 lines | `plan.py` run by the architect over the file (every line covered once) | script |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works | specs 3ev, 3ew, 3ex accepted (aad8a9a, 81d9a1a, 2cf2d65) | built |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0. No consultant design read of the diff (the method was reviewed on specs 3ev to 3ex; consultant quota was low).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | the normalization of acceptance 2 deleted indented `use` lines too, so losing the in-test `use std::io::Write as _;` (line 1828) would pass | deletes anchored at column 0, in this and the later specs; a guard that `before.txt` is not empty |
| Draft review | consultant | `unlisted_agent_channel_is_unbound` is a binding and sync test, not an outbound or reconnect test | moved to `sync_and_members` |
| Implementation | developer | no problems found; acceptance 1-5 pass, scores 6.19 to 10.00 | none |
| Tester pass | tester | no problems found: 42 items byte-identical and placed as the table says, 37 passed | none |
| Architect review | architect | no problems found; rebuilt each file from the spec's ranges and compared (all six identical), sorted-line diff, fmt, clippy, `cargo test --test relay` (37 passed) | none |
