# Spec 3cq: A failing flaky test shows what the session looked like

## 0. Status

LOCKED. Tests only; no change to `src/`.

Scope (files that may be edited): `tests/common/pty.rs`, `tests/attach_cli.rs`,
`tests/delivery.rs`, `tests/daemon_cli.rs`. Does not touch: `src/`, every other test file,
`tests/common/mod.rs`, `AGENTS.md`, `CHANGELOG.md`, `docs/` (the architect updates the backlog
and the roadmap, and the backlog row is already uncommitted in the tree), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `cf4e1af`.
Context id: `a2amx-flaky-diagnostics-2026-11-05`. Public test seam: the `a2amx` binary
(`env!("CARGO_BIN_EXE_a2amx")`), the `a2amx::client`/`wire` API the tests already use, and the
test helpers in `tests/common/pty.rs`.

## 1. Why

Backlog I7, I14 and I18 are PTY and timing tests that fail now and then and pass on a rerun. The
owner decided (2026-11-04) to diagnose before fixing: none of them has reproduced under a debugger,
and spec 3bh showed that a fix written without reading a failing screen explains the wrong cause.
Today a failure of these tests tells the next reader almost nothing:

- I7 (`attach_does_not_capture_session_already_in_drag_mode`) and I14
  (`mouse_wheel_scrolls_three_lines_and_q_returns_to_live`) fail on a bare `assert!` over
  `raw_output()` or `screen_text()`, which prints only `assertion failed`. `wait_for_text` prints
  the screen on a timeout but not the bytes that produced it.
- I18, the delivery tests: `Case::wait_state` and `Case::attach_ready` end in `.unwrap()` on a
  `tokio::time::timeout`, so a timeout prints `Elapsed(())`: no state, no screen. `wait_rest`
  already prints the expected and observed bytes.
- I18, the reset tests in `tests/daemon_cli.rs`: a failed `reset` prints its own output but not
  the session's screen or why the reset gate said no.

This spec changes what these tests print when they fail, and nothing else. They stay open in the
backlog; they are not fixed and the release does not wait on them.

Rejected: raising timeouts or adding retries (hides the failure, and the cause is unknown);
a `src/` debug endpoint (production code for tests, and `a2amx explain` and `a2amx screen`
already give the daemon's view); fixing the suspected race in the attach client (the cause is
unconfirmed, a fix would be a guess).

## 2. `tests/common/pty.rs`

Add to `impl PtyHarness`:

```rust
/// What a failing test needs to see: the screen, the tail of the raw bytes with control
/// bytes escaped, and the child's state.
pub fn diagnostics(&mut self) -> String;

/// `Ok` when the raw output contains `needle`, else an error carrying `diagnostics()`.
pub fn expect_raw_contains(&mut self, needle: &[u8]) -> anyhow::Result<()>;
/// `Ok` when the raw output does not contain `needle`, else an error carrying `diagnostics()`.
pub fn expect_raw_lacks(&mut self, needle: &[u8]) -> anyhow::Result<()>;
/// `Ok` when the screen text does not contain `text`, else an error carrying `diagnostics()`.
pub fn expect_screen_lacks(&mut self, text: &str) -> anyhow::Result<()>;
```

- `diagnostics()` returns, in this order: `screen:` and the screen text (as `screen_text()`
  gives it); `raw output: N bytes, last M shown:` and the last `M = min(N, RAW_TAIL)` bytes
  escaped with `<[u8]>::escape_ascii`; `child: exit code <Option<i32>>, eof <bool>`. `RAW_TAIL` is
  a new `const` of 4096. It calls `drain_ready` first, as the other accessors do.
- Each `expect_*` error message names the needle (escaped for bytes) and what was wanted
  ("expected raw output to contain", "expected raw output not to contain", "expected screen not
  to contain"), then `diagnostics()`.
- `wait_for_text`'s timeout error and `wait_exit`'s timeout error use `diagnostics()` in place of
  `screen was {:?}` and the bare `timed out waiting for PTY child exit`. The first keeps its
  `timed out waiting for {text:?}` prefix.

## 3. `tests/attach_cli.rs`

Convert the plain assertions in the two flaky tests to the new methods, keeping every assertion
and its order:

- `mouse_wheel_scrolls_three_lines_and_q_returns_to_live`: the `assert!` over the
  `\x1b[?1000h\x1b[?1006h` window (lines 363-368) becomes
  `attached.expect_raw_contains(b"\x1b[?1000h\x1b[?1006h")?`; the three
  `assert!(!attached.screen_text().contains(..))` lines (369, 373, 379) become
  `attached.expect_screen_lacks(..)?`.
- `attach_does_not_capture_session_already_in_drag_mode`: the two raw-window `assert!`s
  (lines 477-488) become `expect_raw_lacks(b"\x1b[?1000h")?` and
  `expect_raw_lacks(b"\x1b[?1006h")?`.

No wait, timeout, sleep or command in these tests changes.

## 4. `tests/delivery.rs`

- Add `Case::diagnostics(&self) -> String` (async). It opens a fresh admin connection with
  `Client::connect(self.dir.path())` (the call `Case::start` and `attach_ready` use) and sends,
  one request each: `Request::List` (the recipient's `SessionSummary`, found by `s.id ==
  self.recipient`, printed with `{:?}`), `Request::Screen` (lines joined by newline) and
  `Request::Explain` (`{:?}`). It must not use `self.admin` and must not call `recipient_summary`:
  `Client::request` sends and then reads on one connection with no request id
  (`src/client.rs:102-113`), so a request future dropped by the 10 s `tokio::time::timeout` in
  `wait_state` leaves a stale reply for the next request on that connection, and
  `recipient_summary` panics on a wrong variant or a missing session. A failed connect, a failed
  request, a response that is not the expected variant, or a missing summary is printed as that
  error text in its place; the helper never panics.
- `Case::wait_state`: on the 10 s timeout, panic with `timed out waiting for state {state:?} of
  {id}; last status {:?}` (the last `MessageInfo` seen) followed by `diagnostics()`. The loop
  keeps the last status it read and reads it the same way. The `tokio::time::timeout(WAIT, ..)`
  call stays, with the same `WAIT`.
- `Case::attach_ready`: on the timeout, panic with `timed out waiting for the ready composer;
  mirror screen:` and the mirror's screen text (rows joined by newline, trailing spaces
  trimmed), instead of `.unwrap()` on `Elapsed`.
- `Case::wait_rest` keeps its `&self` receiver and its message, and appends `diagnostics()`.

## 5. `tests/daemon_cli.rs`

Add `fn session_diagnostics(home: &Path, session: &str) -> String`: the stdout and stderr of
`a2amx explain <session>` and `a2amx screen <session>`, each under its own label, via `run`.
Use it in the two reset tests the backlog names:

- `reset_runs_configured_steps_in_order`: append it to the message of each assertion after the
  daemon starts that can fail on the reset: `reset.status.success()` (:461-466, which already
  has a message), `assert_eq!(stdout(&reset), "reset s1: 2 steps\n")` (:467),
  `screen.status.success()` (:469) and the three `text` assertions (:471-476).
- `reset_times_out_when_second_step_stays_not_ready`: append it to the messages of
  `!reset.status.success()` (:418), `step_timeout` (:420), `step 2 of 2` (:421),
  `screen.status.success()` (:423), the two `text.contains` assertions (:425-426) and the retry
  assertions (:428-430).

Call `session_diagnostics` inside the assertion's format arguments, never in a `let` before it:
`assert!` evaluates its message only on failure, so a passing run makes no extra CLI call and
the reset-then-retry sequence of the second test is not disturbed. `run` and the session names
are unchanged. `explain` and `screen` are read-only.

## 6. Out of scope

- Any fix, retry, longer timeout or changed wait in a flaky test (I7, I14, I18 stay open).
- Other uses of `wait_for_text` (`auto_confine.rs`, `team.rs`, `refs_and_list.rs`,
  `daemon_cli.rs`): they get the longer timeout message from §2 and nothing else.
- Any production code, including a `Debug` impl; the wire types already derive it.
- Converting the other `assert!`s in `attach_cli.rs` (the file has 167 uses of `wait_for_text`,
  `wait_exit` or `raw_output` in all, so the rest of the file stays as it is); only the two named tests change.

## 7. Acceptance

Existing baseline (run at lock time, passes on the base commit): `cargo test`,
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. They must pass with no warnings
and no new `#[allow]`.

After implementation (need the new code):

1. Failure messages, by mutation. For each target below the developer edits ONE assertion or
   expected literal in the test, runs that one test, and records the failure message; the
   message must contain the label named in the row. The tester repeats at least three of them
   independently. The edits are probes in the checkout, restored by checksum; the tree matches
   the diff afterward.

   | Test | Probe | Message must contain |
   | --- | --- | --- |
   | `mouse_wheel_scrolls_three_lines_and_q_returns_to_live` | change the `expect_screen_lacks("line-076")` argument to `"line-100"` (on screen at that point) | `screen:`, `raw output:` and a literal `\x1b[` in the escaped tail |
   | `attach_does_not_capture_session_already_in_drag_mode` | `expect_raw_lacks(b"ready")` | `raw output:` and the escaped bytes of `ready` (if `ready` is not contiguous in the raw bytes the probe passes: then use `b"\x1b[?1002h"`) |
   | `a_tab_in_the_body_matches_after_claude_expands_it` | `wait_state(&id, "never")` | `last status`, the `SessionSummary` fields (`{:?}`), the screen lines and the `Explain` response |
   | `reset_runs_configured_steps_in_order` | expect `reset s1: 3 steps` at the `assert_eq!` on `stdout(&reset)` | `explain` and `screen` output |
   | `reset_times_out_when_second_step_stays_not_ready` | expect `step 1 of 2` instead of `step 2 of 2` | `explain` and `screen` output |

   `wait_state` waits 10 s; that probe takes 10 s. The `wait_state` probe must show the summary
   and screen, which is the case that fails if `diagnostics` reuses the timed-out connection.
2. No timing change: `git diff -U0 -- tests | rg '^[-+].*(Duration::|sleep|WAIT)'` matches
   nothing except the existing `tokio::time::timeout(WAIT, ..)` lines of `Case::wait_state`
   and `Case::attach_ready` and the existing `WAIT` uses beside them, re-formed with the same
   value. Judge: the tester reads each match.
3. Scope: `git status --short` lists only the four files in §0, this spec, and
   `docs/backlog.md` (the architect's uncommitted backlog row).
4. Bound: `diagnostics()` shows at most `RAW_TAIL` raw bytes. The tester states `N` and `M` from
   probe 1's message; if `N` is not above 4096, the tester sets `RAW_TAIL` to 64 in the
   checkout for one run of probe 1, shows `M` = 64, and restores it by checksum.
5. Every assertion in the two converted tests of §3 is still present, in the same order: the
   architect reads the diff, removed lines included.

No required test passes only on the old code; the changes are to failure output, so there is no
red phase. Known flaky tests: I7, I14 and I18 can fail in a full run. A failure of one of them in
a full run is rerun once alone and recorded as pre-existing, together with the new failure
message, which is the point of this spec. Run the integration targets one at a time for
evidence (`cargo test --test attach_cli`).

Scratch: none needed. Mutation probes happen in the checkout and are restored by checksum
(`AGENTS.md`, Scratch files).

## 8. Claims about the current tree

| Claim | Where | Verified how |
| --- | --- | --- |
| `wait_for_text` is the only pty timeout message with the screen | `tests/common/pty.rs:184-188` | read |
| `wait_exit` timeout has no detail | `tests/common/pty.rs:224` | read |
| No test pins the text `screen was` | `rg 'screen was' tests` finds only `pty.rs:185` | grepped |
| I7 and I14 use bare `assert!` over `raw_output()` and `screen_text()` | `tests/attach_cli.rs:363-369, 373, 379, 477-488` | read |
| `attach_ready` (:106-123) and `wait_state` (:173-183) end in `.unwrap()` on a timeout | `tests/delivery.rs` | read |
| `Client::request` sends then reads on one connection with no request id | `src/client.rs:102-113` | read |
| `recipient_summary` panics on a wrong variant or a missing session | `tests/delivery.rs:186-195` | read |
| `wait_rest` prints expected and observed | `tests/delivery.rs:228-232` | read |
| `ExplainInfo`, `SessionSummary`, `MessageInfo` derive `Debug` | `src/wire.rs:328, 398, 494` | read |
| `Request::Screen` and `Request::Explain` exist; `Response::Screen { lines }` | `src/wire.rs:200-205, 285-290` | read |
| `a2amx explain SESSION` is in the CLI | `docs/cli.md:21`, `src/main.rs:206` | read |
| `pty.rs` allows `dead_code` and `unwrap` | `tests/common/pty.rs:3` | read |
| The reset tests assert on `reset` output and `screen_contains_until` | `tests/daemon_cli.rs:417-431, 460-476` | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5; developer and tester OMP
`openai-codex/gpt-5.6-luna` (planning, working and advisor slot names are not exposed in the OMP
sessions). BLOCKED/amend cycles: 0.

| Where found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant, Opus 5.5 | `Case::diagnostics` would reuse `self.admin` after a `wait_state` timeout; `Client::request` has no request id, so the next request reads the stale reply and the promised message would show `no list` | Fresh connection per request; `recipient_summary` not used |
| Draft review | consultant, Opus 5.5 | Probe 4 edited an assertion section 5 did not cover; probe 1 named no helper; acceptance 2's regex matched the existing `timeout(WAIT, ..)` lines; `docs/backlog.md` was missing from the status check; a claims row had its line ranges swapped | Section 5 names every assertion by line, probes and acceptance reworded |
| Implementation | developer, gpt-5.6-luna | The first converted assertion in the wheel test briefly became `line-100` instead of `line-076` | Caught by the developer's own diff review and restored before the report |
| Architect review | architect, Sonnet 5.5 | `Case::diagnostics` repeated the same connect-request-match block three times | One private helper, `Case::ask` |
| Tester pass | tester, gpt-5.6-luna | `expect_raw_lacks` accepted an empty needle while `expect_raw_contains` treats it as contained | `expect_raw_lacks` rejects an empty needle |

Accepted survivors: the unbounded raw tail (no fixture has more than 4096 raw bytes; the
`RAW_TAIL = 64` probe shows the bound) and an eager `session_diagnostics` call before an
assertion (no test can see it).
