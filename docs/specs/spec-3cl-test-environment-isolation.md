# Spec 3cl: Tests that depend on the daemon's environment run the real daemon with a controlled one

## 0. Status

LOCKED. Tests only; no change to `src/`.

Scope (files that may be edited): `tests/common/mod.rs`, `tests/common/pty.rs` (Amendment 1),
`tests/auto_confine.rs`, `tests/omp_wiring.rs`. Does not touch: `src/`, every other test file,
`AGENTS.md`, `CHANGELOG.md`, `docs/`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `963755c`.
Context id: `a2amx-test-isolation-2026-11-04`. Public test seam: the `a2amx` binary
(`env!("CARGO_BIN_EXE_a2amx")`) and its `daemon start --foreground` command.

## 1. Why

Backlog F75 and I13 have one cause. The tests start the daemon inside the test process
(`Daemon::start` in `tests/common/mod.rs:45`). That daemon reads `HOME` and `XDG_RUNTIME_DIR`
from the test process (`src/daemon.rs:378`, `:381`) and its sessions inherit the test process's
whole environment.

- F75: four `tests/auto_confine.rs` tests set `HOME` only on the client PTY. The in-process daemon
  still uses the real home and creates directories there (`ensure_data_dirs` and
  `ensure_grant_dirs`, `src/daemon.rs:390-392`; the cargo grant creates `~/.cargo`). Inside a
  confined session the home is read-only, so the start fails with `Read-only file system
  (os error 30)` and the test times out waiting for the consent prompt. Which of the two calls
  fails is not established; the developer records the red-phase screen. In an unconfined session
  the four tests pass today and write into the developer's real home, so only a confined session
  shows F75 as a failure.
- I13: `tests/omp_wiring.rs::generic_sessions_get_no_a2amx_bin` removes `A2AMX_BIN` from the
  client command, but the session also inherits it from the in-process daemon, so it fails when
  the test runs inside an a2amx-hosted session. `AGENTS.md` works around this with
  `env -u A2AMX_BIN -u NO_COLOR`. `NO_COLOR` no longer breaks any test (see section 4).

Rejected: a `home` and `runtime_dir` field on `DaemonConfig`. It fixes `HOME` and
`XDG_RUNTIME_DIR` but not the inherited session environment, and it changes production code to
help tests. Rejected: `std::env::set_var` in tests. It is `unsafe` in edition 2024 and global
across parallel tests.

## 2. `tests/common/mod.rs`

Add one helper that runs the real daemon binary with an explicit environment:

```rust
pub struct DaemonProcess { /* child, state dir */ }

/// Runs `a2amx --home <state> daemon start --foreground --listen 127.0.0.1:0` with the
/// environment cleared and only `PATH` (from this process), `HOME` and `XDG_RUNTIME_DIR` set.
/// Returns once `a2amx list` against the state dir succeeds; panics after 10 s.
pub fn spawn_daemon_process(state: &Path, home: &Path, runtime: &Path) -> DaemonProcess;
```

The helper is synchronous and blocks, like `PtyHarness` and the `daemon_cli.rs` guard already do
inside these `#[tokio::test]` bodies; `AGENTS.md`'s rule against blocking the runtime is about
`src/`. `Drop` runs `a2amx --home <state> daemon stop --yes`, waits up to 15 s for the child, and kills
it if it has not exited, so a failing test leaves no process. The pattern, including the wait
loop and the cleanup guard, is `tests/daemon_cli.rs:66-83` (`Cleanup`, `wait_exit`) and `:162-189`
(`foreground_daemon_honors_stop`); copy its shape, do not
import it (test binaries cannot share code except through `tests/common`). The helper takes the
state dir as given; it does not create it.

`start_daemon` and `start_daemon_in` stay unchanged for every other test.

## 3. Tests that switch to it

- `tests/auto_confine.rs`: the four tests at `:258`, `:334`, `:410`, `:479`
  (`pty_yes_persists_accepted_grants_with_private_mode`,
  `pty_no_and_eof_refuse_without_starting_or_writing`,
  `pty_deduplicates_auto_consent_across_sessions`,
  `pty_subtracts_seeded_auto_grants_from_consent`). Each replaces its
  `common::start_daemon_in(...)` call with `spawn_daemon_process(state, user_home, runtime)`,
  using the `user_home` and `runtime` temp dirs the test already creates. Remove the now unused
  `bwrap-path` symlink setup in those tests only if nothing else in the test reads it. The
  spawned daemon finds `bwrap` through `PATH`.
- `tests/omp_wiring.rs`: `generic_sessions_get_no_a2amx_bin` (`:97`) uses
  `spawn_daemon_process` with a fresh temp dir for `home` and `runtime`.
  `capture_launch` and the other two tests are unchanged.

In `pty_no_and_eof_refuse_without_starting_or_writing` the guard is created inside `run_case`
and lives past the final `list` check (`:390-401`), so each of the two cases has its own daemon.

One assertion is added, to `pty_yes_persists_accepted_grants_with_private_mode`: after the
existing `auto-grants` checks, `user_home.path().join(".cargo")` is a directory (the accepted
`cargo` grant's `create` path, `src/grant.rs:250`, created under the daemon's `HOME`). It holds
in any environment, so it proves the daemon used the test home even when the real home is
writable. No other assertion changes. The `Some("host-a")` host name the in-process daemon used
is not passed (`--host-name`): no assertion in these five tests reads the daemon's host name
(the other `host-a` hits in `auto_confine.rs` are git remote URLs, lines 71, 573, 758, 838).

## 4. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| The tests' daemon is in-process | `tests/common/mod.rs:45` | read |
| The daemon takes `HOME` and `XDG_RUNTIME_DIR` from its own environment | `src/daemon.rs:378-383` | read |
| The four tests set `HOME` only on the client PTY | `tests/auto_confine.rs:304, 381, 457, 528` | read |
| The four tests pass in an unconfined session and fail in a confined one | consultant read of `src/daemon.rs:378-392`; memory of an earlier unconfined pass | read (unconfined pass not rerun by the architect) |
| The four tests fail in this confined session with `Read-only file system` and the others in the file pass | full suite, `--no-fail-fast` | ran: 11 passed, 4 failed; exit 101 |
| `generic_sessions_get_no_a2amx_bin` fails with `A2AMX_BIN` set: left `"/x"`, right `"unset"` | `tests/omp_wiring.rs:100` | ran with `A2AMX_BIN=/x` |
| `NO_COLOR=1` alone does not fail it | same test | ran with `NO_COLOR=1`: passes |
| With `A2AMX_BIN`, `NO_COLOR`, `A2AMX_ADDR` and `A2AMX_TOKEN` set, only `auto_confine` (4) and `omp_wiring` (1) fail | full suite | ran |
| `a2amx` client copies its own environment into the session | `src/main.rs:869` | read |
| `daemon start --foreground --listen 127.0.0.1:0` runs the daemon and `daemon stop --yes` ends it | `tests/daemon_cli.rs:160-185` | read; passes in the full run |
| No assertion in the five tests reads the daemon's host name | `tests/auto_confine.rs`, `tests/omp_wiring.rs` | grep `host-a`: git remote URLs (lines 71, 573, 758, 838) and the `Some("host-a")` arguments to `start_daemon_in` (185, 280, 357, 433, 502, 624, 956), none asserted |
| A cleared environment is enough for the daemon: it reads only `HOME`, `XDG_RUNTIME_DIR` and `PATH`, and finds `bwrap` through `PATH` | `src/daemon.rs:378,381,400,1661-1665`, `src/confine.rs:2026-2052` | read (consultant, then architect spot check of `:378-383`) |
| No other test depends on `A2AMX_*` ambient values | `tests/*.rs` | grep `A2AMX_(BIN|ADDR|TOKEN)`: the hits set or remove them explicitly or are shell text |

## 5. Out of scope

- Flaky tests I7 and I14: separate spec, they need a reproduction first.
- Running other in-process-daemon tests through the new helper; switch them only when one fails
  for this reason.
- Production code, `AGENTS.md` (the architect removes the `env -u` workaround at acceptance if
  the acceptance run supports it), `CHANGELOG.md`, the backlog rows.
- `tests/omp_wiring.rs` `capture_launch`'s `env_remove("A2AMX_BIN")`: stays; it is harmless.

## 6. Acceptance

All run in the main checkout, as an ordinary user.

Environment matters: F75 fails only in a confined a2amx session (read-only home). Criteria 1, 2
and 4 are run in a confined session, where today's code fails 4 + 1 times. An unconfined run
fails only the `omp_wiring` test today and so cannot show F75 fixed; the `.cargo` assertion in
section 3 is what holds in both.

1. `env -u A2AMX_BIN -u NO_COLOR cargo test --no-fail-fast` ends with exit 0 and no
   `FAILED`, inside a confined a2amx session. Judge: the architect reruns it.
2. `A2AMX_BIN=/x NO_COLOR=1 A2AMX_ADDR=127.0.0.1:1 A2AMX_TOKEN=t cargo test --no-fail-fast`
   ends with exit 0, in a confined session. This is the fixture that fails today with 5
   failures. Judge: the tester.
3. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
4. Red phase: before the change, check 2 fails in `auto_confine` (4) and `omp_wiring` (1).
   After the change, the same run passes. The developer records both outputs.
5. No leftover process: after the full run, `pgrep -af 'daemon start --foreground --listen 127.0.0.1:0'`
   lists no process whose `--home` is under a temp directory (the host's own daemon also matches
   the command, so judge by `--home`). In a confined session `pgrep` cannot see other sessions,
   so the judge is the tester running it unconfined, or the architect reading the `Drop` code.
6. Mutation check by the developer, in the checkout, restored by checksum afterward. Each mutant
   must be killed by a named test:
   a. Remove `env_clear()` from the helper: `generic_sessions_get_no_a2amx_bin` fails when
      `A2AMX_BIN=/x` is set.
   b. Set `HOME` to the real home instead of the `home` argument:
      `pty_yes_persists_accepted_grants_with_private_mode` fails on the `.cargo` assertion in any
      environment (and by timeout in a confined one).
   c. Remove the `daemon stop` from `Drop`: the guard check in criterion 5 shows a stray daemon
      (record the observation; no test asserts it).
7. `git status --short` shows only the three files in scope. No scratch files; mutation work uses
   the checkout and `target/scratch/3cl/` if a copy is unavoidable, deleted before reporting.

Known flaky tests: `tests/attach_cli.rs::attach_does_not_capture_session_already_in_drag_mode`
(I7) and `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (I14). A
failure there: rerun that file once; if it passes, record it as pre-existing.

## Amendment 1: the PTY must run in the fixture's directory (BLOCKED report m_5434, 2026-11-04)

Found by the developer after the helper and the section 3 replacements were in place: all four
migrated `auto_confine` tests still time out waiting for the consent prompt. The screen shows
`started a2amx-architect s1` and the other three sessions of the checkout's own `a2amx.toml`,
not the fixture's `worker`.

Cause (read by the architect): the four tests run `a2amx team up --detach` in a PTY whose
working directory is the test process's, the repository checkout. `resolve_team_file`
(`src/main.rs:961-975`) prefers `./a2amx.toml` over `<state>/team.toml`, and the gitignored
`a2amx.toml` of this checkout (`.gitignore:4`) exists. On CI and on a fresh clone it does not, so
the tests passed there. The earlier red-phase screen (`failed a2amx-tester: Read-only file
system`) was already this team file, so the real cause of F75 here is both the ambient
environment (section 1) and the ambient team file. `PtyHarness::spawn_program` sets no working
directory (`tests/common/pty.rs:58-80`).

Rejected: adding `--file <state>/team.toml`. A `--file` makes `resolve_team_file` return
`default_team: None` (`src/main.rs:942-949`), which is not the global-team path these tests
exist to cover ("Accept these for auto mode on all projects").

Changes:

- **A1.1** `tests/common/pty.rs`: give the PTY an optional working directory without changing any
  existing caller. Move the body of `spawn_program` into a private
  `spawn_inner(program, args, home, cwd: Option<&Path>, environment)` that sets
  `tty::Options::working_directory` when `cwd` is `Some`; `spawn_program` calls it with `None`
  and a new `pub fn spawn_with_env_in(args, home, cwd, environment)` calls it with
  `Some(cwd)` and `env!("CARGO_BIN_EXE_a2amx")`. No other function changes.
- **A1.2** `tests/auto_confine.rs`: the four tests call `spawn_with_env_in` with the fixture's
  `project` directory as `cwd` (it holds `Cargo.toml` and no `a2amx.toml`; the tests already
  run their `list` check there). Section 3's other changes stand.
- **A1.3** Section 6 additions: (8) in this checkout, which has an `a2amx.toml`, the four tests
  pass; (9) mutant: pass the repository directory instead of `project` as `cwd` in one test: that
  test fails here (the screen shows the checkout's sessions). It passes on a clone without
  `a2amx.toml`, so record it as an observation in that case.
- **A1.4** Out of scope grows by nothing; `pty.rs` is now in scope only for A1.1.

Claims for the amendment:

| Claim | Where | Verified how |
|---|---|---|
| `resolve_team_file` prefers `./a2amx.toml`, then `<state>/team.toml` | `src/main.rs:939-984` | read |
| `--file` yields `default_team: None` | `src/main.rs:942-949` | read |
| The checkout's `a2amx.toml` is gitignored | `.gitignore:4` | `git check-ignore -v a2amx.toml` |
| `spawn_program` has callers `pty.rs:55`, `attach_cli.rs:862,1704,1726`; `spawn_with_env` has 5 (4 here, `attach_cli.rs:833`) | `tests/` | grep |
| alacritty `tty::Options` has a `working_directory` field | `alacritty_terminal` pinned version | read: `alacritty_terminal-0.26.0/src/tty/mod.rs:30`, version from `Cargo.lock` |
