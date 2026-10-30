# Spec 3bg: the real-bwrap test does not need a writable `$HOME`

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `a9a9156`; no independent spec review: a two-line fixture move, every claim a recorded grep or run). Architect-directed, backlog F40, context
`a2amx-real-bwrap-test-dir-2026-10-30`. Implementation goes to `a2amx-developer` only; no tester pass (see section 4).

**Baseline:** develop at `a9a9156`. In a confined session the exact `cargo test` shows 51 passed, 1 failed in the confine
suite: `real_bwrap_confines_the_session_process` (`Read-only file system` creating a temp directory under `$HOME`). With a
writable `HOME` the suite passes.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** a real confined session through the daemon, `tests/confine.rs`. No production change.

**Scope.** May edit exactly one file: `tests/confine.rs` (the test `real_bwrap_confines_the_session_process`, lines
1845-1850). Does not touch: every other file, including `src/`. Leave the diff uncommitted.
**Ordering:** after spec 3bf is committed (it also edits `tests/confine.rs`).

## 1. Why

The test needs a directory outside the session's workspace and outside `/tmp` (inside the sandbox `/tmp` is a private
tmpfs), on the host filesystem: it proves a write outside the workspace is denied, a `chmod` there is denied, and a
`confine_hide` path there is empty. It gets one with `tempfile::tempdir_in($HOME)`. In a confined session `$HOME` is
read-only, so the test fails there with `Read-only file system`, and an agent that runs the exact `cargo test` sees a
failure that is not a product bug. Skipping the test would hide the only test that runs real bubblewrap; a writable
directory in `$HOME` in the team file weakens the sandbox for a test flaw. Cargo provides the right place:
`CARGO_TARGET_TMPDIR`, a scratch directory inside `target/` for integration tests, writable wherever the build runs.

## 2. Change

In `real_bwrap_confines_the_session_process`, replace the two lines

```rust
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
    let external = tempfile::tempdir_in(&home)?;
```

with

```rust
    let external = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?;
```

`home` is used nowhere else in the test (claims table). Add one comment line above it saying why the directory is not
`$HOME` and not `/tmp` (outside the workspace, not under the sandbox's private `/tmp`, writable when `$HOME` is not).
If removing `home` leaves `PathBuf` or `Context` unused in the file, clippy says so; they are used by other tests today
(claims table), so expect no import change.

## 3. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR A2AMX_REQUIRE_BWRAP=1 cargo test --test confine real_bwrap_confines_the_session_process
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short          # only tests/confine.rs
rg -n 'tempdir_in' tests/confine.rs   # one hit, with CARGO_TARGET_TMPDIR
```

- The first command must pass with `HOME` left at its confined, read-only value (do not override `HOME`); that is the
  point of the change. Use your own `CARGO_HOME`, `RUSTUP_HOME` and `CARGO_TARGET_DIR` as in earlier assignments, and
  `A2AMX_REQUIRE_BWRAP=1` so the test cannot skip. Report the exact-environment full `cargo test` as well: it must now be
  free of the F40 failure (known flaky: backlog I9 and F37, rerun once and record).
- The negative control of the first draft (hide a nonexistent path and see an assertion fail) is withdrawn by Amendment 1:
  it cannot fail on the unchanged test either.

## 4. Out of scope

Production code, other tests, `$HOME` handling anywhere else, the team file, a skip path. No tester pass: a one-line
fixture move whose evidence is the passing run under a read-only `HOME` plus the architect's scratch-copy run before lock
(claims table). The architect repeats the first acceptance command at review.

## 5. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| The test builds its outside fixture with `tempdir_in($HOME)` and uses `home` nowhere else | `tests/confine.rs:1849-1850`; `awk` + `rg '\bhome\b'` over lines 1845-2010 found only those two | grep |
| `CARGO_TARGET_TMPDIR` is set by cargo for integration tests | not read from the cargo docs by the architect; proven by the run below | probed |
| With `tempdir_in(env!("CARGO_TARGET_TMPDIR"))`, a read-only `$HOME` and `A2AMX_REQUIRE_BWRAP=1`, the test passes | architect run, 2026-10-30, scratch `git archive` copy of develop after `9352c1e` with only this change: `1 passed; 0 failed`, 1.09 s | probed |
| No other test in `tests/` uses `$HOME` for a fixture | `rg -n 'tempdir_in' tests` printed one hit, `tests/confine.rs:1850` | grep |
| `PathBuf` and `Context` are used elsewhere in `tests/confine.rs` | `rg -c PathBuf tests/confine.rs` printed 26, `rg -c 'Context|\.context\('` printed 16 | grep |

## Amendment 1 (negative control withdrawn; no scope change)

2026-10-30, after the developer's `BLOCKED` (`m_4618`). Section 3 required a scratch-copy negative control: hide a
nonexistent path and report the assertion that fails. The developer ran it on the final snapshot and the test passed.
Read of the unchanged test (`tests/confine.rs`, the assertions after `done`): `assert!(result_text.contains("hidden"))`
is satisfied by the other markers `hidden-write-denied` and `admin-hidden`, and the script's `test -e {hidden}` is true
for the empty tmpfs a correct hide produces, so the test never checks that the hidden directory is empty. The move to
`CARGO_TARGET_TMPDIR` does not change this; it is a weakness of the committed test, now backlog F42 (a separate tests-only
spec). The spec's claim that the control "guards against weakening the fixture" was false, and the control is removed
from section 3. Acceptance is otherwise unchanged: the focused command passes with the confined, read-only `HOME`.
Gate step 7 re-run: "negative control" appears in section 3 (withdrawn) and in this amendment only.

## Review record

Models: architect Claude Sonnet 5.5; no consultant review (a two-line fixture move, see section 0); developer on the OMP
harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed); no tester pass.
BLOCKED/amend cycles: 1.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Implementation | Developer (`gpt-5.6-luna`) | The spec's negative control (hide a nonexistent path and see an assertion fail) cannot fail on the unchanged test: `contains("hidden")` is satisfied by other markers and `test -e` is true for the empty tmpfs of a correct hide. The architect had not run the control before locking | Amendment 1 withdrew it; backlog F42 for the weak assertions |
| Architect review | Architect | After the change the confine suite passes with the confined, read-only `HOME` (53 passed, 0 failed) | no further problems found |
