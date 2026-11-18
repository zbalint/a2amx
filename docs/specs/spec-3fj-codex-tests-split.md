# Spec 3fj: split `tests/codex.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `ee94298`, spec 3fi accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch6-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `tests/codex.rs`: deleted (its content moves, see section 2).
- `tests/codex/main.rs` and `tests/codex/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test codex`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/codex.rs` is 1224 lines with 20 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fg applied one method to twelve test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has one use of `current_exe`, which is harmless, see section 5 item 8 and the claims table), and a module is judged by what its tests run and assert, not by their names (every test below was read for the types it builds and asserts on, not only its name).

Differences from 3fg: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-20 (attribute, `mod common;`, the imports and `BODY`) stay at the top of `main.rs`, followed by lines 23-275: the `FAKE_CODEX` script constant and its comment, the `Case` fixture used by two modules, and the helpers `complete` and `idle`. Only `ENVELOPE_TEXT` (lines 21-22), used only by two delivery tests, and the helper `waiting_on_approval` (873-876), used by one delivery test, move to a module (`delivery`). The `confined_socket` module needs none of `Case`; it builds its own daemon and reads `FAKE_CODEX`, `BODY`, `complete` and `idle` from `main.rs`. Private items of `main.rs` (struct `Case` with its private fields and methods, the constants and helpers) are visible to child modules, so no visibility change is needed. The file has no indented `use` line, no `#[ignore]`, no `#[cfg]` and no `module_path!`. It has two name-dependence grep hits: `std::env::current_exe()` at line 880, which `codex_new_starts_a_private_app_server_and_attaches_the_tui_to_it` compares with the argument the daemon (running in the same test process) passed to the fake codex, so it holds the path of the test binary on both sides and does not depend on a test's name or module path; and `env!("CARGO_BIN_EXE_a2amx")` at line 985, a compile-time path that is the same in a directory target (as in specs 3fa and 3fb). All 20 tests are `#[tokio::test]`.

Alternatives rejected, as in 3ev: several `tests/codex_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/codex.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1224 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `confined_socket`, `delivery` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Every range starts at an item start and ends at an item end.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-20, 23-275, then the 3 `mod` lines in alphabetical order (`app_server_launch`, `confined_socket`, `delivery`) | the preamble (inner attribute, `mod common;`, imports except `ENVELOPE_TEXT`), the constants `BODY` and `FAKE_CODEX`, the `Case` fixture (`struct Case` and its `impl`) and the helpers `complete` and `idle`, which two modules each use | ~277 |
| `app_server_launch` | 877-1028, 1155-1178, 1192-1224 | 7 tests, `codex_new_starts_a_private_app_server_and_attaches_the_tui_to_it`, `codex_role_reaches_only_the_operator_instructions`, `inherited_role_is_removed_by_the_cli_before_codex_launch`, `a_socket_path_beyond_sun_len_is_reached_through_the_link_the_server_makes`, `no_authorize_peers_leaves_the_operator_line_out`, `killing_the_session_stops_its_app_server`, `app_server_and_tui_start_in_the_requested_directory` (how `codex new` starts and stops the private app server and the TUI: arguments, role, environment, directory, socket path, kill) | ~211 |
| `confined_socket` | 276-872 | 5 tests, `confined_codex_uses_private_socket_dir_and_delivers` .. `confined_codex_guest_cannot_replace_socket_directory` (the confined Codex session's private socket directory and its hostile-link and substitution refusals; each builds its own confined daemon) | ~599 |
| `delivery` | 21-22, 873-876, 1029-1154, 1179-1191 | 8 tests, `a_message_reaches_the_codex_thread_and_the_thread_item_becomes_the_receipt`, `without_a_loaded_thread_the_message_waits`, `an_open_approval_dialog_holds_delivery_until_it_closes`, `the_most_recently_updated_thread_receives_the_message`, `a_second_message_waits_until_the_first_shows_up_in_the_thread`, `a_thread_item_with_different_text_is_no_receipt`, `a_rejected_turn_makes_the_message_undeliverable`, `a_dead_app_server_makes_the_message_wait` (delivery of a message to the Codex thread through the fake app server); constant `ENVELOPE_TEXT` and helper `waiting_on_approval` (used only here) | ~147 |

The 20 tests in the 3 modules are exactly the 20 test functions of the old file (7 + 5 + 8). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/codex.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fj` first; delete it before reporting. Before editing, save `cargo test --test codex -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fj/before.txt` (21 lines expected). After, the same command with `sed -E 's/^(app_server_launch|confined_socket|delivery)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/codex.rs`, `new` = `cat tests/codex/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/codex/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/codex.rs` under `target/scratch/3fj/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which may report N/A if it holds no function.
5. Before editing, run `cargo test --test codex` once and record its passed/ignored counts (21 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test codex` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/codex.rs`, the new `tests/codex/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/codex.rs` prints exactly one line before editing (`std::env::current_exe()` at line 880, see section 1), and the same on `tests/codex` prints one hit after, in `tests/codex/app_server_launch.rs` (the line number changes); any other hit is reported. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names it as a test file; the hits for `codex.rs` are `src/codex.rs` in the architecture doc (a path, not a test) and the F92 and F94 rows (F92 is about dangling `s<N>.sock` symlinks left in `/tmp`, not about these tests)). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 20 tests (`#[tokio::test` 20 and no plain `#[test]` by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `CARGO_MANIFEST_DIR`, `include_*`; `current_exe` once (line 880) and `env!` once (line 985); 41 `common::` uses | `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1224 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `Case` (struct, impl and its 12 methods) used by the tests of `app_server_launch` and `delivery` only; `complete` by `confined_socket` (two tests) and `app_server_launch` (one test); `idle` by `confined_socket` (one test) and `delivery`; `BODY` by `Case::send` and one confined test; `FAKE_CODEX` by `Case::launch`, four confined tests and `inherited_role_is_removed_by_the_cli_before_codex_launch`; `ENVELOPE_TEXT` by two delivery tests only; `waiting_on_approval` by one delivery test only) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths resolve through `super::*` | specs 3ev to 3fg accepted (latest bfeeb5b) | built |
| Baseline | on ee94298: `cargo test --test codex -- --list` 21 `: test` lines (20 tests plus the `common::fake_discord` line); `cargo test --test codex` 21 passed, 0 ignored | run |
| No script, workflow or doc names `tests/codex.rs` or `--test codex` | `rg -n 'tests/codex\|--test codex\|codex\.rs' --glob '!target' --glob '!docs/specs/*'` hits are `src/codex.rs` in `docs/architecture.md`, the F92 row (dangling `s<N>.sock` links) and the F94 row; none names the test file | grep |

## Amendment 1 (architect, after the developer report)

Acceptance 4 says every new score is at least the old file's. The developer found `tests/codex/confined_socket.rs` at 6.58 against the old file's 6.63. The spec's claim was a prediction, not a requirement of the split: the old file's score averages the confined tests with 15 short ones, and `confined_socket.rs` collects the five longest bodies (a 239-line test and three of 80 to 128 lines, all flagged Large Method, one Complex Method), plus a Low Cohesion note, none of which a byte-for-byte move can change. Shortening the tests is out of scope (section 4); moving them again would only chase the number. Decision: acceptance 4 for this spec reads "every new file under 1,000 lines; every new score at least the old one, except `confined_socket.rs`, which may score down to 6.50 and whose warnings are reported". The tester reports the scores; `main.rs`, `app_server_launch.rs` and `delivery.rs` must be at least the old score.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed). No consultant design read of the diff (byte-exact move).
BLOCKED/amend cycles: 0 (one architect amendment after the developer report, below).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | acceptance 8 said the `current_exe` hit stays on "exactly that one line" although the line number changes | reworded before locking |
| Implementation | developer | acceptance 4 ("every new score at least the old") was false for `confined_socket.rs`, 6.58 against 6.63: the module holds the five longest tests (Large Method x4, Complex Method, Low Cohesion); a byte move cannot change that. The architect's draft predicted the scores without scoring the modules | Amendment 1: floor 6.50 for that file, others at least the old score; shortening the tests stays out of scope |
| Tester pass | tester | `main.rs` has one blank line before the `mod` block that the section 2 rebuild text did not state; first reported, then reclassified as the layout of every earlier split main | none, whitespace only |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all four files (modulo the separator blank), fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
