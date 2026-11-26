# Spec 3hm: function-level code health of `src/confine/hides.rs` and the write checks in `src/confine.rs`

## 0. Status

LOCKED after consultant review m_9593 (reachability, equivalent mutant (e), iterator form, `is_home_entry` used by both rules, (f) non-kill applied). Context `a2amx-3hm-confine-hides-write-checks-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `a124854`. Behaviour-preserving refactor, test-first where a message or branch is not yet pinned. Public seams: `confine::plan`, `confine::check_entry_syntax`, `confine::check_write_allowed`, `confine::plan_with_preflight` (everything reaches `apply_grant_unhides`, `container_socket_hides` and the write checks through them).

Scope (edit): `src/confine/hides.rs`, `src/confine.rs` (production code only: `check_entry_syntax`, `check_absolute_entry`, `check_write_allowed`, `check_write_runtime_allowed`; the `#[cfg(test)]` module at `:420` is not edited), and, only if Step A finds an unpinned reachable branch, one new test file `tests/confine/hide_unhide_messages.rs` with one `mod` line in `tests/confine/main.rs`. Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`. The three callers of the write checks (`builder.rs:194`, `harness_data.rs:436`, `hides.rs:158`) keep their calls unchanged.

No behaviour change: same results, same error text byte for byte, same hide order.

## 1. Why

`cs check` (2026-11-26, `a124854`) reports 12 findings:

- `hides.rs`: file-level Overall Code Complexity `:1`; `container_socket_hides` `:52` (cc 9, Bumpy Road 2); `check_grant_path_writable` conditional `:165` (two operators); `apply_grant_unhides` `:179` (cc 12, Bumpy Road 3, Large Method 74 lines).
- `confine.rs`: `check_entry_syntax` conditional `:154`; `check_absolute_entry` conditional `:239`; `check_write_allowed` `:246` (cc 10) with conditional `:258`; the unit test `:428` (Bumpy Road 2, **not** in scope, see section 5).

Each function mixes separable jobs: working out which sockets a grant hands back, then deciding which to hide; resolving a grant path, then testing it against three hidden sets; the explicit-versus-implicit rule written out three times in `check_write_allowed` and again in `check_write_runtime_allowed`.

Rules that bind (earlier refactors, 3hk and 3hl): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function; split only along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

Before any production edit, the developer tries to reach each branch below through a public seam and, where no existing test asserts its exact text, adds a pin in `tests/confine/hide_unhide_messages.rs` (full-text `assert_eq!`, literal expectations, fixtures with temp dirs):

1. `check_grant_path_writable`: the `~/.a2amx` message and the harness-root message. Both are reachable: the grant symlink bail (`hides.rs:202`) checks only the grant's own source path, while the guard roots (`~/.a2amx`, the harness root) are canonicalised and may themselves be symlinks to an ancestor of a real grant path. Fixtures (home only; the test `Fixture` keeps the layout root at `<tmp>/state`, outside home): `home/.a2amx` -> `home/.cache` with grant `python` and a real `home/.cache/pip`; `Harness::Codex` with `home/.codex` -> `home/.cache` and grants `harness-config` plus `python` (Claude with `home/.claude` -> `home/.cache` if the harness-data binding allows it). Pin each with a full-text `assert_eq!`.
2. `apply_grant_unhides`: the "resolves to the hidden path" bail is reachable the same way: for a default-hide source with `home/.kube` -> `home/.cache` (default hides on, grant `python`, real `home/.cache/pip`); for a container hidden path with `home/.docker/run/docker.sock` a symlink to a file under `home/.cache/pip` and no `docker` grant. The mandatory-path arm is reachable only through a symlinked state, run or secrets directory under the layout root; the developer reports it as reachable (pinned) or too contrived (not pinned, reason stated). No test names these texts today (grep `tests/` for each).
3. `check_write_allowed`: each message once (`/` as an explicit write entry, `$HOME` with a non-explicit write equal to home exactly, a2amx directory, mandatory hidden path), explicit and not explicit where both are reachable; `check_write_runtime_allowed` explicit and not. By grep `tests/` has no pin for "writable path / is not allowed" or "overlaps a mandatory hidden path"; the mandatory text is reachable non-explicitly through a canonical ancestor of `/run/WSL` (a path symlinked to `/run`, the check being lexical on the canonical path), else reported unreachable. Existing pins for the others live in `paths_and_git_config.rs` (`:378` for `$HOME`, to confirm), `grant_writes_and_create.rs`, `grants_runtime.rs`; the developer lists which row each covers and adds only what is missing.
4. `check_entry_syntax` and `check_absolute_entry`: `~`, `~/x`, `~x`, `~bob/x`, `x`, `/x`, `` (existing pins `plan_basics.rs:257-280`); add only a missing row.

An unreachable branch is reported, not pinned; no test hook is added.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/confine.rs`

- `fn is_home_entry(entry: &str) -> bool` is `entry == "~" || entry.starts_with("~/")`: the entries that name the user's own home. Both rules use it: `check_entry_syntax` reads `if entry.starts_with('~') && !is_home_entry(entry)` (equivalent to the old three-term condition), `check_absolute_entry` reads `if !is_home_entry(entry) && !Path::new(entry).is_absolute()`. One logical operator each.
- `fn overlaps_for(explicit: bool, path: &Path, other: &Path) -> bool`: `is_within(path, other)` when explicit, else `paths_overlap(path, other)`. It replaces the three `if explicit {..} else {..}` blocks in `check_write_allowed` (state, mandatory loop) and `check_write_runtime_allowed`. Two of those are reached from three callers, so it is a real shared rule, not a finding-clearer.
- `check_write_allowed`: the `$HOME` test becomes `if !explicit && is_within(&home, &path)`; `path == home` is covered because `is_within` includes equality (`:223`). The a2amx-directory and mandatory-root checks use `overlaps_for`. Order of checks and every message are unchanged.

### `src/confine/hides.rs`

- `fn granted_socket_targets(table: &[SocketRow<'_>], given: &BTreeSet<&str>, home: &Path) -> BTreeSet<PathBuf>`: the first loop of `container_socket_hides` (the canonical paths of sockets the given grants hand back). `container_socket_hides` keeps the second loop and calls it; the `given` set is built once in `container_socket_hides` and passed in.
- `fn guarded_harness_root(&self) -> Option<PathBuf>` on `PlanBuilder`: `None` when `harness_config` is set, else `session_harness_root()` canonicalised if it exists. `check_grant_path_writable` then reads `if let Some(root) = self.guarded_harness_root() && is_within(&path, &root)`. One operator.
- `fn resolve_grant_unhide(&mut self, grant_name: &str, name: &str) -> Result<Option<PathBuf>>`: the symlink bail, the `canonical_if_exists` or record-in-`skipped` branch, then `check_grant_path_writable`; `None` means "skipped". It is the contiguous run `hides.rs:201-214`.
- `fn unhide_conflict(&self, path: &Path, default_hide_sources: &[PathBuf], own_paths: &BTreeSet<PathBuf>) -> Option<PathBuf>`: the first hidden path the grant path may not take, in the iterator form so no loop-with-conditional bump remains: `default_hide_sources.iter().filter(|source| !own_paths.contains(*source)).filter_map(|source| canonical_if_exists(source)).find(|hidden| is_within(path, hidden)).or_else(|| self.mandatory_hidden.iter().chain(&self.container_hidden).find(|hidden| paths_overlap(path, hidden)).cloned())`. Default sources come before mandatory and container paths and the first hit wins, as in the old two loops. The two identical `bail!` texts become one in the caller.
- `apply_grant_unhides` keeps the `given` set, `own_paths`, `default_hide_sources`, and loops grants then names: `let Some(path) = self.resolve_grant_unhide(grant.name, name)? else { continue }`, `if let Some(hidden) = self.unhide_conflict(&path, &default_hide_sources, &own_paths) { bail!(...) }`, then the four mutations unchanged (retain hidden, retain default_hidden, add writable, push grant_unhides).
- Not changed: `hide_mandatory`, `hide_defaults`, `apply_grant_stand_ins`, the rest of the file.

The file-level Overall Code Complexity finding (`hides.rs:1`) is expected to clear once the three cc 9-12 functions drop; if it does not, the developer reports the remaining per-function cc numbers and stops (no extra split without the architect).

## 4. Out of scope

`hide_mandatory`, `hide_defaults`, `apply_grant_stand_ins`, `hide_or_skip`, `harness_data.rs`, `builder.rs`, `grant_dirs.rs` (their own findings or none); merging the repeated `check_write_allowed` plus `check_write_runtime_allowed` pair at three call sites (a possible later spec); the unit test `grant_socket_and_environment_names_are_unique` `:428` (a data-table test; its Bumpy Road finding goes to the later tests batch).

## 5. Trust boundary

None crossed: the same checks run in the same order with the same texts. Abuse-case coverage is the existing pins (a write path over `/`, `$HOME`, the a2amx directory, the runtime directory, a hidden path, a symlinked grant path), which Step A confirms and completes.

## 6. Acceptance

Baseline (before any edit, on `a124854`): `cargo test` passes (1510 passed, 3 ignored); whole-tree cs count 878.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/confine/hides.rs
cs check src/confine.rs
git status --short
```

- All pass; `cs check` lists none of the 11 production findings of section 1 (the `:428` unit-test finding stays; `confine.rs` therefore keeps one known warning, to be cleared by the tests batch) and, if the new test file exists, none on it; the whole-tree count is below 878 (judge: the architect's `cs` run).
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources); the whole suite passes on both trees.
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hm/` (both deleted before its REPORT); per mutant run `cargo test --lib` and the test targets `confine`, `auto_confine`, `team`, `teams`, `daemon`, `gateway_up`, `gateway_background`. Mutants, one change each: (a) `check_entry_syntax` accepts `~x`; (b) `check_entry_syntax` rejects `~` alone; (c) `is_home_entry` accepts `~x`; (d) `overlaps_for` ignores `explicit`; (e) the `$HOME` test drops the equality case (`!explicit && path != home && is_within(&home, &path)`; note `Path::starts_with` is true for equal paths, so a strict-`starts_with` variant would be equivalent); (f) `check_write_allowed` checks mandatory roots before the a2amx directory (expected non-kill: only `/` overlaps both, and `/` bails earlier); (g) `granted_socket_targets` returns an empty set; (h) `container_socket_hides` hides a socket also handed back by a given grant; (i) `unhide_conflict` ignores `own_paths`; (j) `unhide_conflict` checks mandatory/container before default sources (killable with the `.kube` plus docker-socket fixtures of Step A 2: the old text names `home/.cache`, the mutant names the socket); (k) `unhide_conflict` uses `is_within` instead of `paths_overlap` for mandatory/container paths; (l) `guarded_harness_root` ignores `harness_config`; (m) `resolve_grant_unhide` does not record a missing path in `skipped`; (n) `apply_grant_unhides` forgets `default_hidden.retain`. Expected kills are named by the tester from its own run; an expected non-kill is reported with its reason. Any survivor in moved code is a fix request (a new test in the files of section 0).

## 7. Claims (architect, `a124854`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the 12 findings and their lines | `hides.rs:1,52,165,179`; `confine.rs:154,239,246,258,428` | `cs check` run, output read |
| `is_within` includes equality, so `path == home \|\| is_within(&home, &path)` equals `is_within(&home, &path)` | `confine.rs:223-225` | read |
| `apply_grant_unhides` has one caller; `container_socket_hides` two (`hides.rs:279`, `:381`); `check_grant_path_writable` one | `builder.rs:94`, `hides.rs` | grep |
| `check_write_allowed` has three callers, `check_write_runtime_allowed` three | `builder.rs:194-195`, `harness_data.rs:436-437`, `hides.rs:158-159`; also `grant_dirs.rs:34` calls `check_write_allowed` | grep |
| every grant `unhides` entry is a fixed `~/...` path | `src/grant.rs:60-95` | read |
| the entry-syntax and absolute-entry messages are pinned | `plan_basics.rs:103,114,143,257-280`, `limits_and_hides.rs` | grep |
| the two "resolves to the hidden path" texts in `apply_grant_unhides` are identical | `hides.rs:223-228`, `:237-242` | read |
| `apply_grant_unhides` unchanged callers pass `no_default_hides` through the request | `builder.rs:94` | read |
