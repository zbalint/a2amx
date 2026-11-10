# Spec 3dr: `team up` waits for Enter after printing the lease code

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-live-test-fixes-2026-11-10`.

Scope (files that may be edited): `src/main.rs` (`run_team_up` and the two new functions `issue_lease_code` and `wait_for_enter`), `tests/teams.rs`, `docs/remote-messaging.md`, `docs/teams.md`.

Does not touch: the daemon, the wire protocol, `src/cli.rs`, the lease code itself, `--detach`, the attach client, `CHANGELOG.md` (the architect writes it), any other test file.

Public test seam: the `a2amx` binary run in a PTY (`tests/common/pty.rs`, `PtyHarness`) against a real confined daemon, as `tests/teams.rs` already does without a PTY.

## 1. Why

`a2amx team up` starts the sessions, prints the remote-messaging lease code when a session has `remote` set, and then attaches to the session the team file marks `attach = true` (or the first one). The attach switches the terminal to the alternate screen (`\x1b[?1049h`, `src/main.rs:2382`), so the code is hidden while attached and only comes back after detaching. The first live Discord test (2026-11-10) lost the code this way and had to run `a2amx team up --detach` a second time to get a new one.

Wanted (owner, 2026-11-10): `team up` stops after printing the code and waits for a keystroke before it attaches. The architect widened this to every case where lease output was written (a refused lease request prints a warning that the attach would hide too).

Alternatives rejected: showing the code again inside the attach client (a new wire message and status-line state, much larger; the status-line state is a separate spec, 3dv); waiting for a single key without Enter (needs raw terminal mode for a one-shot prompt; Enter reuses the line read `confirm` already does).

## 2. `src/main.rs`

Anchors are in the tree at commit `dbc2233`.

`run_team_up` writes the lease output in the block `if remote_requested { ... }` (lines 1771-1815): the `Ok(Response::LeaseCode { .. })` arm prints the code (line 1774), and four other paths write a warning to standard error instead (daemon error, unexpected response, request failure, and no team name). It attaches at the end under `if !detach && stdin_is_terminal() && let Some(session) = attached_id` (lines 1816-1821).

2.1. The wait applies whenever `remote_requested` is true, whatever the lease output was: the code, or a warning that explains why there is no code. The owner needs to read both before the attach hides them. There is no new flag variable.

2.2. Extract lines 1771-1815 (the whole `if remote_requested { ... }` block, unchanged in behaviour and in output text) into one function `issue_lease_code(client: &mut Client, remote_team: Option<String>) -> anyhow::Result<()>` in `src/main.rs`, called as `if remote_requested { issue_lease_code(&mut client, remote_team).await?; }`. Reason: it keeps `run_team_up`'s cognitive complexity from growing when the wait is added (the consultant's hand count puts it near 25 now; the Sonar limit is 30) and gives the block one name. Do not change the messages.

2.3. In the attach block (lines 1816-1821), when `remote_requested` is true, call a second function `wait_for_enter() -> anyhow::Result<()>` before `run_attachment(...)`. It writes the line `press Enter to attach to the session` to standard error with no newline, then reads one line from standard input on a blocking thread (`tokio::task::spawn_blocking`, the same pattern as `confirm`, lines 546-560). `confirm` itself cannot be reused: it refuses a non-terminal input and parses `y`/`yes`. The content of the line is ignored; an end of input (Ctrl-D) also continues; a read error is returned as the command's error (the sessions keep running, as after any failure after start).

2.4. After the wait, read the terminal size again (`terminal_size_with_default()`) and pass that size to `run_attachment`, because the size read at line 1746 can be stale after a pause for a human. Use the new values only for the attach after a wait; without `remote_requested` the code path is unchanged.

2.5. The wait never happens with `--detach`, with a non-terminal standard input, with `--dry-run` (it returns at the `if dry_run {` block, line 1739), or when no session would attach. A team without `remote` set attaches immediately, as before. Ctrl-C at the prompt ends `team up` and leaves the sessions running; `a2amx attach NAME` attaches later.

## 3. Documentation

3.1. `docs/remote-messaging.md`, section "Turn it on for agents and start the lease" (after the paragraph that starts "Post that 12-hex code"): on a terminal `team up` pauses after the lease output with `press Enter to attach to the session`, so the code stays on screen (the attach switches to the alternate screen and hides it); Enter attaches; Ctrl-C at the prompt ends `team up` and leaves the sessions running (`a2amx attach NAME` attaches later); with `--detach` it does not pause or attach, and re-running `a2amx team up --detach` prints a new code and leaves running sessions alone.

3.2. `docs/teams.md` line 167-168: after "`--detach` suppresses attachment, and nonterminal invocation never attaches." add one sentence: when a session in the team has `remote` set, `team up` waits for Enter before attaching.

## 4. Out of scope

- A single keypress without Enter; a timeout on the wait; showing the code inside the attach client or status line (3dv).
- Changing what the lease code message says, or its validity time.
- The other items from the 2026-11-10 live test (background gateway, pairing warning, list output, grace period).
- `CHANGELOG.md` and `a2amx.toml`.

## 5. Acceptance

All commands run in the main checkout with `env -u A2AMX_BIN -u NO_COLOR` as `AGENTS.md` describes.

1. New test in `tests/teams.rs` (red first: it fails on the current code because `team up` attaches at once). Start a confined daemon as the existing remote test at `tests/teams.rs:85-135` does (skip when `bwrap` is missing, as that test does). Team file: `remote = "send"`, one confined session, `attach = true`, command `sh -c "printf SESSION-MARK-9f3a; sleep 30"`. The marker must not be a substring of anything `team up` itself prints (session and team names appear in its output). Run `a2amx team up --file team.toml` in a `PtyHarness` (`spawn_with_env_in`; `--home` in the arguments, `HOME` and `XDG_RUNTIME_DIR` in the environment slice, as `run_binary` sets them). Then: `wait_for_text("press Enter to attach to the session")` first, then assert the screen contains the lease line `post this code in the control channel within` and does not contain the marker (absence is checked only after the prompt is on screen); send `\r`; `wait_for_text` the marker. End the test as `tests/team.rs:2508-2510` does: send Ctrl-B then `d` (bytes 2 and `d`), wait for `[detached from`, `wait_exit` with a bounded timeout and exit code 0, then `daemon.shutdown()`. Command: `cargo test --test teams`.
2. Second new test, same fixture: a team file with `remote = "send"` and no `team` name. `team up` prints `remote messaging is set but the file has no team name; no lease can start` (a warning, no code), and the PTY then also waits for Enter: the prompt appears, the marker does not, and after `\r` the marker appears. Red on the current code.
3. Third new test: the same remote team with `--detach` in a PTY exits with code 0 under `wait_exit` with a 10 s bound (a mutated binary that blocks on the read fails by timeout instead of hanging), prints the lease code, and the screen never contains the prompt. Contract test: it passes on the old code.
4. Contract test already present: `tests/team.rs:2480` (`terminal_attach_honors_file_selection_flag_first_and_detach_override`) runs `team up` on a PTY without `remote` and must keep passing; it is the check that a team without `remote` attaches at once. The developer runs it and names it in the report; it is not edited.
5. The other PTY users of `team up` (`tests/auto_confine.rs:512, 576, 638, 695`) set no `remote` and must keep passing.
6. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
7. `git diff` shows only the files in section 0.
8. Mutation check (developer, assigned checkout, restoring each file by checksum; each run under `timeout 120`): remove the `wait_for_enter` call (test 1 and 2 fail); call it also when `remote_requested` is false (the test named in item 4 fails; the developer names the test that fails); call it with `--detach` (test 3 fails); skip the size re-read (no test needs to fail; the developer reports it as a surviving mutant, see section 7). Name the failing test for each.
9. Complexity: the repo has no local cognitive-complexity tool (no `complexipy` in `scripts/`); the judge is the SonarQube scan the architect runs with `scripts/prepush-sonar.sh` (S3776, limit 30 for `run_team_up`). The developer reports a hand count of `run_team_up` before and after; the architect confirms with the scan.
10. Known flaky tests: none known in `tests/teams.rs`; a failure that does not reproduce on one rerun is recorded as pre-existing with its output.

## 6. Claims table

| Claim | Where | Verified |
| --- | --- | --- |
| The lease code prints in the `LeaseCode` arm, warnings in the other arms | `src/main.rs:1773-1810` | read |
| The attach is under `!detach && stdin_is_terminal() && attached_id` | `src/main.rs:1816-1821` | read |
| `--dry-run` returns before sessions start | `src/main.rs:1739-1745` | read |
| `confirm` reads one line on `spawn_blocking` and refuses a non-terminal stdin | `src/main.rs:546-562` | read |
| `tests/teams.rs` already runs `team up --detach` with remote through a confined daemon and parses the code | `tests/teams.rs:85-135` | read |
| `PtyHarness` has `spawn_with_env_in(args, home, cwd, environment)`, `send`, `wait_for_text`, `screen_text`, `expect_screen_lacks`; `run_binary` in `tests/teams.rs:25-35` sets `HOME` and `XDG_RUNTIME_DIR`, which the PTY test must also pass | `tests/common/pty.rs:59,144,176,194,303`, `tests/teams.rs:25-35` | read |
| No existing test asserts the order of the lease output against the attach | `grep -n "post this code" tests/*.rs` finds `tests/relay.rs:1335` (the pairing prompt, not `team up`) and `tests/teams.rs:130,229` (non-terminal runs only) | grep |

| Existing PTY users of `team up`, none with `remote` | `tests/team.rs:2480-2512`, `tests/auto_confine.rs:512, 576, 638, 695` | read (consultant) |
| The attach switches to the alternate screen | `src/main.rs:2382` (`\x1b[?1049h`) | read (consultant) |
| `PtyHarness::wait_exit` exists | `tests/common/pty.rs:213` | read |
| The no-team path prints a warning to stderr | `src/main.rs:1809-1814` | read |

## 7. Known accepted gaps

The terminal size re-read after the wait (section 2.4) has no test: resizing a PTY during the pause is not worth a fixture. It is a deliberate shortcut and the mutation that removes it is expected to survive.

## Review record

Models: architect and spec author Claude Sonnet 5.5; consultant (one pre-lock review) Claude, model not reported; developer and tester openai-codex/gpt-5.6-luna (planning and working, advisor slot not exposed).

BLOCKED/amend cycles: 0. Fix rounds after the first implementation: 2 (both on the tests).

| found at | by | problem | resolution |
|---|---|---|---|
| draft review | consultant | the wait applied only when a lease code printed, so a refused lease or a missing team name (warnings the attach hides) still scrolled away | the wait applies whenever `remote` is set (section 2.1) |
| draft review | consultant | the claim that complexity stays unchanged was false; the block to extract was not named | `issue_lease_code` extraction named in scope and section 2.2 |
| draft review | consultant | existing PTY tests of `team up` (`tests/team.rs:2480`, `tests/auto_confine.rs`) were not mentioned; test 1 needed an order of checks, an unambiguous marker, an end-of-test pattern and a bounded exit | sections 5 items 1 to 5 |
| draft review | consultant | the attach switches to the alternate screen, it does not clear the screen; the terminal size can be stale after a pause | wording fixed; size re-read added (section 2.4) |
| tester pass | tester | the mutant "`wait_for_enter` returns at once" survived the main test: the absence of the marker was asserted before an instant attach could draw it | fix round 1: a 1.5 s settle loop before Enter in both wait tests; the mutant now fails both |
| implementation | developer advisor | the settle helper used a blocking `std::thread::sleep` in an async test | fix round 2: async helper with `tokio::time::sleep` |
| architect review | architect | none found in the diff; own mutants (inverted condition, dropped `remote messaging for` line, silent no-team branch) were killed by the named tests | none |

Architect checks on the final snapshot: `git diff --check`, fmt, clippy `-D warnings`, `cargo deny check`, `scripts/prepush-fast.sh`, full `cargo test --no-fail-fast` (52 suites passed; two environmental failures in the confined architect session: `tests/gateway_up.rs::sandbox_hides_state_host_and_limits_writes_to_gateway` creates a temp directory under the read-only `$HOME`, and `tests/human_inbound.rs::queued_human_message_rechecks_recipient_eligibility_before_delivery` fails creating a confined session with "Read-only file system"; both reproduce alone and neither touches `team up`; CI runs unconfined and is the judge; recorded as backlog I24).
