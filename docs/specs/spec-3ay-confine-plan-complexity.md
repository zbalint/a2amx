# Spec 3ay: split `confine::plan` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `1560ff0`; consultant review `m_4443`, findings A-D, F, G and H adopted, E needed no change). Architect-directed fix for the red Sonar gate after spec 3ax,
context `a2amx-sonar-3ax-2026-10-30`. Implementation goes to `a2amx-developer` only; the tester pass goes to
`a2amx-tester`.

**Baseline:** develop at `1560ff0`; full suite, clippy and fmt pass there (architect run, 2026-10-30).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::plan` and `Plan::wrap` through `tests/confine.rs`. Every new item is private.

**Scope.** May edit exactly these files and no others.

- `src/confine.rs`: `plan` (line 660) and new private items directly after it
- `tests/confine.rs`: only the characterization tests of section 4

Does not touch: every other file, including `daemon.rs` and `main.rs` (their Sonar fixes shipped in `1560ff0`). No new
dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `confine::plan` at cognitive complexity 71 against 30. The function does eleven jobs in one 220-line
body: request limits, path resolution, workspace and git binding, harness data, operator writes, mandatory, default and
container hides, write release, operator hides and unhides, and the final sort and `Plan` assembly. All of them thread
the same eight lists through local variables.

This is the sandbox's mount plan, so the refactor must not change what any request produces. The complexity number is
a check, not the goal: the goal is one named job per function.

## 2. Rule: behavior is unchanged

Same `Plan` (every field, every list sorted as today), same errors with the same text and in the same order, same
`skipped` entries, same call to `plan.wrap(vec!["true".to_owned()])` at the end. No `pub` item changes. Keep every
`shortcut:` comment with the code it annotates. The order in which jobs run stays as in the current body, because the
first failing check decides the error a caller sees:

1. entry counts of `write`, `hide`, `unhide`, then the `team_file` length, then `check_harness`
2. canonical `home`, `state_dir`, `runtime_dir`, then `cwd` and its directory check, then `plan_empty_file`
3. cwd and git binding, harness data, operator writes and team file
4. mandatory, default and container hides
5. release of operator writes (container refusal first, then the `retain` calls)
6. operator hides, then operator unhides (each unhide: canonical lookup, `check_unhide_allowed`, container refusal, the
   four list updates)
7. `readonly` pruned of explicit writes, `protected` computed, all lists sorted, `Plan` built, `wrap` preflight

## 3. Target shape (architect decision, attacked by the consultant before lock)

One small named type carries the lists through the jobs, and each job is a method with the inputs it actually needs.
`plan` becomes a short sequence.

```rust
/// The mount lists while the jobs of `plan` fill them.
struct PlanBuilder<'a> {
    host: &'a Host,
    harness: Harness,
    home: PathBuf,
    state_dir: PathBuf,
    runtime_dir: PathBuf,
    writable: Vec<PathBuf>,
    readonly: Vec<PathBuf>,
    hidden: Vec<PathBuf>,
    skipped: Vec<PathBuf>,
    explicit_writes: BTreeSet<PathBuf>,
    mandatory_hidden: Vec<PathBuf>,
    default_hidden: Vec<PathBuf>,
    container_hidden: Vec<PathBuf>,
}

#[derive(Clone, Copy)]
enum HideKind { Default, Container }
```

| Item | One responsibility | Signature |
| --- | --- | --- |
| `check_request_limits` (free) | Job 1 | `(request: &ConfineRequest, harness: Harness) -> Result<()>` |
| `PlanBuilder::new` | Canonical `home`, `state_dir`, `runtime_dir`; empty lists | `(host: &'a Host, harness: Harness) -> Result<Self>` |
| `bind_workspace` | `add_validated_write(cwd)` plus the git common dir | `(&mut self, cwd: &Path) -> Result<()>` |
| `protect_git_metadata` | Read-only `hooks`, `config`, `config.worktree` (one loop over the three names; the list is sorted later) and `core.hooksPath` of a common dir | `(&mut self, common: &Path, worktree: &Path)` |
| `bind_harness_data` | `harness_data`, `validate_harness_data_paths`, `add_harness_data_mounts` | `(&mut self) -> Result<()>` |
| `bind_operator_writes` | `request.write` into `writable` and `explicit_writes`; the team file read-only inside a writable root | `(&mut self, request: &ConfineRequest) -> Result<()>` |
| `hide_mandatory` | Runtime and state dirs and `MANDATORY_ROOTS` | `(&mut self)` |
| `hide_defaults` | `DEFAULT_HIDES`, the other harnesses' credential files, `/run/dbus`; starts with `if request.no_default_hides { return; }` | `(&mut self, request: &ConfineRequest)` |
| `hide_containers` | `CONTAINER_HIDES`; starts with `if request.allow_containers { return; }` | `(&mut self, request: &ConfineRequest)` |
| `hide_or_skip` | Canonicalize an existing path into `hidden` and the list `HideKind` names; record a missing path in `skipped` | `(&mut self, source: PathBuf, kind: HideKind)` |
| `check_container_socket` | The shared refusal for `confine_write` and `confine_unhide` | `(&self, path: &Path, option: &str, allow_containers: bool) -> Result<()>` |
| `release_operator_writes` | Job 5 | `(&mut self, request: &ConfineRequest) -> Result<()>` |
| `apply_operator_hides` | Job 6, hides then unhides | `(&mut self, request: &ConfineRequest) -> Result<()>` |
| `unhide` | One operator unhide | `(&mut self, path: &str, allow_containers: bool) -> Result<()>` |
| `protected_paths` (free, pure) | The readonly paths inside a writable root, deduplicated by `path_key`, sorted | `(readonly: &[PathBuf], writable: &[PathBuf]) -> Vec<PathBuf>` |
| `finish` | Job 7 | `(self, request: &ConfineRequest, cwd: PathBuf, empty_file: PathBuf) -> Result<Plan>` |

`plan` itself: call `check_request_limits`, `PlanBuilder::new`, canonicalize `cwd`, check it is a directory, call
`plan_empty_file`, then `bind_workspace`, `bind_harness_data`, `bind_operator_writes`, `hide_mandatory`,
`hide_defaults`, `hide_containers`, `release_operator_writes`, `apply_operator_hides` and `finish`.

The developer may adjust names and the exact argument list where the code shows a better shape, and reports each
change. These are not allowed: a helper that is only "the second half of a loop body", a flag parameter that switches
a function between two jobs (`HideKind` selects a destination list, not a behavior), a new out-parameter accumulator
besides `PlanBuilder`, a `NOSONAR` or `#[allow]`, and changing `add_unique`, `add_validated_write`,
`add_harness_data_mounts` or the other existing helpers' contracts. Where a job needs one existing helper's `&mut
Vec` arguments (`add_validated_write(.., &mut self.writable, &mut self.skipped)`), pass the fields; do not rewrite the helper.

`/run/dbus` is not added to `skipped` when missing, while the other default hides are, so `hide_defaults` handles it
inline (`if let Some(path) = canonical_if_exists(Path::new("/run/dbus"))` writing `default_hidden` and `hidden`)
instead of calling `hide_or_skip`. `hide_defaults` and `hide_containers` return early on their option rather than
wrapping their body in `if !option { .. }`: the wrapper adds a nesting level to every branch inside it.

`host` is read only to call `add_validated_write` and `plan_empty_file`. Every other method uses the canonical
`home`, `state_dir` and `runtime_dir` fields, never `self.host.home` and the like: with a symlinked home the two
differ and the plan changes.

## 4. Safety net

`tests/confine.rs` already asserts, by test name: default hides and read-only unhide of home secrets
(`plan_skips_missing_and_unhides_existing_home_secrets_read_only`), mandatory hides under `no_default_hides`, the
container-socket refusals for write and unhide, the git hooks and `core.hooksPath` protection, worktree and submodule
common dirs, the home repository exclusion, the outside team file, symlinked state and runtime paths, and the entry-count
caps. NOT asserted (grep of `tests/confine.rs` at `1560ff0` found no `run/dbus`, `auth.json` or `config.worktree`):

1. `/run/dbus`: when it exists its canonical path is in `hidden` and `default_hidden`, and in neither under
   `no_default_hides`; when it is missing it is not in `skipped`
2. `.codex/auth.json` is hidden for a non-Codex harness and `.claude/.credentials.json` for a non-Claude harness (the
   two `credentials.json` hits in `tests/confine.rs`, lines 828 and 868, are the Claude harness-data allowlist, not
   this hide). `check_harness` refuses Codex, so the test uses Generic, Claude or Omp only
3. `config.worktree` of a common dir is read-only inside the writable repo
4. (no test) the `container_hidden.retain` in an unhide cannot remove anything: an unhide of a path inside a container
   socket is refused unless `allow_containers` is set, and then `container_hidden` is empty. Keep the `retain` as it is
   (rule in section 2). It is a no-op by `is_within` equality and the refusal before it; the tester's "skip the
   container refusal" mutant makes it live, so that one is not equivalent
5. error precedence between jobs 5 and 6: with a container socket as `write` and a bad unhide (a state directory) in
   one request, the error is the `confine_write` refusal (job 5 runs before job 6)

Existing tests that will catch most mutants and that the tester maps its mutants to:
`exact_writes_replace_default_hides_and_equal_depth_hides_precede_binds` (job 5 retains),
`git_common_directory_and_exact_writes_override_protection` (job 7 prune and `protected`),
`container_socket_hides_are_bound_and_can_be_disabled` (container hides),
`canonical_paths_cover_symlinked_workspace_and_hide_targets`,
`symlinked_harness_data_paths_are_refused_before_setup` (job 3 error order),
`mandatory_hides_remain_when_default_hides_are_disabled`.

Before the first edit, add one test per missing item above, each passing on the unchanged code, with expected values
written as literals. `/run/dbus` cannot be created in a test, and it exists on the architect's host. The test asserts the branch the
host's state selects (check with `Path::new("/run/dbus").exists()` inside the test) and the report names the branch the
host could not exercise. Record
`cargo test --test confine` counts before and after.

## 5. Out of scope

Every other function in `confine.rs`, `wrap`, the notice text, the mount order, `daemon.rs`, `main.rs`, the changelog,
renames of `pub` items, and the S2612 and S7493 fixes already shipped.

## 6. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test confine
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/confine.rs and tests/confine.rs
```

- The tests of section 4 pass on the unchanged code first (contract tests: they do not fail before the refactor).
- Trust boundary: the refactor adds no path. The abuse cases stay pinned by existing tests: the unhide guard
  (`state_omp_cannot_be_operator_unhidden`, `unhide_cannot_lift_container_socket_restriction`,
  `unhide_guard_canonicalizes_symlinked_state_runtime_and_container_paths`), the write guards
  (`codex_is_refused_and_write_guards_are_reciprocal`, `writable_paths_override_exact_hides_but_not_container_sockets`)
  and the caps (`path_lists_and_argv_sizes_are_capped`).
- The tester runs a mutation pass on the moved code: for each of job 1, 3, 4, 5, 6 and 7, break one behavior (drop the
  `retain` of a list, skip the container refusal, swap jobs 5 and 6, drop `config.worktree`, drop a sort) and name the
  test that fails; restore. Mutate in a scratch copy of the reviewed snapshot.
- Complexity: no local proxy exists for Rust (`complexipy` reads Python; backlog F25). The architect and consultant
  hand-count each new function and require every one at or below 15, which leaves a margin; Sonar confirms after the
  push (`rust:S3776` count zero on `src/confine.rs`).
- Known flaky: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`: rerun once, record as
  pre-existing if it then passes. `tests/confine.rs` real-bwrap tests need `bwrap`; record any that skip.

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `plan` is the only function over 30 in `confine.rs` | Sonar issue list for develop, 2026-10-30 | read (Sonar MCP) |
| `plan` has callers in `daemon.rs:486`, `main.rs:1062`, `main.rs:1148` and 49 `confine::plan(` calls in `tests/confine.rs`, none inside `confine.rs` | grep `confine::plan` | read |
| `/run/dbus` is the only default hide not added to `skipped` when missing | `src/confine.rs:780` (the other three use `add_unique(&mut skipped, ..)` in their `else`) | read |
| No test names `run/dbus`, `auth.json` or `config.worktree` | `tests/confine.rs` | grep, 0 hits each |
| `Plan` has private `bwrap`, `empty_file`, `state_dir` fields built inside `plan` | `src/confine.rs:121` | read |
| Full suite, clippy, fmt pass at `1560ff0` | architect run 2026-10-30 | ran |
| `container_hidden.retain` in an unhide is a no-op while the refusal before it stays | `src/confine.rs:178` (`is_within` includes equality) and the unhide loop in `plan`; test at `tests/confine.rs:377` | read by architect and consultant; not probed |
| Hand counts: every new function at most 11 (`hide_defaults` 5 with `hide_or_skip`), against the 15 bar | consultant `m_4443` | hand count only; Sonar confirms after the push |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (model name not exposed to the architect);
developer and tester on the OMP harness, planning and working model `openai-codex/gpt-5.6-luna` (the advisor model was
not exposed to either). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Architect (Claude Sonnet 5.5), by grep | The first draft stated four claims from recall: `/run/dbus` at lines 758-763 (it is line 780), 20 test callers (49), the `credentials.json` test hits as covering the non-Claude hide (they are the harness-data allowlist), and a test for the unhide `container_hidden.retain` (the line cannot be observed) | Claims table and section 4 corrected before the consultant saw the draft |
| Draft review | Consultant | `hide_defaults` with the `if !option` wrapper counts 18, not 11 | Early return in `hide_defaults` and `hide_containers` |
| Draft review | Consultant | The skip-missing rule would be repeated at four call sites | `hide_or_skip(source, kind)` owns it; `/run/dbus` stays inline |
| Draft review | Consultant | The `/run/dbus` test asserted nothing on a host where the path exists; the spec had the hosting condition backwards | Test asserts the branch the host selects and the report names the other |
| Draft review | Consultant | Raw `host` paths and canonical fields side by side on the builder invite a symlinked-home bug | Section 3 says `host` is read only by `add_validated_write` and `plan_empty_file` |
| Draft review | Consultant | No test pinned that a `confine_write` refusal beats a bad unhide | Added as section 4 item 5 |
| Architect review | Architect | The developer rebound `path` from `check_unhide_allowed`'s return, which the old code discarded | Developer restored the old form |
| Architect review | Architect | A two-line comment was reflowed to one 126-character line | Developer restored the wrapping |
| Tester pass | Tester | Doc comment on `plan` lost a double space | Architect restored it; no behavior change |
| Tester pass | Tester | Four mutants survive in the moved code: `finish` `skipped.sort()`, missing container paths dropped from `skipped`, the write/hide entry-limit order, the unhide `default_hidden.retain`. The same mutants survive on the code before this spec | Tests-only hardening item (backlog F34), not a blocker |
| After the push of `1560ff0` | Sonar | The architect's hand count for `create_session` after `split_confine` was 30; Sonar reports 31 | `plan_confinement` extracted from `create_session` in the commit that accepts this spec |
