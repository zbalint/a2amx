# Spec 3fh: split `tests/presence.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `bfeeb5b`, spec 3fg accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch6-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `tests/presence.rs`: deleted (its content moves, see section 2).
- `tests/presence/main.rs` and `tests/presence/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test presence`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/presence.rs` is 1501 lines with 22 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fg applied one method to twelve test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names (every test below was read for the types it builds and asserts on, not only its name).

Differences from 3fg: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-16 (attribute, `mod common;` and the imports) stay at the top of `main.rs`, and the inner attribute covers the whole test binary, so no module carries it. Two helpers are used by more than one module and stay in `main.rs` (`owner_state`, `receive_data`); every other helper is used by one module and moves there (`instant` to `presence_clock`; `remote_status_until` and `status_once` to `remote_status`; `session_held` to `viewers`; `message_infos`, `envelope_attr`, `envelope_body` and `message_subjects` to `away_notice`). The file has no indented `use` line, no `#[ignore]`, no `#[cfg]` and no `env!`. The 22 tests are 7 `#[test]` (the pure `Presence` clock tests) and 15 `#[tokio::test]`.

Alternatives rejected, as in 3ev: several `tests/presence_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/presence.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1501 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `away_notice`, `presence_clock` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Every range starts at an item start and ends at an item end; two range ends (393 and 1432) have no blank line before the next item, and both end a module.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-16, 128-134, 180-193, then the 5 `mod` lines in alphabetical order (`away_notice`, `owner_presence`, `presence_clock`, `remote_status`, `viewers`) | the preamble (inner attribute, `mod common;`, imports) and the helpers used by several modules: `owner_state` (used by `owner_presence` and `away_notice`), `receive_data` (used by `owner_presence`, `viewers` and `away_notice`) | ~43 |
| `away_notice` | 146-179, 394-410, 947-1273 | 3 tests, `away_notice_is_sent_only_to_the_transitioning_session`, `away_notice_is_fixed_and_sent_once_to_remote_sessions_only`, `away_notice_body_matches_the_fixed_operator_message` (the present-to-away notice delivered to remote sessions); helpers `message_infos`, `envelope_attr`, `envelope_body`, `message_subjects` (used only here) | ~380 |
| `owner_presence` | 411-663, 664-706, 900-946, 1274-1432 | 7 tests, `interactive_presence_refuses_human_send_then_allows_it_after_grace`, `attached_owner_presence_is_scoped_to_the_sending_session`, `grace_presence_is_scoped_and_reports_its_cause`, `viewer_only_remote_session_can_message_owner`, `force_takeover_replaces_presence_without_double_counting`, `exited_session_attach_does_not_make_owner_present`, `exited_attach_does_not_drop_another_interactive_count` (when the daemon counts the owner as present, and the human-send accept or refuse that follows; three of them assert only the owner state) | ~504 |
| `presence_clock` | 17-127 | 7 tests, `presence_starts_away_and_grants_grace_after_last_interactive_ends` .. `five_grace_cycles_have_one_away_poll_transition` (the pure `messaging::Presence` clock, no daemon); helper `instant` (used only here) | ~113 |
| `remote_status` | 194-393 | 2 tests, `attach_status_reports_remote_lease_and_gateway_lifecycle`, `attach_status_reports_expiry_and_nonmember_and_nonremote_states` (the status frame an attached client receives about the remote lease and gateway); helpers `remote_status_until`, `status_once` (used only here) | ~202 |
| `viewers` | 135-145, 707-899, 1433-1501 | 3 tests, `viewer_attach_keeps_input_read_only_while_sizing_session`, `viewer_attach_is_marked_in_session_and_agent_lists`, `release_clears_interactive_hold_but_viewer_release_is_ignored` (attach modes: viewer read-only input and sizing, the viewer flags in lists, and the Release/hold rule); helper `session_held` (used only here) | ~275 |

The 22 tests in the 5 modules are exactly the 22 test functions of the old file (3 + 7 + 7 + 2 + 3). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/presence.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fh` first; delete it before reporting. Before editing, save `cargo test --test presence -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fh/before.txt` (23 lines expected). After, the same command with `sed -E 's/^(away_notice|owner_presence|presence_clock|remote_status|viewers)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/presence.rs`, `new` = `cat tests/presence/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/presence/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/presence.rs` under `target/scratch/3fh/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which may report N/A if it holds no function.
5. Before editing, run `cargo test --test presence` once and record its passed/ignored counts (23 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test presence` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/presence.rs`, the new `tests/presence/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/presence.rs` prints nothing before editing, and the same on `tests/presence` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names it; the hits for `presence.rs` are the F94 list only. Timing note: several daemon tests wait 3.5 s past the 3 s away timeout, so a different test order under the module prefixes can overlap them differently; a failure is rerun once under this rule and reported only if it repeats). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 22 tests (`#[test]` 7 and `#[tokio::test` 15 by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `env!`, `CARGO_MANIFEST_DIR`, `include_*` (the grep prints nothing); four `common::` uses | `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1501 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `instant` used by the seven clock tests only; `owner_state` by eight tests of `owner_presence` and `away_notice`; `receive_data` by `grace_presence_is_scoped_and_reports_its_cause` (`owner_presence`), two viewer tests and one away-notice test; `session_held` by one test; `message_infos`, `envelope_attr`, `envelope_body` by one test; `message_subjects` by two away-notice tests; `remote_status_until` and `status_once` by the two `attach_status_*` tests) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths resolve through `super::*` | specs 3ev to 3fg accepted (latest bfeeb5b) | built |
| Baseline | on bfeeb5b: `cargo test --test presence -- --list` 23 `: test` lines (22 tests plus the `common::fake_discord` line); `cargo test --test presence` 23 passed, 0 ignored | run |
| No script, workflow or doc names `tests/presence.rs` or `--test presence` | `rg -n 'tests/presence\|--test presence\|presence\.rs' --glob '!target' --glob '!docs/specs/*'` hits are the F94 row only | grep |
