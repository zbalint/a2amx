# Spec 3hk: function-level code health of `src/confine/wrap.rs` and `src/confine/launch.rs`

## 0. Status

LOCKED and accepted. Locked after consultant review m_9530 (all findings applied: B1 behaviour check replaced by the Step A text pins, B2 three full-text tests added, single-use helpers dropped, shared `unset_credentials`, corrected callers and anchors, concrete mutants; no second review because the review-driven changes drop helpers and add pins and do not change a behaviour rule). Context `a2amx-3hk-confine-output-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `ef78e96`. Behaviour-preserving refactor, test-first. Public seams: `Plan::wrap`, `Plan::wrap_app_server`, `confine::notice`, `confine::dry_run_lines`.

Scope (edit): `src/confine/wrap.rs`, `src/confine/launch.rs` (its production code only: the `#[cfg(test)]` unit tests in the file, from `:244` on, are not edited), and `tests/confine/overlay_symlinked_agent.rs` (three new tests, section 2; it already holds the symlinked-OMP fixture `symlinked_agent_fixture`). Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

This spec changes no argv token or order, no notice or dry-run text (byte for byte), no limit, no error text.

## 1. Why

`cs check` (2026-11-26, `ef78e96`) reports 12 findings in these two files: `wrap.rs` `mounts` (`:64`, Bumpy Road, cc 11), `env_unsets` (`:114`, Bumpy Road, cc 9, Complex Conditional `:134`), `wrap_with_codex_bind` (`:153`, Large Method 103 lines, cc 13, Bumpy Road); `launch.rs` `dry_run_lines` (`:44`, cc 14) and `notice` (`:104`, Large Method 101 lines, cc 12). Each is one long sequence of independent paragraphs (argv sections, text sections) with every paragraph's condition inline. The refactor cuts them along those paragraphs and adds nothing else.

The argv is already pinned byte for byte (`tests/confine/harnesses.rs:127`, the exact `assert_eq` of prefix, unsetenv order, mount order and remounts at `:185-318`). The notice and the dry-run text are not: only line-level asserts exist. A "byte-identical" claim needs a full-text pin committed first, and a before/after capture of the output cannot be done in this repo (fixtures use random temp paths and print nothing), so Step A adds the pin.

Rules that bind (earlier refactors): a helper that exists only to clear a finding is rejected; no out-parameter accumulator; at most four parameters on every new function; split only into contiguous runs so order stays; no suppression. A helper returns what it produces (`Vec<String>`, `String`, `Option<String>`); it does not push into the caller's buffer.

Alternatives rejected: a trait per mount kind (a `match` is the whole variation); a template engine for the notice (fixed prose); single-use helpers `check_argument_sizes`, `codex_socket_bind`, `first_occurrences` and `mount_order` (each is one inline expression or closure; consultant review).

## 2. Step A: pin the full text first (tests, green on the current code)

In `tests/confine/overlay_symlinked_agent.rs` add three tests, written and run against the unchanged source first (contract tests, no red phase). Plan fields `overlay_parents`, `overlay_protected`, `unsaved_files`, `overlay_notice` and `harness_config` are private to `Plan`, so the tests reach them only through real plans: the symlinked OMP fixture (an overlay parent; `harness_config` is the default OMP root), `Plan::without_overlays(reason)` (sets the overlay notice and clears the parents) and the public fields (`writable`, `skipped`, `grants`, `keep_env`, `auto`, `reasons`, `unresolved`). `unsaved_files` is empty for every plan `confine::plan` builds (spec 3hi), so the `unsaved` lines of both texts are pinned only by the `launch.rs` unit tests (unchanged); a mutant that drops them is reported by the tester as covered by those tests or as unreachable through the public seams.

| Test | Plan | Expected |
| --- | --- | --- |
| `notice_text_is_pinned_with_an_overlay_parent` | the symlinked OMP fixture with `grants` containing one grant that has sockets (so the container-engines line differs), `keep_env` non-empty, and `writable` reordered so the cwd is not first | the whole `notice(&plan)` as one `assert_eq!` against a literal text built with `format!` over the fixture's paths |
| `notice_text_is_pinned_after_the_overlays_are_dropped` | the same plan after `without_overlays("reason")`, with the `harness-config` grant absent | the whole notice (the overlay notice line shows, the overlay line is gone, the harness-config line says "default for this harness") |
| `dry_run_text_is_pinned_for_an_auto_plan_with_every_section` | the symlinked plan with `auto` set and `reasons` for two grants, one in `accepted`, `grants`, `keep_env`, writable including the cwd, protected, hidden, one unresolved path and one skipped path that is not unresolved | the whole `dry_run_lines(&plan, cwd, &accepted)` as one `assert_eq!` against a literal text |

The literal texts are written by hand from the current output (probe it once, read it, copy it into the test); the fixture's temp paths are the only substituted parts. A field that cannot be set as described makes the developer report BLOCKED; no accessor is added.

## 3. Step B: target shape (the design is the instruction; the `cs` result is the check)

`src/confine/wrap.rs`:

- `MountKind::layer(self) -> u8`: `0` for `Overlay`, `Tmpfs`, `Empty`; `1` for the rest (the closure `rank` of `mounts`, a method on the type it ranks).
- `Mount::bwrap_args(&self, confine_empty: &Path) -> Vec<String>`: the argv tokens of one mount, the current `match mount.kind` arms moved as they are (the `Overlay` `shortcut:` comment moves with its arm; `Empty` binds `confine_empty` onto the path; `Tmpfs` is `--tmpfs P`).
- `fn mounts(&self)`: one `extend` per source in the current order (overlay parents, hidden with `Tmpfs` for directories and `Empty` otherwise, writable, readonly, protected both `ReadOnly`, overlay_protected `ReadOnlyTry`), then the OMP directory as one flat `if`, so a later source still overrides an earlier one; sorts by depth, then `layer`, then path (the comparator stays an inline closure over `layer`).
- `pub(super) fn unset_credentials(&self) -> impl Iterator<Item = &'static str>` on `Plan`: the credentials that are neither released by a grant nor kept (`keep_env`), in `CREDENTIAL_ENV` order. It replaces the filter duplicated in `env_unsets` (`:133-136`) and `launch.rs` `notice` (`:169-174`); it has two callers.
- `fn env_unsets(&self)`: `ALWAYS_UNSET`, then the `env` of every grant not given, skipping names in `CREDENTIAL_ENV` (the `!CREDENTIAL_ENV.contains(name)` filter stays), then `unset_credentials()`; collected once with an inline first-occurrence dedupe (`let mut seen = BTreeSet::new(); ... .filter(|name| seen.insert(*name))`). Order and membership equal today's `unset.contains` checks (`ALWAYS_UNSET` has no duplicates, `confine.rs:45-52`).
- `fn wrap_with_codex_bind(&self, argv, app_server)`: same order as today: empty check, per-argument size check (one inline `if argv.iter().any(|a| a.len() > MAX_ARG_BYTES) { bail!(...) }`), `wrapped` starts with the bwrap path (its own push), then the fixed prefix as `const BWRAP_PREFIX: [&str; 15]` (the 15 tokens of `:166-180`), `--unsetenv` pairs, mounts (`let mounts = self.mounts();` iterated twice), the Codex directory bind (app server only), `--remount-ro P` for every `Tmpfs` mount in mount order (what `remounts.push` produced), the Codex socket bind (inline, `--bind` under `app_server`, `--ro-bind` otherwise), `--chdir`, `--`, argv, total size check. `confine_empty()` is read once before the loop.

`src/confine/launch.rs` (production code only):

- `dry_run_lines`: `auto_reason_lines(plan: &Plan, accepted: &BTreeSet<String>) -> String` (the reasons and the "not yet accepted" line; the caller calls it `if plan.auto`), `path_lines(plan: &Plan, resolved_cwd: &Path) -> String` (writable except the cwd, protected, hidden, overlay, unsaved, in that order), `unresolved_lines(plan: &Plan) -> String` (unresolved paths, then the skipped-missing hidden lines that are not unresolved). The `confinement:`, `grants:` and `keep_env:` lines stay inline.
- `notice`: `fn writable_first(plan: &Plan) -> Cow<'_, [PathBuf]>` (the reorder putting the cwd first), `fn harness_config_line(plan: &Plan) -> Option<String>`, `fn credential_line(plan: &Plan) -> String` (built over `plan.unset_credentials()`, then `; kept:`), `fn container_engines_line(plan: &Plan) -> String`. The fixed paragraphs stay inline; the result still goes through `truncate_before_closing`.

## 4. Out of scope

- Any change to what the agent sees, to the argv, or to a limit.
- The overlay and `without_overlays` machinery (owner decision pending).
- Every other file with findings (`confine.rs`, `bwrap.rs`, `git.rs`, `grant_dirs.rs`, `hides.rs`): separate specs.

## 5. Trust boundary

Creates none: no new input path; argv and notice come from the same plan, in the same order, byte for byte (Step A pins the text).

## 6. Acceptance

Baseline (before any edit, on `ef78e96`): `cargo test` passes (1497 passed, 3 ignored).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/confine/wrap.rs
cs check src/confine/launch.rs
cs check tests/confine/overlay_symlinked_agent.rs
git status --short
```

- All pass; `cs check` lists none of the 12 findings of section 1 and no new finding on the three files; the whole-tree cs count is below 905 (judge: the architect's `cs` run, `scripts/prepush-fast.sh`; owner rule: every commit lowers it).
- `git status --short` lists only the three files of section 0. No suppression, no `#[allow]`, no unused item, no `shortcut:` comment without an upgrade trigger; the `#[cfg(test)]` unit tests of `launch.rs` are unchanged (`git diff` shows no hunk past `:244`).
- Behaviour check: the three Step A tests pass on the unchanged source (the developer reports it; the tester reruns them on `git show HEAD:` sources of the two files in its scratch snapshot) and on the diff, and the whole suite including `--lib` and `--test auto_confine` passes on both trees.
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hk/` (both deleted before its REPORT); run the confine, team, auto_confine, codex and lib tests per mutant. Mutants, each by one change: (a) `layer` returns `0` for every kind; (b) the sort ignores the layer; (c) `mounts` inserts hidden after writable; (d) the `Empty` arm binds the path onto itself instead of `confine_empty`; (e) remounts are emitted for `Empty` mounts too; (f) `env_unsets` drops the `ALWAYS_UNSET` source; (g) `unset_credentials` ignores `keep_env`; (h) `env_unsets` drops the `!CREDENTIAL_ENV.contains(name)` filter of the grant step; (h2) `env_unsets` drops the dedupe; (i) the Codex socket bind uses `--ro-bind` under `app_server`; (j) the per-argument limit allows one byte more; (k) `writable_first` does not move the cwd; (l) `credential_line` omits `; kept:`; (m) `container_engines_line` ignores grants with sockets; (n) `unresolved_lines` prints skipped paths that are unresolved; (o) `path_lines` prints the cwd. Expected killers (consultant's read, to be confirmed by the tester): (a), (b), (d), (e), (i) `harnesses.rs:127` and `:340`; (f) `wrap_and_sockets.rs:17-34`; (g) `wrap_and_sockets.rs` keep_env tests and the new notice pins; (j) `limits_and_hides.rs:569`; (k) `notices_and_preflight.rs:75`; (m) `notices_and_preflight.rs:4`; (n) `tests/team/dry_run.rs:127-130`; (o) the `launch.rs` unit test `dry_run_lines_uses_resolved_cwd_when_filtering_writable_paths` and the new dry-run pin. Expected survivors or equivalents, reported with the reason: (c) if the builder never puts one path in both `hidden` and `writable`; (h2) EQUIVALENT (the sources are disjoint, pinned by the `confine.rs` unit test `grant_socket_and_environment_names_are_unique`); (h) a survivor if no ungiven grant carries a credential name. A survivor in moved code is a fix request to the developer (a new test in `tests/confine/`); one no public request can reach is reported with that reason and dispositioned by the architect.

## 7. Claims (architect and consultant, `ef78e96`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the 12 findings and their lines | `wrap.rs:64,114,134,153`; `launch.rs:44,104` | `cs check` run, whole-tree output read |
| `mounts` later sources override earlier ones | `wrap.rs:64-91`, `BTreeMap::insert` | read |
| the `Tmpfs` arm emits `--tmpfs` (`:229-232`); remounts (`:238-240`) come after the Codex directory bind and before the socket bind (`:235-248`) | `wrap.rs` | read |
| `wrap` and `wrap_app_server` differ only by the directory bind and the socket bind mode | `wrap.rs:144-151`, `:235-248` | read |
| the bwrap prefix is 15 tokens, after the separate `bwrap` push | `wrap.rs:163-180` | counted |
| the argv is pinned byte for byte | `tests/confine/harnesses.rs:127`, `:185-318` | read (consultant) |
| notice and dry-run text have no full-text pin | `notices_and_preflight.rs` (line-level only), `tests/team/dry_run.rs:114-135`, `launch.rs:244-356` unit tests | read (consultant); grep |
| callers: `notice` at `src/session/spawn.rs:230`, `src/daemon/launch.rs:610`; `dry_run_lines` at `src/commands/team/dry_run.rs:156`, `tests/confine/overlay_symlinked_agent.rs:55`; `wrap` at `src/confine/builder.rs:530`; `wrap_app_server` at `src/codex.rs:565`; `launch_argv` calls neither text function | grep | grep (consultant) |
| tests that also pin wrap, notice or dry-run: `tests/confine/{harness_config,limits_and_hides,grants_links_and_env,overlay_symlinked_agent,plan_basics,paths_and_git_config,grant_writes_and_create,real_bwrap}.rs`, `tests/auto_confine/{dry_run_reports,daemon_use}.rs`, the `launch.rs` unit tests | grep | grep (consultant) |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-senior-developer` on the agy harness, planning and working model Gemini 4 Hydrogen (Medium), no advisor (single-slot harness); tester `a2amx-tester` on the omp harness, planning and working model `opencode-go/deepseek-v4.1-flash` (advisor model not exposed).

BLOCKED/amend cycles: none (the pre-lock review's findings were applied before locking, no second review).

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The behaviour check (capture output before and after) cannot run: fixtures use random temp paths and tests print nothing; nothing pins the notice and dry-run text byte for byte | Replaced by Step A, three full-text pin tests written first and green on the old code |
| Draft review | consultant | Single-use helpers (`check_argument_sizes`, `codex_socket_bind`, `first_occurrences`, `mount_order`) only hid branches; the credential filter was duplicated in `wrap.rs` and `launch.rs` | Helpers dropped; one shared `Plan::unset_credentials` with two callers |
| Draft review | consultant | Callers, anchors and the test inventory in the claims table were wrong or incomplete (`launch_argv` calls neither text function; the Tmpfs and Codex bind anchors drifted); mutant (h) was undefined | Corrected before locking; (h) and (h2) defined as concrete changes |
| Draft | architect | `BWRAP_PREFIX` was drafted as 14 tokens; it is 15 | Counted and corrected in the draft |
| Tester pass | tester | Spec 6 expected mutant (h) to survive; it is killed (an ungiven credential grant plus `keep_env` makes the order observable) | Recorded; no fix needed |
| Design read | consultant | Nits only: `path_key(&self.path)` repeated per arm in `Mount::bwrap_args`; six near-identical `mounts.extend` calls; `env_unsets` collects only to keep its `impl Iterator` return; `unset_credentials` would sit better beside `granted_env` in `confine.rs`; the pins sit in the overlay test file | Not applied: the first three follow the locked spec's shape and a change would invalidate the tested snapshot; placement and test location go to the later `confine.rs` and test specs |

Tester mutants (scratch with its own `CARGO_TARGET_DIR`, deleted afterwards): 20 runs, (a)-(o), (h2) and four added: 18 killed, (c) hidden-after-writable SURVIVED as expected (the builder keeps `hidden` and `writable` disjoint), (h2) dedupe removal EQUIVALENT (the three env sources are disjoint). Architect spot-check (own scratch copy and target dir, deleted; `wrap.rs` restored and `cmp`-identical): (g) `unset_credentials` ignoring `keep_env` killed by four tests (both new notice pins and two `wrap_and_sockets` tests); (p) Codex directory bind moved after the remounts killed by `harnesses::codex_plan_wraps_own_socket_dir_with_read_only_and_read_write_variants`. Fresh checks by the architect: `cargo test` 1500 passed, 0 failed (1497 baseline plus three pins); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean; `cs check` 10.00 on `wrap.rs` (was 8.45), `launch.rs` (was 9.07) and the test file; whole-tree cs warnings 905 to 894.
