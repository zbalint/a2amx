# Spec 3fn: split `src/team.rs` into the team root, `directory`, `validate` and `tests` child modules (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `8abe148`, spec 3fm accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch7-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `src/team.rs`: shrinks to the root (section 2).
- `src/team/directory.rs`, `src/team/validate.rs`, `src/team/tests.rs`: new.

Does not touch: every other file under `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge. The architect updates the backlog and the review record in the accepting commit.

Public test seam: the `team` module's public functions as used by `src/main.rs` and the integration tests (`cargo test --test teams`, `--test team`, `--test auto_confine`, and the unit tests inside the crate, `cargo test --lib team::`). No test is added, removed or changed.

## 1. Why

`src/team.rs` is 1364 lines, over the owner's goal of no `.rs` file over 1,000 lines; the owner told the architect to continue the refactor with the source files (first split: spec 3fm, `src/discord.rs`). The file has four responsibilities by its own section structure: the team file model and the up-plan (types, `parse`, `merge_confine`, `plan`, `inspect`, `reset_targets`, `flag_sessions`), the checks that work from a project directory: its team name and the refusal of unsafe directories for a global team (`team_name_for`, `check_global_allowed`, `home_reason`, `system_reason`, `container_reason`, `is_mnt_drive`, lines 268-482), the per-session validation rules (`validate_activity` .. `join_with_and`, lines 928-1171) and 193 lines of inline unit tests (lines 1172-1364). Measured by script on HEAD (all names are function names of `src/team.rs`): the global-check range calls no function defined outside it, and only `team_name_for` is called from outside it (by `parse`, line 503); the validation range calls no function outside it, and only `validate_sessions` is called from outside it (by `parse`, line 554, and `flag_sessions`, line 891); the six unit tests (`cargo test --lib -- --list` shows them as `team::tests::...`) use only root items (the `TeamSession::auto_confine` method, `new_auto_grants`, `escape_notices`), none from the global or validation ranges. The functions `validate_confine_list` and `validate_confine_syntax` (589-620) are parse-time helpers of `merge_confine` and stay in the root despite their names. The module table row for `team` ("The team file and flag formats and the up-plan (pure)") stays true; the children are implementation of it.

Alternatives rejected: a `src/team/mod.rs` (the repository uses flat files; spec 3fm chose the 2018 root-file layout); merging `directory` and `validate` into one `checks` module (they share nothing and answer different questions, one about the project directory, one about each session's fields); the name `global` for the first (`team_name_for` is not a global-only check: `parse` calls it for every team file without a `team` key, and `main` calls it too, so the module is named after what its functions share, the project directory, and it does not collide with `parse`'s `global` parameter); leaving the tests inline and splitting only the code (the code split alone leaves about 905 lines, and the inline tests are the cheapest 193 lines to move).

## 2. Target shape

`src/team.rs` stays the root file and declares the children. Every public path stays: `team::{parse, plan, flag_sessions, inspect, reset_targets, new_auto_grants, escape_notices, team_name_for, check_global_allowed, validate_session_confine, ...}`.

| File | Old lines of `src/team.rs` (HEAD) | Contents |
| --- | --- | --- |
| `src/team.rs` | 1-267, 483-927, and the lines below | the imports, the types (`TeamActivity`, `TeamSession`, `ConfineTable`, `TeamFile`, `Action`, `Conflict`, `ConflictReason`, `Entry`, `EntryState`, `ResetTarget`, `ResetState`), `new_auto_grants`, `escape_notices`, `reset_targets`, `inspect`, `parse` and its helpers, `merge_confine`, `build_scope`, `session_from_table`, `resolve_agent_allow`, `resolve_references`, `flag_sessions`, `plan` (about 710 lines) |
| `src/team/directory.rs` | 268-482 | `team_name_for`, `check_global_allowed`, `home_reason`, `system_reason`, `container_reason`, `is_mnt_drive` (about 220 lines with imports) |
| `src/team/validate.rs` | 928-1171 | `validate_activity`, `activity_cycle`, `validate_sessions` (now `pub(super)`), `validate_session_confine_options`, `validate_session_remote_options`, `validate_session_confine` (`pub`), `validate_session_name`, `validate_session_command`, `validate_session_reset`, `validate_session_links`, `validate_session_heartbeat`, `validate_session_role`, `join_with_and` (about 255 lines with imports) |
| `src/team/tests.rs` | 1174-1363 (the body of `mod tests { ... }`), dedented one level | the six unit tests, with their `use super::*;`, `use std::collections::BTreeSet;`, `use crate::confine::ConfineRequest;`, `use crate::grant::Grant;`, `use crate::wire::GrantReason;` lines unchanged (about 190 lines) |

The edits, and the only edits besides the moves:

1. **Declarations in `src/team.rs`.** After the `use` lines, in this order: `mod directory;`, `mod validate;`, then `pub use directory::{check_global_allowed, team_name_for};`, `pub use validate::validate_session_confine;`, `use validate::validate_sessions;`. At the end of the file, the two lines `#[cfg(test)]` and `mod tests;` replace the old inline module (old lines 1172-1364).
2. **Visibility.** The only visibility change: `fn validate_sessions` becomes `pub(super) fn validate_sessions` (it is called from the root). Every other moved function keeps its visibility (`validate_session_confine`, `team_name_for` and `check_global_allowed` are already `pub`; the rest are private and are used only inside their own file).
3. **Imports.** Each file keeps exactly the `use` lines it needs (the compiler and clippy's `unused_imports` are the judge). The architect's grep of the moved ranges expects: `directory.rs` needs `use anyhow::bail;` and `use crate::messaging::validate_name;` (it spells `std::path::Path` out in full) and nothing from `super`; `validate.rs` needs `use super::TeamSession;`, `std::collections::{HashMap, HashSet}`, `std::path::Path`, `anyhow::bail`, `crate::harness::Harness`, `crate::harness::validate_prompt_options` and `crate::messaging::{parse_interval, validate_name, validate_reset_steps, validate_role}`. `src/team.rs` drops the names only the moved code used (`parse_interval`, `validate_reset_steps`, `validate_role`, `Harness`, `Path`), so `rustfmt` collapses its `crate::messaging` import to one line. An unused import, or a missing one, is reported by the compiler and clippy; the architect's list is a pointer, not a requirement beyond a clean build.
4. **The tests file** is the old `mod tests { ... }` body without the `mod tests {` line and the closing `}`, each line dedented by one level (`cargo fmt` does the dedent; run it once after assembly). `#[cfg(test)]` stays in `src/team.rs` above `mod tests;`.
5. **Everything else** is moved byte-for-byte, comments included, with its order inside each file unchanged. The doc comment directly above a function moves with it (`team_name_for`, line 268, starts at its `///` line).

A child module sees the root's private items (types and their private fields); the root does not see the children's private items. The only root-to-child references are the three declarations in edit 1.

## 3. What must not change

- Every public path and signature, and every behavior. No renames, no new `#[allow]`, `#[cfg]` (other than the one on `mod tests;`) or suppression, no changed `// shortcut:` comment.
- No test body changes; all six unit tests keep their names. Because they move into `team::tests`, their paths stay `team::tests::<name>`.

## 4. Out of scope

Shortening `merge_confine` (96 lines), `parse` (75), `system_reason` (67) and `activity_cycle` (61) (a later step if `cs check` still flags them); other `src/` files; the backlog, review record and changelog (the architect).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`. Create `target/scratch/3fn` first and delete it before reporting (keep it until the architect accepts).

1. **Tests.** Before editing, save `cargo test --lib -- --list 2>/dev/null | rg ': test$' | rg '^team::' | sort > target/scratch/3fn/before.txt` (6 lines expected, each `team::tests::<name>: test`) and the pass counts of `cargo test --lib`; after the split the saved list is identical (`diff` exit 0) and the counts equal. Check `before.txt` is not empty first (`2>/dev/null` hides a build failure).
2. **Same code.** `old` = `git show HEAD:src/team.rs`, `new` = `cat src/team.rs src/team/directory.rs src/team/validate.rs src/team/tests.rs`. After `sed '/^use .*{$/,/^};$/d; s/^[[:space:]]*//; /^$/d; /^\(use \|mod \|pub use \)/d'` and `sort` (the first expression deletes the one multi-line `use` block at column 0, so rustfmt's collapse of it does not show), `diff old new` shows exactly these three differences and nothing else: `< fn validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()> {`, `> pub(super) fn validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()> {`, and one removed `}` (the closing brace of the old `mod tests`). The architect ran this on a scripted rebuild of the section 2 ranges before locking and it produced exactly these three lines (with the multi-line `use` block deleted from both sides).
3. **Order inside each file.** The architect rebuilds the four files from the section 2 ranges and compares them in order with the developer's; the tester repeats this independently.
4. `wc -l src/team.rs src/team/*.rs`: every file under 1,000. `cs check` on each new file and on the old file (saved as `target/scratch/3fn/team.rs`, same basename, no "test" in the path); report the scores. `tests.rs` is judged against the old file's inline tests only by the architect, because `cs check` scores by path.
5. `cargo build --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; the whole `cargo test` (61 suites) passes. `rg -n 'team::' src tests` resolves unchanged (no edit outside scope).
6. `git status --short` shows only ` M src/team.rs` and `?? src/team/`.

Known flaky tests: `tests/team/reset_and_validation.rs::team_up_forwards_reset_configuration_and_consent` ("screen not ready", backlog I28, unrelated). A failure of it is rerun once and recorded as pre-existing if it passes.
