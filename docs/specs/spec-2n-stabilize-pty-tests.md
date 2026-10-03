# Spec 2n: stabilize the load-sensitive PTY tests

## 0. Status

**LOCKED.** Builds on spec 2m's baseline tree (`6ec9761` plus the uncommitted 2m diff when
this runs after it; the failures below reproduce on `6ec9761` alone). The work is done
directly on `develop` in the main checkout, no worktree. Shared task `context_id`:
`a2amx-pty-tests`. Public test seams: the test files themselves; no new seam.

This is a diagnose-then-fix spec: the root cause is not yet known, so the deliverable is a
causal finding with evidence and the smallest fix that removes it.

**Scope.** May edit exactly these files and no others.

- `tests/attach_cli.rs`, `tests/delivery.rs`, `tests/common/mod.rs` and other files under
  `tests/common/` (only helper timing, readiness waits and the PTY harness)
- `docs/architecture.md` only if the fix changes a documented behavior (not expected)

Does not touch: every file under `src/`, `Cargo.toml`, `Cargo.lock`, `scripts/`, the other
test files, `AGENTS.md`, `README.md`, the locked spec documents. Do not commit and do not
merge: leave the diff uncommitted.

If the evidence shows the cause is a product race in `src/` (not a test that waits for the
wrong thing), stop, do not edit `src/`, and send `BLOCKED — SPEC ADJUDICATION REQUIRED` with
the reproduction and the evidence. That is an expected outcome, not a failure.

## 1. Why

Some PTY tests fail intermittently on a clean tree. Every review's fresh `cargo test` then
needs a rerun, which hides real regressions and costs time. Measured at spec time:

- `attach_cli::status_line_toggles_and_shows_the_session_address`, run alone 12 times on
  `6ec9761`: 3 failed, 9 passed. Failure text, same in the failures read:
  `Error: timed out waiting for "10 40"; screen was "24 80\n23 80\n9 40\n\n\n\n\n\n\n"`. The
  child prints `stty size` once at start and again on each SIGWINCH; after the status-line
  toggle (`Ctrl-B s`) the expected `10 40` line never appears, although `9 40` did.
- Earlier, on clean `8788797`: the `delivery` suite had a failure in 7 of 25 full runs, and
  `modal_after_paste_holds_without_cr_until_explicit_release` and
  `custom_prefix_and_sigwinch_resize_reach_attached_session` were seen failing under
  full-suite load. Those numbers come from the 2k/2l review (memory-recorded, not re-measured
  here); re-measure them first.

## 2. Procedure (in this order)

1. **Reproduce and count.** For each of the tests named in section 1, run it alone N times
   and the whole suite N times, recording pass/fail counts and each distinct failure text.
   Suggested: `for i in $(seq 1 20); do cargo test --test attach_cli <name> || echo FAIL; done`
   and the same for `--test delivery`. Report the table.
2. **Find the cause of each distinct failure.** One hypothesis at a time, with evidence (for
   example: log the order of resize, toggle and WINCH events; read how `PtyHarness` in
   `tests/common` feeds input; check whether the test sends the prefix before the previous
   resize reached the child). Candidate classes to test, not to assume: (a) the test types
   before the previous step's effect is observable; (b) a fixed sleep or short timeout that
   load exceeds; (c) signals coalescing (two SIGWINCH close together run the trap once) so an
   expected intermediate line never prints; (d) a real daemon or client race.
3. **Fix at the root cause**, in the tests or their helpers: wait for an observable condition
   (a screen line, a state in `a2amx list`) instead of a fixed delay; make a step wait for the
   previous step's effect; accept the coalescing case where the test should not depend on an
   intermediate line. Do not raise a timeout as the fix unless the evidence shows the wait is
   genuinely bounded by work, and say so.
4. **Class (d) stops the work** as described in section 0.

## 3. Out of scope

- Any `src/` change; any change to what a test asserts about product behavior (a test may
  change how it waits, not what outcome it requires).
- Deleting, ignoring (`#[ignore]`) or serializing tests to hide a failure.
- Test failures that do not reproduce in step 1; list them as not reproduced.

## 4. Acceptance

Run after the fix, from the main checkout, and report counts:

```sh
for i in $(seq 1 30); do cargo test --test attach_cli status_line_toggles_and_shows_the_session_address || echo FAIL; done
for i in $(seq 1 25); do cargo test --test delivery || echo FAIL; done
for i in $(seq 1 5); do cargo test || echo FAIL; done
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The word `FAIL` must not appear in the first three loops' output. If a test that step 1
reproduced still fails after the fix, the report says which, with the failure text, and the
work is not done. The final report names, per failing test, the cause found, the evidence
that showed it, and why the fix removes it.
