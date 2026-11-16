# Spec 3ev: split `tests/confine.rs` into themed modules of one test binary (refactor pilot)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `ca1823c` or later.
Shared task `context_id`: `a2amx-codescene-refactor-2026-11-16`.

**Scope.** May edit exactly these paths and no others.
- `tests/confine.rs`: deleted (its content moves, see section 2).
- `tests/confine/main.rs` and `tests/confine/<module>.rs` for the modules in section 2 (all new).

Does not touch: `src/`, `tests/common/`, every other test, `Cargo.toml`, `Cargo.lock`, `docs/` (the architect updates the
backlog and records the result), `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge.

Public test seam: the test binary itself, `cargo test --test confine`. No test is added, removed or changed, so
no new behavior is tested; the mutation check does not apply, the equivalence checks in section 5 replace it.

## 1. Why

`tests/confine.rs` is 7,086 lines with 159 top-level functions (146 tests) and no structure. CodeScene scores it
3.48 with 58 findings (low cohesion, number of functions in a module, many large methods). The owner's goal is no
`.rs` file over 1,000 lines, tests included, so that a human can take over the project. This is the pilot of that
refactor: test-only, so it cannot change product behavior, and it is the safety net of `src/confine.rs`, the next
large file.

Decisions already taken with the owner: tests count toward the 1,000-line goal; small steps, one file at a time; the
CodeScene report stays advisory.

Alternatives rejected:
- Several top-level files `tests/confine_<theme>.rs`: each is its own test binary, so each recompiles the
  `a2amx` crate link and `tests/common`, which costs disk and build time on a machine where disk is the scarce
  resource. A directory target `tests/confine/main.rs` with submodules is one binary.
- Splitting inside the file with `mod` blocks: the file stays 7,000 lines, which is the problem.
- Shortening the long tests (CodeScene "Large Method" findings): a separate, later step. A move keeps every test body
  byte-for-byte, which is what makes this change checkable mechanically.

Cargo auto-discovers `tests/<name>/main.rs` as the integration test target `<name>`, so `cargo test --test confine`
keeps working and CI, the Sonar gate and `scripts/prepush-sonar.sh` need no change (`rg -n 'test confine|tests/confine' .github scripts sonar-project.properties`
finds nothing; the 42 older specs and `docs/backlog.md` F35 name `tests/confine.rs` as history and stay as they are).

## 2. Target shape

`tests/confine/main.rs` is today's lines 1-171 verbatim (the inner `#![allow(clippy::expect_used, clippy::unwrap_used)]`
as line 1, `mod common;` with `#[path = "../common/mod.rs"]` added above it because the file is now one directory deeper, every
import, `Fixture` and all helpers up to `confined_shell_request`), then `populate_claude_overlay` and `populate_omp_overlay`
moved from the end of the file (they are called from `harnesses`, lines 3641-3682, and from `harness_config`, 6353-6844),
then one `mod <name>;` line per module below. The preamble stays whole in `main.rs`, including the helpers that only
`real_bwrap` uses (`probe_fail_stub_path_dir`, `bwrap_probe_fail_stub_path_dir`, `start_confine_daemon_with_env`).

**No visibility changes.** Private items and private imports in the crate root are visible to child modules, so each module
file starts with exactly `use super::*;` and adds no other `use` line, no `#![allow]` and no `pub(crate)`. Helpers defined
after the preamble that only one module uses move into that module and stay private: `plan_with_timeout` into
`paths_and_git_config`, `git_common_dir_with_timeout` into `git_dirs_and_roots`, `run_ctrl_c_case` into `real_bwrap`.

Modules, by the line range in the current `tests/confine.rs`. A range starts at the first attribute or doc comment of the
function listed and ends before the attribute of the next listed one, except for the named moves below; boundaries are not negotiable
by taste. Each module must end up under 1,000 lines; the ranges below are 120 to 800.

| Module | First function | Last function before the next module | Lines |
| --- | --- | --- | --- |
| `plan_basics` | `plan_skips_missing_and_unhides_existing_home_secrets_read_only` (173) | `skipped_paths_are_sorted` (486) | ~340 |
| `grants_runtime` | `runtime_grants_select_sockets_and_environment` (511) | `granted_paths_may_be_missing_without_becoming_unresolved` (1130) | ~640 |
| `grants_links_and_env` | `grant_alias_to_hidden_or_unhidden_path_is_rejected` (1148) | `ssh_grant_runs_ssh_query_through_bwrap` (1906) | ~800 |
| `limits_and_hides` | `write_limit_error_precedes_hide_limit_error` (1948) | `omp_exposes_only_state_omp_on_top_of_mandatory_state_hide` (2403), plus the named moves | ~600 |
| `wrap_and_sockets` | `wrap_unsets_base_and_container_environment_in_stable_order` (2449) | `container_socket_hides_are_bound_and_can_be_disabled` (3009) | ~625 |
| `harnesses` | `codex_is_accepted_and_write_guards_are_reciprocal` (3074) | `confine_info_grants_are_sorted_deduplicated_and_include_implicit_harness_config` (3680) | ~640 |
| `paths_and_git_config` | `canonical_paths_cover_symlinked_workspace_and_hide_targets` (3801) | `plan_finds_hooks_path_after_lines_without_equals` (4117) | ~425 |
| `git_dirs_and_roots` | `git_worktree_and_submodule_common_dirs_are_bound` (4138) | `home_repository_is_not_bound_and_outside_team_file_stays_hidden` (4839) | ~700 |
| `notices_and_preflight` | `notice_describes_restrictions_and_keeps_closing_sentence` (4877) | `preflight_rejects_missing_bwrap` (5092), plus the named moves | ~330 |
| `real_bwrap` | `real_bwrap_confines_the_session_process` (5098) | `ctrl_c_reaches_a_confined_raw_mode_reader_as_input` (5635), with `run_ctrl_c_case`, plus the named moves | ~800 |
| `grant_writes_and_create` | `grants_are_validated_and_reported_by_the_plan` (5748) | `workspace_in_home_is_still_refused` (6260), minus the named moves | ~560 |
| `harness_config` | `harness_config_default_replaces_data_overlays` (6351) | `harness_config_grant_persists_regular_writes` (6872); `populate_*` leave for `main.rs` | ~600 |

The preamble stays in `main.rs` (about 230 lines with the moved overlay helpers and the `mod` lines). If a function sits
between two listed ranges it follows the module above it.

Named moves, so that each module holds one concern (whole functions with their attributes; each moved function keeps its
body byte-for-byte and the test count does not change):
- `path_lists_and_argv_sizes_are_capped` (3713) and `total_argv_size_checks_sum_arguments_below_per_argument_cap` (3782) go to `limits_and_hides`.
- `launch_argv_prefixes_generic_with_env_wrappers` (5655) and `preflight_env_refuses_an_env_without_the_flags` (5702) go to `notices_and_preflight`.
- `find_executable_ignores_relative_and_empty_entries` (6317) and `resolve_bwrap_refuses_a_relative_file_path` (6341) go to `notices_and_preflight`.
- `ssh_grant_runs_ssh_query_through_bwrap` (1906), `real_bubblewrap_cargo_grant_keeps_credentials_and_bin_protected` (6040), `real_bubblewrap_hides_nested_credentials_but_allows_cache_writes` (6090), `real_bubblewrap_cache_grant_hides_the_token_and_writes_the_cache` (6134) and `real_bwrap_confined_tree_hides_state_run_and_secrets` (7014) go to `real_bwrap`.
Estimated sizes after the moves: all modules stay between about 250 and 850 lines.

Names: module names above are fixed. Test function names do not change.

## 3. What must not change

- Every test body, attribute (`#[test]`, `#[tokio::test]`, `#[ignore]`, `#[cfg]`) and helper body is moved byte-for-byte.
  The only allowed edits are the `mod` lines and `#[path]` in `main.rs` and the one `use super::*;` line in each module.
- The `CARGO_TARGET_TMPDIR` and `CARGO_MANIFEST_DIR` uses do not change (neither depends on the file's location).
- `// SAFETY:` and `// shortcut:` comments move with their code.
- No new `#[allow]`, no `#[cfg(...)]` change, no suppressions.
- No test is renamed, merged, split, skipped or newly ignored.

## 4. Out of scope

- Shortening large tests, extracting shared setup, fixing CodeScene "Large Method" or "Large Assertion Blocks" findings.
- `tests/common/`, other test files, `src/confine.rs`.
- Updating the 42 older specs or backlog row F35 that name `tests/confine.rs`.
- Changing the AGENTS.md module table (it lists `src` modules only).

## 5. Acceptance

Run from the main checkout, as an ordinary user, with the environment form in `AGENTS.md`.

1. **Same tests, same names.** Before any edit, save `cargo test --test confine -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3ev/before.txt`
   (147 lines expected: the 146 tests plus `common::fake_discord::tests::push_message_survives_gateway_reconnect`, which every binary
   with `mod common;` lists). After the change, the same command with `sed -E 's/^(plan_basics|grants_runtime|grants_links_and_env|limits_and_hides|wrap_and_sockets|harnesses|paths_and_git_config|git_dirs_and_roots|notices_and_preflight|real_bwrap|grant_writes_and_create|harness_config):://'`
   applied, then sorted, equals `before.txt` (`diff` exit 0). Judge: `diff` exit code; the architect and the tester rerun it.
2. **Same code.** Let `old` be `git show HEAD:tests/confine.rs`, `new` the concatenation of `tests/confine/*.rs`. Normalize
   both: delete blank lines and lines that start (after indentation) with `mod `, `use ` or `#[path`, then `sort`. The two
   outputs are identical (`diff` exit code 0). The old file has no multi-line `use` and none inside a function body (`rg -n '^\s+use ' tests/confine.rs` finds none), and
   the new files add only `use super::*;` and `mod` lines, so no group handling is needed. Judge: the architect's script and the tester's independent one.
3. **File size.** `wc -l tests/confine/*.rs` shows every file under 1,000 lines. `cs check` on each new file gives a
   code-health score no lower than the old file's 3.48 and no new finding category; "Number of Functions in a Single Module"
   and "Low Cohesion" are expected to go away. Advisory: report the numbers; a score below 3.48 is a finding.
4. `cargo test --test confine` passes in the developer's own (confined) session with the same counts as before the change
   (record passed/ignored from the baseline run before editing), and `cargo clippy --all-targets -- -D warnings` and
   `cargo fmt --check` pass.
5. `git status --short` shows only the deletion of `tests/confine.rs` and the new `tests/confine/` files.
6. Fresh full gate by the architect: `cargo test` (whole suite), `scripts/prepush-fast.sh`.

Known flaky tests: none known in this file. A failure is rerun once; a second failure is reported, not hidden.

## 6. Claims checked before locking

| Claim | Evidence | How |
| --- | --- | --- |
| Cargo.toml has no `autotests` or `[[test]]` entries, so targets are auto-discovered | `rg -n 'autotests|\[\[test\]\]' Cargo.toml` finds nothing | grep |
| Nothing but history names `tests/confine.rs` | the grep in section 1 | grep |
| `tests/confine.rs` has 146 `#[test]`/`#[tokio::test]` and 159 top-level fns, 7,086 lines, preamble lines 1-171 | `rg -c '^#\[(tokio::)?test'`, fn list, consultant m_7742 | read |
| `tests/common/fake_discord.rs:682` has a `#[tokio::test]` that every binary with `mod common;` lists | consultant m_7742 | read; not run |
| Private parent items and imports are visible to child modules through `use super::*` | Rust module visibility (the `mod tests` pattern) | language rule; confirmed at build time |
| No file depends on the path `tests/confine.rs`; `sonar.coverage.exclusions=tests/**` is recursive | greps above, `sonar-project.properties:24` | read |
| `tests/<name>/main.rs` is one target `<name>` | Cargo's documented target auto-discovery | **docs, not yet probed in this repo**: the developer confirms with `cargo test --test confine -- --list` after the first module is split |
| `mod common;` needs `#[path = "../common/mod.rs"]` from `tests/confine/main.rs` | module path resolution relative to the file's directory | to confirm at build time |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock and design read, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | acceptance 1 expected 146 lines, but `tests/common/fake_discord.rs` has a test that every binary with `mod common;` lists, so a correct split gave 147 and the module-prefix `sed` also mangled the `common::` line | the filter names the 12 modules; 147 expected |
| Draft review | consultant | acceptance 2 told the developer to write each `use` on one line, which `rustfmt` can rewrap, so the code diff could fail on a correct split | modules start with exactly `use super::*;`; the old file has no multi-line `use` |
| Draft review | consultant | the helper rule ("used by one module moves there") contradicted the preamble list for three helpers only `real_bwrap` uses | the preamble stays whole in `main.rs`; the one-module rule applies only to helpers after it |
| Draft review | consultant | `pub(crate)` on shared items was needless edit surface | no visibility changes |
| Draft review | consultant | line anchors off by one (7,086 lines, preamble 1-171); four modules mixed concerns by file order | anchors fixed; named moves added for the launch/preflight, bwrap-lookup, real-bwrap and limit tests |
| Draft | architect | the draft called `populate_*` private to one module; they are called from two | moved to `main.rs` |
| Design read | consultant | six tests still sit next to a neighbour of another theme (two harness-config tests in `harnesses`, two harness-data tests in `git_dirs_and_roots`, one git test in `limits_and_hides`), and `grants_runtime` is a weak name | not fixed here (the ranges were locked); backlog F94 |
| Implementation | developer | no problems found; acceptance 1-5 pass, scores 5.90 to 9.60 | none |
| Tester pass | tester | no problems found: 159 functions byte-identical, names and attributes equal, 147 passed | none |
| Architect review | architect | no problems found; own run of acceptance 1 and 2, fmt, clippy, `cargo test --test confine`; one of my own comparison scripts was wrong (empty old-names list) and was redone | none |
