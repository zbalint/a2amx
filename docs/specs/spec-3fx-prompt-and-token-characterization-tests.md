# Spec 3fx: characterization tests for the confirmation prompts of `daemon stop` and `kill`, and the empty gateway token

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the `a2amx` binary
(`env!("CARGO_BIN_EXE_a2amx")`) and the existing PTY harness `tests/common/pty.rs`.

Scope (edit): `tests/daemon_cli/daemon_lifecycle.rs` (one tightened assertion, one new test),
`tests/daemon_cli/kill_and_down.rs` (one new test, one added assertion), new `tests/gateway_token.rs` (one test).

Does not touch: every file under `src/`, `Cargo.toml`, `Cargo.lock`, every other test file, `docs/` other than this
spec, `AGENTS.md`, `CHANGELOG.md`. All new tests and assertions are contract tests: they pass on the base code (they pin
behaviour that exists and has no test today). No red phase.

## 1. Why

The command layer is about to be refactored (typed daemon replies, a launch type, report renderers). A reviewer
can only call such a change behaviour-preserving for what a test pins. The consultant's survey of `tests/` found
these command paths unpinned or loosely pinned: the exact text of the `daemon stop` refusal without a terminal
(`stop_refuses_to_end_sessions_without_a_terminal`, `daemon_lifecycle.rs:72-87`, only checks that stderr contains
`1 running session`), the `daemon stop` and `kill` decline paths (`not stopped`, `not killed`; both need a terminal on stdin, so a PTY), and
`gateway token` with an empty stdin (`gateway token is empty`), and that a successful `kill --yes` prints nothing
(`kill_running_session_requires_confirmation_without_a_terminal` asserts only success). The kill refusal without a terminal, the
`--exited` paths, `list --team/--as`, and `reset` refusals already have tests (`tests/daemon_cli/kill_and_down.rs`,
`tests/refs_and_list.rs`, `tests/daemon_cli/reset.rs`). Not characterized here, with the reason: `team reset`'s
decline path (`not reset`; needs a running team and a PTY, a heavy fixture for a branch that mirrors `kill`'s);
`gateway stop`'s success line (needs a running gateway); `no free generated session name` (needs every generated
name taken); the `invalid control request` mapping of `new --confine` (needs a scripted control-protocol daemon;
the typed-reply spec that touches it carries that test).

## 2. The tests (written against the handlers in `src/commands/`; expected strings are literals from the code)

All use the helpers of `tests/daemon_cli/main.rs` (`run`, `stdout`, `stderr`, `Cleanup`, `PtyHarness`), an
isolated `home` in a tempdir (`dir.path().join("state")`), and start the daemon with
`run(&home, &["daemon", "start"])` like the neighbouring tests.

1. Tighten `stop_refuses_to_end_sessions_without_a_terminal` (`daemon_lifecycle.rs:72-87`): replace its
   `contains("1 running session")` assertion with `assert_eq!(stderr(&refused), "a2amx: refusing to stop: 1 running
   session(s); pass --yes to end them\n")`; keep its other assertions. No new test. Source: `src/commands/daemon.rs`
   `run_stop`, the `confirm` refusal text.
2. `daemon_stop_declined_at_the_prompt_leaves_the_daemon_running` (`daemon_lifecycle.rs`): same setup; spawn
   `daemon stop` under `PtyHarness::spawn(&args, &home)` (args as `--home <home> daemon stop` strings, like
   `tests/daemon_cli/reset.rs:109-119`), wait for the prompt text `Continue? [y/N]`, send `n\n`, wait for the text
   `not stopped`, `wait_exit(Duration::from_secs(10))` returns 0. Then `run(&home, &["list"])` succeeds and lists the id. The prompt text
   is `Stopping the daemon ends 1 running session(s). Continue? [y/N] `.
3. `kill_declined_at_the_prompt_leaves_the_session_running` (`kill_and_down.rs`): start a daemon, create a running
   session as in `kill_running_session_requires_confirmation_without_a_terminal`, spawn `kill <id>` under the PTY,
   wait for `Continue? [y/N]` (the prompt is `Killing <id> ends a running session. Continue? [y/N] `), send
   `n\n`, wait for `not killed`, `wait_exit(Duration::from_secs(10))` returns 0; `list` still shows the id as `running`.
   Also in `kill_running_session_requires_confirmation_without_a_terminal`, after the forced kill add
   `assert_eq!(stdout(&forced), "")` (a successful `kill --yes` prints nothing: `kill_session` returns `Ok(())`).
4. `gateway_token_with_an_empty_line_is_refused` (new `tests/gateway_token.rs`, a separate target that declares its
   own `#![allow(clippy::expect_used, clippy::unwrap_used)]`, `const BIN: &str = env!("CARGO_BIN_EXE_a2amx")` and
   uses `tempfile::tempdir()`; it cannot use the `daemon_cli` helpers and needs no `mod common`): run
   `a2amx --home <tempdir>/state gateway token` with piped stdin, write `"\n"` and close stdin (the `Command` and
   piped-stdin shape of `tests/migrate_cli.rs:74-95`, but with `--home`, so the migration branch is skipped). Expect
   a failing status and stderr exactly `a2amx: gateway token is empty\n`; the file `<home>/config/secrets/discord.token`
   does not exist afterwards (the directories do: `run_gateway_token` runs `layout.ensure()` before reading, so
   assert on the file only).

A PTY test waits with the harness's `wait_for_text` (10 s limit) and never sleeps blindly; the daemon is stopped
by `Cleanup` at the end of each test.

## 3. Out of scope

Everything in section 1 marked "Not characterized"; any change under `src/`; moving or renaming existing tests;
the `team reset` decline path.

## 4. Acceptance

- `cargo test --test daemon_cli --test gateway_token` passes; the three new tests are listed in the output.
- Mutation check (the developer, on the assigned checkout; each mutant is a one-line edit to `src/commands/`,
  restored by sha256 of the file before and after, with non-empty edits): (1) in `daemon.rs` change the text
  `refusing to stop` to `refusing to stopX` (killed by the tightened test 1); (2) in `daemon.rs` change `not stopped` to `not stoppedX`; (3) in
  `daemon.rs` change `if !confirm(` of `run_stop` to `if confirm(`; (4) in `sessions.rs` change `not killed` to
  `not killedX`; (5) in `sessions.rs` change `if !confirm(` of `run_kill` to `if confirm(`; (6) in `gateway.rs`
  change `gateway token is empty` to `gateway token is emptyX`; (7) in `gateway.rs` change `if token.is_empty()`
  to `if !token.is_empty()`. Each mutant must make the named test fail and no other: (1) test 1; (2) test 2; (3) test 2
  (under the PTY the answer `n` then proceeds to shut the daemon down); (4) test 3; (5) test 3; (6) test 4; (7)
  is an equivalent mutant for test 4: `relay_files::write_token` (`src/relay_files.rs:82-85`) rejects an empty token
  with the same text, so test 4's observable result is unchanged; only
  `tests/migrate_cli.rs::test_14_gateway_token_migrates_and_explicit_home_is_ignored` fails (a non-empty token then
  bails). The developer records 7 as killed by that test only (amendment 1). The developer lists the killing test per mutant. After the pass `git diff --stat --
  src` is empty.
- `cargo test --no-fail-fast` all pass (baseline at the base commit: 64 suites ok, 1393 passed, 0 failed,
  3 ignored; after: 65 suites, 1396 passed), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
  Known flaky tests are listed in the spec of record 3fw section 6 (the same set; rerun a failure once and record
  it as pre-existing if it passes).
- `git status --short` lists only the three files of section 0 (`daemon_lifecycle.rs`, `kill_and_down.rs`, `gateway_token.rs`).
- The tester repeats the mutation pass independently; the architect reviews the diff.

## 5. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| The texts `refusing to stop: {n} running session(s); pass --yes to end them`, `not stopped`, `Stopping the daemon ends {n} running session(s). Continue? [y/N] ` | `src/commands/daemon.rs:218,221,225` | read |
| The texts `Killing {id} ends a running session. Continue? [y/N] `, `refusing to kill: ...`, `not killed` | `src/commands/sessions.rs:165-172` | read |
| `confirm` refuses with the given text when stdin is not a terminal, otherwise writes the prompt to stderr and reads one line, `y`/`yes` accepts | `src/terminal.rs` (`confirm`) | read |
| The error line is `a2amx: {error:#}` on stderr and the exit status is failure | `src/main.rs` `main` | read; confirmed by the kill test `stderr(&refused)` at `tests/daemon_cli/kill_and_down.rs:18` |
| `gateway token` reads one line from stdin, trims it, and bails with `gateway token is empty` if empty, before any file write | `src/commands/gateway.rs:29-43` (empty check at 39; the read is inside `run_gateway_token`, 14-27, after `migrate_old_state` and `layout.ensure()`) | read; the file-absent assertion is checked by the developer against the base |
| No test contains `refusing to stop`, `not stopped`, `not killed` or `gateway token is empty` as an exact text; the stop refusal is asserted loosely (`daemon_lifecycle.rs:72`) | `rg -n 'refusing to stop\|not stopped\|not killed\|gateway token is empty' tests`; consultant read | run, no match; read |
| The kill refusal without a terminal, `kill --exited`, `list --team`, `list --as` and the reset refusals are already tested | `tests/daemon_cli/kill_and_down.rs:5`, `tests/refs_and_list.rs:146,473-483`, `tests/daemon_cli/reset.rs:538` | read (consultant: `daemon_lifecycle.rs:72-87` is the loose stop refusal test) |
| `relay_files::write_token` also bails with `gateway token is empty` before writing (the guard is duplicated, so mutant 7 is equivalent for test 4) | `src/relay_files.rs:82-85` | read (developer BLOCKED m_8295, architect read) |
| `tests/daemon_cli/` files stay under 1,000 lines | `wc -l`: `daemon_lifecycle.rs` 414, `kill_and_down.rs` 332 | run |
| `PtyHarness::spawn` takes `&[String]` args including `--home`, `wait_for_text`, `send`, `wait_exit` | `tests/common/pty.rs:47-213`, `tests/daemon_cli/reset.rs:109-121` | read |

## Amendment 1

Mutant 7 of section 4 is redefined as above (equivalent for test 4, killed by `migrate_cli` test 14 only): the base has two empty-token guards (`gateway.rs:39` and `relay_files::write_token`). Nothing else changes.

## Review record

Models: architect and consultant Claude Sonnet 5.5 (fresh consultant session for the draft review); junior developer
OMP openai-codex/gpt-5.6-luna; tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 1.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the `daemon stop` refusal without a terminal already had a test with a loose `contains` assertion; the draft's new test 1 would have duplicated it | test 1 became the tightening of the existing assertion; counts 1396 |
| draft review | consultant | mutant 7 was said to fail exactly one test but `migrate_cli` test 14 also fails it; `layout.ensure()` creates the directories before the token read; line ranges off | text corrected; token test asserts the file only |
| draft review | consultant | a successful `kill --yes` printing nothing was unpinned | `assert_eq!(stdout(&forced), "")` added |
| implementation (BLOCKED m_8295) | developer | mutant 7 cannot be killed by the new test because `relay_files::write_token` has the same guard and text | amendment 1 (d8ed141): mutant 7 recorded as equivalent for test 4, killed by `migrate_cli` test 14 only |
| tester pass | tester | the PTY decline tests waited only for `Continue? [y/N]`, so rewording the prompt prefix passed (mutants 12 and 13 survived); the stop-decline test did not assert `running` | tests wait for the full prompt text and assert `running`; mutants 12 and 13 now fail them |
