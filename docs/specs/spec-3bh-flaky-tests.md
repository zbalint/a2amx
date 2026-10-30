# Spec 3bh: remove the timing races from three flaky tests and make the fourth diagnosable

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `cf1e8e4`; analysis from consultant `m_4593`, anchors re-read by the architect; no second consultant pass on this spec). Architect-directed, backlog F37 and I9, context
`a2amx-flaky-tests-2026-10-30`. Consultant analysis `m_4593` (read at `5c50f35`).
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `cf1e8e4`; the full suite passes there (700 passed, 0 failed, 3 ignored, writable HOME); the developer records the counts again before editing.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** the daemon through `Client` and the CLI through `run`/`PtyHarness`, in the four test files below.
No production change.

**Scope.** May edit exactly these files and no others: `tests/daemon.rs`, `tests/attach_cli.rs`, `tests/daemon_cli.rs`,
`tests/confine.rs`. Does not touch: every other file, including `src/` and `tests/common/`. No new dependency. Leave the
diff uncommitted.

## 1. Why

Four tests pass on rerun and fail now and then. The consultant read the code behind each (`m_4593`); there is no single
shared cause.

| # | Test | Race | Fix class |
| --- | --- | --- | --- |
| 1 | `tests/daemon.rs::exited_claude_session_never_reports_needs_input` (I9) | The child is `printf <prompt>; sleep 11`. The daemon reports `input?` only after 10 s of an unchanged prompt, measured from the first 1 s sampling tick that sees it (`src/session.rs:581-601`, `src/daemon.rs:796-817`). If that tick runs 1 s after the start, `input?` can appear only about 11 s in, when the child exits and the signal is cleared. The window is a few milliseconds wide or negative. | Hold the child on a file the test releases |
| 2 | `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` | After scrolling back to live, the test asserts `[scroll: q to exit]` is on screen with no wait. The client writes the full-render data first (it starts with a screen clear) and the indicator in a separate write (`src/main.rs:2517-2527`); the harness can read between the two writes. | Wait for the text |
| 3 | `tests/daemon_cli.rs::activity_notices_report_peer_transitions_once_in_order` | Peers `p` and `a` are created one CLI round trip apart. The watcher sorts the changes of one 1 s heartbeat tick by name (`src/daemon.rs:860-892`). If a tick falls between the two idle transitions, `p` and `a` arrive in different messages in creation order, and `idle_a < idle_p` fails. | Assert what the code guarantees |
| 4 | `tests/confine.rs::late_created_workspace_socket_is_visible_inside_confined_session` | A local `deadline has elapsed` from the 5 s accept timeout. The cause cannot be read from the code (candidates: slow `python3` start under load; the session exited after `Created`). | Make the failure diagnosable; no cause is claimed |

## 2. Changes

All in test code; expected values are literals.

1. **Test 1.** In `exited_claude_session_never_reports_needs_input`, replace the child command by
   ``printf '\033[2J\033[H Enter to confirm · Esc to cancel'; while [ ! -e <release> ]; do sleep 0.05; done`` where
   `<release>` is `dir.path().join("release")` formatted into the command (`dir` is the temp state directory returned by
   `common::start_daemon`). Keep the 10.2 s sleep and the first `eventually_sessions` wait for `input?`. Then create the
   release file (`std::fs::write`) and keep the second wait for `exit_code.is_some() && hold_reason.is_none()`. The test
   still proves that `input?` appears while the session is live and is gone after the exit.
2. **Test 2.** In `mouse_wheel_scrolls_three_lines_and_q_returns_to_live`, replace
   `assert!(attached.screen_text().contains("[scroll: q to exit]"));` (the one right after the second
   `wait_for_text("line-100", WAIT)`) by `attached.wait_for_text("[scroll: q to exit]", WAIT)?;`. The other assertions stay.
3. **Test 3.** In `activity_notices_report_peer_transitions_once_in_order`, keep the creation order and the wait loop. Replace
   the four cross-peer position assertions (`idle_a < idle_p`, `working_a < working_p`, and the `idle_p < working_p`
   line, which holds per peer and stays) by: split the screen text into messages at each `peer activity:` header;
   assert that `- a: idle (was working` and `- p: idle (was working` each occur exactly once in the whole text and that,
   per peer, idle comes before working; and, in every message that contains both `- a: idle` and `- p: idle`, assert
   `a` precedes `p`, likewise for the two working lines. Keep the `third:` absence assertion. The sort by name is thus
   checked whenever both transitions share a message, and no assertion depends on the CLI round-trip time. (Whether a
   missing sort is caught in a run where the transitions split is backlog F41, a pure-function test.)
4. **Test 4.** In `late_created_workspace_socket_is_visible_inside_confined_session`, keep the script and the assertions.
   Keep the session id from `Response::Created { session }`. Replace
   `timeout(Duration::from_secs(5), listener.accept()).await??` by a `tokio::select!` between
   `timeout(Duration::from_secs(10), listener.accept())` and a loop that lists sessions every 100 ms and completes when
   this session has an `exit_code`. If the session wins, `panic!` with its exit code (a session that ended cannot connect);
   if the timeout wins, `panic!` with the session summary from a final `Request::List` (exit code, hold reason) so the
   next flake says what the session was doing. The 100-iteration result poll after the accept stays.

## 3. Out of scope

Production code, `tests/common/`, other tests, a fix for test 4's unknown cause (this spec only names it when it
recurs), the pure-function test for the activity sort (F41), and any change to timers or constants.

## 4. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test daemon --test attach_cli --test daemon_cli --test confine
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the four files of section 0
```

- Flake evidence: run each of tests 1-3 twenty times in a row, alone and once more while `cargo test --test daemon_cli`
  runs in the background to load the machine; report the counts. A flake in the 20 is a fix request. Test 4 needs the
  real `bwrap` (`A2AMX_REQUIRE_BWRAP=1`); run it ten times.
- Mutants (the developer on a scratch copy, then the tester): (a) test 1: make `Session::needs_input` ignore `exit_code`
  (`src/session.rs:593-601`), so a signal outlives the exit; the second wait must fail. (b) test 2: drop the
  `state.output.send(scroll_status(..))` write after the data frame (`src/main.rs:2525-2527`); test 2 must fail on the new
  wait. (c) test 3: remove the per-tick sort `changes.sort_by(..)` at `src/daemon.rs:896`; the test fails when both transitions
  share a message and may pass when they split (report which; a pass is not a fix request, see F41). (d) test 4: make the confined session exit at once (for example a script `exit 3`); the test must panic with
  the exit code, not wait out the timeout.
- Known flaky outside this spec: none expected after the change. In a confined session the exact `cargo test` also
  shows F40 until spec 3bg lands (it lands first).

## 5. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| Test 1's child, sleeps and waits | `tests/daemon.rs:939-963` (on `5c50f35`) | read |
| `needs_input()` reports only after 10 s and while `exit_code` is `None` | `src/session.rs:581-601` | read (consultant, architect 2026-10-30) |
| Sampling ticks every 1 s from task start | `src/daemon.rs:801-` (`sample_activity`) | read (consultant, architect 2026-10-30) |
| `eventually_sessions` has a 10 s deadline and 10 ms polls | `tests/daemon.rs:631-646` | read |
| Test 2's unwaited assertion follows the second `wait_for_text("line-100", WAIT)` | `tests/attach_cli.rs:372-373` | read |
| The client writes the data frame, then the status, then the scroll indicator as separate `output.send` calls | `src/main.rs:2517-2527` (on `cf1e8e4`) | read (consultant, architect 2026-10-30) |
| Test 3 creates `w`, `p`, `a`, `third` in that order and asserts the four positions | `tests/daemon_cli.rs:2228-2340` | read |
| The watcher sorts one tick's changes by name and sends one message per tick | `src/daemon.rs:860-897` (sort at `:896`) | read (consultant, architect 2026-10-30) |
| Test 4's accept timeout is 5 s and its script retries until `python3` connects | `tests/confine.rs:2016-2070` | read |

## 6. Pre-lock record

Anchors re-read at `cf1e8e4`: `Session::needs_input` (`src/session.rs:592-601`: signal reported after `NEEDS_INPUT_AFTER` and while `exit_code` is `None`), `note_needs_input` (`:581-590`), `sample_activity` (`src/daemon.rs:801-`, a `tokio` interval ticking at start), the heartbeat loop and its sort (`src/daemon.rs:860-897`), `activity_body` (`:186-196`, header `peer activity:` and lines `- <name>: <state> (was <state>)`). The wait helper `eventually_sessions` exists at `tests/daemon.rs:631`.  Gate step 7: tests 1-4 are described once in section 1 and once in section 2 with the same facts. Step 13: no trust boundary.
