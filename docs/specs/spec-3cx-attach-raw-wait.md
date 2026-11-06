# Spec 3cx: The mouse-capture attach test waits for raw bytes instead of checking once (I14)

## 0. Status

LOCKED.

Scope (files that may be edited): `tests/common/pty.rs` and `tests/attach_cli.rs`. Does not touch: `src/`,
every other `tests/*.rs`, `docs/backlog.md` and `CHANGELOG.md` (the architect updates the backlog; no changelog
line, nothing user-visible), `expect_raw_lacks`, `raw_output`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seam: `PtyHarness` in `tests/common/pty.rs` (a test helper) and the attach
command through a real PTY.

## 1. Why

Backlog I14. `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` failed in CI on
`master` (run 42288598802, attempt 1) with `expected raw output to contain \x1b[?1000h\x1b[?1006h`. The
diagnostics added by spec 3cq show why. The captured raw bytes in that failure message do contain the needle,
in the order `...line-100 ... \x1b[?1000l ... \x1b[?1006l \x1b[?1000h \x1b[?1006h \x1b7 <status bar>`: the
client paints the snapshot, then enables mouse capture, then draws the status bar. The test
(`tests/attach_cli.rs:362-363`) calls `wait_for_text("line-100")`, which returns as soon as the screen model
has that line, and then checks the raw bytes once with `expect_raw_contains`. The PTY delivers the
snapshot and the mouse-capture bytes in separate reads, so on a slow runner the check runs between them. The
failure message is built after the check by `diagnostics()`, which calls `screen_text()` and drains more
bytes, so it shows a later state than the one the check saw. That is why the failure looked impossible.
The earlier failure of the same test (`ESC[?1000h ESC[?1006h` missing at the old line 140, rerun passed) is
the same check.

This is a race in the test, not a product bug: the client does write the bytes, in a fixed order. Rejected:
a `sleep` before the check (a guess that still flakes on a slower runner); asserting the needle with
`wait_for_text` (the screen model never shows control bytes); moving the check to the end of the test (the
bytes are the precondition of the first scroll, and the screen model would no longer prove they were enabled
before the wheel event).

## 2. Change

`tests/common/pty.rs`: replace `expect_raw_contains(&mut self, needle: &[u8])` with
`wait_for_raw_contains(&mut self, needle: &[u8], timeout: Duration) -> anyhow::Result<()>`. It loops like
`wait_for_text` (lines 176-193): `drain_ready`, return `Ok` when the raw bytes contain the needle (an empty
needle is contained), otherwise return an error at the deadline, else `recv_one` for at most 50 ms. The
error text says it timed out waiting for the raw output to contain the escaped needle and carries
`diagnostics()`. Update the doc comment. `expect_raw_contains` has one caller (`rg -n expect_raw_contains
tests` shows `tests/attach_cli.rs:363` and its definition); do not keep a second function.

`tests/attach_cli.rs:363`: call `attached.wait_for_raw_contains(b"\x1b[?1000h\x1b[?1006h", WAIT)?;`.

Add two tests to `tests/attach_cli.rs` that pin the helper at its public surface, using
`PtyHarness::spawn_program("sh", ...)` with no daemon:
- a late-bytes test: the child prints `first`, sleeps about one second, prints `second`, then sleeps (kept alive
  by a long `sleep`); the test waits for the screen text `first`, asserts that
  `wait_for_raw_contains(b"second", WAIT)` returns `Ok`. A check made immediately after `first` appears would
  have failed;
- a timeout test: the child is `sh -c 'sleep 30'`; `wait_for_raw_contains(b"never",
  Duration::from_millis(300))` returns an error whose text contains `timed out` and `raw output:`.

## 3. Out of scope

`expect_raw_lacks` (a check that something is absent cannot fail by racing, but it can pass too early; the
two callers, `tests/attach_cli.rs:472-473`, follow `wait_for_text("ready")` and are not part of I14), the direct
`raw_output()` users, `src/`, changing `WAIT`, and any retry-on-failure wrapper around the whole test.

## 4. Acceptance

1. `rg -n 'expect_raw_contains' tests src` prints nothing; `rg -n 'wait_for_raw_contains' tests` shows the
   definition, the call in `mouse_wheel_scrolls_three_lines_and_q_returns_to_live` and the two new tests.
2. Red phase: before replacing the function, the developer writes the late-bytes scenario against the OLD
   `expect_raw_contains` (`wait_for_text("first")`, then `expect_raw_contains(b"second")` with the one-second
   gap) and shows it failing; that reproduces the I14 mechanism on purpose. The test is then converted to
   `wait_for_raw_contains` and passes. The timeout test is a contract test (the old function has no timeout);
   the developer reports that.
3. Mutation check on a restore-by-checksum edit of `tests/common/pty.rs` in the checkout (the tree must equal
   the developer's own diff afterward): (a) make `wait_for_raw_contains` return `Ok` without checking, the
   timeout test fails; (b) make it check once and not loop (return the error at the first miss), the
   late-bytes test fails; (c) make it ignore the deadline (loop forever is not allowed, so make it return `Ok`
   at the deadline), the timeout test fails. The developer names the failing test for each.
4. The mouse test passes 10 times in a row: `for i in $(seq 10); do cargo test --test attach_cli
   mouse_wheel_scrolls_three_lines_and_q_returns_to_live || { echo "failed on run $i"; exit 1; }; done` exits 0
   (a bare `|| break` would hide a failure). Then the full
   `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass with the ambient
   environment of a hosted session. If a test other than the three named here fails, rerun it once and
   record it as pre-existing; the known flakes are I7, I14 and I18 in `docs/backlog.md`.
5. Scratch files stay under `target/scratch/3cx/` and are removed before the report.
