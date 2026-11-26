# Spec 3hg: confined launch never changes the mode of a harness data directory or sidecar that already exists

## 0. Status

LOCKED after consultant reviews m_9311 and m_9314 (all findings applied; the second pass found no blocker, so no third review). Context `a2amx-3hg-data-dir-chmod`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Public seams: `confine::ensure_data_dirs` and `confine::ensure_data_files` (both re-exported in `src/confine.rs:33` and already driven by `tests/confine/harnesses.rs`).

Scope (edit): `src/confine/harness_data.rs` (`secure_data_dir`, `ensure_sidecars`, and the `use` line that loses `PermissionsExt` if nothing else needs it) and `tests/confine/harnesses.rs` (the one existing test and the six new tests named in section 3). Does not touch: `src/daemon/launch.rs` (the call order stays), `src/confine/bwrap.rs`, `src/confine/grant_dirs.rs`, `docs/`, `CHANGELOG.md` (the architect adds the line when committing), any other test.

The spec records the owner's decisions of 2026-11-26 (direct, in the session): changing the permissions of directories a2amx did not create is wrong ("we should not change random dirs permissions"); creating a missing directory with a private mode is fine; existing users and their workflow must not break; the dead-overlay removal and the `status.rs` split (spec 3he) follow this spec. Additions the owner did not raise: a dangling link at a data path is left to the plan's own refusal instead of failing in `ensure_data_dirs` with a raw `AlreadyExists` (section 2, rule 3); the sidecar rule is changed together with the directory rule.

## 1. Why

A confined session start runs, in the daemon and outside the sandbox, `confine::ensure_data_dirs` and `ensure_data_files` (`src/daemon/launch.rs:157-158`). They exist for a good reason: the sandbox opens a few harness data directories (`projects`, `cache`, ... under `~/.claude`) for writing, a bind-mount source must exist, and a read-only parent would stop the harness creating the directory itself (spec 3ax). For OMP the sandbox also needs empty SQLite `-wal` and `-shm` sidecars, because SQLite cannot create them in a read-only directory on a cold start.

The code does more than that. `secure_data_dir` (`harness_data.rs:303-317`) calls `fs::set_permissions(directory, 0o700)` on a directory that already exists, and `ensure_sidecars` (`:334-347`) does `0o600` on an existing sidecar. Both follow symlinks. Two consequences:

1. a2amx changes the mode of a directory the user or the harness made, on every confined start, with no consent and no need.
2. It does so before the write guards run: `launch.rs:157` (`ensure_data_dirs`) comes before `launch.rs:160` (`plan_with_preflight`), and the guards that refuse a data path resolving outside its harness root (`tests/confine/harnesses.rs:497`) or dangling (`:910`) live in the plan. A data path that is a link to another directory is chmodded to 0700, and the plan refuses it afterwards. Spec 3ax says `ensure_data_dirs` "never creates or chmods through such a link" (`docs/specs/spec-3ax-confine.md:504`); the call order makes that sentence false.

Rule after this spec: a2amx creates a missing data directory with mode 0700 and never changes the mode of anything that exists. Whatever has to be refused about a link stays the plan's job, as today.

Alternatives rejected: refusing every symlinked data path in `ensure_data_dirs` (duplicates the plan's guard and refuses a link that resolves inside its root, which the plan accepts); moving `ensure_data_dirs` after the plan (the plan reads which data directories exist, so it must run first); `create_dir_all` followed by a chmod only when we created it (the check-then-chmod window can land on a link that appears between the two calls; creating with the mode in one call has no such window).

## 2. `src/confine/harness_data.rs`

`secure_data_dir(directory: &Path) -> Result<()>` becomes three rules, in this order:

1. Nothing exists at the path (`fs::symlink_metadata` returns `NotFound`): create the missing parents with `fs::create_dir_all(parent)` as today (their mode stays what the process umask gives, as today; a parent that is itself a dangling link still fails with the raw error, as today), then create the leaf with `std::fs::DirBuilder::new().mode(0o700).create(directory)` (`DirBuilderExt`). An `AlreadyExists` error from that call means something appeared in between: evaluate rules 2 and 3 once on the path and create nothing (if that evaluation sees `NotFound` again, return `Ok(())`). This branch cannot be reached through the public seam and is untested on purpose. Any other error is returned.
2. Something exists and resolves to a directory (`fs::metadata` succeeds and `is_dir()`: a real directory, or a link to one): return `Ok(())` and change nothing, no `set_permissions`, no `create_dir_all`.
3. Something exists and does not resolve to a directory: if `fs::metadata` fails with `NotFound` (a dangling link), return `Ok(())` and change nothing; the plan refuses it with its existing text (`harness data path cannot be canonicalized: ...`). If `fs::metadata` fails with any other error (a link loop, a target whose path cannot be searched), return that error unchanged. If it succeeds and the target is not a directory, return the existing error, text unchanged: `harness data path is not a directory: {path}`.

Any `symlink_metadata` error other than `NotFound` is returned.

`ensure_sidecars(database: &Path)`: for each of the `-wal` and `-shm` paths, create it with `OpenOptions::new().write(true).create_new(true).mode(0o600).open(sidecar)`. On `AlreadyExists`: if `fs::metadata(sidecar)` succeeds the file (or the link's target) is left exactly as it is; if it fails (a dangling or looping link) return the `AlreadyExists` error, so a launch that failed before still fails loudly. Any other error is returned. Delete the `exists()` check and the `fs::set_permissions` line. Keep the why-comments to one line each: why the sidecars exist (SQLite cannot create them in a read-only directory) and why an existing one is not touched. No comment in the file may contain the text `create_dir_all` or `set_permissions` (the acceptance greps count comments too).

`use` line (`:4`): add `DirBuilderExt`, remove `PermissionsExt` (nothing else in the file uses it after this change), keep `OpenOptionsExt`.

The mode of a created directory or sidecar is the mode given here minus the process umask bits, where the old code's `set_permissions` gave exactly 0700 and 0600 under any umask. A umask can only remove bits, so a created entry is never less private than before; a umask that clears owner bits (unusual) now yields an unusable entry where the old code gave a usable 0700 or 0600. Accepted.

`ensure_data_dirs` and `ensure_data_files` keep their signatures, their root checks (`data_root` returns `None` for Codex; the harness root must be a directory) and their loops.

Placement: both functions already belong to the module that owns harness data setup; no new helper, no new module.

## 3. `tests/confine/harnesses.rs`

Existing tests that pin the old behaviour (found by `rg -n 'set_permissions|0o700|0o600|0o755|0o644' tests/confine/harnesses.rs`): `ensure_data_dirs_and_files_pin_errors_and_permissions` (`:1096`) asserts an existing 0755 directory becomes 0700 and an existing 0644 sidecar becomes 0600. Change exactly those two assertions to the new rule (0755 stays 0755, 0644 stays 0644) and the comments above them; keep its not-a-directory error assertion and its `.db` directory assertions unchanged. `ensure_data_dirs_creates_agy_data_only_when_root_exists` (`:451`) stays as it is: it pins creation at 0700 and passes before and after.

New tests, in `tests/confine/harnesses.rs`, through `ensure_data_dirs` and `ensure_data_files` only, expected values as literals:

| Test | Setup | Expected | Red on old code? |
| --- | --- | --- | --- |
| `existing_data_dir_symlinked_elsewhere_keeps_its_target_mode` | Agy fixture; `~/.gemini/antigravity-cli` is a symlink to a 0755 directory outside `~/.gemini` | `Ok`; the target is still 0755; the link is still a link | yes (old chmods the target) |
| `dangling_data_dir_symlink_is_left_for_the_plan` | same path is a symlink to a path that does not exist | `Ok`; the link is still a dangling link; nothing was created at its target | yes (old returns `AlreadyExists`) |
| `missing_data_dir_is_created_private_with_default_parents` | OMP fixture with `~/.omp` present and `~/.omp/agent` absent; a data directory under `agent` is listed | the leaf is 0700; its parent `agent` exists (its mode is not asserted) | no (contract test) |
| `sidecar_symlinked_to_a_file_keeps_that_files_mode` | OMP fixture with `agent/a.db`; `a.db-wal` is a symlink to an existing 0644 file outside the harness root | `Ok`; the target file is still 0644; the link is still a link | yes (old chmods the target to 0600) |
| `dangling_sidecar_symlink_still_fails_the_launch` | OMP fixture with `agent/a.db`; `a.db-wal` is a symlink to a path that does not exist | `Err` (the `AlreadyExists` error); nothing was created at the link's target | no (contract test: old code fails the same way) |
| `data_dir_symlink_loop_still_fails` | Agy fixture; `~/.gemini/antigravity-cli` is a symlink to itself | `Err` (assert `is_err()` only: the error kind changes from `AlreadyExists` to a loop error) | no (contract test: old code fails too) |

Creation of missing sidecars at 0600 is already pinned by `omp_sidecars_cover_existing_agent_and_stats_databases_only` (`:563`, mode asserts at `:568-575`); it stays unchanged, and no new test duplicates it.

## 4. Out of scope

- `src/daemon/launch.rs` call order and the plan's guards. Whether a data path that is a link into the a2amx state directory should be refused before any directory is created is a separate question: with this spec no mode is changed, but a missing directory under a symlinked parent (`~/.omp/agent` linked into the state directory, `tests/confine/harnesses.rs:497`) is still created through that link before the plan refuses it. Follow-up for the owner.
- `src/confine/bwrap.rs:191` (`ensure_empty_file`): a2amx's own stand-in file under its own state directory, 0400, not a user path.
- `src/confine/grant_dirs.rs:124`: `create_dir_all` on a grant path after `grant_dir_problem` has refused links, and no chmod.
- Removing the dead overlay machinery in `harness_data.rs` and the `status.rs` split: separate specs, in that order after this one.
- A flag or config key to keep the old tightening. A directory with a mode the harness cannot use inside the sandbox makes the harness fail with its own permission-denied or read-only error (for example an existing directory without the owner write bit, which the old chmod repaired), and the user fixes it. The owner has accepted this narrowing.

## 5. Trust boundary

Attack path: a symlink at a harness data path (made by the user, a tool, or an agent with `harness-config`) makes the unconfined daemon change the mode of a directory it did not create (`~/.claude/projects` linked to `$HOME` would set `$HOME` to 0700), (or a sidecar link to a file such as `~/.bashrc` would set that file to 0600), or fails the launch with a raw error. Disposition: removed. No `set_permissions` remains on either path, and creation happens only when nothing is at the path (rule 1; `create_new` for sidecars, which does not follow a link). Abuse-case tests: rows 1, 2 and 4 of the section 3 table (a data directory link's target keeps its mode; a dangling data directory link creates nothing; a sidecar link's target file keeps its mode). Residual, constrained but not removed: directories created under a symlinked parent before the plan refuses it (section 4, first bullet).

## 6. Compatibility

No public interface changes: no CLI flag, config or team-file key, wire message, database schema or MCP tool changes. The one behaviour change is that an existing data directory or sidecar keeps its mode instead of being set to 0700 or 0600 at each confined start. The architect adds one `Changed` line under Unreleased. A confined start that worked before still works: every case that passed `ensure_*` before (real directory, link to a directory, missing directory, OMP database with or without sidecars) passes now. Three cases change result: a dangling link at a data directory leaf fails in `ensure_data_dirs` with a raw `AlreadyExists` before and is refused by the plan with its own text now (a dangling parent still fails with the raw error); an existing data directory without the owner write bit, and a umask that clears owner bits, both now fail the session that the old chmod made work (the `ensure_*` step still passes and the harness fails inside the sandbox; section 4, last bullet; the owner accepted the first). A dangling sidecar link and a data directory link loop fail the launch before and after.

## 7. Claims (verified by the architect on `3cc1605`)

| Claim | Where | Verified how |
| --- | --- | --- |
| Only `launch.rs:157-158` calls the two functions outside tests | `rg -n 'ensure_data_dirs\|ensure_data_files' src` | probed |
| `set_permissions` occurs only at `harness_data.rs:311, 315, 343` | `rg -n 'set_permissions' src/confine/harness_data.rs` | probed |
| `ensure_data_dirs` runs before the plan | `launch.rs:157` before `:160` | read |
| The plan refuses a data link resolving outside its root and a dangling one | `tests/confine/harnesses.rs:497`, `:910` (inside `harness_data_mounts_skip_missing_or_non_dir_root_and_refuse_dangling_dir_link`, `:865`) | read |
| Tests that pin the old chmod: only `:1096` | `rg -n 'ensure_data_dirs\|ensure_data_files' tests` (`:451`, `:563`, `:584`, `:590`, `:1096`, `harness_config.rs:188, :566`) | probed and read |
| `harness_config.rs:188, :566` call `ensure_data_dirs` only as setup on fresh directories | read | read |
| A dry run does not call either function | `rg -n 'ensure_data' src` shows only `launch.rs` | probed |
| `create_dir_all` on a dangling link returns `AlreadyExists` | std behaviour | recall (consultant read the std source from memory too); the developer's red-first run of the dangling data directory test settles it |
| `DirBuilder::mode` and `OpenOptions::mode` are masked by the umask; this host's umask is 0022 | std behaviour; `umask` | recall; the new tests assert 0700 and 0600 and the developer states the umask of the run |
| A link to a file at a data path errors `harness data path is not a directory` in the old code and in rule 3 | `secure_data_dir` `:304-310` | read |
| The plan skips a dangling sidecar link instead of mounting it | `add_canonical_existing` (`:428`) | not read; this spec makes the launch fail before the plan, so the claim is not needed |
| No document states this chmod; hits are philosophy.md:12 and architecture.md:302-303, :463, none about harness data directories | `rg -n -i 'chmod\|tighten\|0600\|0700' docs/confinement.md docs/architecture.md docs/philosophy.md README.md` | probed |

## 8. Acceptance

| Check | Judge |
| --- | --- |
| `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass | developer report, architect fresh run |
| The three abuse-case tests (rows 1, 2 and 4) and the two changed assertions fail on the old code first (record the failing output), then pass; the three contract tests (rows 3, 5 and 6) pass both before and after | developer report with outputs; tester reruns the red phase on a scratch snapshot |
| `rg -n 'set_permissions' src/confine/harness_data.rs` prints nothing; `rg -c 'create_dir_all' src/confine/harness_data.rs` prints 1 | architect |
| `git diff --stat` lists only `src/confine/harness_data.rs` and `tests/confine/harnesses.rs`; the diff outside the one named test and the six new tests touches no existing test | architect |
| Mutation pass in a scratch copy (tester skill), each mutant killed by the named test: re-add the chmod for an existing directory (`ensure_data_dirs_and_files_pin_errors_and_permissions`); follow the link and chmod its target (`existing_data_dir_symlinked_elsewhere_keeps_its_target_mode`); leaf mode `0o755` (`ensure_data_dirs_creates_agy_data_only_when_root_exists`); sidecar mode `0o644` (`omp_sidecars_cover_existing_agent_and_stats_databases_only`); follow a sidecar link and chmod its target (`sidecar_symlinked_to_a_file_keeps_that_files_mode`); drop the dangling data directory branch (`dangling_data_dir_symlink_is_left_for_the_plan`); drop the dangling-sidecar failure (`dangling_sidecar_symlink_still_fails_the_launch`); return `Ok` for every `fs::metadata` error in rule 3 (`data_dir_symlink_loop_still_fails`). The two mode mutants are killed only under a umask that keeps group and other bits (such as 0022); the tester states the umask of its run | tester |
| `CS_DISABLE_VERSION_CHECK=1 cs check src/confine/harness_data.rs`: no finding it did not have before; record before and after | developer, architect |
| `cargo test --test confine` and `cargo test --test daemon` pass | architect |

After the REPORT the developer stays idle and starts nothing until the architect assigns again.

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-senior-developer` on the agy harness (it reported "Antigravity (Gemini 4 Hydrogen (Medium))" and did not separate planning, working and advisor models); tester: the pass was not run (below).

BLOCKED/amend cycles: 0 spec amendments. Three BLOCKED replies, none a contradiction in the spec: the junior developer's (m_9319, before any edit) and the tester's two (m_9357, m_9359), all citing the OMP runtime rule "NEVER run checks between edits".

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Before the first draft | architect (Sonnet 5.5) | Told the owner that a data directory symlinked elsewhere "works today"; false: the plan already refuses an outside-root or dangling link (`tests/confine/harnesses.rs:497`, `:910`), and the defect was the call order (chmod at `launch.rs:157`, guards at `:160`) | Corrected to the owner; the spec was written on the real behaviour, with a claims table |
| Draft review, first pass | consultant | The sidecar test row duplicated `:563`; the case with no test was a sidecar that is a link to a file (the old code chmods the target) | Row replaced by `sidecar_symlinked_to_a_file_keeps_that_files_mode`; the attack path named in section 5 |
| Draft review, first pass | consultant | A dangling sidecar link would have started a launch that failed before | Rule changed so it still fails; contract test added |
| Draft review, first pass | consultant | Rule 3 left errors other than `NotFound` undecided; section 6 listed one changed case and missed two; the race branch skipped rule 3; the `use` line and the umask were unstated | Rule 3 split into three outcomes, the race branch re-evaluates rules 2 and 3, section 6 and the umask paragraph rewritten |
| Draft review, second pass | consultant | The loop-error branch of rule 3 had no test, so a mutant survived; section 6 said "two cases" and listed three; an "unreadable target" example was wrong (stat on a mode-000 directory succeeds); mutants were not mapped to tests | `data_dir_symlink_loop_still_fails` added, wording fixed, mutant-to-test list written into acceptance |
| Implementation | junior developer (omp) | Refused the assignment before any edit (m_9319); when the owner asked later it called the refusal a misreading | The owner released the architect to assign any peer; the spec went to the senior developer |
| Tester pass | tester (omp) | Refused twice (m_9357, m_9359), quoting its runtime rule "Run nothing while edits are still coming ... NEVER run checks between edits"; no pass ran | Architect ran the mutants (below); owner told that omp agents cannot run a red phase or a mutation pass |
| Architect review | architect | The first full suite had one failure, `daemon_lifecycle::failed_start_after_the_tokens_are_written_removes_every_state_file` ("another a2amx daemon is running"), a lock race under load in code this diff does not reach | Passes alone three times and with the whole `daemon` target (52 of 52); a second full run was green; recorded as an intermittent failure, not a diff problem |

No problem was found in the senior developer's diff itself.

Architect checks (2026-11-26, tree `a119e8e` plus the diff, umask 0022): production file sha256 `b70fe46e…` unchanged by every run; 11 of 11 mutants killed (the eight of acceptance plus `0o770` leaf, a non-directory passing, `create_new(false)`), each by the named test; red phase on `git show HEAD:src/confine/harness_data.rs` fails exactly the four tests of section 3 (rows 1, 2, 4 and the changed assertions) and passes the other seventeen `harnesses::` tests; five reruns of `harnesses::` stable (21 passed); `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean; `cs check` 9.09 before and after with the same four warnings; full `cargo test --no-fail-fast` 65 suites, 1488 passed, 0 failed, 3 ignored (baseline 1482 plus six new tests).

## Amendment 1 (2026-11-26, after the push gate)

Why: the Sonar pre-push scan of `7d3771c` failed the new-code coverage condition (64.0 against 80). A narrow `cargo llvm-cov test --test confine harnesses::` shows the zero-count new lines of `secure_data_dir` in `src/confine/harness_data.rs`: the branch that returns a `create` error other than `AlreadyExists` (lines 312-314 at `11ed84d`) and the branch that returns a `symlink_metadata` error other than `NotFound` (line 316). The `AlreadyExists` race branch has no deterministic test and stays untested on purpose (it needs a second process to create the directory between the two calls; the following `fs::metadata` match is covered by the other tests).

Scope: `tests/confine/harnesses.rs` only; two new tests; no production change. The tests go through `confine::ensure_data_dirs(Harness::Agy, &fixture.home)` with the existing `Fixture`, in the style of `existing_data_dir_symlinked_elsewhere_keeps_its_target_mode`. The data directory is `<home>/.gemini/antigravity-cli` and its root is `<home>/.gemini`.

1. `unsearchable_parent_fails_the_launch_with_the_metadata_error`: create `<home>/.gemini` (no `antigravity-cli` below it), set it to mode `0o000`, call `ensure_data_dirs`, set the mode back to `0o755` (always, before any assertion, so the fixture can be deleted), then assert the call returned `Err` and that `<home>/.gemini/antigravity-cli` still does not exist.
2. `read_only_parent_fails_the_launch_with_the_create_error`: the same with mode `0o500` on `<home>/.gemini`.

Both are contract tests: the old code also fails on these inputs (so they have no red phase on the old code). Each must kill a mutant: for 1, replace the final `Err(error) => return Err(error.into()),` of the first `match` by `Err(_) => return Ok(()),` (a mutant that only ignores the error would survive, because the second `match` returns the same error); for 2, replace `return Err(create_err.into());` by `return Ok(());`. The mutants run in the checkout one at a time, restored by sha256 (AGENTS.md), by the architect after the REPORT. Both tests run as an ordinary user; a run as root is refused by the repository already. Acceptance: `cargo test --test confine harnesses::` passes (23 tests); `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean; `git diff --stat` lists only `tests/confine/harnesses.rs`. The architect reruns the Sonar scan afterwards.
