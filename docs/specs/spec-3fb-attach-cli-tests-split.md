# Spec 3fb: split `tests/attach_cli.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at the commit that accepts the previous refactor spec or later.
Shared task `context_id`: `a2amx-codescene-refactor-batch3-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/attach_cli.rs`: deleted (its content moves, see section 2).
- `tests/attach_cli/main.rs` and `tests/attach_cli/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test attach_cli`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/attach_cli.rs` is 2951 lines with 48 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fa applied one method to six test files (one directory test target, byte-for-byte moves, mechanical equivalence checks, accepted with 0 BLOCKED cycles); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, and the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`).

Differences from 3fa: the file has no inner `#![allow]`; line 1 is an inner doc comment (`//!`) that stays line 1 of `main.rs`. Two indented `use` lines sit inside one test (1684-1685), so the normalization of acceptance 2 keeps them on both sides. The fake daemon (struct `FakeDaemon`, its `impl`, and `fake_recv_frame`, `fake_send_frame`, `fake_team`, `fake_sessions`, `fake_status`, `fake_connection`, `fake_attach`, `fake_serve_detach`, `run_fake_daemon`) is used by three modules and stays in `main.rs`; `start_fake_daemon` is used only by `viewer` and moves there; `wait_for_file` is used only by `claude_new_wires_mcp_and_optional_peer_authorization` and moves with it into `attach_session`.

Alternatives rejected, as in 3ev: several `tests/attach_cli_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/attach_cli.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2951 is in exactly one of them (the architect checked this with a script that also lists helper use sites). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-218, 237-396, then the 6 `mod` lines in alphabetical order (`attach_session`, `list_and_messages`, `picker`, `scroll_and_mouse`, `status_and_team_line`, `viewer`; `cargo fmt` sorts them) | the preamble (inner doc comment, `mod common;`, imports, `WAIT`) and the helpers used by several modules: `cli_args`, `run_cli`, `run_binary`, and the fake daemon (see section 1) | ~384 |
| `attach_session` | 397-457, 897-1081, 1170-1258, 1681-1809, 2219-2244 | 10 tests, `new_list_attach_detach_and_literal_prefix_use_real_terminal` .. `switch_waits_for_old_attachment_eof_with_delayed_fake_daemon`, including `claude_new_wires_mcp_and_optional_peer_authorization`; helper `wait_for_file` (used only by that test) | ~492 |
| `scroll_and_mouse` | 458-679 | 10 tests, `scroll_mode_parses_split_sequences_and_returns_to_live_bottom` .. `mouse_wheel_is_dropped_while_session_picker_is_open` | ~224 |
| `picker` | 680-896, 1810-1937 | 5 tests, `picker_switches_sessions_and_exit_status_is_reported` .. `picker_folds_teams_and_filters` | ~347 |
| `status_and_team_line` | 1568-1680, 1938-2218 | 5 tests, `status_line_toggles_and_shows_the_session_address` .. `status_toggle_reattaches_to_resize_the_session` | ~396 |
| `viewer` | 219-236, 2245-2951 | 13 tests, `attach_viewer_sends_no_resize_frame` .. `viewer_mouse_reports_are_dropped_with_and_without_team_rows`; helpers `start_fake_daemon`, `assert_viewer_after_stays_interactive`, `assert_viewer_mouse_is_not_forwarded` (used only here) | ~727 |
| `list_and_messages` | 1082-1169, 1259-1567 | 5 tests, `named_session_appears_in_the_list_table` .. `cancel_is_silent_on_success_and_reports_errors`; helpers `wait_for_message_state`, `wait_for_held` (used only here) | ~399 |

Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least `WAIT`, `cli_args`, `run_cli`, `run_binary`, `PtyHarness` or `Client` from the preamble).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- The `env!` uses (`CARGO_BIN_EXE_a2amx` at lines 44, 1022, 1047, 1197, 1960, 1982) do not change; none depends on the file's location.
- Comments, `// SAFETY:` and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94; known: the three `raw_wait_*` tests in `scroll_and_mouse` exercise the `PtyHarness` itself, not a2amx); the backlog rows that name `tests/attach_cli.rs::<test>` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fb` first; delete it before reporting. Before editing, save `cargo test --test attach_cli -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fb/before.txt` (49 lines expected). After, the same command with `sed -E 's/^(attach_session|scroll_and_mouse|picker|status_and_team_line|viewer|list_and_messages)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/attach_cli.rs`, `new` = `cat tests/attach_cli/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path` (an indented `use` inside a function is kept on both sides, so losing one fails the check), then `sort`; `diff` exit 0. For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The two indented `use` lines inside the test at 1684-1685 are untouched.
3. `wc -l tests/attach_cli/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/attach_cli.rs` under `target/scratch/3fb/` before editing and run `cs check` on that copy); every new score at least the old one.
4. Before editing, run `cargo test --test attach_cli` once and record its passed/ignored counts (49 passed, 0 ignored expected on a quiet machine; it needs no other test run in parallel). After the split `cargo test --test attach_cli` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
5. `git status --short` shows only the deletion of `tests/attach_cli.rs`, the new `tests/attach_cli/` files and nothing else from you.
6. Fresh full gate by the architect.

Known flaky tests (backlog rows I7 and I18, both open): `attach_does_not_capture_session_already_in_drag_mode` (in `scroll_and_mouse`; it fails now and then with an unexpected `ESC[?1000h`, passes alone) and `status_toggle_reattaches_to_resize_the_session` (in `status_and_team_line`; failed once under load). A failure of one of them is rerun once; if the rerun passes, report it as the known flake with the failing output; a second failure, or any failure of another test, is reported as new.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 48 tests; two indented `use` lines inside a test (1684-1685); `env!` uses: `CARGO_BIN_EXE_a2amx` at lines 44, 1022, 1047, 1197, 1960, 1982; no `#![allow]` | `rg -c '^#\[(tokio::)?test' tests/attach_cli.rs`, `rg -n '^\s+use '`, `rg -n 'env!'`, `rg -n 'allow\('` | grep |
| Helper use sites by module and the partition of all 2951 lines | `plan.py` run by the architect over the file (every line covered once; fake daemon helpers used by `attach_session`, `status_and_team_line` and `viewer`; `start_fake_daemon` only by `viewer`; `wait_for_file` only by `claude_new_wires_mcp_and_optional_peer_authorization` (`attach_session`); `wait_for_message_state` and `wait_for_held` only by `list_and_messages`) | script |
| Known flakes naming this file | `docs/backlog.md` rows I7 (open), I18 (open; also names two `tests/daemon_cli.rs` tests), I14 and F88 (fixed, so not flakes now) | read |
| Every module uses at least one preamble item, so `use super::*;` is not unused | `rg -o` of preamble names over each module's ranges (all six use `WAIT`, `cli_args`, `run_cli`, `run_binary`, `PtyHarness` or `Client`) | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works | specs 3ev to 3fa accepted (latest 2c79340) | built |
| Baseline test list | `cargo test --test attach_cli -- --list` on HEAD 2c79340, 49 lines (48 tests plus the `common::fake_discord` line) | run |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review and design read, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | section 5 said no known flaky tests, but backlog rows I7 and I18 are open for this file (the line was copied from 3fa without grepping `docs/backlog.md` for the file name) | I7 and I18 named with the rerun-once rule |
| Draft review | consultant | the order of the `mod` lines was not stated; `cargo fmt` sorts them | alphabetical order stated |
| Draft review | consultant | acceptance 4 compared with a baseline that was never recorded | a pre-edit `cargo test --test attach_cli` run required |
| Draft review | consultant | `claude_new_wires_mcp_and_optional_peer_authorization` was a theme misfit and `attach_and_switch` too wide a name | the test and its only helper `wait_for_file` moved to the module, renamed `attach_session` |
| Implementation | developer | no problems found; acceptance 1 to 5 pass, scores 8.65 to 9.38 (old file 5.51) | none |
| Tester pass | tester | no problems found: 7 files identical to the spec's ranges, 49 passed | none |
| Design read | consultant | `switch_waits_for_old_attachment_eof_with_delayed_fake_daemon` switches by a team-line click and could sit in `status_and_team_line`; `start_fake_daemon` is viewer-private while two modules repeat its setup inline | left for backlog F94 (a range change would need an amendment for a placement preference) |
| Architect review | architect | no problems found; rebuilt all files from the spec's ranges (identical), sorted-line diff, test list, `cs check`, fmt, clippy, full `cargo test` (61 suites, 1347 passed, 3 ignored) | none |
