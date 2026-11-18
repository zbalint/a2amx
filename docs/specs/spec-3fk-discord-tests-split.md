# Spec 3fk: split `tests/discord.rs` into themed modules of one test binary (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `3ac9b25`, spec 3fj accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch6-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `tests/discord.rs`: deleted (its content moves, see section 2).
- `tests/discord/main.rs` and `tests/discord/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, `tests/fixtures/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary, `cargo test --test discord`. No test is added, removed or changed; the equivalence checks in section 5 replace a mutation check.

## 1. Why

`tests/discord.rs` is 1194 lines with 30 tests. The owner's goal is no `.rs` file over 1,000 lines, tests included. Specs 3ev to 3fg applied one method to twelve test files (one directory test target, byte-for-byte moves, mechanical equivalence checks); this spec applies it unchanged. Lessons carried over: ranges are explicit and partition the file, a helper used by more than one module stays in `main.rs`, a helper used by one module moves there, the `mod` lines are in alphabetical order (`cargo fmt` sorts them), a full test run is recorded before editing, the test list of a binary with `mod common;` has one extra line (`common::fake_discord::tests::push_message_survives_gateway_reconnect`), a test that re-executes the binary by its bare name breaks under a module prefix (spec 3fc Amendment 1; this file has none, see the claims table), and a module is judged by what its tests run and assert, not by their names (every test below was read for the types it builds and asserts on, not only its name).

Differences from 3fg: line 1 is the inner attribute `#![allow(clippy::expect_used, clippy::unwrap_used)]`, line 3 `mod common;`; lines 1-19 (attribute, `mod common;` and the imports) are all `main.rs` holds besides its `mod` lines, and the inner attribute covers the whole test binary, so no module carries it. The file has four helpers, `answer_heartbeat`, `serve_first_connection`, `serve_replayed_connection` and `wait_for_resume` (lines 1018-1094); they serve one test, `gateway_queue_reads_heartbeats_while_consumer_is_slow`, and move with it into `gateway_queue`. The file has no indented `use` line, no `#[ignore]`, no `#[cfg]` and no `env!`. It has 30 tests: 4 `#[test]` and 26 `#[tokio::test]` by attribute count (see the claims table), 30 test functions. The gateway tests are split by what they assert: `gateway_reconnect` (how the client connects again and when it stops: backoff, silent hello, op 7, zombie close, untrusted resume URL, unanswered heartbeat, fatal close codes) and `gateway_session` (what the client sends and keeps across connections: identify, resume with session and sequence, session clearing and growing identify gaps on 4007/4009 and invalid session); the two `gateway_resume_seq_*` tests and the handoff test assert the tracked sequence number, so they sit in `gateway_session`.

Alternatives rejected, as in 3ev: several `tests/discord_<theme>.rs` binaries (disk and link cost), `mod` blocks inside one file, shortening long tests (a later step).

## 2. Target shape

Each file below is the concatenation, in the order listed, of the line ranges of the current `tests/discord.rs` (1-based, both ends included), preceded in every module by `use super::*;` and a blank line, and followed in `main.rs` by the `mod` lines. The ranges partition the file exactly: every line 1-1194 is in exactly one of them (the architect checked this with a script that also lists helper use sites and the preamble names each module uses). A range starts on the first attribute, comment or declaration line of an item and ends on the line before the next item's first line. No visibility changes: no `pub(crate)`, no other `use` line at column 0, no `#![allow]` in a module. `cargo fmt` may remove a trailing blank line at the end of a file; that is allowed, and because the last range of `gateway_queue`, `gateway_reconnect`, `gateway_session`, `rest` ends on a blank line, the developer runs `cargo fmt` once after assembling the files, so a raw `cargo fmt --check` failure before that is not a BLOCKED. Every range starts at an item start and ends at an item end.

| File | Line ranges | Contents | Lines |
| --- | --- | --- | --- |
| `main.rs` | 1-19, then the 5 `mod` lines in alphabetical order (`config`, `gateway_queue`, `gateway_reconnect`, `gateway_session`, `rest`) | the preamble only (inner attribute, `mod common;`, imports); the file has no helper used by more than one module | ~25 |
| `config` | 20-56, 1188-1194 | 4 tests, `defaults_use_discord_gateway_and_api`, `debug_and_errors_redact_token_and_fixed_text`, `rustls_gateway_config_is_constructed_without_network`, `close_classification_and_url_defaults_are_independent_of_network` (`DiscordConfig` defaults, token redaction, the rustls config and the config/`Fatal` defaults check at the end of the file (`close_classification_and_url_defaults_are_independent_of_network`, which only builds a `DiscordConfig` and a `Fatal`)) | ~46 |
| `gateway_queue` | 1018-1187 | 2 tests, `gateway_queue_reads_heartbeats_while_consumer_is_slow`, `malformed_gateway_frames_do_not_stop_later_events` (the event queue under a slow consumer and malformed frames); helpers `answer_heartbeat`, `serve_first_connection`, `serve_replayed_connection`, `wait_for_resume` (used only here) | ~172 |
| `gateway_reconnect` | 334-417, 583-615, 825-948 | 7 tests, `gateway_reconnect_backoff_is_capped` .. `gateway_fatal_close_codes_are_persistent_errors` (connecting again and stopping: backoff, silent hello, op 7, zombie close, untrusted resume URL, unanswered heartbeat, fatal close codes) | ~243 |
| `gateway_session` | 418-582, 616-824, 949-1017 | 8 tests, `gateway_identifies_filters_guilds_and_heartbeats_sequence` .. `gateway_resume_seq_tracks_last_handoff_before_reconnect` (what the client sends and keeps across connections: identify, resume with session and sequence, session clearing and identify gaps, tracked sequence) | ~445 |
| `rest` | 57-333 | 9 tests, `rest_finds_and_creates_channels_and_parses_lists` .. `rest_timeout_and_response_limit_are_errors` (the REST calls: channel lookup and creation, send limits and headers, path and guild validation, status errors, rate-limit retries, timeout and response limit) | ~279 |

The 30 tests in the 5 modules are exactly the 30 test functions of the old file (4 + 2 + 7 + 8 + 9). Module names are fixed; test and helper names do not change. The `#[path = "../common/mod.rs"]` line goes directly above `mod common;` in `main.rs`. A helper moved into a module keeps its body and sits where its range puts it. The developer confirms each helper placement with `rg -n` and stops if a use site contradicts the table. If `cargo clippy` reports `use super::*;` as unused in a module that needs nothing from the preamble, the developer removes that one line in that module, says so in the report, and leaves everything else as is (the architect checked that every module uses at least one preamble name).

## 3. What must not change

- Every test body, attribute and helper body is moved byte-for-byte. The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- Comments and `// shortcut:` comments move with their code. No new `#[allow]`, `#[cfg]` or suppression. No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

Shortening large tests or extracting shared setup; `tests/common/`; other test files; moving tests between modules beyond the table (misplaced tests go to backlog F94); the backlog rows that name `tests/discord.rs` (the architect updates them in the accepting commit).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** (The module of each test is checked by the architect and the tester against the table in section 2, because the sed and the sorted-line check hide it.) Create `target/scratch/3fk` first; delete it before reporting. Before editing, save `cargo test --test discord -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fk/before.txt` (31 lines expected). After, the same command with `sed -E 's/^(config|gateway_queue|gateway_reconnect|gateway_session|rest)::(.*)$/\2/'` applied (the `common::fake_discord::tests::...` line is left alone), then sorted, equals `before.txt` (`diff` exit 0). Check that `before.txt` has the expected number of lines first: `2>/dev/null` hides a build failure and would leave it empty.
2. **Same code.** `old` = `git show HEAD:tests/discord.rs`, `new` = `cat tests/discord/*.rs`. Delete blank lines and lines that start at column 0 with `mod `, `use ` or `#[path`, then `sort`; `diff` shows nothing (exit 0). For example `sed '/^$/d; /^\(mod \|use \|#\[path\)/d' | sort`. The file has no indented `use` line, so none can be lost unseen.
3. **Order inside each file.** The architect rebuilds each new file from the section 2 ranges and compares it, in order, with the developer's file; the tester repeats this independently. The developer does not have to run it.
4. `wc -l tests/discord/*.rs`: every file under 1,000 lines. `cs check` on each new file: report the scores and the old file's score (save `git show HEAD:tests/discord.rs` under `target/scratch/3fk/` before editing and run `cs check` on that copy); every new score at least the old one, except `main.rs`, which may report N/A if it holds no function. A score below the old one is reported with its warnings and not fixed (a byte move cannot change the code health of a test's own body); the architect decides whether it is acceptable.
5. Before editing, run `cargo test --test discord` once and record its passed/ignored counts (31 passed, 0 ignored expected; it needs no other test run in parallel). After the split `cargo test --test discord` has the same counts; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
6. `git status --short` shows only the deletion of `tests/discord.rs`, the new `tests/discord/` files and nothing else from you.
7. Fresh full gate by the architect.
8. Name-dependence check (every split spec carries it): `rg -n -e 'current_exe|--exact|A2AMX_TEST_|module_path!|type_name' tests/discord.rs` prints nothing before editing, and the same on `tests/discord` prints nothing after. The tester also answers in its report: could the equivalence checks miss behaviour that depends on a test's own name or module path?

Known flaky tests: none open for this file (checked in `docs/backlog.md` by path and by every test name: no row names it; the hits for `discord.rs` are the F94 list only). A failure is rerun once; a second failure is reported.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| 30 tests (`#[test]` 4 and `#[tokio::test` 26 by attribute count (30)); no indented `use`, `#[ignore]`, `#[cfg]`, `current_exe`, `--exact`, `A2AMX_TEST_`, `module_path!`, `type_name`, `env!`, `CARGO_MANIFEST_DIR`, `include_*` (the grep prints nothing); one `common::` use (line 7) | `rg -n -e 'current_exe\|--exact\|A2AMX_TEST_\|module_path!\|type_name\|env!\|#\[ignore\|#\[cfg\|^\s+use \|CARGO_MANIFEST\|include_'` (in this table every backslash before a pipe is only a Markdown escape: type a plain `|` when you run it, as acceptance 8 writes it) | grep |
| Helper use sites by module and the partition of all 1194 lines | `items.py` and `plan.py` run by the architect over the file (every line covered once; `answer_heartbeat` used only by `serve_first_connection`; `serve_first_connection`, `serve_replayed_connection` and `wait_for_resume` only by `gateway_queue_reads_heartbeats_while_consumer_is_slow`) | script |
| Every module uses at least one preamble item, so `use super::*;` is not unused | per-module name counts over each module's ranges | script |
| Known flakes naming this file | `docs/backlog.md` searched by path and by test names | grep |
| A directory target with `#[path = "../common/mod.rs"] mod common;` and `use super::*;` works, and `common::` paths resolve through `super::*` | specs 3ev to 3fg accepted (latest bfeeb5b) | built |
| Baseline | on 3ac9b25: `cargo test --test discord -- --list` 31 `: test` lines (30 tests plus the `common::fake_discord` line); `cargo test --test discord` 31 passed, 0 ignored | run |
| No script, workflow or doc names `tests/discord.rs` or `--test discord` | `rg -n 'tests/discord\|--test discord\|discord\.rs' --glob '!target' --glob '!docs/specs/*'` hits are the F94 row only | grep |
