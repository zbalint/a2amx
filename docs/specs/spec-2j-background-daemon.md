# Spec 2j: run the daemon in the background, and stop it

## 0. Status

**LOCKED.** Decisions D1 to D8 in section 2 were settled with the owner. Builds on spec 2i.
The work is done directly on `develop` in the main checkout, no worktree. Shared task
`context_id`: `a2amx-background-daemon`. Public test seam: the `a2amx` binary
(`env!("CARGO_BIN_EXE_a2amx")`) driven as a subprocess, as `tests/mcp.rs` and
`tests/omp_wiring.rs` do.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/cli.rs`, `src/main.rs`, `src/wire.rs`, `src/daemon.rs`
- `tests/daemon_cli.rs` (new)
- `README.md`, `docs/architecture.md`

Does not touch: `src/client.rs` (the existing `Client::connect` and `Client::request` are enough),
every other file under `src/`, every other file under `tests/` (`tests/common` included),
`scripts/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the locked spec documents. No new
dependency. Do not commit and do not merge: leave the diff uncommitted in the working tree.

`Request` gains a variant, so existing tests that match `Request` exhaustively must keep
compiling; none does today (checked with `rg "match .*request" tests`), so no test file other
than the new one changes.

## 1. Why

`a2amx daemon` runs only in the foreground, so every working session starts with a spare
terminal or a `nohup`/`tmux` wrapper, and the only way to stop it is Ctrl-C or `kill` in that
terminal. With the architect, developer and tester workflow the human talks to one session and
the daemon is just infrastructure. A `--background` flag starts it detached from the terminal
and a `stop` command ends it cleanly.

The daemon already takes an exclusive `flock` on `<state>/daemon.lock`, writes `addr` and
`admin.token`, removes `addr` on a graceful shutdown, and shuts down gracefully on SIGINT and
SIGTERM (`src/main.rs`, `run_daemon`). Nothing stores a pid, so `stop` is a new admin request
rather than a signal.

## 2. Decisions settled with the owner

- **D1.** `a2amx daemon --background` starts the daemon detached, prints what the foreground
  daemon prints (`listening on <addr>` per address), and exits 0. Without the flag nothing
  changes.
- **D2.** The flag is implemented by the parent re-executing the same binary
  (`std::env::current_exe()`) with `--home <resolved state dir> daemon` plus the original
  `--listen` and `--host-name` arguments. No `fork` inside the running Tokio runtime.
- **D3.** The detached daemon's stdin is `/dev/null`; stdout and stderr append to
  `<state dir>/daemon.log`, created 0600. No rotation.
- **D4.** New command `a2amx stop [--yes]` ends the daemon through a new admin-only request.
  It never signals a pid.
- **D5.** Stopping ends every session (they die with the daemon). If any session is running,
  `stop` asks for confirmation on a terminal, and refuses without a terminal, unless `--yes`.
- **D6.** `a2amx stop` with no daemon running prints `no daemon running` and exits 0, so it is
  safe in scripts.
- **D7.** `--background` refuses to start when a daemon already answers for the state dir.
- **D8.** Out of scope as a feature: auto-start on demand, a systemd unit, a restart command,
  a pid file, log rotation. They are listed in section 8.

## 3. `src/wire.rs`

Add `Shutdown` (no fields) to `Request`, next to `Kill` (`src/wire.rs:138`). No new `Response`:
success is `Response::Ok`.

Session tokens are already rejected for any request outside the allow-list at
`src/daemon.rs:955`, and `Shutdown` is not added to it, so the request is admin-only without
further code. Do not add it to that list.

## 4. `src/daemon.rs`

- `Runtime` gains a `tokio::sync::Notify` named `stop_requested`.
- `Daemon` gains

  ```rust
  /// Completes when an admin client asked the daemon to shut down.
  pub async fn stop_requested(&self);
  ```

  It waits on `stop_requested.notified()`. Use `notify_one` on the sending side so a request
  that arrives before anyone awaits is not lost.
- `Request::Shutdown` in the request match (beside `Request::Kill`, `src/daemon.rs:1010`):
  call `runtime.stop_requested.notify_one()` and answer `Response::Ok`. The response is sent
  before shutdown work begins, because shutdown closes the listeners and connections.

`Daemon::shutdown` is unchanged: it is already what SIGTERM calls.

## 4a. `src/main.rs`: `run_daemon` and `--background`

`run_daemon` gains a third wait: its `tokio::select!` (currently `ctrl_c` and `sigterm`) gets
a branch on `daemon.stop_requested()`. The foreground daemon therefore also honors `stop`.

`Command::Daemon` gains `background: bool` (section 5). `dispatch` passes it on.
When `background` is true `run_daemon` does this instead of serving:

1. If `Client::connect(&home)` succeeds, fail with
   `a2amx daemon is already running (listening on <first addr from the addr file>)`.
   Any connect error means "not running" and falls through (a stale `addr` file after a
   crash is therefore harmless).
2. Create the state directory 0700 if missing, then open `<home>/daemon.log` with
   `OpenOptions::new().create(true).append(true).mode(0o600)`. Reuse the directory-creation
   logic that `Daemon::start` already has rather than copying it: if it cannot be reused
   without moving it out of `Daemon::start` (which the developer must not restructure beyond
   extracting a function), extract it into a small `pub fn` in `src/daemon.rs` and call it
   from both places. Directory creation and file open run in `spawn_blocking`.
3. Spawn the child with `std::process::Command` as an argument vector:
   `current_exe`, `--home`, `<home>`, `daemon`, then `--listen <addr>` for each listen
   address and `--host-name <name>` when given. `stdin` null; `stdout` and `stderr` the log
   file (`try_clone` for the second). In `pre_exec`, call `rustix::process::setsid()` so the
   child has its own session and survives the terminal closing. `pre_exec` is `unsafe`:
   add a `// SAFETY:` comment saying the closure only calls the async-signal-safe `setsid`.
   Strip a trailing `" (deleted)"` from `current_exe` the way `Daemon::start` does at
   `src/daemon.rs:584`; reuse that code by extraction if it is cheap, otherwise the same
   three lines. Do not set `A2AMX_*` environment variables for the child.
4. Poll every 100 ms, for at most 10 s, with the child handle held in the parent:
   - `child.try_wait()` returns `Some(status)`: fail with
     `daemon exited during startup (<status>); see <home>/daemon.log`.
   - `Client::connect(&home)` succeeds: read the `addr` file, print
     `listening on <addr>` for each line to stdout, exit 0. The child is left running; the
     parent drops the handle without waiting.
   - Timeout: fail with `daemon did not become ready within 10s; see <home>/daemon.log`. The
     child is not killed. `// shortcut: a slow start is reported, not reaped; add a kill
     if a half-started daemon ever lingers.`

The polling loop sleeps with `tokio::time::sleep`; `try_wait` and `Client::connect` do not
block the runtime beyond what they already do.

`--background` is the only new way a daemon starts, and `--listen` and `--host-name` pass
through unchanged, so the loopback default and the "no warnings for operator-chosen
addresses" rule in `AGENTS.md` stay as they are.

## 5. `src/cli.rs`

- `Command::Daemon` gains

  ```rust
  /// Start detached from the terminal and print the listen address. Output goes to <state dir>/daemon.log.
  #[arg(long)]
  background: bool,
  ```

  and its doc comment changes from `Run the host daemon in the foreground.` to
  `Run the host daemon (in the foreground unless --background).`
- New variant after `Kill`:

  ```rust
  /// Stop the daemon. This ends every session.
  Stop {
      /// Do not ask for confirmation.
      #[arg(long)]
      yes: bool,
  },
  ```

## 6. `src/main.rs`: `run_stop`

`Command::Stop { yes }` dispatches to `run_stop(home, yes)`:

1. `Client::connect(&home)` fails: print `no daemon running` to stdout, return `Ok(())`.
   Every connect failure counts (refused, no `addr`), matching how `list` already reports
   them as one condition.
2. `Request::List`; count sessions whose `exit_code` is `None`. If the count is above zero
   and `yes` is false:
   - stdin is a terminal (`std::io::IsTerminal`): write
     `Stopping the daemon ends <n> running session(s). Continue? [y/N] ` to stderr, read one
     line from stdin on a blocking thread, continue only for `y` or `yes` in any case,
     otherwise print `not stopped` and return `Ok(())`.
   - stdin is not a terminal: fail with
     `refusing to stop: <n> running session(s); pass --yes to end them`.
3. `Request::Shutdown`. `Response::Error { message }` becomes the error; any other response
   than `Ok` is `unexpected daemon response`, as in `run_kill`.
4. Wait up to 15 s (the daemon gives each child 3 s before SIGKILL) polling every 100 ms
   until `Client::connect(&home)` fails, then print `stopped`. On timeout fail with
   `daemon did not stop within 15s`.

`list`'s "state" for a session is `exit_code`: `None` is running (`src/wire.rs:243`).

## 7. Tests: `tests/daemon_cli.rs` (new)

Write the failing test first, one behavior at a time. Every test uses its own temp state dir
(`--home <tempdir>`, no shared paths) and runs the real binary. A guard struct stops the
daemon in `Drop` (`stop --yes`, then kill if it fails) so a failing test leaves no process.
Match `tests/mcp.rs` for how it launches the binary and creates temp dirs. If a test needs an
exact stdout line, the expected value is a literal.

| # | Behavior | Check |
|---|---|---|
| 1 | background start | `daemon --background` exits 0 within 10 s; stdout's first line starts with `listening on 127.0.0.1:`; `list` then exits 0; `daemon.log` exists with mode 0600 |
| 2 | already running | a second `daemon --background` for the same `--home` exits non-zero; stderr contains `already running` |
| 3 | stop ends it | `stop --yes` exits 0 and prints `stopped`; `list` then exits non-zero |
| 4 | no daemon | `stop` on a fresh `--home` exits 0 and prints `no daemon running` |
| 5 | refusal | with one detached session running (`new --detach -- sh`), `stop` with stdin from `/dev/null` exits non-zero, stderr contains `1 running session`, and `list` still works; `stop --yes` then exits 0 |
| 6 | foreground honors stop | a foreground `daemon` (a child process, port 0) exits 0 after `stop --yes` |
| 7 | startup failure | a `--home` that is an existing regular file makes `daemon --background` exit non-zero and stderr contain `state path is not a directory` or `daemon exited during startup` |

Test 5 uses `sh` because `tests/daemon.rs` already starts generic sessions with it. The
interactive prompt (`y`/`N` on a terminal) has no test: `tests/common/pty.rs` is out of scope.
`// shortcut: the confirmation prompt is verified by hand, see section 9.`

## 8. Out of scope

Auto-starting the daemon from other commands, a systemd or launchd unit, `a2amx restart`, a
pid file, log rotation or a `logs` command, `stop` for remote hosts, changing how SIGINT or
SIGTERM shut the daemon down, a graceful session-exit period (see the improvement memory on
SIGTERM before SIGKILL), `src/client.rs`, and refactoring `Daemon::start` beyond the
extraction in section 4a step 2.

## 9. Acceptance

Run all three, no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`cargo test` includes `tests/daemon_cli.rs`; its seven tests pass. Existing suite counts do not
shrink. These need the new code, so they run after implementation, not at lock time.

Manual check by the architect after implementation (also not a lock-time check), with a
throwaway state dir and never the live daemon's:

```sh
H=$(mktemp -d); a2amx --home $H daemon --background; ls -l $H/daemon.log
a2amx --home $H new --detach -- sh; a2amx --home $H stop      # type n, then y
a2amx --home $H list                                          # now fails
```

`git diff --stat` shows only the files in section 0; `git diff --check` is clean.
