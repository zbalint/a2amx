# Spec 3bj: `~` in `--confine-write`, `--confine-hide`, `--confine-unhide` and the team-file keys

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `f135a74`; found by the spec 3bi consultant review `m_4629`, finding 1; no second consultant review). Architect-directed bug fix, context `a2amx-confine-tilde-2026-10-30`.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `f135a74`; full suite 698 passed, 0 failed, 3 ignored (writable HOME).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** `a2amx team up --dry-run` through `tests/team.rs`.

**Scope.** May edit exactly these files and no others: `src/main.rs` (`resolve_confine_paths`, line 592) and
`tests/team.rs` (the new test). Does not touch: `src/confine.rs`, `src/team.rs`, `CHANGELOG.md` (the architect edits it),
docs, every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

The daemon expands a leading `~` (`expand_home`, `src/confine.rs:272-278`: `~` and `~/...` only). But the CLI and
the team commands run every confinement path through `resolve_confine_paths(base, paths)` first, which joins any path
that is not absolute to `base` (the session's working directory). `"~/.cargo"` is not absolute, so it becomes
`<cwd>/~/.cargo`, a path that does not exist and is silently skipped: a `confine_write = ["~/.cargo"]` in a team file, and
a quoted `--confine-write '~/.cargo'`, do nothing. Only an unquoted `~` (the shell expands it) or an absolute path works.
The tests that use `~` call `confine::plan` directly and never pass through `main.rs`. Spec 3ax lists `~` forms
(`~/.aws`, `confine_unhide = ["~/.ssh/config"]`) and the docs and the example team file (spec 3bi) rely on them. This is a
bug, not a feature.

## 2. Change

`resolve_confine_paths` leaves a path alone when it is `~` or starts with `~/`; every other relative path is joined to
`base` as before; absolute paths are unchanged:

```rust
fn resolve_confine_paths(base: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .map(|path| {
            if path == "~" || path.starts_with("~/") {
                return path.clone();
            }
            ...unchanged...
```

Keep the existing structure and the `map`/`collect` shape as far as it reads naturally; the only behavior change is the
early return. The tilde forms are exactly the ones `expand_home` accepts: do not add `~user`. The expansion stays in the
daemon (`confine::plan`), and the dry run plans in the CLI process with the same function, so both see the same `HOME`.
All six call sites (`src/main.rs:461-463`, `:1049-1051`, `:1136-1138`) use this function; none changes.

## 3. Test to add (name is a suggestion)

`dry_run_expands_a_home_relative_confine_write` in `tests/team.rs`, `#[tokio::test]`, in the style of
`dry_run_prints_wired_commands_without_starting_or_installing` (it starts `common::start_daemon()` and runs the binary):

1. A fixture user home: `tempfile::tempdir()`, containing the directory `.tool-cache` (`std::fs::create_dir`).
2. A team file with one confined Generic session:

   ```toml
   [[session]]
   name = "demo"
   command = ["sh", "-c", "echo demo"]
   cwd = "."
   confine = true
   confine_write = ["~/.tool-cache", "~/.missing-tool"]
   ```

3. Run `team up --dry-run --file <file>` with `Command::new(env!("CARGO_BIN_EXE_a2amx"))` built by the test (the helper
   `run_binary_in_with_home` in `tests/team.rs` sets `HOME` but not `XDG_RUNTIME_DIR`, which the dry run requires and CI
   may not set): `--home <daemon state dir>`, `current_dir` the state dir, `HOME` the fixture home, `XDG_RUNTIME_DIR` a
   second temp directory, stdin null. Extending the helper with a runtime-directory parameter is allowed if it keeps its
   callers unchanged (a new helper next to it is also fine).
4. Assert success and that stdout contains the line `  writable: <fixture home>/.tool-cache` (canonical path:
   compare with `std::fs::canonicalize(home.join(".tool-cache"))`), that stdout contains no `/~/` or `~/` substring, and
   that the missing `~/.missing-tool` is not reported as writable. How a missing write path appears (`skipped`) is not
   asserted.
5. The test fails on the unchanged `src/main.rs` (red phase: the writable line is absent and the output names
   `<cwd>/~/.tool-cache` as a skipped path) and passes after the change.

## 4. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test team
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/main.rs and tests/team.rs
```

- Known flaky: backlog I9 and F37; the 3bf run also saw `tests/team.rs::team_up_forwards_reset_configuration_and_consent`
  (`not_ready`) once: rerun once and record as pre-existing.
- The tester applies these mutants to a scratch copy of the final snapshot (`git archive`, one shared `CARGO_TARGET_DIR`,
  restore after each) and names the test that fails: (1) remove the early return (the original bug); (2) match only `~/`
  and not a bare `~`; (3) match `starts_with('~')` so `~user/x` and `~x` are left alone (a relative file named `~x` would
  no longer be joined to the base); (4) keep the early return but apply it only to `confine_write` call sites (revert one of
  the six calls to a join). Mutant 2 is expected to survive unless the developer's test also uses a bare `~`; the spec does
  not require a test for it, and the tester reports it. Mutant 3 is expected to survive too: it is a refinement
  the spec chose not to pin; report it. Mutant 4 needs a hide or unhide `~` case to be killed: expected survivor unless the
  developer adds one, which is allowed (one more entry in the same dry run: `confine_hide = ["~/.tool-hide"]` is shown
  as a `hidden:` line) and encouraged.

## 5. Out of scope

`confine::plan`, `expand_home`, `~user` forms, environment-variable expansion, Windows paths, the `new --confine-write`
command line (its quoted `~` goes through the same function; a CLI test needs a daemon with real bubblewrap and is not
required), docs (spec 3bi), and the changelog.

## 6. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `resolve_confine_paths` joins every non-absolute path to the base | `src/main.rs:592-604` | read |
| It is used for write, hide and unhide at three places: `new`, the team start, the dry run | `src/main.rs:461-463`, `:1049-1051`, `:1136-1138` | grep |
| The daemon's `expand_home` expands `~` and `~/...` only | `src/confine.rs:272-278` | read |
| The dry run plans in the CLI process using the local `HOME` and `XDG_RUNTIME_DIR` (the latter is required) | `src/main.rs:1038-1062` | read |
| The dry run prints `writable: <path>` for writable paths other than the cwd, and `hidden:` / `hidden: ... (skipped missing)` | `src/main.rs:1096-1108` | read |
| `run_binary_in_with_home` sets `HOME` and `--home` but not `XDG_RUNTIME_DIR` | `tests/team.rs:599-613` | read |
| No existing test passes a `~` path through the CLI or team layer | consultant `m_4629` (greps of `tests/`), architect `rg -n 'confine_write' tests/team.rs` shows only parse tests | grep |
| `confine::plan` canonicalises existing paths, so the writable line is the canonical path | spec 3ax 2.3 | read |
