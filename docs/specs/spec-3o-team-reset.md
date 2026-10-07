# Spec 3o: `a2amx team reset`

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `9dc715a`; gate notes in section 9;
consultant review `m_764`).
Follow-up idea 3 from the consultant triage `m_733` (context `a2amx-followups`), approved by the
owner ("all the ideas sound good, work them out"). Architect decisions are in section 2. Builds on the
existing `a2amx reset` (spec 2s) and the `team down` command shape (spec 2o). Independent of specs 3m
and 3n.

**Baseline:** develop at `9dc715a`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-team-reset`.
Public test seam: the `a2amx` binary against a temp-state daemon with fake sessions, in the style of
the reset tests in `tests/daemon_cli.rs` (about lines 270 to 470).

**Scope.** May edit exactly these files and no others.

- `src/team.rs`, `src/cli.rs`, `src/main.rs`
- `tests/team.rs`, `tests/daemon_cli.rs`
- `README.md`, `docs/backlog.md`

Does not touch: `src/daemon.rs`, `src/wire.rs`, `src/session.rs`, `src/messaging.rs`,
`docs/architecture.md`, every other test file, every spec document, `AGENTS.md`, the owner's
gitignored `a2amx.toml`. No new dependency, no wire change, no daemon change. Do not commit, stage or
merge: leave the diff uncommitted in the working tree.

## 1. Why

Between tasks the owner wants a whole team back to clean contexts. Today that is one `a2amx reset
NAME` per session, or a `reset_session` call per peer. A team command is the same loop `team down` has,
with the safety the operation needs: a reset clears a live conversation, and for the attached
session that conversation is the human's own work.

## 2. Decisions

- **D1. Command.** `a2amx team reset [--file FILE | NAME...] [--except NAME]... [--include-attached] [--yes]`, shaped
  like `team down` (`--file` conflicts with `NAME` arguments; default file `a2amx.toml` in the invoking
  directory; names are full session names as in `team down`, including any team prefix). Without
  names it resets every session named in the file, in file order. **Attached sessions are skipped by
  default** when they come from the file (a human is attached; a reset would clear the conversation he is
  looking at, AGENTS.md: never send input that destroys human work): `skipped NAME ID (attached; use
  --include-attached or name it)`. A session given explicitly as a NAME argument, or any attached
  session with `--include-attached`, is reset. The selection is a pure function in `src/team.rs`,
  `pub fn reset_targets(names: &[String], explicit: bool, except: &[String], include_attached: bool,
  existing: &[SessionSummary]) -> Vec<ResetTarget>` with `ResetTarget { name, state }` and `state` one of
  `Reset { id }`, `Missing`, `Exited { id }`, `SkippedAttached { id }`, in input order (the `names` with
  `except` removed). `--except` (repeatable, full
  session name) removes names from the selection; an `--except` name that is not selected is
  ignored.
- **D2. Per-session behavior.** Resolve each selected name against one `Request::List`. A name
  with no session prints `no session NAME` on stdout (no failure, like `team down`); an exited
  session prints `skipped NAME ID (exited)`; a running one gets one `Request::Reset` using the admin
  connection (the CLI is always allowed; `control_from` plays no role). Success prints `reset NAME ID:
  N step(s)` (new wording with the name; `a2amx reset` keeps its own `reset ID: N step(s)` line). A `Failed` or `Error` response prints `failed NAME:
  CODE: MESSAGE` on stderr and the loop continues with the next session; there is no retry and no
  waiting loop. The daemon's own refusals (a draft in the composer, a composer not ready, a reset
  already running) arrive this way, so a session that is mid-turn or has a human's draft is left
  alone and reported, never forced.
- **D3. Confirmation.** Before the first reset, unless `--yes`, the command prints the sessions it
  will reset (one `NAME ID` per line, `(attached)` after a session with a human attached) and asks
  `Reset N session(s)? This clears their conversations. [y/N] ` through the existing `confirm`
  helper (`main.rs` about line 282), exactly as `daemon stop` does; a non-terminal stdin without
  `--yes` makes `confirm` return the error `refusing to reset: N running session(s); pass --yes to reset
  them`, which `main` prints as `a2amx: refusing to reset: ...` with exit 1.
  A `n` answer prints `not reset` and exits 0 without sending anything. With nothing selected and
  running, nothing is asked.
- **D4. Order.** File order (or the order of the NAME arguments), one session at a time. The CLI
  resets one session at a time (the daemon's reset guard is per session, so this is this command's own
  choice, for ordered output); each step takes at least 1.5 seconds (`RESET_SETTLE`), so a five-agent
  team takes roughly ten to twenty seconds when every session is ready, and a session that accepts the
  first step but never becomes ready can hold its turn for up to the 30 second step timeout;
  each session prints its line as soon as it finishes. No
  dependency ordering (it would be orchestration, a stated non-goal) and no parallelism.
- **D5. Exit status.** 0 when every selected running session reset or was legitimately skipped
  (missing, exited); 1 when at least one `failed` line was printed, after the loop has tried every
  session, with the error `one or more team sessions could not be reset`.
- **D6. Caller not detected.** The command does not detect that the caller is one of the selected agents.
  An agent running it from a tool call is mid-turn, so the daemon's gate refuses its own session
  (`not_ready`), but an idle human-driven session could reset itself; the state directory is the trust
  boundary, as documented. The README says so and recommends `--except` for the session that runs the
  command.

## 3. `src/cli.rs`

`TeamAction` gains `Reset { file: Option<PathBuf>, names: Vec<String>, except: Vec<String>,
include_attached: bool, yes: bool }` with `#[arg(long, conflicts_with = "names")] file`,
`#[arg(long)] except`, `#[arg(long)] include_attached`, `#[arg(long)] yes` and
the doc comment `Reset the team's sessions (clear their conversations).`

## 4. `src/main.rs`

`dispatch` gains the `TeamAction::Reset` arm calling a new `run_team_reset(home, file, names, except,
yes)`. It reuses `read_team` and `request_sessions` (as `run_team_down` does) and the `confirm` helper,
and prints through `write_stdout`/`write_stderr`. It uses `team::reset_targets` for the selection. The request and response handling of `run_reset`
(`Response::Reset { steps }`, `Failed`, `Error`) is shared: factor it into `async fn reset_one(client,
session_id) -> anyhow::Result<usize>` returning the steps or the error (`code: message` for `Failed`,
the message for `Error`), used by `run_reset` and `run_team_reset` rather than copied (`run_reset`'s
output stays byte-identical: `reset ID: N step(s)` and the same errors).

## 5. Tests (failing first, expected values are literals)

`tests/team.rs`, pure: 0. `reset_targets` returns, for names `[a, b, c, d, e]` with `d` excepted and
existing sessions (a running, b exited, c running and attached, e absent): `[Reset{a-id}, Exited{b-id},
SkippedAttached{c-id}, Missing(e)]` in that order with `include_attached = false`, and c as
`Reset{c-id}` with `include_attached = true`; with `explicit = true` an attached session is `Reset`
without the flag; `except` of an unselected name changes nothing.

`tests/daemon_cli.rs`, binary against a temp daemon with the same fake sessions the existing reset
tests use (a ready fake with a configured `--reset`, a not-ready one, an exited one):

1. `team reset --file F --yes` over a file with two ready sessions prints `reset NAME ID: 1 step`
   twice in file order and exits 0; both fakes recorded the reset input.
2. A selection with a missing name, an exited session and one ready session prints `no session X`,
   `skipped Y ID (exited)` and the reset line, exit 0.
3. A not-ready session among ready ones: the others are reset, a `failed NAME: not_ready: ...` line
   is printed on stderr (`not_ready`, as asserted at `tests/daemon_cli.rs` about line 358),
   exit 1, and the ready sessions after it in the file were still reset (the loop continues).
4. `--except NAME` leaves that session untouched (its fake recorded nothing); an `--except` name not
   in the selection is accepted.
5. Without `--yes` and with stdin not a terminal (the test harness default): exit 1, stderr
   `a2amx: refusing to reset: ...`, and nothing reset. The interactive `y` and `n` paths are not tested (no PTY seam for
   `confirm` here); say so in the report.
6. `team reset NAME...` with explicit full names behaves like the file form; `--file` with a name
   argument is rejected by clap.
7. `a2amx reset NAME` output is byte-identical to before (the existing tests in the file cover it and
   stay unchanged).

## 6. Documentation

- `README.md` "Teams": the `team reset` command, its confirmation, `--except`, `--include-attached`
  and why attached sessions are skipped by default, the exit status, that busy or drafting sessions
  are skipped and reported, the file-order rule, the worst-case timing, and D6.
- `docs/backlog.md`: Closed row `C19`, `team reset`, `spec 3o`.

## 7. Out of scope

Dependency ordering, parallel resets, a retry or wait option, `--only`, detecting the caller,
a daemon-side batch request, and an MCP tool (agent-initiated team reset stays impossible).

## 8. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list.

## 9. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` passes on `9dc715a` (404 passed, 27 suites, 3
  ignored; tree unchanged since).
- Consultant review `m_764`: two wording errors fixed (B1 per-session reset guard and honest timing, B2
  success wording and `reset_one` returning a value); `confirm` refusal text is printed by `main` with the
  `a2amx: ` prefix (D3, test 5). The owner-decision point was resolved by the architect on the owner's
  standing instruction to decide: option (b), attached sessions are skipped unless named or
  `--include-attached` (D2), because AGENTS.md forbids input that destroys human work; the selection
  is a pure function so the attached rule is testable without a PTY (test 0).
- Verified facts: refusal codes `not_ready`, `draft_present`, `exited` and the per-session busy guard
  (`session.rs` about 359 and 662, `tests/daemon_cli.rs` about 292-331, 358, 377, 568); a refusal is
  immediate; seams for tests 1-6 exist (`run`, `fake_reset_composer`, exited and not-ready generic
  fixtures, `a2amx screen` to read the recorded input).
- Rules stated twice, diffed: attached rule (D2, section 3, test 0, README bullet); confirm text (D3, test 5).
- Acceptance commands need the new code and run after implementation.
