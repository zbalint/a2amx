# Spec 2t: the `a2amx daemon` command group and a confirming `kill`

## 0. Status

**LOCKED** (2026-10-03, pre-lock gate run against the tree at the baseline below). Decisions D1
to D9 come from the owner's design of 2026-10-03 (backlog item F6). Gate findings folded in:
the acceptance `rg` first matched `tests/omp_smoke.rs` (an unrelated `"stop"` literal) and was
tightened; the old hint text and every old spelling were grepped across the whole tree and each
hit is in scope; the baseline `cargo test --no-fail-fast` was green (0 failed, 3 ignored).

**Baseline:** develop at `49da4da`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-daemon-command-group`. Public test seam: the `a2amx` CLI against a real daemon in a temp
state dir (`tests/daemon_cli.rs`, `tests/refs_and_list.rs`, `tests/common`).

**Scope.** May edit exactly these files and no others.

- `src/cli.rs`, `src/main.rs`, `src/client.rs` (the `UNREACHABLE` hint literal only)
- `tests/daemon_cli.rs`, `tests/refs_and_list.rs`, `tests/broker.rs` (one spawn line),
  `tests/daemon.rs` (the hint literal only)
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/wire.rs`, `src/daemon.rs`, `src/session.rs`, the team code paths
(`run_team_up`, `run_team_down`), `scripts/`, `extension/`. No new dependency and no new wire
request. Do not commit, stage or merge: leave the diff uncommitted for review.

## 1. Why

Daemon lifecycle is spread over three unrelated spellings: a bare `a2amx daemon` that runs in
the foreground, `a2amx daemon --background` that detaches, and a top-level `a2amx stop`. There
is no way to ask whether a daemon is up, and the common case (start it and get the prompt back)
needs a flag. `a2amx kill` ends a running session without asking, while `stop` asks. This spec
groups the lifecycle under `a2amx daemon {start,status,stop}`, makes detaching the default and
gives `kill` the same confirmation `stop` already has.

## 2. Decisions

- **D1.** `a2amx daemon` becomes a command group with required subcommands `start`, `status`
  and `stop`. The bare form `a2amx daemon` is no longer a command: clap prints its usage error
  and exits non-zero. The top-level `a2amx stop` and the option `--background` are REMOVED, not
  aliased or hidden.
- **D2.** `a2amx daemon start [--listen ADDR]... [--host-name NAME] [--foreground]`.
  Without `--foreground` it does what `daemon --background` does today (`run_daemon_background`
  unchanged in behavior: refuses when a daemon already answers, appends output to
  `<state dir>/daemon.log`, prints `listening on <addr>` per address, waits up to 10 s). With
  `--foreground` it runs what bare `daemon` ran (`run_daemon` unchanged). The detached re-exec
  now runs `--home <h> daemon start --foreground [--listen ...] [--host-name ...]`.
- **D3.** `a2amx daemon stop [--yes]` is the current `stop`, unchanged in behavior and messages
  (`no daemon running` with exit 0 when none answers; the y/N prompt; the refusal
  `refusing to stop: N running session(s); pass --yes to end them`; `not stopped`; `stopped`).
- **D4.** `a2amx daemon status` connects as `run_stop` does. When a daemon answers it prints
  exactly these lines to stdout and exits 0:

  ```
  running
  listening on 127.0.0.1:41000
  sessions: 2 running, 1 exited
  ```

  with one `listening on <addr>` line per line of `<state dir>/addr`, and the counts taken from
  `Request::List` (`exit_code.is_none()` is running). When no daemon answers it prints
  `not running` to stdout, prints nothing to stderr and exits 1. Any other failure is an
  ordinary error (stderr, exit 1).
- **D5.** `a2amx kill <session> [--yes]`. The reference is resolved client-side as today. If
  the resolved id names a session in `Request::List` whose `exit_code` is `None`, `kill`
  confirms before sending `Request::Kill`: on a terminal it prints
  `Killing <id> ends a running session. Continue? [y/N] ` to stderr and reads a line; `y` or
  `yes` (case-insensitive) continues, anything else prints `not killed` to stdout and exits 0.
  Without a terminal it fails with `refusing to kill: session <id> is running; pass --yes to
  end it` and sends nothing. `--yes` skips the confirmation. An exited session, and an id the
  list does not contain, skip the confirmation: the daemon's existing answer (success, or
  `unknown session <ref>`) is the result, as today.
- **D6.** The prompt reader is shared, not duplicated: extract the confirmation block of
  `run_stop` (terminal check, prompt, line read, y/yes test) into one helper in `src/main.rs`
  used by `run_stop` and `run_kill`. Messages stay per caller (D3, D5).
- **D7.** `team down` still kills without confirming (it sends `Request::Kill` itself) and is
  untouched.
- **D8.** The unreachable-daemon hint becomes
  ``cannot reach the a2amx daemon: start it with `a2amx daemon start` ``. It is the `UNREACHABLE`
  constant in `src/client.rs`; the literal is asserted in `tests/daemon.rs`.
- **D9.** `status` must be able to exit 1 without the `a2amx: <error>` line `main` prints for an
  `Err`. How `dispatch` carries that (for example returning an exit code) is the developer's
  choice inside `src/main.rs`; the observable result is D4.

## 3. `src/cli.rs`

Replace `Command::Daemon { listen, host_name, background }` (line 34) with
`Command::Daemon { action: DaemonAction }` and add `DaemonAction { Start { listen, host_name,
foreground }, Status, Stop { yes } }` next to `TeamAction`. Remove `Command::Stop` (line 99).
Add `yes: bool` to `Command::Kill` (line 88), doc text as `Stop`'s `--yes`. Update the doc
comments (`Run the host daemon...` becomes `Manage the host daemon: start, status, stop.`).

## 4. `src/main.rs`

- `dispatch` (lines 66 to 93): route `Command::Daemon { action }` to `run_daemon` /
  `run_daemon_background` (Start), a new `run_daemon_status` (Status) and `run_stop` (Stop);
  delete the `Command::Stop` arm; pass `yes` to `run_kill`.
- `run_daemon_background` (line 181): the re-exec arguments become `daemon start --foreground`
  as in D2.
- `run_daemon_status`: new, D4. `run_stop` (line 232): confirmation block extracted per D6.
- `run_kill` (line 572): per D5. It needs the session list for the exit state; reuse
  `request_sessions` and the client it already opens, do not add a wire request.
- Line 160 (`a2amx daemon is already running`) and the other messages stay as they are.

## 5. Tests

Existing tests are moved to the new spellings, then new behavior is added (write each new test
failing first).

- `tests/daemon_cli.rs`: every `["daemon", "--background"]` becomes `["daemon", "start"]`;
  every `["stop", ...]` becomes `["daemon", "stop", ...]` (lines 70, 125, 137, 152, 157, 177
  and the `Cleanup` helper); `foreground_daemon_honors_stop` (line 166) starts
  `["daemon", "start", "--foreground", "--listen", "127.0.0.1:0"]`.
- `tests/broker.rs:24`: `["daemon", "start", "--foreground", "--host-name", "host-a", "--listen",
  "127.0.0.1:0"]`.
- `tests/daemon.rs:515`: the literal of D8.
- `tests/refs_and_list.rs`: the two `kill` calls on a running session (lines 47 and 125) pass
  `--yes`; line 138 (`kill nobody`) is unchanged and keeps proving D5's unknown-id path.
- New in `tests/daemon_cli.rs`:
  1. `daemon status` with no daemon: stdout `not running\n`, empty stderr, exit 1.
  2. `daemon status` with a daemon and one running and one exited session: the exact three lines
     of D4 with the real address (read from `<state dir>/addr`), exit 0.
  3. Bare `daemon` and `--background` and the top-level `stop` are rejected (non-zero).
  4. `kill` without a terminal on a running session: refused with the D5 message, the session is
     still listed as running; with `--yes` it succeeds.
  5. `kill` of an exited session without `--yes` succeeds.
  6. `daemon start` while a daemon runs still fails with the existing already-running message.
- The y/N prompt of `kill` is verified once by hand in a pty on a temp home (as 2j did for
  `stop`) and recorded in the completion report; no pty test is required.

## 6. Docs

- `README.md` lines 97 to 108: replace the four daemon lines with `a2amx daemon start
  [--foreground]`, `a2amx daemon status`, `a2amx daemon stop [--yes]`, and `a2amx daemon start
  --host-name host-a`; `kill` line gets `[--yes]`.
- `docs/architecture.md` lines 242 to 246: the same renames (`a2amx daemon start`, `a2amx daemon
  stop`).
- `docs/backlog.md`: delete row F6; the F1 and F7 rows keep their wording.

## 7. Out of scope

- Graceful exit, the exit event and the heartbeat (F7 to F9), and `kill --exited` (F4).
- Any change to `team up/down`, the wire protocol or daemon internals.
- Aliases or deprecation warnings for the removed forms.
- `daemon restart`, a pid file, or `status --json`.

## 8. Acceptance

Run after implementation, in the main checkout, and all must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
rg -n 'daemon --background|\["stop"|"--background"|a2amx stop|a2amx daemon`|\["daemon", "--' README.md docs src tests --glob '!docs/specs/*' --glob '!docs/backlog.md'
```

The last command prints nothing (at the baseline it matches only README.md, docs/architecture.md,
src/client.rs, tests/daemon.rs, tests/daemon_cli.rs and tests/broker.rs, all in scope). It
deliberately does not match the bare string `"stop"`, which `tests/omp_smoke.rs` uses for an
unrelated OMP protocol message. `git status` shows only files from section 0.
