# Spec 3fq: split `src/confine.rs` into the confine root and `builder`, `hides`, `harness_data`, `grant_dirs`, `git`, `wrap`, `bwrap` and `launch` child modules (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `a9f7415`, spec 3fp accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch8-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `src/confine.rs`: shrinks to the root (section 2).
- `src/confine/builder.rs`, `src/confine/hides.rs`, `src/confine/harness_data.rs`, `src/confine/grant_dirs.rs`, `src/confine/git.rs`, `src/confine/wrap.rs`, `src/confine/bwrap.rs`, `src/confine/launch.rs`: new.

Does not touch: every other file under `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `docs/`, `CHANGELOG.md`, `.semgrep/`, `scripts/`, `AGENTS.md`. No semgrep rule, script or workflow names `src/confine.rs` (checked, section 6). Do not commit, stage or merge. The architect updates the backlog (the `src/confine.rs` pointers in U9 (`:97-102`, now `hides.rs`), U10 (`grant_dirs.rs`), F52 (`harness_data.rs`), F69 and F76 (`builder.rs`) and F89 (`wrap.rs`, to be checked)) and the review record in the accepting commit.

Public test seam: the whole suite, above all `tests/confine/*` (13 files, about 150 `confine::plan` and related calls), `tests/daemon/confine.rs`, `tests/team/*` and `tests/relay/sync_and_members.rs` (`cargo test`). The ten unit tests of `src/confine.rs` move with the code they test (section 2); no test is added, removed or changed apart from the module paths of six of them.

## 1. Why

`src/confine.rs` is 2,849 lines, over the owner's goal of no `.rs` file over 1,000 lines (CodeScene `cs check` 2.65); the owner chose it as the first of the three remaining large source files (`src/main.rs`, `src/daemon.rs`, `src/confine.rs`) after specs 3fm to 3fp. It holds nine responsibilities that are already separate groups of functions with few crossings:

- the plan's data and shared rules (the request, host and plan types, the write and unhide allow checks, the path helpers) stay in the root;
- building the plan (`PlanBuilder`: binding the workspace, operator writes and protected paths, `finish`, and the `plan`, `plan_with_path`, `plan_with_preflight` entry points);
- the hide rules (default hides, container sockets, grant unhides and stand-ins, operator hides, `unhide`);
- harness data (what each harness may write, overlay parents, `ensure_data_dirs`, `ensure_data_files`, the harness-data and harness-config binds);
- grant directories (`ensure_grant_dirs` and its scope and problem checks);
- git metadata (`git_common_dir`, the `core.hooksPath` parse and the pointer readers);
- the Plan's runtime half (`impl Plan`: mounts and the `MountKind` and `Mount` model, environment unsets, the bubblewrap argument vector, accessors);
- the bubblewrap probes (`resolve_bwrap`, `preflight*`, `supports_tmp_overlay`) and the host-side empty-file stand-in (`ensure_empty_file`, called by the daemon in the same preflight sequence);
- the text the operator and the process see (`notice`, `dry_run_lines`, `launch_argv`).

`impl PlanBuilder` alone is 588 lines (1304-1891) and `plan_with_preflight` orchestrates it from the entry-point functions.

Measured by script on HEAD (every `fn`, `const`, `static`, `struct` and `enum` name of the file, checked for definition and use across the groups, method names matched as `.name(`): the root's items (types, constants, path helpers, `Plan` fields, `PlanBuilder` fields) are private but visible to every child, so most cross-uses need no edit; exactly 17 private items are called across a child boundary and become `pub(super)` (section 2); every other cross-use is of an item that is `pub` or stays in the root. The names other modules reach through `confine::` (`rg` on `src` and `tests`) are all `pub` or `pub(crate)` and are re-exported where they move (section 2).

Alternatives rejected: a `src/confine/mod.rs` (the repository uses the 2018 root-file layout, as in specs 3fm to 3fp); one `builder.rs` holding the hide rules too (about 850 lines, and the hide rules are their own responsibility with their own constants and tests; the price is seven `pub(super)` on the hide steps); moving the path helpers and allow checks into a child (they are used by three to five children each, so each would become `pub(super)` for no gain; they are the module's shared rules and stay in the root; `MountKind`, `Mount` and `path_depth` are the exception because only `wrap.rs` uses them); keeping the unit tests in one `src/confine/tests.rs` (six of their private subjects, `NOTICE_LIMIT`, `DEFAULT_HIDES`, `DEFAULT_HIDES_EXTRA`, `SocketRow` with its fields and `container_socket_hides`, would all need `pub(super)`; each test stays next to its subject instead); splitting `finish` (104 lines), `apply_grant_unhides` (70 lines) or `plan_with_preflight` (65 lines) (a later step, not a move).

## 2. Target shape

The root `src/confine.rs` stays the root and declares the children. Methods of `impl PlanBuilder` that move go into `impl<'a> PlanBuilder<'a> { ... }` blocks in the child (three children hold one: `builder`, `hides`, `harness_data`; the struct and its fields stay in the root, which a child can read). The `impl Plan` block moves whole to `wrap.rs`; its fields stay in the root.

| File | Old lines of `src/confine.rs` (HEAD) | Contents |
| --- | --- | --- |
| `src/confine.rs` | 1-28, 33-76, 109-221, 238-266, 271-416, 998-1016, 1215-1236, 1892, 1906-1914, 2572-2575, 2673-2772, 2849, plus the declarations below (about 515 lines) | module doc, imports, `MAX_LIST_ENTRIES`, `CODEX_SOCKET_SUBDIR`, `MAX_PATH_BYTES`, `ENV_PATH`, `CLOSING_NOTICE`, `ALWAYS_UNSET`, `CREDENTIAL_ENV`, `MANDATORY_ROOTS`, `ConfineRequest` and its `impl`, `Unresolved`, `check_entry_syntax`, `Host`, `Plan` (the struct), the path helpers and allow checks (`canonical_existing` .. `add_unique` except `path_depth`, including `check_write_allowed`, `check_unhide_allowed`), `add_canonical_existing`, the `PlanBuilder` struct, `granted_env`, and the test module with the four tests `grant_socket_and_environment_names_are_unique`, `apply_auto_keeps_requested_grants_and_records_only_additions`, `apply_auto_sets_auto_when_no_grant_is_added`, `explicit_grants_exclude_auto_added_grants` |
| `src/confine/builder.rs` | 979-997, 1017-1032, 1090-1214, 1304-1330, 1341-1375, 1435-1521, 1757-1891, 1893-1905 | `plan_empty_file`, `add_validated_write`, `plan`, `plan_with_path`, `plan_with_preflight`, `check_request_limits`, then one `impl<'a> PlanBuilder<'a>` block with `new`, `bind_workspace`, `protect_git_metadata`, `resolve_operator_path`, `bind_operator_writes`, `protect_system_paths`, `check_implicit_writes_not_hidden`, `finish`, then `protected_paths` (about 460 lines with imports) |
| `src/confine/hides.rs` | 77-108, 1237-1303, then `impl<'a> PlanBuilder<'a> {` + 1522-1756 + `}`, then the test block (below) with old lines 2613-2672 | `DEFAULT_HIDES`, `DEFAULT_HIDES_EXTRA`, `SocketRow`, `container_socket_hides`, `default_hide_names`, `default_hide_sources_for`, and the methods `hide_mandatory`, `default_hide_sources`, `hide_defaults`, `check_grant_path_writable`, `apply_grant_unhides`, `apply_grant_stand_ins`, `hide_containers`, `hide_or_skip`, `check_container_socket`, `release_operator_writes`, `apply_operator_hides`, `unhide`; the tests `granted_socket_target_wins_over_an_ungranted_alias` and `credential_grant_unhides_are_default_hide_sources` (about 400 lines) |
| `src/confine/harness_data.rs` | 417-687, 812-844, 1033-1089, then `impl<'a> PlanBuilder<'a> {` + 1331-1340 + 1376-1434 + `}` | `harness_data`, `harness_overlay_parents`, `overlay_protected_entries`, `unsaved_harness_files`, `validate_harness_data_paths`, `ensure_data_dirs`, `ensure_data_files`, `add_harness_data_mounts`, and the methods `session_harness_root`, `bind_harness_data`, `bind_harness_config` (about 435 lines) |
| `src/confine/grant_dirs.rs` | 688-811 | `GrantDirScope`, `grant_dir_scope`, `grant_dir_problem`, `ensure_grant_dirs` (about 125 lines) |
| `src/confine/git.rs` | 845-978 | `parse_core_hooks_path`, `POINTER_LIMIT`, `GIT_CONFIG_PLAN_LIMIT`, `read_pointer_text`, `is_git_dir_shape`, `git_common_dir` (about 135 lines) |
| `src/confine/wrap.rs` | 29-30, 222-237, 267-270, 1915-2163 | `MAX_ARG_BYTES`, `MAX_ARGV_BYTES`, `MountKind`, `Mount` and `path_depth` (the bubblewrap mount model; used only by this file's `Plan::mounts` and `wrap_with_codex_bind`), the whole `impl Plan` block (about 270 lines) |
| `src/confine/bwrap.rs` | 32, 2164-2345 | `PROBE_TIMEOUT`, `command_with_timeout`, `find_executable`, `resolve_bwrap`, `preflight_args`, `preflight`, `supports_tmp_overlay`, `preflight_env`, `ensure_empty_file` (about 185 lines) |
| `src/confine/launch.rs` | 31, 2346-2571, then the test block (below) with old lines 2576-2612 and 2773-2848 | `NOTICE_LIMIT`, `capped_paths`, `truncate_before_closing`, `dry_run_lines`, `notice`, `ctrl_c_wrapper`, `launch_argv`; the tests `closing_notice_is_always_present`, the helper `dry_run_plan`, and the three `dry_run_lines_*` tests (about 345 lines) |

The ranges, the root's complement and the added lines partition lines 1-2849 exactly (checked by script on the architect's scratch rebuild, together with the expected diff in section 5). Lines 1304 (the `impl<'a> PlanBuilder<'a> {` opener) and 1891 (its closing `}`) are `builder.rs`'s; `hides.rs` and `harness_data.rs` each get their own opener and closing `}`, which are the only added `impl` lines. A method range starts on the first attribute or comment line of its method. The test blocks: the root keeps old lines 2572-2575 (`#[cfg(test)]`, `mod tests {`, `use super::*;`, a blank line), the four tests above and the closing `}` (2849). `hides.rs` and `launch.rs` each end with their own `#[cfg(test)] mod tests { use super::*; ... }` block holding the tests named in the table, moved byte-for-byte; a `use` that only a test needs goes inside that `mod tests`, so the non-test build has no unused import.

The edits, and the only edits besides the moves:

1. **Declarations in `src/confine.rs`.** After the `use` lines: `mod builder;`, `mod bwrap;`, `mod git;`, `mod grant_dirs;`, `mod harness_data;`, `mod hides;`, `mod launch;`, `mod wrap;`, a blank line, then these `pub use` lines, one per module, so every public path stays: `pub use builder::{plan, plan_with_path, plan_with_preflight};`, `pub use bwrap::{ensure_empty_file, find_executable, preflight, preflight_env, resolve_bwrap, supports_tmp_overlay};`, `pub use git::git_common_dir;`, `pub use grant_dirs::ensure_grant_dirs;`, `pub use harness_data::{ensure_data_dirs, ensure_data_files};`, `pub use hides::default_hide_names;`, `pub use launch::{dry_run_lines, launch_argv, notice};`. `impl Plan`'s public methods need no re-export.
2. **Visibility.** Exactly 17 private items become `pub(super)`, because a sibling child calls them. The signature text is otherwise unchanged; `cargo fmt` may wrap three of them (section 5, acceptance 2). In `harness_data.rs`: `harness_data`, `harness_overlay_parents`, `overlay_protected_entries`, `unsaved_harness_files` (called by `finish`), `bind_harness_data` and `bind_harness_config` (called by `plan_with_preflight`), `session_harness_root` (called by `check_grant_path_writable`). In `git.rs`: `parse_core_hooks_path` (called by `protect_git_metadata`). In `hides.rs`: `default_hide_sources_for` (called by `grant_dir_scope`), `hide_mandatory`, `hide_defaults`, `hide_containers`, `apply_grant_unhides`, `apply_grant_stand_ins`, `release_operator_writes`, `apply_operator_hides` (all called by `plan_with_preflight`). In `builder.rs`: `resolve_operator_path` (called by `apply_operator_hides` and `unhide`). No other visibility changes; any further private item the compiler reports across a boundary is a BLOCKED, not a free change.
3. **Imports.** Each file keeps exactly the `use` lines it needs (the compiler and clippy's `unused_imports` are the judge); a child names the root items it uses through `use super::{...}` and a sibling's through `use super::<module>::...`. The root drops the imports only moved code used. Keep a test-only `use` on one line (the `sed` of acceptance 2 deletes a `use` block only from column 0, so a wrapped indented block inside `mod tests` would leave continuation lines in the diff).
4. **Everything else** is moved byte-for-byte, comments and `// shortcut:` comments included, with the relative order inside each file unchanged. `cargo fmt` runs once after assembly.

## 3. What must not change

- Every public path and signature, and every behavior. No renames, no new `#[allow]`, `#[cfg]` (apart from the two `#[cfg(test)]` of section 2) or suppression.
- No test file under `tests/` changes. The names of six unit tests change with their module: `confine::tests::granted_socket_target_wins_over_an_ungranted_alias` and `confine::tests::credential_grant_unhides_are_default_hide_sources` become `confine::hides::tests::...`; `confine::tests::closing_notice_is_always_present` and the three `confine::tests::dry_run_lines_*` become `confine::launch::tests::...`.
- The `tracing::debug!` event of `resolve_operator_path` moves to `builder.rs`, so its default target becomes `a2amx::confine::builder` instead of `a2amx::confine`; the one in `add_canonical_existing` stays in the root. This is accepted under Amendment 1 of spec 3fp (log target text is not public interface; a `RUST_LOG` directive for `a2amx::confine` still matches by prefix; no test asserts on a target). No explicit `target:` is added.
- No `unsafe` exists in the file (checked), so no `// SAFETY:` comment moves.
- The security checks stay where they are in the root (`check_write_allowed`, `check_write_runtime_allowed`, `check_unhide_allowed`, `check_entry_syntax`, `check_absolute_entry`, `check_entry_count`) and every caller still passes through them; no check is duplicated or moved behind a child boundary.

## 4. Out of scope

Shortening `finish`, `apply_grant_unhides`, `plan_with_preflight`, `wrap_with_codex_bind` or `notice`; moving the allow checks or path helpers out of the root; moving the unit tests to `tests/`; the AGENTS.md module table (it has no `confine` row; a docs task); other `src/` files; the backlog, review record and changelog (the architect).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`. Create `target/scratch/3fq` first and keep it until the architect accepts; delete nothing the architect made there.

1. **Baseline.** Before editing: save `cargo test -- --list 2>/dev/null | rg ': test$' | LC_ALL=C sort > target/scratch/3fq/before.txt` (check it is not empty), the totals of a full `cargo test --no-fail-fast` (the architect's baseline at `a9f7415`: 61 suites, 1347 passed, 0 failed, 3 ignored, exit 0), `rg -n 'confine::' src tests | LC_ALL=C sort > target/scratch/3fq/uses-before.txt`, and `git show HEAD:src/confine.rs` as `target/scratch/3fq/confine.rs`. After the split: the sorted list differs from `before.txt` only in the six renamed unit tests of section 3 (`diff` shows exactly those six removed lines and six added lines), the totals are equal, and `rg -n 'confine::' src tests | LC_ALL=C sort` diffs empty against `uses-before.txt` (no `src` or `tests` line changes).
2. **Same code.** `old` = `git show HEAD:src/confine.rs`, `new` = `cat src/confine.rs src/confine/builder.rs src/confine/hides.rs src/confine/harness_data.rs src/confine/grant_dirs.rs src/confine/git.rs src/confine/wrap.rs src/confine/bwrap.rs src/confine/launch.rs`. After `sed '/^\(pub \|pub(crate) \|pub(super) \)\?use .*{$/,/^};$/d; s/^[[:space:]]*//; /^$/d; /^\(use \|mod \|pub use \|pub(crate) use \|pub(super) use \)/d'` and `LC_ALL=C sort`, `diff old new` (after `cargo fmt`) shows exactly the lines of the block below and nothing else. The architect ran this on a scripted rebuild of the section 2 ranges, formatted with the repository's `rustfmt.toml`, before locking: removed lines are the 17 old signature lines; added lines are the 17 `pub(super)` forms, two `#[cfg(test)]`, two `impl<'a> PlanBuilder<'a> {`, four `}`, and the lines that `cargo fmt` produces by wrapping three of the new signatures (`apply_grant_unhides`, `resolve_operator_path`, `unsaved_harness_files`):

```text
< fn apply_grant_stand_ins(&mut self, given: &[String]) {
< fn apply_grant_unhides(&mut self, given: &[String], no_default_hides: bool) -> Result<()> {
< fn apply_operator_hides(&mut self, request: &ConfineRequest) -> Result<()> {
< fn bind_harness_config(&mut self, grants: &[String]) -> Result<()> {
< fn bind_harness_data(&mut self, request: &ConfineRequest) -> Result<()> {
< fn default_hide_sources_for(home: &Path, harness: Harness) -> Vec<PathBuf> {
< fn harness_data(harness: Harness, home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
< fn harness_overlay_parents(
< fn hide_containers(&mut self, given: &[String]) {
< fn hide_defaults(&mut self, request: &ConfineRequest) {
< fn hide_mandatory(&mut self) -> Result<()> {
< fn overlay_protected_entries(
< fn parse_core_hooks_path(common: &Path, worktree: &Path) -> Result<Option<PathBuf>> {
< fn release_operator_writes(&mut self) -> Result<()> {
< fn resolve_operator_path(&mut self, option: &'static str, source: &Path) -> Option<PathBuf> {
< fn session_harness_root(&self) -> Option<PathBuf> {
< fn unsaved_harness_files(data_files: &[PathBuf], overlay_parents: &[PathBuf]) -> Vec<PathBuf> {
> #[cfg(test)]
> #[cfg(test)]
> &mut self,
> &mut self,
> ) -> Option<PathBuf> {
> ) -> Result<()> {
> ) -> Vec<PathBuf> {
> data_files: &[PathBuf],
> given: &[String],
> impl<'a> PlanBuilder<'a> {
> impl<'a> PlanBuilder<'a> {
> no_default_hides: bool,
> option: &'static str,
> overlay_parents: &[PathBuf],
> pub(super) fn apply_grant_stand_ins(&mut self, given: &[String]) {
> pub(super) fn apply_grant_unhides(
> pub(super) fn apply_operator_hides(&mut self, request: &ConfineRequest) -> Result<()> {
> pub(super) fn bind_harness_config(&mut self, grants: &[String]) -> Result<()> {
> pub(super) fn bind_harness_data(&mut self, request: &ConfineRequest) -> Result<()> {
> pub(super) fn default_hide_sources_for(home: &Path, harness: Harness) -> Vec<PathBuf> {
> pub(super) fn harness_data(harness: Harness, home: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
> pub(super) fn harness_overlay_parents(
> pub(super) fn hide_containers(&mut self, given: &[String]) {
> pub(super) fn hide_defaults(&mut self, request: &ConfineRequest) {
> pub(super) fn hide_mandatory(&mut self) -> Result<()> {
> pub(super) fn overlay_protected_entries(
> pub(super) fn parse_core_hooks_path(common: &Path, worktree: &Path) -> Result<Option<PathBuf>> {
> pub(super) fn release_operator_writes(&mut self) -> Result<()> {
> pub(super) fn resolve_operator_path(
> pub(super) fn session_harness_root(&self) -> Option<PathBuf> {
> pub(super) fn unsaved_harness_files(
> source: &Path,
> }
> }
> }
> }
```

   The comparison is of whole lines (a `diff` of the two sorted files, ignoring the line numbers `diff` prints); `mod` lines are deleted by the `sed`, so the three `mod tests {` and the eight `mod` declarations do not appear.
3. **Order inside each file.** The architect rebuilds the nine files from the section 2 ranges and compares in order with the developer's; the tester repeats this independently.
4. `wc -l src/confine.rs src/confine/*.rs`: every file under 1,000. `cs check` on each new file and on the old copy; report the scores.
5. `cargo build --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; the whole `cargo test` passes; no new warning. The semgrep rules (`scripts/prepush-fast.sh`, run by the architect) pass.
   Acceptance 2 compares the multiset of lines and cannot see a line in the wrong file or position, so also: `rg -c '^impl<.a> PlanBuilder<.a> \{' src/confine/*.rs` prints 1 for each of `builder.rs`, `hides.rs` and `harness_data.rs` and no other file prints; `rg -c '^impl Plan \{' src/confine/wrap.rs` prints 1; `rg -c '^    #\[test\]' src/confine.rs src/confine/hides.rs src/confine/launch.rs` prints 4, 2 and 4 (the ten tests); and acceptance 3 is the structural check.
6. `git status --short` shows only ` M src/confine.rs` and `?? src/confine/`.

Known flaky tests: `tests/team/reset_and_validation.rs::team_up_forwards_reset_configuration_and_consent` (backlog I28) and `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error` (backlog I29), both unrelated. A failure of either is rerun once and recorded as pre-existing if it passes.

## 6. Claims checked before locking

| Claim | Where | Verified how |
| --- | --- | --- |
| `src/confine.rs` is 2,849 lines; `impl PlanBuilder` is lines 1304-1891 and `impl Plan` is 1915-2162 | `wc -l`, `src/confine.rs` | read |
| Every item range of section 2 partitions the file (no overlap, no gap); the cross-child private calls are exactly the 17 of section 2.2 | architect scratch script over `git show HEAD:src/confine.rs` (item ranges, name-based cross-reference with method calls matched as `.name(`) | probed |
| The only external users of `confine::` are `src/{wire,team,session,main,daemon,codex,gateway_launch}.rs`, `src/session/spawn.rs`, `src/team/tests.rs` and `tests/**`; all names they use are `pub` | `rg -n 'confine::' src tests` | probed |
| The root's private items and fields are visible to children (types, `Plan` and `PlanBuilder` fields, constants, helpers) | spec 3fp's `src/session/spawn.rs` builds the private root struct `Shared` through `use super::{..., Shared, ...}` | read |
| `pub use` re-export keeps a moved public path | `src/team.rs:15-16`, `src/config.rs:18-22`, `src/session.rs:27-28` | read |
| No semgrep rule, script or workflow names `src/confine.rs` | `rg -n 'confine' .semgrep scripts .github` printed nothing | probed |
| `src/confine.rs` has no `unsafe` | `rg -c unsafe src/confine.rs` printed nothing | probed |
| Two `tracing::debug!` events: line 1011 (`add_canonical_existing`, stays in the root) and line 1449 (`resolve_operator_path`, moves to `builder`) | `rg -n 'tracing::' src/confine.rs` | read |
| The ten unit tests use these private items: `NOTICE_LIMIT`, `ALWAYS_UNSET`, `CREDENTIAL_ENV`, `DEFAULT_HIDES`, `DEFAULT_HIDES_EXTRA`, `SocketRow`, `container_socket_hides`, `Plan` fields, `ConfineRequest` | cross-reference script over lines 2572-2849; the test placement in section 2 puts each next to its subject | probed |
| Three of the 17 new `pub(super)` signatures exceed 100 columns, so `cargo fmt` wraps them | rustfmt run on the scratch rebuild with the repository's `rustfmt.toml` (edition 2024) | probed |
| The ten unit tests are listed by `cargo test --lib -- --list` as `confine::tests::<name>` | architect run at `a9f7415` | probed |
| No other existing test pins the module path of the six renamed unit tests | `rg -n 'confine::tests' tests src docs` found no use outside this spec | probed |
| Baseline at HEAD: full `cargo test --no-fail-fast` result | architect run at `a9f7415`, output in `target/scratch/3fq/baseline-full.txt`: 61 suites, 1347 passed, 0 failed, 3 ignored | probed |

Trust boundaries: the change moves code and widens visibility only inside the crate, so it creates no attack path; the allow checks that enforce the confinement's limits stay in one place in the root and are called by the same callers, as section 3 says. The existing `tests/confine/*` abuse-case tests (they reject writes and unhides outside the allowed areas) are the contract tests; all pass on the old code and must pass unchanged.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review and design read; model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots. No tester pass: the tester's weekly quota was 6%, so the architect ran six mutants of its own on the checkout (restored by checksum).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | `MountKind`, `Mount` and `path_depth`, used only by `wrap.rs`, sat in the root | moved to `wrap.rs`; no visibility change, expected diff unchanged |
| Draft review | consultant | the claim about external users of `confine::` omitted `src/codex.rs`; section 1 called `ensure_empty_file` a probe; backlog pointers beyond U9 were stale | claim and wording corrected, pointers updated by the architect |
| Draft review | consultant | a wrapped test-only `use` block inside `mod tests` would leave continuation lines in the normalized diff | rule added: keep test-only `use` lines on one line |
| Implementation | developer | no problems found | none |
| Architect review | architect | six mutants on moved code: five killed (oversized git config limit, Codex harness data, `ALWAYS_UNSET`, `paths_overlap` in the hide rules, `NOTICE_LIMIT`); one survivor in committed code: `PROBE_TIMEOUT` 5 s to 1 ms in `bwrap.rs`, which the confine, daemon and team tests do not catch | recorded in backlog F94, not fixed (out of scope for a move) |
| Design read | consultant | three optional follow-ups: `ensure_empty_file` sits in `bwrap.rs` but belongs next to `plan_empty_file`; `finish` pulls four `harness_data` functions only for overlay state; the `PlanBuilder` doc comment could name its impl locations | recorded in backlog F94, not changed |
| Architect review | architect | no problems found in the diff: code bodies equal the scripted rebuild of the section 2 ranges; the sorted unit-test list differs only in the six renamed tests; fmt, clippy `-D warnings`, full `cargo test` (61 suites, 1347 passed, 3 ignored) pass; `cs check` old file 2.65, root 8.67, builder 7.36, hides 8.24, harness_data 7.07, grant_dirs 8.28, git 8.52, wrap 8.45, bwrap 8.67, launch 9.07 | none |
