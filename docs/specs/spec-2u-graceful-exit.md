# Spec 2u: graceful exit for `kill`, `team down` and the daemon stopping

## 0. Status

**LOCKED** (2026-10-03, pre-lock gate run against develop at `3160073`, before spec 2t's
implementation landed (the greps below were taken there); the gate greps found 11 test literals and 2 `src/main.rs` uses of `Request::Kill`, one `Shutdown` use in `src/main.rs`, no test that names `Shutdown` or the 15 s text). Decisions D1 to D9 come from the owner's
design of 2026-10-03 (backlog items F1 and F7) and the architect's probe of the same day (section 3 lists the facts this
spec relies on).

**Amendment 1** (2026-10-04, first developer BLOCKED, verified against `src/daemon.rs:212-243`
and `src/codex.rs:372-404`): a `--harness codex` session always starts a Codex app-server from
`argv[0]`, so a plain `sh -c` cannot be a Codex-harness fixture, and an OMP-harness session fails the
reset gate without a connected bridge. Section 8 now uses the Claude harness (no app-server, no
bridge; its gate is the `claude_ready` screen) for every automated fixture, the one-key OMP and
Codex path is covered by the manual real-session check, and D7 is clarified about `Drop`.

**Baseline:** develop at `7adc821`, the commit that implements spec 2t (`a2amx daemon
{start,status,stop}`, `kill [--yes]`), plus the commit that adds this spec. **Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch
`develop`. Shared task `context_id`: `a2amx-graceful-exit`. Public test seams: the `a2amx` CLI and
the wire `Request`s against a real daemon in a temp state dir (`tests/daemon_cli.rs`,
`tests/daemon.rs`, `tests/common`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs` (`Request::Kill` and `Request::Shutdown` gain `now`)
- `src/session.rs` (`Session::kill`, the graceful sequence, two constants)
- `src/daemon.rs` (the `Kill` and `Shutdown` arms, the stop flag, `Daemon::shutdown`)
- `src/cli.rs`, `src/main.rs` (`--now` on `kill`, `daemon stop` and `team down`; the stop wait)
- tests: `tests/daemon_cli.rs`, `tests/daemon.rs`, and the mechanical `now: true` additions to
  every existing `Request::Kill` literal in `tests/codex.rs`, `tests/delivery.rs`,
  `tests/attach_cli.rs`, `tests/broker.rs`
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `docs/backlog.md`

Does not touch: `src/messaging.rs`, `src/harness.rs`, `src/codex.rs`, `src/mcp.rs`, `src/team.rs`,
`src/delivery.rs`, `scripts/`, `extension/`, `run_team_up`. No new dependency. Do not commit,
stage or merge: leave the diff uncommitted for review.

## 1. Why

`Session::kill` sends SIGHUP to the process group, waits 3 s and then SIGKILLs. For an idle agent
session that is a crash-style end: the harness cannot save its session file or run its exit hooks.
All three agent harnesses quit cleanly on Ctrl-D at an empty composer (section 3). The reset
runner of spec 2s already has the gate, the delivery hold and the typing path this needs, so a
clean exit is one more sequence on top of it. A session that is busy, held or showing a draft
keeps today's behavior, because a keypress there could destroy work or do nothing.

## 2. Decisions

- **D1.** Graceful exit is the default for `a2amx kill`, `a2amx team down` and the daemon
  stopping (`a2amx daemon stop`). `--now` on each of the three skips it and runs exactly today's
  sequence. `team up`, `reset` and the session itself are unchanged.
- **D2.** Wire: `Request::Kill { session, now }` and `Request::Shutdown { now }`, with `now:
  bool`, `#[serde(default, skip_serializing_if = "is_default")]` as the other optional fields of
  `Request` are written. `Shutdown` changes from a unit variant to a struct variant. An old
  client that sends `{"Kill":{"session":"s1"}}` therefore means graceful; the wire has no version
  to bump.
- **D3.** `Session::kill(&self)` becomes `Session::kill(&self, graceful: bool)`. Callers:
  `Request::Kill` passes `!now`, `Daemon::shutdown` passes `!now` of the stop request. When
  `graceful` is false, or the session is Generic, or it already exited, the body is today's body
  unchanged.
- **D4.** The graceful attempt (`Session::graceful_exit(&self) -> bool`, private, true when the
  child exited) runs before the HUP step:
  1. Take `begin_reset()`; when it fails (a reset is running) return false. The guard holds
     delivery for the duration, exactly as a reset does.
  2. Require `reset_gate_reason()` to be `None` (running, no hold, harness ready, OMP idle, no
     draft). Otherwise return false at once, so a busy or drafting session falls straight to HUP
     with no waiting.
  3. Type the exit keys with `enqueue` (raw bytes, no paste, no CR): OMP and Codex send `0x04`
     once; Claude sends `0x04`, waits `CLAUDE_EXIT_KEY_GAP` (300 ms) and sends `0x04` again,
     without a second gate check between the two (the second must land inside Claude's
     pending-exit window, which is under 1.5 s). Never Ctrl-C. If `enqueue` reports the session
     gone, the child exited: return true.
  4. Wait for exit up to `GRACEFUL_TIMEOUT` (10 s, `wait_exit`). Return whether it exited.
  5. Emit one `tracing::info!` line with the session id and outcome (`exited` or `timeout`) so a
     test and an operator can see which path ran; a gate refusal logs `skipped` with the gate
     code.
- **D5.** Generic sessions never attempt it (no known exit key). The harness is `self.harness`.
- **D6.** After a failed attempt the existing HUP, 3 s wait, KILL sequence runs unchanged. Worst
  case per session is 13 s plus 3 s of KILL wait already there today.
- **D7.** `Daemon::shutdown` already kills every session concurrently in a `JoinSet`; keep that.
  It reads the stop flag: a new `AtomicBool` `stop_now` on the daemon runtime, written by the
  `Request::Shutdown { now }` arm before it notifies `stop_requested`; `Relaxed` is enough
  because the notify orders it. An explicit `Daemon::shutdown()` that no request preceded (the
  SIGTERM and Ctrl-C paths in `main`) finds `stop_now` false and so is graceful. `Drop for
  Daemon` and `Drop for Session` are unchanged: they stay the synchronous HUP, because a
  destructor cannot run the async graceful sequence.
- **D8.** The `a2amx daemon stop` client wait grows from 15 s to 30 s (the daemon needs up to 10
  s of graceful wait plus 3 s plus store close). The error text becomes `daemon did not stop
  within 30s`. The deadline is one constant in `src/main.rs`.
- **D9.** `team down` kills sessions one after another as today; each idle session leaves in
  about a second and a refused one costs nothing extra. No change to its messages.

## 3. Probe facts this spec relies on (2026-10-03, isolated daemon, installed binary)

- OMP 18.5.1 and Codex 0.160.0: one `0x04` at an empty composer exits with code 0.
- Claude Code 2.1.288: the first `0x04` prints `Press Ctrl-D again to exit` and keeps running; a
  second `0x04` 0.5 s later exits with code 0; 1.5 s later it does not.
- The `human_draft` hold seen after the first key in the probe came from the attach client's own
  typing; keys the daemon types through `enqueue` do not set a hold (`note_human_input` is only
  called for attached clients). Whether `claude_ready` stays true during the `Press Ctrl-D
  again` footer is unprobed, which is why D4.3 forbids a gate between the keys.

## 4. `src/wire.rs`

`Request::Kill` (line 142) gains `now`, `Request::Shutdown` (line 152) becomes `Shutdown { now }`,
both as in D2. Update the doc comment of `Shutdown`. Every `Request::Kill { session }` and the
`Shutdown` use in `src/` and `tests/` is updated (sections 5 to 7).

## 5. `src/session.rs`

- Two constants next to `RESET_TIMEOUT` (line 29): `GRACEFUL_TIMEOUT` and `CLAUDE_EXIT_KEY_GAP`.
- `kill` (line 775) per D3; `graceful_exit` per D4, reusing `begin_reset`, `reset_gate_reason`,
  `enqueue` and `wait_exit`. `// shortcut:` comment at the Claude key gap: fixed 300 ms against a
  window of about 1 s, replace with a screen-based wait if a Claude release shortens the window.
  `// shortcut:` comment at the gate: a human can start typing between the gate check and the key;
  the cost is one forward-delete in the composer, add a re-check inside the input lock if it is ever
  seen.

## 6. `src/daemon.rs`

`Request::Kill` arm (line 1135): `session.kill(!now)`. `Request::Shutdown { now }` arm (line 1128):
set `stop_now` before `stop_requested.notify_one()`. `Daemon::shutdown` (line 892): `session.kill(
!self.runtime.stop_now.load(...))`.

## 7. `src/cli.rs` and `src/main.rs`

- `Command::Kill`, `DaemonAction::Stop` and `TeamAction::Down` gain `now: bool`, doc text `Skip the
  graceful exit and stop at once`. They pass it into `Request::Kill { now }` (lines 539 and 575
  of the pre-2t file) and `Request::Shutdown { now }` (line 262).
- `run_stop`'s deadline per D8.

## 8. Tests

Existing `Request::Kill` literals get `now: true` so those tests keep proving today's sequence
(`tests/codex.rs:440`, `tests/delivery.rs:354,442`, `tests/attach_cli.rs:857`,
`tests/broker.rs:251,365`, `tests/daemon.rs:201,334,459,593,623`). Then new tests, each failing
first, through the CLI against a real daemon in a temp state dir. Fixtures are `--harness claude`
sessions running a `sh -c` script (no app-server and no bridge are needed for that harness). The
script turns bracketed paste on, draws a screen that satisfies `claude_ready` (a `❯` plus U+00A0
prompt row between two rule rows, away from the first and last rows, cursor visible), and records
every byte it reads to a file in the temp dir. Variants differ only in when they exit.

1. Fixture that exits 0 on the first `0x04`: `a2amx kill` ends it, the record file shows a
   `0x04`, and the daemon log holds the `exited` line. `kill --now` on the same fixture ends it
   by HUP: the record file shows no `0x04` and the log has no `exited` line.
2. Two-key window: a fixture that exits only after two `0x04` within 600 ms (it records
   timestamps with `date +%s%N` or equivalent) is ended by a plain `kill`.
3. A fixture that ignores `0x04`: `kill` falls back to HUP and the session is gone; the log line
   says `timeout`. It runs with the real 10 s `GRACEFUL_TIMEOUT` (about 13 s) in the normal run,
   not `#[ignore]`d, and no new timing knob is added.
4. Gate refusal: a fixture whose screen is NOT ready (no prompt row, so `claude_ready` is false):
   `kill` sends no `0x04` (the record file is empty) and ends it by HUP; the log line says
   `skipped`. A human draft is not simulated.
5. A Generic session is killed by HUP with no `0x04` (record file empty).
6. `daemon stop --yes` with one idle fixture ends it through Ctrl-D (the log line) and the daemon
   exits; `daemon stop --now --yes` records no `0x04`.
7. The wire: a `Request::Kill` JSON without `now` deserializes to `now == false`; `Request::Shutdown
   { now: false }` serializes without a `now` key (assert on the literal JSON in `tests/daemon.rs`
   or a unit test that already covers `wire.rs` serialization).

The one-key Codex path may additionally be tested with the wrapper pattern of `tests/codex.rs`
(a program whose `app-server` branch links a socket and whose TUI branch runs the `sh -c` body);
the developer may add one such test for test 1's behavior with `--harness codex`. An OMP session
needs a connected bridge, so OMP's one-key path has no automated test; the manual check below
covers OMP (and Codex when no wrapper test is added), and the completion report says which.

Manual check, recorded in the completion report: on a temp home, start real `omp` and `codex`
sessions in a trusted directory, run `a2amx kill` on each and read `daemon.log` for `exited`
(never touch the working daemon at `~/.a2amx` and never run `scripts/install.sh`; use a temp
`--home` with your own `target/debug/a2amx` build, because the installed binary lacks this spec).

## 9. Docs

- `README.md`: `kill [--yes] [--now]`, `daemon stop [--yes] [--now]`, `team down [--now]` and one
  sentence on the graceful default.
- `docs/architecture.md`: the kill and shutdown flow (Ctrl-D, then HUP, then KILL).
- `docs/delivery.md`: the graceful sequence reuses the reset gate and hold, never Ctrl-C.
- `docs/backlog.md`: delete rows F1 and F7. Add a row: `Claude readiness during the Press Ctrl-D
  again footer is unprobed (spec 2u sends both keys without a gate between them)`.

## 10. Out of scope

- Exit events (F8) and heartbeat (F9), `kill --exited` (F4), any change to `reset`.
- Per-harness exit keys configurable in the team file; Generic exit keys.
- A forced Ctrl-C ever; typing `/exit` or `/quit` instead of Ctrl-D.
- Changing the 3 s HUP-to-KILL wait.

## 11. Acceptance

Run after implementation, in the main checkout, no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four commands pass. A missed `Request::Kill` or `Request::Shutdown` use fails to compile, so
no scan is needed for them. `git status` shows only files from section 0.
