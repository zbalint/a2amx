# Spec 3by: every grant is read-write (confinement redesign, step 1)

## 0. Status

**LOCKED** (2026-11-02; two consultant passes applied). Context `a2amx-confine-redesign-2026-11-02`. Location: main checkout, branch `develop`.
Public test seams: `confine::plan` / `confine::plan_with_path` with a `ConfineRequest` built directly
(`Plan::writable`, `Plan::readonly`, `Plan::protected`, `Plan::hidden_paths`, `Plan::with_bwrap(..).wrap(..)`,
`Plan::notice`), `grant::all`, `grant::lookup`, the `a2amx confine list` binary test in `tests/grant.rs`.

Scope (may edit): `src/confine.rs`, `src/grant.rs`, `tests/confine.rs`, `tests/grant.rs`, `docs/confinement.md`.
Does not touch: `src/cli.rs`, `src/daemon.rs`, `src/main.rs`, `src/team.rs`, `src/suggest.rs`, `src/wire.rs`,
`docs/teams.md`, `docs/backlog.md`, `CHANGELOG.md`, `a2amx.toml*`, other specs. The grant table keeps its shape
(no new fields), so no other module changes.

This is step 1 of a redesign the owner settled on 2026-11-02. Later steps (their own specs): toolchain and
credential grants; removal of the default build paths and of `build_paths`; the `harness-config` grant folded into the
default; the config cleanup; `confine suggest` printing grants. This spec changes none of those.

## 1. Why

Owner decision (2026-11-02): a grant means full read-write access to what it names, and the user owns what is in it.
Today the three path grants, `gh` (`~/.config/gh`), `git-credentials` (`~/.git-credentials`,
`~/.config/git/credentials`) and `ssh` (`~/.ssh`), unhide their paths read-only (`PlanBuilder::apply_grant_unhides`
adds them to `readonly`, src/confine.rs:1498). The docs already record what that breaks: the first connection to a new
host cannot write `known_hosts` (docs/confinement.md:364-367), `gh auth setup-git` and an old-format `hosts.yml`
fail on the read-only mount (docs/confinement.md:310-317). The owner hit this himself: a granted tool that cannot
write its own state is a default-path bug. The harness-config grant is already read-write; the rest follow it.

## 2. Behaviour

1. **Grant paths are writable, and validated like a write.** `apply_grant_unhides` (src/confine.rs:1443) adds each
   canonical grant path to `writable` (`add_unique(&mut self.writable, path.clone())`) instead of `readonly`, and
   still records it in `grant_unhides` and still removes it from `hidden` and `default_hidden`. Before the path is
   added it goes through `check_write_allowed(&path, &self.host.home, &self.host.state_dir, false)` and
   `check_write_runtime_allowed(&path, &self.host.runtime_dir, false)`, the checks every operator and build write
   passes (`add_validated_write`, src/confine.rs:857), and a failure is returned as the plan error. The reason: a grant
   path may be a symlink (tests/confine.rs:828-869 allow it on purpose), and a symlink to `$HOME` or an ancestor
   of it, or to `/`, must not become writable. (The state and runtime directories are already refused earlier,
   because `hide_mandatory` puts them in `mandatory_hidden` before the grants run, src/confine.rs:1011 and
   1482-1494; the new call adds the "not home or above" refusal.) The same function also refuses a canonical grant
   path that is or lies inside `~/.a2amx`, or is or lies inside the session's harness root (`~/.claude` for Claude,
   `~/.omp` for OMP), unless `self.harness_config` is set (the harness-config access is in effect, which
   `bind_harness_config` has already decided at src/confine.rs:1009). The reason: an alias into the harness root or
   `~/.a2amx` would otherwise make it persistently writable without the harness-config access. A symlinked
   `~/.ssh` into a dotfiles repository stays valid. All other checks in the function stay (the overlaps with the
   default, mandatory and container hides).
2. **System paths stay protected after the grants.** In `plan_with_preflight` (src/confine.rs:1018-1021) the call
   `builder.protect_system_paths(path_var.as_deref())` moves out of the `if request.build_paths` block, so it also
   runs when `build_paths` is false; `bind_build_writes` stays inside the block. It runs after the grants are
   applied, so a grant path that resolves to `~/.a2amx` or a `PATH` directory inside home ends read-only:
   `mounts()` inserts `readonly` entries after `writable` ones, so the protected entry wins.
3. **A plain `unhide` of the exact grant path does not make it read-only.** In `PlanBuilder::unhide`
   (src/confine.rs:1586-1595) the `add_unique(&mut self.readonly, path)` is skipped when `path` equals a path in
   `grant_unhides`; the removal from the hidden lists still happens. An `unhide` of a path inside a granted
   directory (for example `~/.ssh/config` with the `ssh` grant) keeps its current effect: it is added to `readonly`
   and re-protected inside the writable directory. A plain `unhide` of any path no grant opened is unchanged.
   `write` of a granted path stays valid and changes nothing.
4. **Conflict checks unchanged.** A `hide` of a granted path still fails with "conflicts with the hide" (the loop
   over `grant_unhides` in `finish`); a `hide` inside a granted directory still works (tests/confine.rs:1080-1094).
5. **Exposure text.** The `exposes` strings in `src/grant.rs` name the access and what comes with it:
   - `gh`: `your GitHub CLI login (~/.config/gh, read-write) and its tokens in the environment: the agent can push, open and merge pull requests, read private repositories as you, and change or delete the login`
   - `git-credentials`: `the plain-text git credential files (read-write in place): every host and password stored there, which the agent can also overwrite`
   - `ssh`: `your ssh keys and config (~/.ssh, read-write): the agent can log in to every host those keys open as you, push with them, and change or delete them`
   The `git-credentials` paths are single files in the home directory. A single-file bind keeps its inode, so the file
   can be written in place but not replaced by a rename or deleted, and git's store helper writes through a lock file
   in the read-only home directory. That is why the text says "in place" and the docs keep the write-back caveat
   (2.7). The container, `snap` and `harness-config` rows keep their text.
6. **Notice.** `confine::notice` (src/confine.rs:2144) lists a granted path under "Writable locations" because it is
   now in `plan.writable`; no code change is expected there, and no "Read-only" line may name a granted path.
7. **Docs.** `docs/confinement.md`: the `ssh` row (line 79), the container section's sentence that the `ssh` grant
   "opens it read-only" (line 289), the credential section (lines 298-323) and the ssh section (lines 364-370) say the
   `gh` and `ssh` paths are read-write and drop the read-only workarounds (`gh auth setup-git` on a read-only mount,
   `ssh` unable to add a host key). Add one sentence: with the `ssh` grant, a separate `write` entry for a single file inside `~/.ssh` is redundant and
   makes that file a single-file bind that cannot be replaced by a rename (`ssh-keygen -R` fails), so drop it.
   For `git-credentials` the docs say the files are writable in place and keep the
   caveat that a credential helper that replaces the file (git's `store` helper writes a lock file in the home
   directory and renames it) still fails. Keep every other statement; the limit that `Include` of a root-owned file in
   the ssh config fails stays.

## 3. Claims about existing code

| claim | where | verified how |
| --- | --- | --- |
| Grant unhides go to `readonly` and are recorded in `grant_unhides` | src/confine.rs:1498-1499 | read |
| `finish` drops `readonly` entries that are explicit writes | src/confine.rs:1605-1606 | read |
| `finish` fails when a grant path is hidden | src/confine.rs:1621-1628 | read |
| Operator `unhide` runs after the grants and adds to `readonly` | src/confine.rs:1014-1017, 1586-1595 | read |
| Grant paths skip `check_write_allowed`; operator and build writes pass it | src/confine.rs:1462-1499 vs 857-868 | read |
| `mounts()` keys by path, inserts hidden, then writable, then readonly, then protected; the last insert wins | src/confine.rs:1735-1757 | read |
| The second `protect_system_paths` call is inside `if request.build_paths` and the first runs before the grants | src/confine.rs:1020, 1311 | read |
| The grant tests that also assert read-only | tests/confine.rs `ssh_grant_unhides_home_ssh_and_stands_in_host_config` (~950, `--ro-bind` ~991), `grant_alias_to_hidden_path_is_rejected_but_unhidden_alias_is_allowed` (~828-869, `plan.readonly.contains(target)` ~866) | read |
| A single-file bind cannot be replaced or deleted, and git's store helper writes a lock file in the home directory | recalled from a past bwrap probe and from git's behavior | unverified: the tester probes it (Acceptance 5) |
| `write` of a granted path already ends in `writable` | tests/confine.rs `exact_write_lifts_a_granted_credential_path` | read |
| Tests pin read-only grants | tests/confine.rs `credential_grants_lift_named_paths_read_only` (~585), `credential_grant_readonly_path_is_reprotected_inside_writable_parent` (~634), `exact_write_lifts_a_granted_credential_path` (~675), the git-credentials test asserting `plan.readonly.contains` (~770-803); tests/grant.rs `credential_grant_rows_have_exact_exposure_text` (~140) and the `confine list` test (~246-258) | read |
| Docs state the read-only breakage | docs/confinement.md:310-317, 364-367 | read |
| A real bwrap run of a writable bind under a hidden parent works, and `--remount-ro` is not recursive | not verified | the tester probes it (Acceptance 5) |

## 4. Out of scope

Toolchain grants, `build_paths`, `~/.cache` and `~/.cargo` defaults, the `harness-config` grant and its implicit
default, the config keys, `confine suggest`, container grants, `keep_env`, operator `unhide` semantics for
non-granted paths, `CHANGELOG.md` (the architect adds the Unreleased line when committing).

## 5. Acceptance

Baseline (before any edit, from the checkout; the architect ran it on 2026-11-02: 779 tests passed, 0 failed):
`env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all pass.

After implementation, judged by these commands and by the architect at final review:

1. `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   pass with no warnings. `scripts/prepush-fast.sh` passes.
2. Tests in `tests/confine.rs`, one per behaviour. Contract tests (they pass on the old code) are marked.
   - For each of `gh`, `git-credentials`, `ssh`, with the grant and the path present (for `git-credentials` both
     files are created): the path is in
     `plan.writable`, not in `plan.readonly`, not in `plan.protected`, not in `plan.hidden_paths()`, and the wrapper
     binds it with `--bind`, never `--ro-bind`. Without the grant the path is in `plan.hidden_paths()` (contract
     test; the fixture creates the path, since a missing path is skipped, not hidden).
   - `write: ["~/.config"]` plus the `gh` grant: `gh` is in `writable`, not in `protected`, not in `readonly`, and the
     wrapper has no `--ro-bind` for it. (This replaces `credential_grant_readonly_path_is_reprotected_inside_writable_parent`.)
   - A plain `unhide` of the exact granted path keeps it in `writable` and out of `readonly`.
   - A plain `unhide` of `~/.ssh/config` with the `ssh` grant is `--ro-bind` and in `protected`; only the
     `--ro-bind` assert is a contract test, the `protected` assert fails on the old code.
   - A plain `unhide` of a path no grant opened is read-only (contract test).
   - A `hide` of a granted path fails with "conflicts with the hide" (contract test); a `hide` inside a granted
     directory still works (contract test, tests/confine.rs:1080-1094).
   - Alias to home: `~/.config/gh` is a symlink to the fixture home, `no_default_hides: true` (with default hides on
     the overlap check already refuses it), `gh` grant: `confine::plan` returns an error containing
     "writable path $HOME is not allowed". This fails on the old code (it accepted the alias read-only).
   - Alias into the state directory: same setup, target inside the state directory: an error naming the hidden path
     (contract test; it passes on the old code and does not kill a mutant of 2.1).
   - Alias into the harness root: Claude harness, `build_paths: false`, `~/.config/gh` a symlink to `~/.claude`, `gh`
     grant: an error. Same alias with `build_paths: true` (the harness-config access is in effect): no error.
   - Alias to a `PATH` directory inside home: the test passes `path_var`, `build_paths: false` (the
     `ConfineRequest` default, stated explicitly), `gh` grant: the path is in `plan.readonly` or `plan.protected` and
     the wrapper uses `--ro-bind` for it. Passes on the old code (the grant was read-only), so it is a contract test
     for 2.1 and the test that kills the 2.2 mutant.
   Rewritten, not deleted: `credential_grants_lift_named_paths_read_only` (~585), `exact_write_lifts_a_granted_credential_path`
   (~677, still passes), the `git-credentials` test asserting `plan.readonly.contains` (~770-803),
   `ssh_grant_unhides_home_ssh_and_stands_in_host_config` (~950, the `--ro-bind` at ~991 becomes `--bind`) and
   `grant_alias_to_hidden_path_is_rejected_but_unhidden_alias_is_allowed` (~828-869, the readonly assert at ~866
   becomes a writable assert). No other test is deleted.
3. `tests/grant.rs`: the three exposure texts (`credential_grant_rows_have_exact_exposure_text`, ~138) and the
   `confine list` rows (~246-258) equal the strings of 2.5 exactly.
4. `rg -n "read-only" docs/confinement.md`: no remaining sentence calls the `gh`, `git-credentials` or `ssh` path
   read-only, except the `git-credentials` caveat of 2.7 (the architect reads every remaining hit at final review).
5. Tester, with a real `bwrap` when the host has one (skip and report otherwise), in a temp home, with each fixture
   path created first (`~/.ssh`, `~/.config/gh`, `~/.git-credentials`):
   - `ssh` grant: a confined command appends to `~/.ssh/known_hosts` and creates `~/.ssh/new_key`; both succeed.
     Without the grant, `~/.ssh` looks empty and the writes do not reach the real directory.
   - `gh` grant: a file write under `~/.config/gh` succeeds.
   - `hide: ["~/.config"]` plus the `gh` grant: a write under `~/.config/gh` still succeeds (the nested bind stays
     writable; this checks the claim that `--remount-ro` is not recursive).
   - `git-credentials` grant: an append to `~/.git-credentials` succeeds; a `mv` of a new file over it and an `rm` of
     it fail, which is the in-place limit the text states. Report the exact errors.
   The tester reports the exact commands and output. If any of these disagree with the spec, report it as a
   spec question, not as a bug fix.
6. Mutation check by the tester on a scratch copy; each mutant must fail the named test:
   - revert 2.1's writable add (back to `readonly`): the per-grant writable tests;
   - drop the `check_write_allowed` call of 2.1: the alias-to-home test;
   - drop the harness-root and `~/.a2amx` refusal of 2.1: the harness-root alias test;
   - keep `protect_system_paths` inside the `build_paths` block (2.2): the `PATH`-directory alias test;
   - drop the skip in `unhide` (2.3): the exact-path `unhide` test.
