# Spec 3hj: function-level code health of the overlay planning in `src/confine/harness_data.rs`

## 0. Status

LOCKED after consultant review m_9512 (four findings applied, F1 and F3 changed how the helpers are written, not what they do, so no second review). Context `a2amx-3hj-overlay-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `5529c85`. Behaviour-preserving refactor, test-first. Public seams: `confine::plan` and `Plan::wrap` (the `--ro-bind-try` mounts), `Plan::overlay_parents`, `confine::notice`, all through the existing `Fixture`.

Scope (edit): `src/confine/harness_data.rs` (the three overlay functions and the new items of section 3), `src/confine/builder.rs` only for `overlay_data` (`:444-469`), `tests/confine/overlay_symlinked_agent.rs` (three new tests and one helper, section 2). Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`, `src/confine/wrap.rs`, `launch.rs`, `bwrap.rs`, `daemon/launch.rs`.

This spec changes no mount, no hide, no ordering of the sorted outputs, no error text, no file mode and no text the agent sees. It does not remove the overlay machinery; that stays the owner's decision.

## 1. Why

`cs check src/confine/harness_data.rs` (2026-11-26, `5529c85`) reports four findings on two functions: `harness_overlay_parents` (`:136`, 6 arguments) and `overlay_protected_entries` (`:164`, Complex Method cc 10, 7 arguments, and two complex conditionals at `:183`). Spec 3hi pinned `overlay_parents()`, the notice and the dry run for the one input that reaches this code (an OMP `~/.omp/agent` that is a symlink outside `~/.omp`), but `overlay_protected` has no accessor, so nothing yet pins which entries of the overlay parent become `--ro-bind-try` mounts. Refactoring that function without a pin would be guessing.

The cause of the argument counts is shared: the plan's four path sets (writable, hidden, read-only, protected) travel together as separate slices through both functions, and the harness data (directories and files) is already a named type (`HarnessData`) that `overlay_data` takes apart again. Rule from earlier refactors that binds here: no helper that exists only to clear a finding, no out-parameter accumulator, at most four parameters on every new function (`self` included), split only into contiguous runs so the order of checks and results stays.

Alternatives rejected: removing the machinery (behaviour change for symlinked OMP agent directories, owner decision, not taken); `#[allow(clippy::too_many_arguments)]` or a suppression (forbidden, and hides the cause); one big options struct holding every input including `home` and `parents` (a bag: `home` and `parents` belong to one function each).

## 2. Step A: characterize `overlay_protected` first (tests, green on the current code)

In `tests/confine/overlay_symlinked_agent.rs`, add one helper and three tests. Write and run them against the unchanged source first; they must pass there (contract tests, no red phase), before any change in section 3.

Layout (a new helper next to `symlinked_agent_fixture`, which it extends rather than copies): the outside target X (canonical) additionally holds a directory `extra/`, a regular file `notes.txt`, and a symlink `link` to `notes.txt`; it already holds `sessions/` and `a.db`.

Helper `ro_bind_try_names(argv: &[String], target: &Path) -> Vec<String>` (the test file needs `use std::path::Path;`): the file names of every `--ro-bind-try P P` triple in a wrapped command (`plan.wrap(vec!["true".into()])`) whose `P` is directly inside `target`, sorted.

| Test | Request | Expected `ro_bind_try_names` (literals) |
| --- | --- | --- |
| `overlay_protected_entries_are_the_unlisted_plain_entries_of_the_overlay_parent` | default | `["extra", "notes.txt"]` (`link` is a symlink and skipped; `a.db` is a data file and `sessions` a data directory, both already mounted `--bind`) |
| `a_written_entry_is_not_overlay_protected` | `write: [X/extra]` | `["notes.txt"]` |
| `a_hidden_entry_is_not_overlay_protected` | `hide: [X/extra]` | `["notes.txt"]` |

Probe results on `5529c85` (throwaway test, removed): default `--ro-bind-try /extra`, `--ro-bind-try /notes.txt` (plus `--bind /a.db`, `--bind /sessions`, `--tmp-overlay` of X); write-extra `--bind /extra`, `--ro-bind-try /notes.txt`; hide-extra `--ro-bind-try /notes.txt` only.

## 3. Step B: target shape (the design is the instruction; the `cs` result is the check)

In `src/confine/harness_data.rs`:

- `pub(super) struct PathSets<'a> { pub(super) writable: &'a [PathBuf], pub(super) hidden: &'a [PathBuf], pub(super) readonly: &'a [PathBuf], pub(super) protected: &'a [PathBuf] }`: the plan's four path sets that decide who owns a path. Methods: `fn entries(&self) -> impl Iterator<Item = &PathBuf>` (every entry of the four sets; the one place the four sets are listed) and `fn covers(&self, path: &Path) -> bool`, written as `self.entries().any(|entry| is_within(path, entry))` (today's nested `.any` over the four-set array).
- `harness_overlay_parents(home: &Path, data_files: &[PathBuf], sets: &PathSets) -> Vec<PathBuf>` (3 parameters; same loop, same order of `continue` checks, `sets.covers(&parent)` in place of the array expression).
- `overlay_protected_entries(parents: &[PathBuf], data: &HarnessData, sets: &PathSets) -> Vec<PathBuf>` (3 parameters). One helper:
  - `fn plain_entries(parent: &Path) -> Vec<PathBuf>`: the regular files and directories directly in `parent`; symlinks and other file types, an unreadable parent and an unreadable entry are skipped, as today. Write the type test with one operator: `let kind = metadata.file_type(); if !kind.is_file() && !kind.is_dir() { continue; }` (from `fs::symlink_metadata`). The `is_symlink()` test of the current code (`:183`) is redundant, because `symlink_metadata` of a link reports neither a file nor a directory, so dropping it keeps the result; moving the current `is_symlink() || (!is_file() && !is_dir())` over unchanged would only move the Complex Conditional finding.
  - The exclusion check stays inline in the loop, with no helper: an entry is skipped when some `other` in the concatenation of `data.dirs`, `data.files`, `sets.entries()` and `parents` satisfies `other == &path || canonical_if_exists(other).is_some_and(|canonical| canonical == path)`, evaluated per entry and per `other` exactly as today (do not precompute the canonical forms: the behaviour diff check at review stays literal). Order of the concatenation does not matter.
  - The function keeps the `add_unique` accumulation and the final `sort()`.
- `unsaved_harness_files` is not touched.

In `src/confine/builder.rs`, `overlay_data` builds one `PathSets` from `self.writable`, `self.hidden`, `self.readonly` and the `protected` argument, and passes it to both calls together with `&data`. Nothing else in the file changes.

The consultant attacks this shape before the lock; the architect decides afterwards.

## 4. Out of scope

- Removing or redesigning the overlay machinery, and any behaviour change for the symlinked OMP agent directory case. Follow-up for the owner.
- `unsaved_harness_files`, `DataRoots`, and every other function in the file.
- Real bubblewrap behaviour of the overlay mounts (`tests/confine/real_bwrap.rs`).

## 5. Trust boundary

Creates none: no new input path, no change to which paths are opened or protected; the characterization tests of section 2 show it.

## 6. Acceptance

Baseline (run before any edit, on `5529c85`): `cargo test --test confine overlay_symlinked_agent` passes 4 tests.

Post-implementation:

```sh
cargo test --test confine overlay_symlinked_agent
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/confine/harness_data.rs
git status --short
```

- The first command runs exactly 7 tests and all pass; the three new ones also passed on the unchanged source (the developer reports the run; the tester reruns it on `git show HEAD:src/confine/harness_data.rs` in the scratch snapshot).
- `cs check src/confine/harness_data.rs` lists none of the four findings of section 1 and no new finding; the code health score is above 9.09. The whole-tree `cs` count does not rise (judge: the architect's `cs` run and `scripts/prepush-fast.sh`, owner rule of 2026-11-26).
- `git status --short` lists only the three files of section 0. No suppression, no new `shortcut:` comment without an upgrade trigger, no unused item.
- Mutation check, run by the tester only in a scratch snapshot under `target/scratch/3hj/` with its own `CARGO_TARGET_DIR` under `target/scratch/3hj/target` (never the checkout's `target`), deleting both before its REPORT. Mutants on the refactored code, each by one change: (a) `harness_overlay_parents` returns empty; (b) `covers` always false; (c) `covers` ignores `writable`; (d) `plain_entries` follows links (uses `fs::metadata`); (e) the exclusion check is always false; (f) the exclusion concatenation drops `data.files` and `data.dirs`; (g) the concatenation drops `sets.hidden`; (h) the concatenation drops `parents`; (i) `overlay_protected_entries` skips the final `sort`. Expected: (a) killed by tests 1 and 2 of 3hi; (b) killed by 3hi rows 3 and 4 and the new write test; (c) killed by 3hi rows 3 and 4; (d) killed by the default test of section 2 (`link` appears); (e) killed by the default test (`a.db` and `sessions` appear); (f) SURVIVED: existing data files and directories are already in `Plan.writable` at their canonical paths before the overlay is planned (`harness_data.rs` `mount_data_file` and `mount_data_dir`), so the entry is excluded through `sets.writable` anyway; the tester reports it with that reason, and removing the redundant part is a later behaviour question, not this spec; (g) killed by the hide test, because `overlay_protected` is inserted into the mount map after `hidden` (`wrap.rs:69-87`) and would override the hide; (h) none: for OMP only one overlay parent can exist (the canonical `~/.omp` is writable, so `stats.db` plans none), so `parents` never holds an entry of another parent; report it as SURVIVED with that reason, and the same for dropping `sets.readonly` or `sets.protected`, which no request in this suite puts an entry of the parent into (`ConfineRequest` has no read-only field, `confine.rs:89-101`; `protected` comes from `protected_paths(readonly, writable)`, `builder.rs:548`); (i) none: mounts are keyed by path in a `BTreeMap` (`wrap.rs:64`), so the order of the result is not visible through `wrap` and no `Plan` accessor shows it; report it as EQUIVALENT through the public seams, the architect dispositions. A mutant outside this list that survives is a finding.
- Behaviour check: `git diff` of the two overlay functions shows the same comparisons, in the same order, with the same `continue` and `add_unique` semantics; the architect reads it at review.

## 7. Claims (architect, `5529c85`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the four findings and their lines | `harness_data.rs:136`, `:164`, `:183` | `cs check` run |
| `overlay_protected` shows as `--ro-bind-try P P` in `plan.wrap` argv | `wrap.rs:86-87`, `:215-221` | read and probed |
| default / write / hide expected mount names | probe on the layout of section 2 | probed (throwaway test, removed) |
| `overlay_data` is the only caller of both functions | `builder.rs:444-469`; grep `harness_overlay_parents` and `overlay_protected_entries` in `src/` and `tests/` | read; grep shows only `builder.rs:13` import and the two calls |
| no test calls either function directly | grep of `tests/` | grep (no hit) |
| the harness-config grant puts the parent in `writable` for the OMP default | `builder.rs:209-235`, probe of 3hi row 1 | read and probed |
