# Spec 3fl: split `tests/mcp.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `7da4436`, spec 3fk accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch6-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `tests/mcp.rs`: deleted (its content moves, see section 2).
- `tests/mcp/main.rs` and `tests/mcp/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test mcp`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/mcp.rs` is 1132 lines with 21 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fg applied one method to twelve test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names (every test below was read for the types it builds and asserts on, not only its name).

Differences from 3fg: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-15 (attribute, `mod common;` and the imports) stay at the top of `main.rs`, and the inner attribute covers the whole test binary, so no module carries it. The shared fixture is `McpProcess` (lines 16-21, `impl` 68-116, `Drop` 137-143) and its scripted-server helpers (22-55 and 62-67; `write_raw_frame`, 56-61, is used by one `transport_retry` test and moves there), `tool_result` and `failed_tool_result` (144-165); they stay in `main.rs`. The `impl McpProcess` block (68-116) and the `impl Drop for McpProcess` block (137-143) surround one helper, `spawn_inert_mcp` (117-136), which only `mcp_optional_without_environment_is_inert` uses, so it moves to `process_environment` and the two `impl` blocks stay in `main.rs` as separate ranges. `accepted_tool_result` (417-450) is used only by the two tests that follow it and sits inside the range of `tools_and_messages`. Private items of `main.rs` (struct `McpProcess` and its private fields, helper functions) are visible to child modules, so no visibility change is needed. The file has no indented `use` line, no `#[ignore]` and no `#[cfg]`; it has six `env!` uses (lines 70, 118, 202, 830, 896 and 931: five `CARGO_BIN_EXE_a2amx` and one `CARGO_PKG_VERSION`), all compile-time values that do not depend on a test's name or module path. It has 21 tests: 13 `#[test]` and 8 `#[tokio::test]`.

Alternatives rejected, as in 3ev: several `tests/mcp_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/mcp.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1132 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `process_environment`, `tools_and_messages`, `transport_retry` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Every range starts at an item start and ends at an item end.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-55, 62-116, 137-165, then the 4 `mod` lines in alphabetical order (`line_limits`, `process_environment`, `tools_and_messages`, `transport_retry`) | the preamble (inner attribute, `mod common;`, imports) and the helpers used by several modules: `McpProcess` (struct, `impl` and `Drop`), `spawn_scripted_server`, `read_frame`, `read_request`, `write_response`, `expect_hello`, `tool_result`, `failed_tool_result` | ~144 |
| `line_limits` | 569-739, 1089-1132 | 6 tests, `mcp_rejects_oversized_line_and_resumes` .. `mcp_rejects_oversized_send_message_before_daemon_write` (size limits: oversized, limit-sized and unterminated input lines, and the oversized send refused before the daemon write) | ~217 |
| `process_environment` | 117-136, 828-942 | 4 tests, `mcp_missing_or_invalid_environment_exits_without_protocol_output`, `mcp_optional_without_environment_is_inert`, `mcp_optional_partial_or_invalid_environment_stays_strict`, `mcp_reports_a_stdin_read_error_and_exits_nonzero` (the process and its environment: required, optional and partial environment, stdin read error exit; helper `spawn_inert_mcp` (used only here)) | ~137 |
| `tools_and_messages` | 166-568, 740-827 | 7 tests, `mcp_initialize_tools_and_message_calls` .. `mcp_protocol_errors_notifications_and_argument_validation` (the tool surface: initialize, tool list, message calls, reset, the accepted and status results with their optional fields, protocol errors, notifications and argument validation; helper `accepted_tool_result` (used only here)) | ~493 |
| `transport_retry` | 56-61, 943-1088 | 4 tests, `mcp_dead_daemon_is_a_tool_failure_and_stdout_stays_json`, `mcp_retries_read_once_after_transport_io`, `mcp_does_not_retry_read_parse_errors`, `mcp_send_never_retries_after_transport_loss` (what happens when the daemon connection fails: dead daemon, one retry for reads, no retry for parse errors or sends; helper `write_raw_frame` (used only here)) | ~154 |

The 21 tests in the 4 modules are exactly the 21 test functions of the old file (6 + 4 + 7 + 4). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/mcp.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fl` first; delete it before reporting. Before editing, save `cargo test --test mcp -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fl/before.txt` (22 lines expected). After, the same command with `sed -E 's/^(line_limits|process_environment|tools_and_messages|transport_retry)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/mcp.rs`, `new` = `cat tests/mcp/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/mcp/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/mcp.rs` under `target/scratch/3fl/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which may report N/A if it holds no function. A score below the old one is reported with its warnings and not fixed (a byte move cannot change the code health of a test's own body); the architect decides whether it is acceptable.
5. Before editing, run `cargo test --test mcp` once and record its passed/ignored counts (22 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test mcp` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/mcp.rs`, the new `tests/mcp/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/mcp.rs` prints nothing before editing, and the same on `tests/mcp` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names a test of it; I15 mentions `tests/mcp.rs` only as a file that exercises `src/mcp.rs` for coverage (a path reference the architect updates, not a flake)). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 21 tests (`#[test]` 13 and `#[tokio::test` 8 by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `CARGO_MANIFEST_DIR`, `include_*`; `env!` six times (lines 70, 118, 202, 830, 896, 931: five `CARGO_BIN_EXE_a2amx` and one `CARGO_PKG_VERSION`, all compile-time); 17 lines with `common::` | `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1132 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `McpProcess` (struct, `impl`, `Drop`) and the scripted-server helpers `spawn_scripted_server`, `read_frame`, `read_request`, `write_response`, `expect_hello` by tests of two or more modules; `write_raw_frame` by one `transport_retry` test (`mcp_does_not_retry_read_parse_errors`) only; `tool_result` by `tools_and_messages`, `line_limits` and `transport_retry`; `failed_tool_result` by `tools_and_messages` and `transport_retry`; `accepted_tool_result` by two `tools_and_messages` tests; `spawn_inert_mcp` by one `process_environment` test) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths resolve through `super::*` | specs 3ev to 3fg accepted (latest bfeeb5b) | built |
| Baseline | on 7da4436: `cargo test --test mcp -- --list` 22 `: test` lines (21 tests plus the `common::fake_discord` line); `cargo test --test mcp` 22 passed, 0 ignored | run |
| No script, workflow or doc names `tests/mcp.rs` or `--test mcp` | `rg -n 'tests/mcp\|--test mcp\|mcp\.rs' --glob '!target' --glob '!docs/specs/*'` hits are `src/mcp.rs` in `docs/architecture.md`, the I15 row (which names `tests/mcp.rs`; the architect updates that name in the accepting commit) and the F94 row | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed). No consultant design read of the diff (byte-exact move).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | section 1 said the file has one `env!` use (line 118) while the claims table listed six (70, 118, 202, 830, 896, 931) | section 1 corrected before locking |
| Draft review | architect | `write_raw_frame` was kept in `main.rs` although only one `transport_retry` test uses it, against the single-user-helper rule | moved to `transport_retry` before locking |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; the old file's `cs check` score, saved under a path without "test", was 5.82, matching the developer's | none |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all five files, fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
