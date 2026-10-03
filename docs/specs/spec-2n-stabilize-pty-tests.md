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

## Amendment 1 (architect, after the first REPORT)

**Evidence.** The developer's final loops reported `status_line_toggles_and_shows_the_session_address`
30/30 with the polling-child observer. The architect's fresh runs of the same command, same
tree, gave 5 failures in 100 runs (1 in 30, then 0 in 60, then 4 in 70), every one with the
original text `timed out waiting for "10 40"; screen was "24 80\n23 80\n9 40\n\n\n\n\n\n\n"`.
The child now prints every size change by polling `stty size`, so a missing `10 40` is not
a lost signal. Either the PTY was not resized, or the daemon's bytes for it did not reach or
did not update the attach client's screen model. The developer's diagnostics already showed the
daemon reporting 40x10 and the child's `stty size` reporting `10 40` in a failing run, which
points at the output path, not the resize. The delivery loop is clean: the architect ran
`cargo test --test delivery` 25 times, 0 failures.

**Decisions.**
- **A1.1.** The `tests/delivery.rs` change is accepted as the final part-A deliverable.
- **A1.2.** The `tests/attach_cli.rs` observer change is withdrawn: restore that file to its
  committed state (`git checkout tests/attach_cli.rs`). It does not remove the failure, and
  it adds a 100 Hz process spawn to a load-sensitive suite.
- **A1.3.** The status failure moves to part B below, which runs after spec 2o, not now. Part A
  is complete when A1.2 is done and the architect has committed A1.1.

### Part B: root-cause the status-line failure (starts only after the architect's GO)

Scope for part B: `tests/attach_cli.rs`, `tests/common/` and, only for a cause demonstrated
by the evidence below, `src/daemon.rs`, `src/emulator.rs`, `src/main.rs` and `src/session.rs`.
Everything else in section 0's "does not touch" list still applies.

1. In a failing run, capture what reached the client: the raw `ServerFrame::Data` bytes the
   attach client received after the toggle (a retained byte log in the test harness that is
   printed on failure; the `PtyHarness` screen model lives in `tests/common`). Decide from it
   which of these holds: (a) the daemon never sent the `10 40` line; (b) it sent it and the
   harness's screen model dropped or overwrote it; (c) it sent it before a full redraw that
   discarded it.
2. Read the Resize arm of the attach loop (`src/daemon.rs`, the `ClientFrame::Resize` match
   arm that calls `resize_locked`, sets `state.dirty = false` and sends `render_full`) against
   the `notify` arm that sends `render_update`: check whether output the child wrote around
   the resize can be dropped or never rendered to the client.
3. Fix the confirmed cause. For a product cause, write a deterministic failing test first
   (the race should be forced with the real daemon, not with sleeps). For a test-harness cause,
   fix the harness. State the cause and the evidence in the REPORT.
4. Acceptance, after the fix: `status_line_toggles_and_shows_the_session_address` run 100
   times with 0 failures, the 25 `delivery` runs and 5 full runs with 0 failures, then clippy and
   fmt, all with `env -u A2AMX_BIN`. 100 runs because the failure rate is about 5%: 30 runs
   pass by chance about one time in five.
