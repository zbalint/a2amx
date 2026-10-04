# Spec 2z: `a2amx kill --exited` removes every exited session

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `ba25113`; gate notes in section 6).
Backlog items I1 and F4. Decisions D1 to D5 are the architect's, marked **(architect)**, settled
on the owner's instruction of 2026-10-04 to write a spec for every planned backlog item.

**Baseline:** develop at `ba25113`, after spec 2y is accepted and committed (this spec touches
`src/cli.rs` and `src/main.rs`, which spec 2y also edits in `src/main.rs`; do not start before
2y is committed). **Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch
`develop`. Shared task `context_id`: `a2amx-kill-exited`. Public test seam: the `a2amx kill` CLI
against a real daemon (`tests/refs_and_list.rs`, `tests/daemon_cli.rs`).

**Scope.** May edit exactly these files and no others.

- `src/cli.rs` (`Command::Kill`)
- `src/main.rs` (`run_kill` and its dispatch)
- tests: `tests/daemon_cli.rs` or `tests/refs_and_list.rs`, whichever already drives `a2amx kill`
  (read both, extend one), and `tests/common/mod.rs` only if a helper is needed
- `README.md`, `docs/architecture.md` (only where `kill` is described), `docs/backlog.md`

Does not touch: `src/daemon.rs`, `src/wire.rs`, `src/session.rs`, the team code. No new
`Request`, no daemon change, no new dependency. Do not commit, stage or merge.

## 1. Why

An exited session keeps its name until `a2amx kill NAME`, so a new session cannot reuse the name
(I1), and `team down` clears only the names in the team file (F4). Cleaning up after a day of
sessions means one `kill` per name. The daemon already removes an exited session on `Request::Kill`;
only a bulk entry point is missing.

## 2. Decisions

- **D1. (architect)** `a2amx kill --exited` kills every session whose `exit_code` is `Some`, one
  `Request::Kill` per session, using the list the client already fetches (`request_sessions`).
  Running sessions are never touched by this flag.
- **D2. (architect)** `--exited` and the positional `session` are mutually exclusive, and one of
  them is required: `a2amx kill` with neither is a usage error. `--exited` conflicts with `--now`
  (nothing to stop) and with `--yes` (never prompts, so the flag has no meaning).
- **D3. (architect)** No confirmation: removing an exited session loses only its retained screen
  and name, the same as `kill NAME` on an exited session today, which does not prompt.
- **D4. (architect)** Output is one line per removed session, `removed NAME-or-ID`, where the
  name is used if the session has one. With nothing to remove it prints `no exited sessions` and
  exits 0. If one kill fails, the loop continues, each failure is reported on stderr as
  `NAME-or-ID: MESSAGE`, and the command exits non-zero after the loop.
- **D5.** The existing `kill SESSION` behavior, flags and messages are unchanged.

## 3. `src/cli.rs`, `src/main.rs`

`Command::Kill` (line 88 at `cb6f831`): make `session` an `Option<String>`, add
`#[arg(long, conflicts_with_all = ["session", "now", "yes"])] exited: bool`, and require one of
the two with clap's `required_unless_present`. `run_kill` (line 657) keeps its body for the
positional case; a sibling branch for `--exited` reuses `request_sessions` and the same
`Request::Kill { session, now: false }` call. Reuse, do not duplicate, the response matching.

## 4. Tests (failing first)

Through the CLI against a real daemon: start two sessions that exit at once (`sh -c 'exit 0'`
style) and one that keeps running; `kill --exited` prints two `removed` lines, `list` then shows
only the running one, and a new session can take one of the freed names. A run with no exited
sessions prints `no exited sessions` and exits 0. `kill` with neither argument, and `--exited`
with a session name, are usage errors with a non-zero exit.

## 5. Docs and out of scope

`README.md`/`docs/architecture.md`: add the flag where `kill` is described. `docs/backlog.md`: move
I1 and F4 to Closed with this spec. Out of scope: `team down` clearing extra names, automatic
removal of exited sessions, name reuse without removal, and filtering by age or exit code.

## 6. Acceptance and gate notes

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

Gate notes at lock time: `Command::Kill` is defined at `src/cli.rs:88` with fields `session`,
`yes`, `now`; `run_kill` at `src/main.rs:657` is dispatched at `src/main.rs:102`; `Request::Kill`
(`src/wire.rs:146`) and its daemon arm (`src/daemon.rs:1455`) are reused unchanged. The behavior
that `kill NAME` removes an exited session without a prompt is taken from backlog item I1 and the
`run_kill` code (the prompt is only for `running` sessions); the developer confirms it with a first
failing test before relying on it.

## Amendment 1 (2026-10-04): the baseline confirmation is a passing test, not a failing one

The developer reported `BLOCKED` (m_348): section 6 says the developer confirms with "a first
failing test" that `kill NAME` removes an exited session without a prompt, but that behavior
exists already and is covered by `tests/daemon_cli.rs:938` to `952`
(`kill_exited_session_skips_confirmation`, 1 passed at the baseline). A test of existing behavior
cannot be red. The claim is verified: `run_kill` prompts only when the session is running
(`src/main.rs:658` to `680`).

Decision (architect): the section 6 sentence is replaced by this rule. The existing
`kill_exited_session_skips_confirmation` is the baseline confirmation of D3 and D5; the developer
runs it first and reports its result. No new test of the existing named behavior is required. The
first new test, and the first red, is the `kill --exited` test of section 4, written before any
production edit. Section 0 scope and D1 to D5 are unchanged, and extending `tests/refs_and_list.rs`
(which already drives the CLI against a real daemon) is allowed as section 0 says.
