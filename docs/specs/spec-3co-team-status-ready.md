# Spec 3co: `team status` says when a running session has not reached readiness

## 0. Status

LOCKED. Code, tests and docs; an additive wire field.

Scope (files that may be edited): `src/wire.rs`, `src/daemon.rs`, `src/main.rs`, `docs/teams.md`,
`tests/wire.rs`, `tests/team.rs`, `tests/attach_cli.rs`.
Does not touch: every other `src/` file, `CHANGELOG.md`, `docs/backlog.md`, `docs/roadmap.md`,
`docs/architecture.md`, any other test file, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base the commit that
contains this spec. Context id: `a2amx-team-ready-2026-11-04`. Public test seams: the `a2amx`
binary (`team status`) as in `tests/team.rs:1411`, and the JSON of `Response::Sessions`
(`tests/wire.rs`).

## 1. Why

Backlog F19 (roadmap 0.2.0): `team status` prints `running` when only the name and team match
(`docs/teams.md:191`), so a session stopped at a startup dialog (for example Claude Code's
workspace-trust prompt) looks fine and an operator or architect finds out only when a message
never arrives. The daemon already knows: a session tracks whether its harness screen has ever
matched the harness's readiness check (`Session::starting`, `src/session.rs:543-547`, set by
`observe_ready`, `:531-540`), and the attach status frame already carries it
(`src/daemon.rs:2599, 2609`). The session list that `team status` reads
(`SessionSummary`, `src/wire.rs:355`) does not.

Owner decision (2026-11-04): show it as NOTE text on the `running` row, using the same screen
readiness check the reset gate uses. No new STATE value and no new column. Backlog I16 (a send
that stayed in the composer) is not diagnosed and stays out.

Decision recorded here (the architect's, listed to the owner): the note uses the existing latch
(`starting`: the daemon has not yet seen the harness at a ready screen since the session
started), not a live check. Reasons: the live check `harness::ready` is false for any Generic
program without bracketed paste, so Generic rows would have to be excluded anyway; the other
live signal in the summary, `hold_reason`, also carries a human draft or an unsubmitted envelope,
which are not "not ready" in F19's sense (a session stuck at a startup dialog). The cost, stated
in the docs: a session that was ready earlier and later shows a dialog is not marked. The
latch is sampled only when something looks (a list request, the status frame, a delivery check);
`list_sessions` calls `session_activity` before it builds each summary (`src/daemon.rs:2174`),
which calls `observe_ready`, so the list request that `team status` makes samples every session.

Rejected: a new STATE value `waiting` (changes what scripts mean by `running`); a READY column
(widens the table); deriving readiness in the client from `activity` (activity is idle, working or
busy and does not carry it).

## 2. `src/wire.rs`

Add to `SessionSummary` a field `starting: bool` with
`#[serde(default, skip_serializing_if = "is_default")]`, placed after `activity_secs`, documented
as: true while a session of a harness with a readiness check has not yet been ready since it
started; false for exited sessions and for Generic sessions. An older daemon or client ignores
it (additive; the missing field reads as false).

## 3. `src/daemon.rs`

In the list handler closure (`:2173-2207`) add `let starting = session.starting();` after the
`quota` line (`:2180`) and before `let state = session.lock();` (`:2181`), and set `starting` in
the `SessionSummary { .. }` literal (`:2187`) from that variable. It must run after
`session_activity` (`:2174`), or a session that is ready right now would not have latched yet, and
before the lock: the literal runs while `state` holds the session's `std::sync::Mutex`, and
`Session::starting` calls `exit_code()`, which locks it again (`src/session.rs:543-545, 1098-1100`),
so calling it inside the literal deadlocks every non-empty list (Amendment 1).
`Session::starting` already returns false for an exited session
(`src/session.rs:543-547` checks `exit_code().is_none()`) and for Generic. No other change.

## 4. `src/main.rs`

In `run_team_status` (`:1089`), in the `Running` arm (`:1103-1110`), when the matching summary has
`starting == true` the NOTE is `<id> (not ready: starting)` instead of `<id>`. STATE stays
`running`, CONFINE as before, the exit code unchanged (a not-ready session is not a conflict;
`team up` would still proceed). Look the summary up once (the arm already finds it for the
confinement mode) rather than searching twice.

## 5. Docs: `docs/teams.md`

In the `team status` paragraph (`:187-196`) add: a `running` row's NOTE is the session id, and
`<id> (not ready: starting)` while the daemon has not yet seen the session's harness at a ready
screen since it started (for example a Claude Code session stopped at a startup dialog; the
check runs when the daemon lists sessions, so a screen that was ready only between two looks is
not remembered); Generic sessions have no readiness check and never show it; a session that was
ready earlier and later shows a dialog is not marked. Keep the sentence that `running` means only that name and team match, and add that
the note is the only readiness signal in this table.

## 6. Tests

- `tests/wire.rs`: the three existing `SessionSummary` literals (`:318, :351, :715`) gain
  `starting: false`; add one round trip that a summary with `starting: true` serializes the key
  `"starting":true` and one that a summary with `starting: false` omits the key; add that JSON
  without the key deserializes to `false`.
- `tests/attach_cli.rs:85` and `tests/team.rs:403` (the `summary` helper): add `starting: false`
  (mechanical). Do not add a parameter to the `tests/team.rs` helper (it has many callers); set
  the field in a new test with struct update syntax if one is needed.
- `tests/team.rs`, next to `status_reports_running_and_missing_without_changing_sessions`
  (`:1411`) and in the style of `:1459`: the team file has no `harness` key (`TeamSession` is
  `deny_unknown_fields`, `src/team.rs:25-54`; the harness is inferred from the command's
  basename, `src/harness.rs:51-62`), so start the sessions directly with
  `a2amx new --detach --harness claude --name worker -- sh -c 'sleep 30'` (a Claude-harness
  session that never shows Claude's ready screen) and `a2amx new --detach --name plain -- cat`
  (Generic), both ungrouped (`new` has no team flag, `src/cli.rs:46-60`); write a team file with
  no `team` key naming both (`command = ["cat"]` is enough, `team::inspect` matches by name and
  team scope only, `src/team.rs:231-266`) and run `a2amx team status --file <file>`. Assert the
  whole rows, newline included (the style of `:1441`): `worker - running off <id> (not ready:
  starting)\n` and `plain - running off <id>\n`, both STATE `running`, exit 0. Put the Generic
  row's assertion in its own test function or before the Claude row's, so the Generic assertion
  can pass on the old code. If the helper `tests/fixtures/fake_claude.sh` (it draws Claude's ready
  screen; read its header for the environment it needs) can start a third, ready, Claude-harness
  session in a few lines, assert its row has the bare id; that kills mutant (e). If it cannot,
  report mutant (e) as a survivor.

## 7. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| `Session::starting` locks the session mutex (through `exit_code()`), and the summary literal runs while `let state = session.lock()` is held, so `starting()` must be read before that lock | `src/session.rs:543-545, 1098-1100, 1114-1119`, `src/daemon.rs:2181-2207` | read (developer's BLOCKED m_5507, architect confirmed) |
| `Session::starting` is true only for a non-Generic, not-exited session that has not latched ready | `src/session.rs:543-547` | read |
| `list_sessions` calls `session_activity` (which calls `observe_ready`) at `:2174`, before the summary literal at `:2187`; `observe_ready` also runs in the delivery check (`src/delivery.rs:148`) and the status frame | `src/daemon.rs:2174, 2511-2513` | read (consultant), architect read the literal |
| A team file has no `harness` key; the harness comes from the command's basename | `src/team.rs:25-54`, `src/harness.rs:51-62` | read (consultant) |
| `team::inspect` matches a running session by name and team scope only | `src/team.rs:231-266` | read (consultant) |
| `a2amx new` has no team flag, so its session is ungrouped | `src/cli.rs:46-60` | read (consultant) |
| A Claude-harness `sh -c 'sleep 30'` never satisfies `claude_ready` (it needs bracketed paste, a visible cursor and not scrolled) | `src/harness.rs:184-205` | read (consultant) |
| `observe_ready` latches `ready_latched` when `harness::ready` is true | `src/session.rs:531-540` | read |
| The status frame already carries `starting` | `src/daemon.rs:2599, 2609` | read |
| `SessionSummary` has no readiness field and is built in one place | `src/wire.rs:355`, `src/daemon.rs:2187`; grep `SessionSummary {` finds also tests/attach_cli.rs:85, tests/wire.rs:318,351,715, tests/team.rs:403 | grep |
| The `Running` arm already finds the summary for the confinement mode | `src/main.rs:1103-1110` | read |
| `team status` rows today: Running note is the id; the conflict-note split on "; " is only in the Conflict arm | `src/main.rs:1103-1122`, `docs/teams.md:187-196` | read |
| No doc lists the `SessionSummary` fields | `rg 'activity_secs\|uptime_secs' docs README.md` finds nothing | grep |
| Existing `team status` tests use Generic commands, for which `starting` is false, so their rows do not change | `tests/team.rs:1411, 1459` | to be read by the developer |

## 8. Out of scope

A new STATE value or column; I16; readiness after the first ready screen; the `explain` command
(spec 3cp); the MCP `list_agents` result (it does not read `starting`); `team up` and `team
reset` behavior; `CHANGELOG.md` and the backlog row (the architect adds them).

## 9. Acceptance

1. `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   pass. Judge: the architect reruns them.
2. Red phase: the Claude-row assertion fails before the code (the note is missing); the `starting`
   wire tests do not compile on the old shape (the field does not exist). The Generic-row
   assertion passes on the old code when it is in its own test function; name which tests are
   contract tests. The developer records the output.
3. With the built binary and a temp `A2AMX_HOME` (start a daemon in it): `a2amx new --detach
   --harness claude --name worker -- sh -c 'sleep 30'`, a file holding
   `[[session]]\nname = "worker"\ncommand = ["claude"]` (no `team` key), then `a2amx team status
   --file <file>` shows `running` and `(not ready: starting)`. Judge: the tester.
4. Mutation check by the developer, in the checkout, restored by checksum: (a) set
   `starting: false` in the daemon literal: the new team test fails; (b) print the note for
   every Running row: the Generic-row assertion fails; (c) print it for a `starting` session but
   change STATE to something else: the STATE assertion fails; (d) drop `skip_serializing_if`:
   the omit-the-key wire test fails; (e) `starting: session.harness() != Generic` (ignores the
   latch): killed only if the ready-Claude session of section 6 exists; otherwise a recorded
   survivor.
5. Public interface: the `Response::Sessions` JSON gains an optional `starting` key; the
   changelog line is the architect's.
6. `git status --short` shows only files in scope. Scratch only under `target/scratch/3co/`.

## Amendment 1: read `starting()` before the session lock (BLOCKED report m_5507, 2026-11-04)

The first draft of section 3 told the developer to call `session.starting()` inside the
`SessionSummary` literal. The literal runs while `state = session.lock()` is held
(`src/daemon.rs:2181`) and `Session::starting` locks the same non-reentrant mutex through
`exit_code()`, so that call deadlocks. Section 3 now places the call between `:2180` and `:2181`.
Neither the architect's draft nor the pre-lock consultant review (which reported `:2174-2187` as
read) noticed that a guard is live across the literal. The ordering requirement is unchanged:
after `session_activity`, before the summary is built. No other section changes; scope is
unchanged.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5 (fresh); developer and tester OMP
with `openai-codex/gpt-5.6-luna` (advisor model not exposed by the harness). BLOCKED/amend cycles: 1.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | The test setup and acceptance step used a `harness` key that team files do not have (`deny_unknown_fields`); ungrouped matching was assumed without reading `team::inspect` | Sessions are started with `new --harness claude` and an ungrouped team file via `--file` |
| Draft review | consultant | The latch differs from a live check (a session ready earlier that later shows a dialog is not marked); whole-row assertions needed; anchors; red-phase wording; the latch is sampled by `list_sessions` | Decision and limit recorded in the spec and `docs/teams.md`; listed to the owner |
| Implementation | developer (BLOCKED) | Section 3 put `session.starting()` inside the summary literal while the session lock is held, and `starting()` locks again: a deadlock of every non-empty list. The architect and the consultant missed it | Amendment 1: read before the lock, after `session_activity` |
| Developer report | architect | Mutant (e) (ignore the latch) survived because the ready-Claude fixture was judged impractical | Architect added `status_leaves_a_ready_claude_session_unmarked` (the fixture needs three environment variables); the mutant now fails it |
| Tester pass | tester | The pass ended after the static review (the harness ran out of tool access): no real-binary transcript and no mutants | Recorded as not run; the architect ran the deadlock mutant (it hangs) and mutant (e) and relied on the developer's mutants (a) to (d) |
