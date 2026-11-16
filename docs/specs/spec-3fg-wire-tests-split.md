# Spec 3fg: split `tests/wire.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `8a61b20`, spec 3ff accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch5-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/wire.rs`: deleted (its content moves, see section 2).
- `tests/wire/main.rs` and `tests/wire/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test wire`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/wire.rs` is 1556 lines with 39 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3ff applied one method to eleven test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names.

Differences from 3ff: this file has no `mod common;`, no helper, no inner attribute and no `#![allow]`; lines 1-7 are the imports (`use a2amx::emulator::Scroll;` and one `use a2amx::wire::{...};` block), and all 39 items are `#[test]` functions. So `main.rs` holds the imports and the `mod` lines only, there is no `#[path]` line, and the test list has exactly 39 lines (no `common::fake_discord` line). Eight `use` lines sit indented inside tests (lines 46, 323, 792, 899, 940, 1169, 1170, 1336), so the normalization of acceptance 2 keeps them on both sides. The file has no `env!`, `#[ignore]` or `#[cfg]`.

Alternatives rejected, as in 3ev: several `tests/wire_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (`messaging_control_variants_round_trip` is 148 lines; a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/wire.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1556 is in exactly one of them (the architect checked this with a script that also lists the preamble names each module uses). A range starts on the first attribute line of a test and ends on the line before the next test's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0 in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `frames`, `requests`, `summaries` and `visibility` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Three seams have no blank line between items in the old file (lines 43/44, 519/520, 1087/1088); each lies inside one range, so no range boundary falls on one.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-7, then the 5 `mod` lines in alphabetical order (`frames`, `messaging`, `requests`, `summaries`, `visibility`) | the imports | ~13 |
| `summaries` | 8-73, 454-637 | 6 tests, `summary_activity_serializes_literals_and_old_payloads_remain_unknown`, `quota_info_serializes_percent_left_field_names`, `response_and_session_summary_shapes_remain_unchanged`, `session_summary_durations_round_trip_when_present`, `session_summary_starting_is_optional_and_round_trips`, `viewer_summary_fields_round_trip_and_default_for_old_clients` | ~252 |
| `frames` | 74-137, 648-789 | 13 tests, `length_prefixed_frame_round_trips` .. `frame_decoder_bounds_partial_payload_buffer_and_handles_following_frame` (the length-prefixed frame codec), `every_server_stream_frame_round_trips` .. `mode_frame_has_a_boolean_body_and_rejects_invalid_encodings` (the stream frames) | ~208 |
| `requests` | 138-453, 638-647, 1318-1333, 1467-1486 | 10 tests, `request_json_uses_exact_tagged_shapes`, `explain_wire_round_trips_and_omits_absent_options`, `activity_watch_round_trips_on_new_session`, `activity_watch_validation_uses_literal_bounds_and_errors`, `version_request_and_response_use_exact_shapes`, `lease_requests_and_response_use_exact_shapes`, `reset_request_and_response_use_exact_shapes`, `screen_response_json_uses_exact_shape` (only the JSON of `Response::Screen`), `attach_top_rows_preserves_old_wire_and_acknowledges_new_wire` (only the `Request::Attach` and `Response::Attached*` shapes), `report_quota_request_round_trips` | ~364 |
| `messaging` | 790-983, 1487-1556 | 4 tests, `messaging_control_variants_round_trip`, `prompt_reports_verdicts_and_optional_evidence_have_exact_json`, `human_outbound_requests_and_response_use_exact_shapes`, `gateway_agents_request_and_response_use_exact_shapes` | ~266 |
| `visibility` | 984-1317, 1334-1466 | 6 tests, `additive_visibility_fields_round_trip`, `confinement_visibility_fields_are_backward_compatible`, `status_frames_and_attach_opt_in_preserve_wire_compatibility`, `status_members_default_omit_and_round_trip`, `team_fields_round_trip_and_old_new_session_defaults_are_compatible`, `confine_request_round_trips_and_defaults_missing_fields` | ~469 |

The 39 tests in the five modules are exactly the 39 test functions of the old file (6 + 13 + 10 + 4 + 6). Module names are fixed; test names do not change. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name from the `wire` import block or `Scroll`).

## 3. What must not change

- Every test body and attribute is moved byte-for-byte. The only allowed edits are the `mod` lines in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/wire.rs` (the architect updates them in the accepting commit); the module doc of `src/wire.rs` (backlog I12).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fg` first; delete it before reporting. Before editing, save `cargo test --test wire -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fg/before.txt` (39 lines expected). After, the same command with `sed -E 's/^(frames|messaging|requests|summaries|visibility)::(.*)$/\2/'` applied, then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/wire.rs`, `new` = `cat tests/wire/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The column-0 lines `use a2amx::emulator::Scroll;` and the `use a2amx::wire::{` block (lines 1-7) are removed by this normalization except for the block's lines 3-6 (the three indented continuation lines and the closing `};`), which stay on both sides; the eight indented `use` lines inside tests stay on both sides, so losing one fails the check.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/wire/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/wire.rs` under `target/scratch/3fg/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which holds no function and may report N/A.
5. Before editing, run `cargo test --test wire` once and record its passed/ignored counts (39 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test wire` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/wire.rs`, the new `tests/wire/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/wire.rs` prints nothing before editing, and the same on `tests/wire` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names it; the hits for `wire.rs` are `src/wire.rs` in I12 and the F94 list). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 39 tests, all `#[test]` (by attribute count); eight indented `use` lines at 46, 323, 792, 899, 940, 1169, 1170, 1336; no `mod common;`, `common::`, `env!`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `CARGO_MANIFEST_DIR`, `include_*` | `rg -c '^#\[test\]' tests/wire.rs`, `rg -c 'common::' tests/wire.rs` (no match), `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| The partition of all 1556 lines and the helper-free shape | `items.py` and `plan.py` run by the architect over the file (every line covered once; the file has no function that is not a `#[test]`) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by each of the 39 test names: no row (I12 names `src/wire.rs`; F94 lists this file) | grep |
| A directory target with `use super::*;` and no `mod common;` works | specs 3ev to 3ff accepted (latest 8a61b20); `tests/messaging/main.rs` (spec 3ez) has no `mod common;` and its modules start with `use super::*;` (`rg -c '^mod common;' tests/*/main.rs` shows `tests/messaging/main.rs` as the only one without) | built |
| Baseline | on 78ed06d (`tests/wire.rs` unchanged through 8a61b20): `cargo test --test wire -- --list` 39 `: test` lines; `cargo test --test wire` 39 passed, 0 ignored, 0.01 s | run |
| No script, workflow or doc names `tests/wire.rs` or `--test wire` | `rg -n 'tests/wire\|--test wire\|wire\.rs' --glob '!target' --glob '!docs/specs/*'` hits are `src/wire.rs` in backlog I12 and the F94 row only | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review and design read, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | `attach_top_rows_preserves_old_wire_and_acknowledges_new_wire` (only `Request::Attach` and `Response::Attached*` shapes, in `visibility`) and `screen_response_json_uses_exact_shape` (only `Response::Screen` JSON, in `summaries`) were in the wrong module | both moved to `requests` before locking (counts 6/13/10/4/6) |
| Draft review | consultant | acceptance 2 named two lines of the multi-line `use a2amx::wire::{` block that stay on both sides; there are four (lines 3-6) | wording fixed before locking |
| Draft review | architect, consultant (both missed) | `confine_request_round_trips_and_defaults_missing_fields` and `team_fields_round_trip_and_old_new_session_defaults_are_compatible` assert only `Request` shapes and sit in `visibility`; the draft review walked only some tests' asserted types | found by the consultant's design read; not moved (a post-implementation amendment would re-open developer and tester for a theme-only gain); recorded in backlog F94 |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; the old file's `cs check` score read 4.99 on its copies against 5.59 reported by the developer (unexplained, every new score is at least either) | none |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all six files, normalized diff empty, fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
