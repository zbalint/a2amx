# Spec 3fe: split `tests/human_inbound.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at `f634a95` (spec 3fd accepted) or later.
Shared task `context_id`: `a2amx-codescene-refactor-batch4-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/human_inbound.rs`: deleted (its content moves, see section 2).
- `tests/human_inbound/main.rs` and `tests/human_inbound/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test human_inbound`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/human_inbound.rs` is 2011 lines with 33 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fd applied one method to nine test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), and a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table).

Differences from 3fd: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]` (no doc comment), line 3 `mod common;`; lines 1-21 (attribute, `mod common;` and all the imports) stay at the top of `main.rs`, and the inner attribute covers the whole test binary, so no module carries it. No helper is used by more than one module, so `main.rs` holds the preamble and the `mod` lines only: `run_binary` (used only by the two `cli_remote_*` tests) moves to `remote_mode`, `kill` (used only by `lease_membership_is_fixed_and_survivors_keep_access_after_partial_down`) to `inbound_admission`, `gateway_other_requests` (used only by `gateway_can_only_send_human_inbound_and_keeps_connection_usable`) to `gateway_capability`. `run_binary` is declared `pub fn` and keeps that; it is used only inside `remote_mode`. The file has no indented `use` line inside a test, no `#[ignore]`, no `#[cfg]`, and one `env!("CARGO_BIN_EXE_a2amx")` (line 29, inside `run_binary`), which is resolved at compile time and works in any module file (`tests/attach_cli/main.rs:45` and `tests/common/pty.rs:56` already use it).

Alternatives rejected, as in 3ev: several `tests/human_inbound_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (`enabled_agent_can_send_to_human_and_gateway_reports_outcome` is 164 lines; a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/human_inbound.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-2011 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute or comment line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `inbound_admission` (1069-1171) and `remote_mode` (1456-1481) ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Four seams have no blank line between items in the old file (lines 21/22, 38/39, 890/891, 1498/1499), so a range that ends there is followed directly by the next range's first line (in `main.rs`, line 21 is followed directly by the `mod` lines, and `remote_mode` starts with `pub fn run_binary` after `use super::*;` and a blank line): a missing blank line at a seam stays missing (adding one is allowed; acceptance 2 ignores blank lines and `rustfmt` adds none between items).

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-21, then the 5 `mod` lines in alphabetical order (`gateway_capability`, `inbound_admission`, `outbound`, `rate_limits`, `remote_mode`) | the preamble (inner attribute, `mod common;`, imports) | ~28 |
| `inbound_admission` | 39-491, 1069-1171 | 10 tests, `inbound_requires_a_valid_team_lease_before_storage` .. `gateway_inbound_is_stored_as_a_human_message_for_a_confined_session`, `inbound_requires_a_running_confined_recipient`, `inbound_rejects_a_forged_envelope_body`; helper `kill` (used only here) | ~558 |
| `outbound` | 492-890 | 5 tests, `enabled_agent_can_send_to_human_and_gateway_reports_outcome` .. `human_outbound_rechecks_sender_eligibility_before_gateway_fetch` | ~401 |
| `remote_mode` | 22-38, 891-1068, 1456-1481 | 4 tests, `inbound_requires_explicit_both_mode_and_stores_nothing_when_refused`, `cli_remote_requires_confinement_and_creates_nothing`, `cli_remote_mode_reaches_new_session_and_controls_inbound_direction`, `remote_mode_wire_shape_is_stable_and_backward_compatible` (a serde test of the `remote` field of `Request::NewSession`); helper `run_binary` (used only here) | ~223 |
| `gateway_capability` | 1172-1455, 1482-1498 | 6 tests, `gateway_can_only_send_human_inbound_and_keeps_connection_usable` .. `bind_failure_removes_both_capability_tokens`, `human_inbound_wire_shape_is_stable`; helper `gateway_other_requests` (used only here) | ~303 |
| `rate_limits` | 1499-2011 | 8 tests, `inbound_rejects_a_body_over_the_human_chat_limit_before_lookup` .. `outbound_rate_refusal_does_not_push_a_timestamp` | ~515 |

The 33 tests in the five modules are exactly the 33 test functions of the old file (10 + 5 + 4 + 6 + 8). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least `Client`, `Deliver`, `Harness` or `Duration` from the preamble).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/human_inbound.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fe` first; delete it before reporting. Before editing, save `cargo test --test human_inbound -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fe/before.txt` (34 lines expected). After, the same command with `sed -E 's/^(gateway_capability|inbound_admission|outbound|rate_limits|remote_mode)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/human_inbound.rs`, `new` = `cat tests/human_inbound/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/human_inbound/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/human_inbound.rs` under `target/scratch/3fe/` before editing and run `cs check` on that copy); every new score at least the old one.
5. Before editing, run `cargo test --test human_inbound` once and record its passed/ignored counts (34 passed, 0 ignored, measured on 0559046 with the file unchanged since; it needs no other test run in parallel and takes about a minute). After the split `cargo test --test human_inbound` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/human_inbound.rs`, the new `tests/human_inbound/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/human_inbound.rs` prints nothing before editing, and the same on `tests/human_inbound` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: backlog I24 names `queued_human_message_rechecks_recipient_eligibility_before_delivery` (it fails with `Read-only file system` in some confined sessions and passed 34 of 34 in others; the failure depends on the session, not on this file). If it is the only failure, run it alone once on `HEAD` (before your change) in the same session: if it fails there too, report it as pre-existing (I24) and do not chase it; if it fails only after the split, that is a finding. Any other failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 33 tests (`#[test]` 2 and `#[tokio::test]` 31 by attribute count); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`; `env!` only at line 29 | `rg -c '^#\[test\]'`, `rg -c '^#\[tokio::test'`, `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use '` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 2011 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `run_binary` used only by the two `cli_remote_*` tests, `kill` only by `lease_membership_is_fixed_and_survivors_keep_access_after_partial_down`, `gateway_other_requests` only by `gateway_can_only_send_human_inbound_and_keeps_connection_usable`) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by each test name: I24 (open, session-dependent) names `queued_human_message_rechecks_recipient_eligibility_before_delivery`; no other row | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, `env!("CARGO_BIN_EXE_a2amx")` resolves in module files, and `common::` paths and the `use common::{...}` names resolve through `super::*` | specs 3ev to 3fd accepted (latest 402c8a5 for 3fc); `rg -n 'CARGO_BIN_EXE' tests/*/*.rs` | built |
| Baseline | on 0559046 (`tests/human_inbound.rs` unchanged through f634a95): `cargo test --test human_inbound -- --list` 34 `: test` lines (33 tests plus the `common::fake_discord` line); `cargo test --test human_inbound` 34 passed, 0 ignored, 65.32 s, in the architect's confined session | run |
| No script, workflow or doc names `tests/human_inbound.rs` or `--test human_inbound` | `rg -n 'human_inbound' --glob '!target' --glob '!docs/specs/*'` hits are the config key `human_inbound_per_minute`, the daemon method `human_inbound`, `Request::HumanInbound` and the backlog rows only | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review and design read, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | `remote_mode_wire_shape_is_stable_and_backward_compatible` is a serde test of the `remote` field and sat in `gateway_capability`; it belongs with the other `RemoteMode` tests | moved to `remote_mode` before locking (counts 10/5/4/6/8) |
| Draft review | consultant | the seam at lines 21/22 (preamble to `run_binary`) was not listed, so "three seams" could make a careful developer stop | fourth seam stated in section 2 before locking |
| Draft review | architect | the draft's HEAD and baseline references still pointed at 0559046 and "the commit that accepts 3fd" | fixed to f634a95 before locking |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; baseline list was source-derived because the developer's scratch (including `before.txt`) was already deleted | none |
| Design read | consultant | `rate_limits` also holds the body-size and queue-limit tests (`limits` would be exact); `main.rs` keeps single-module imports and has no blank line before the `mod` lines (the byte-for-byte rule forbids moving them) | left in backlog F94 |
| Architect review | architect | no problems found: ordered rebuild from the ranges exact for all six files, normalized diff empty, fmt, clippy and full `cargo test` (61 suites, all ok, exit 0) | none |
