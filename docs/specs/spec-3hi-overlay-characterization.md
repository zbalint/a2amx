# Spec 3hi: characterize the overlay plan for an OMP agent directory that is a symlink

## 0. Status

LOCKED after consultant review m_9481 (four minor findings applied; no rule changed, so no second review). Context `a2amx-3hi-overlay-removal`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Test-only: no production file changes. Public seams: `confine::plan`, `Plan::overlay_parents`, `Plan::unsaved_files`, `confine::notice` and `confine::dry_run_lines` (all already exported; every one except `dry_run_lines` is already used by `tests/confine/`).

Scope (edit): new file `tests/confine/overlay_symlinked_agent.rs` and one `mod overlay_symlinked_agent;` line in `tests/confine/main.rs` (alphabetical, after `mod notices_and_preflight;`). Does not touch: any `src/` file, any other test file, `docs/` other than this spec, `CHANGELOG.md`.

## 1. Why

The overlay machinery in `src/confine/harness_data.rs` (`harness_overlay_parents`, `overlay_protected_entries`, `unsaved_harness_files`, and their plumbing through `builder.rs`, `wrap.rs`, `bwrap.rs`, `launch.rs`, `daemon/launch.rs`) is about 250 production lines. Spec 3hc concluded the three overlay lists are always empty through `confine::plan`. That is wrong for one input, found by a consultant read and confirmed by an architect probe on `4e8b7b3`: OMP with `~/.omp/agent` a symlink to a directory X outside `~/.omp` that holds a `*.db`. `DataRoots` treats `~/.omp/agent` as its own data source, so X is accepted, and X is in none of the writable, hidden, read-only or protected sets (the canonical `~/.omp` is writable through the automatic harness-config grant, and X is not within it), so it becomes an overlay parent. The agent is then told "X: writable, but only the harness's data files are saved; other writes are discarded at exit." Nothing tests a successful plan for this input; `tests/confine/harnesses.rs:530-547` and `:1051-1068` cover only refusals (X is the a2amx state directory; a cache escape).

The owner has said not to break existing users who symlink their agent directory. Whether to keep, remove or refactor the machinery is a later decision; this spec pins what exists so that decision, and any removal, is measured against tests. Alternatives rejected: removing the machinery now (changes behaviour for the symlinked case, owner has not decided); adding the tests to `tests/confine/harnesses.rs` (1,392 lines, already past the 1,000-line code-health goal, so the new tests get their own small file).

## 2. `tests/confine/overlay_symlinked_agent.rs`

Uses the shared `Fixture` (`tests/confine/main.rs`), `confine::plan(&ConfineRequest, Harness::Omp, &fixture.cwd, &fixture.host())`, and `std::os::unix::fs::symlink`. Each test builds `~/.omp` under `fixture.home`, a target directory with `sessions/` and `a.db`, and `~/.omp/agent` as a symlink to it. Do not reuse `populate_omp_overlay` (`main.rs:205`): it builds a real agent directory and a `~/.omp/stats.db`, and the mutation reasoning below needs the target's `a.db` to be the only data file in play. The outside target lives under the fixture's temp root (`fixture._root.path()`, reachable from a child module of `main.rs`; the architect's probe did exactly this and compiled), never under `fixture.home/.omp`. Every expected path is the canonicalized target (`std::fs::canonicalize`), written into the assertion as that variable, not recomputed with plan logic.

Four tests, all contract tests (they pass on the current code; there is no red phase):

| Test | Setup | Expected |
| --- | --- | --- |
| `omp_agent_symlinked_outside_the_root_plans_the_target_as_overlay_parent` | target outside `~/.omp`, default request | `Ok`; `overlay_parents()` equals `[canonical target]`; `unsaved_files()` is empty; `writable` contains the canonical `~/.omp` |
| `overlay_parent_shows_in_the_notice_and_the_dry_run` | same plan | `confine::notice(&plan)` contains the line `{target}: writable, but only the harness's data files are saved; other writes are discarded at exit.`; `confine::dry_run_lines(...)` contains the line `  overlay: {target}` |
| `omp_agent_symlinked_inside_the_root_has_no_overlay_parent` | target `~/.omp/agent-real` (inside the root) | `Ok`; `overlay_parents()` is empty |
| `writing_the_symlinked_agent_target_has_no_overlay_parent` | target outside the root, request `write: [target]` | `Ok`; `overlay_parents()` is empty |

Probe results on `4e8b7b3` (throwaway tests, removed): row 1 plan had `overlay_parents == [target]`, `unsaved_files == []`; row 3 gave `Ok([])`; row 4 gave `Ok([])`; the notice for row 1 contained the line quoted in row 2.

## 3. Out of scope

- Any production change, including removing or refactoring the overlay machinery, and any change to the text the agent sees. Follow-up for the owner once these tests exist.
- `overlay_protected_entries` has no public accessor, and the spec adds none; it is exercised only through the plan. `unsaved_files()` is pinned empty here; a non-empty case is not reachable with an OMP layout (`sqlite_files` sidecars are skipped when the database exists), and no test is invented for it.
- Real bwrap behaviour of the overlay mount (`tests/confine/real_bwrap.rs`).

## 4. Trust boundary

Creates none: tests only, in a temp fixture, no new input path.

## 5. Acceptance

Run from the checkout, with the repo's documented forms:

```sh
cargo test --test confine overlay_symlinked_agent
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short
```

- The first command runs exactly 4 tests and all pass (judge: its output).
- `git status --short` lists only the two files of section 0 (judge: the tester and the architect).
- Mutation check, run by the tester only in a scratch snapshot under `target/scratch/3hi/` per the tester skill's recipe, never in the shared checkout; the tester deletes the snapshot before its REPORT. The tester breaks each behavior in `src/confine/harness_data.rs` and names the test that fails: (a) `harness_overlay_parents` returns an empty list (kills tests 1 and 2); (b) `harness_overlay_parents` drops its whole `.any(... is_within ...)` skip, so it plans every data-file parent (kills tests 3 and 4, whose targets are inside the root or in `writable`). A mutant no new test kills is a finding.
- No test in the new file depends on the machine's real home, real `~/.omp`, or a fixed port (judge: the architect reads the file).
- `cs` check: the whole-tree `cs` count must not rise, and the new file stays under 100 lines (judge: `cs delta` in `scripts/prepush-fast.sh`).

## 6. Claims (architect, `4e8b7b3`)

| Claim | Where | Verified how |
| --- | --- | --- |
| symlinked outside target gives `overlay_parents == [target]`, `unsaved == []` | `harness_data.rs:136-162` | probed (throwaway test, removed) |
| target inside root gives no overlay parent; `write: [target]` gives none | same | probed |
| the notice line text and `  overlay: ` dry-run prefix | `launch.rs:79-84`, `:137-141` | read; notice text seen in probe output |
| `notice` and `dry_run_lines` are exported; `notice` is used by tests, `dry_run_lines` by none (grep of `tests/`) | `confine.rs:35`; `tests/confine/notices_and_preflight.rs:16` | read, grepped |
| `Fixture` fields are reachable from a child module | `tests/confine/main.rs:24-31` | probe compiled |
| no existing test covers a successful symlinked-agent plan | `tests/confine/harnesses.rs:530-547` and `:1051-1068`, refusals only | read; grep `overlay_parents` in `tests/` hits only emptiness asserts |
| `dry_run_lines` signature `(plan, resolved_cwd, accepted: &BTreeSet<String>)` | `launch.rs:44` | read; no test calls it, so the developer passes `fixture.cwd` as the resolved cwd and an empty `BTreeSet<String>`, and BLOCKs if that does not print the `overlay:` line |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-junior-developer` on the omp harness, planning and working model `openai-codex/gpt-5.6-luna` (advisor model not exposed by the harness); tester `a2amx-tester` on the omp harness, planning and working model `opencode-go/deepseek-v4.1-flash` (advisor model not exposed).

BLOCKED/amend cycles: one spec amendment (`bfc0dba`, the mutation form), no BLOCKED reply.

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Before the draft | consultant (read-only) | Spec 3hc's claim that the three overlay lists are always empty through `confine::plan` is false for an OMP `~/.omp/agent` symlinked outside `~/.omp` | Architect probe confirmed it (`overlay_parents == [target]`); this spec pins it |
| Draft review | consultant | The claims table cited one refusal test where two exist (`harnesses.rs:530-547` and `:1051-1068`); the spec did not forbid reusing `populate_omp_overlay`, which would break the mutant reasoning; the mutation form was ambiguous; the "harness root" wording described a check the code does not have | All four applied before locking (`3f9807b`) |
| Tester pass | tester (omp) | The spec's in-checkout mutation form (taken from AGENTS.md) contradicts the tester skill's scratch-only rule; the tester declined to mutate the shared source and asked | Architect first told the tester to skip mutation (wrong: mutation is the tester's job); the owner corrected it; spec amended (`bfc0dba`) and the tester ran the pass in scratch |
| Tester pass | tester (omp) | Its mutation scratch used the checkout's `CARGO_TARGET_DIR`, so its first reruns after the mutants may have run a mutant binary | Self-found; `cargo clean -p a2amx` and a full pristine rerun; the assignment had not required an isolated target dir |
| Architect review | architect | none found in the diff: four tests match section 2, literal expected values, no suppressions, no deleted tests | n/a |

Tester mutants: (a) empty return killed by tests 1 and 2; (b) dropped skip killed by tests 3 and 4; its own (c) `parent == home` guard survives the new file and is killed by five existing tests; its own (d) `!parent.is_dir()` is equivalent. Architect spot-check (scratch copy with its own target dir, deleted afterwards; `harness_data.rs` restored and `cmp`-identical after each): (a) killed by tests 1 and 2, (b) killed by tests 3 and 4, own mutant (e) dropping `writable` from the skip list killed by tests 3 and 4. Fresh checks by the architect: `cargo test` 1494 passed, 3 ignored, 0 failed (1490 baseline plus the four new tests); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
