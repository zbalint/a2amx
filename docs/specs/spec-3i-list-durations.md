# Spec 3i: UPTIME and IN-STATE columns in `a2amx list`

## 0. Status

**LOCKED** (2026-10-05, pre-lock gate run against develop at `368b952`, the commit that accepted
spec 3h; gate notes in section 9). The owner asked whether `a2amx list` could show how long a
session has been running, how long it has been idle and how long it has been working, and left the
choice of which of those have value to the architect. Decisions D1 to D9 are in section 2 and are
the architect's, marked **(architect)** where they are a choice rather than a consequence.

**Baseline:** develop at `368b952`; `cargo test --test wire --test refs_and_list --test team
--test daemon_cli` passes (9, 27, 21 and 52 tests, with the clean environment). **Location and
branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `a2amx-list-durations`. Public test seams: `a2amx::session::observe_activity` and
`a2amx::messaging::display_duration` (pure), `Request::List` through `Client` against a real daemon
(`tests/daemon.rs`), and the `a2amx list` binary (`tests/refs_and_list.rs`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs`, `src/session.rs`, `src/daemon.rs`, `src/messaging.rs`, `src/main.rs`
- `tests/wire.rs`, `tests/team.rs`, `tests/daemon.rs`, `tests/refs_and_list.rs`,
  `tests/attach_cli.rs`, `tests/daemon_cli.rs`, `tests/common/mod.rs` (one shared helper,
  section 7)
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: the session picker (`PICKER_HEADERS`, `picker_value_rows`), the status bar
(`src/status.rs`), `list_agents` and the MCP surface (`src/mcp.rs`), `StatusInfo`, every other
test file, every other spec document, `AGENTS.md`. No new dependency. Do not commit, stage or
merge: leave the diff uncommitted in the working tree.

## 1. Why

`a2amx list` shows what a session is doing now (`ACTIVITY`) but not for how long. "working for
40 minutes" suggests a stuck agent and "idle for two hours" a forgotten one; the architect and the
owner both need that at a glance, and they also need to know how long a session has existed (a
session that was reset or restarted is young). The daemon keeps neither value today: `activity` is
computed on demand from the screen (`src/daemon.rs`, `session_activity`).

## 2. Decisions

- **D1. (architect)** Add two columns to `a2amx list` and `a2amx list --details`: `UPTIME` (how
  long the session has existed) and `IN-STATE` (how long its current `ACTIVITY` has lasted).
  Cumulative idle and cumulative working totals are not added: they need running accounting for the
  whole session lifetime and tell the user nothing the two columns do not. The picker, the status
  bar, `list_agents` and the MCP surface are unchanged.
- **D2.** `UPTIME` is the time since the session was spawned, for a running session; an exited
  session shows `-`. `IN-STATE` is the time the session's current `ACTIVITY` (`idle`, `working` or
  `busy`) has lasted; `-` when `ACTIVITY` is `-`.
- **D3.** Both cells use `display_duration`, which moves unchanged from `src/daemon.rs` to
  `src/messaging.rs` as a `pub fn` (that module owns rendering; `main` cannot reach a daemon
  helper cleanly) and which `src/daemon.rs` then imports. It already floors: `Ns` below one minute
  with a minimum of `1s`, `Nm` below one hour, otherwise `Nh`.
- **D4.** Tracking. `Session` gains a plain field `started: Instant` (set when the session is spawned; never
  changes, so reading it takes no lock) and `session::State` gains
  `activity_since: Option<(Activity, Instant)>` (initially `None`). A new pure
  `pub fn observe_activity(slot: &mut Option<(Activity, Instant)>, current: Activity, now: Instant)
  -> Duration` in `src/session.rs`: if the slot holds the same activity, it returns `now` minus the
  stored instant (saturating) and leaves the slot; otherwise it stores `(current, now)` and returns
  zero. `Session` gets `pub(crate) fn note_activity(&self, current: Activity, now: Instant) ->
  Duration` (takes the state lock, calls `observe_activity` on `activity_since`) and `pub(crate)
  fn started(&self) -> Instant` (a plain field read).
- **D5.** A per-session sampler task, spawned with the other per-session tasks in session
  creation (`src/daemon.rs`, beside `observe_exit`), ticks every `ACTIVITY_POLL` (new const, one
  second, like `HEARTBEAT_POLL`), calls `session_activity`, and when it returns `Some(activity)`
  calls `note_activity`. It returns when the session has exited. Leave a `// shortcut:` comment:
  one-second resolution and one screen read per session per second; a transition hook in the
  output path if that cost matters.
- **D6.** The `Request::List` handler also calls `note_activity` with the activity it just
  computed (before it takes the state lock), so the age it reports is exact at list time, not up to
  a second stale.
- **D7.** `SessionSummary` (`src/wire.rs`) gains, after `activity`, two fields:
  `#[serde(default, skip_serializing_if = "Option::is_none")] pub uptime_secs: Option<u64>` and the
  same for `activity_secs: Option<u64>`. A session is running when the `exit_code` read from the state guard the
  summary is built under is `None`; then `uptime_secs` is `Some(started elapsed seconds)` and
  `activity_secs` is the whole seconds returned by `note_activity` when its activity is `Some`
  (else `None`). When that `exit_code` is `Some`, both are `None`, even if `session_activity` returned
  `Some` an instant earlier (an exit between the two reads). With both `None` the JSON is unchanged.
- **D8.** Column order in the table: `ID NAME HARNESS STATE UPTIME ACTIVITY IN-STATE ATTACHED
  PENDING HELD QUOTA SIZE` and, with `--details`, `... SIZE CWD COMMAND`.
- **D9.** Nothing is persisted; the clocks start with the daemon (sessions do not survive it).

## 3. `src/wire.rs`

Add the two fields of D7 to `SessionSummary` after `activity`. No other wire change.

## 4. `src/session.rs`

`Session` gains the field `started: Instant` and `State` gains `activity_since: Option<(Activity,
Instant)>`, initialised in `Session::spawn` (`started: Instant::now()` on the session,
`activity_since: None` next to `last_change`). Add
`observe_activity`, `Session::note_activity` and `Session::started` per D4; import
`crate::wire::Activity` as needed.

## 5. `src/daemon.rs`

`display_duration` is removed from this file and imported from `crate::messaging` (its callers at
about lines 87 to 106 are unchanged). Add `const ACTIVITY_POLL: Duration = Duration::from_secs(1);`
beside `ACTIVITY_QUIET`. Spawn the sampler (D5) with the other per-session tasks. In the
`Request::List` mapping, after `let activity = session_activity(session);` and **before** the existing
`let state = session.lock();` (the guard is held while the summary is built, and `note_activity`
takes the same non-reentrant lock), compute
`activity_secs = activity.map(|a| session.note_activity(a, Instant::now()).as_secs())`; then, when
building the summary, set `uptime_secs` from `session.started()` (no lock) and both fields to `None`
when `state.exit_code` is `Some` (D7). `StatusInfo` and every other
caller of `session_activity` are unchanged.

## 6. `src/main.rs`

`session_value_rows` stays at 10 cells (the picker uses it unchanged). `format_session_table` builds
its 12-cell and 14-cell rows from it, inserting `UPTIME` after `STATE` (index 4) and `IN-STATE` after
`ACTIVITY` (then index 6) from `session.uptime_secs` and `session.activity_secs`, each
`display_duration(Duration::from_secs(n))` (`a2amx::messaging::display_duration`) or `-`. `SESSION_HEADERS`
becomes `[&str; 12]` and `DETAIL_HEADERS` `[&str; 14]` in the D8 order. `PICKER_HEADERS` and
`picker_value_rows` do not change.

## 7. Tests (failing first, expected values are literals)

Pure:

1. `observe_activity` with `t0 = Instant::now()`: a `None` slot, `Idle`, now `t0` returns 0s and the
   slot holds `(Idle, t0)`; the same slot at `t0 + 5s` with `Idle` returns 5s and is unchanged; at
   `t0 + 7s` with `Working` returns 0s and the slot holds `(Working, t0 + 7s)`; at `t0 + 9s` with
   `Working` returns 2s.
2. `a2amx::messaging::display_duration`: 0s gives `1s`, 1s `1s`, 59s `59s`, 60s `1m`, 3599s `59m`, 3600s `1h`,
   90000s `25h`.

`tests/wire.rs`: the two `SessionSummary` literals gain `uptime_secs: None, activity_secs: None`
and their exact-JSON assertions are unchanged; add one case where both are `Some` (5 and 3) that
round-trips and whose JSON contains `"uptime_secs":5,"activity_secs":3`.

`tests/daemon.rs` (real daemon, in the style of the existing `NewSession` tests): create a generic
`sh -c "sleep 30"` session (its activity is `working` throughout: it never becomes ready). `Request::List`
right after: `uptime_secs` is `Some(0)` or `Some(1)`. After a real `tokio::time::sleep` of 2.2s:
`uptime_secs` is in `2..=5` and `activity_secs` is `Some` and in `2..=5`. Kill the session, wait until
`exit_code` is set: both fields are `None`.

CLI (`tests/refs_and_list.rs`): the exact header line for the existing two-column fixture becomes
`ID  NAME        HARNESS  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE`; the
running row's cells, split on whitespace, are `s1 agent-plan generic running <u> working <i> no 0 - -
80x24` where `<u>` and `<i>` each match `^[0-9]+[smh]$`. An exited session shows `-` in both new cells.

**Existing assertions that change.** The new columns shift positions and add varying text, so every
existing assertion that depends on the `list` table layout is updated. Found with
`rg -n "running  |ACTIVITY|\.nth\(" tests` and checked by hand; these are the known ones, and the
report must list each one changed: `tests/team.rs:445-446` (a `contains` of the row prefix);
`tests/attach_cli.rs:505-509`, `:518`, `:667-670`, `:793`, `:801` (header, row and details
assertions); `tests/refs_and_list.rs:299-302`; `tests/daemon_cli.rs:769`, `:777`, `:779` (cell
indices: `ATTACHED` moves from 5 to 7 and `SIZE` from 9 to 11; `STATE` at 3 is unchanged).
`tests/attach_cli.rs:339` is the picker header and does not change. Add one shared helper to
`tests/common/mod.rs`, `pub fn list_cells(listing: &str, id: &str) -> Vec<&str>` (the whitespace-split
cells of the row that starts with `id`), and use it in the files above instead of repeating the
split; the two duration cells are asserted with the `^[0-9]+[smh]$` shape (no `regex` crate: check
digits then a trailing `s`, `m` or `h`), never with a literal value.

## 8. Documentation

- `README.md` (the paragraph that describes the `list` columns, near line 154): `UPTIME` is how
  long the session has existed (`-` when exited) and `IN-STATE` how long its current `ACTIVITY` has
  lasted (`-` when ACTIVITY is `-`); both are floored, shown as `Ns`, `Nm` or `Nh`, at one-second
  sampling resolution, and start counting when the daemon starts.
- `docs/architecture.md` (the "Human controls" bullet near line 439): the two columns and the
  one-second sampler.
- `docs/backlog.md`: Closed row `C13`, `UPTIME and IN-STATE columns in a2amx list`, `spec 3i`.

## 9. Pre-lock gate notes

- Baseline run: the four test files above pass on `368b952` (counts in section 0).
- `SessionSummary {` literals: `src/daemon.rs:1429`, `tests/wire.rs:176` and `:530`,
  `tests/team.rs:16` (grep over `src` and `tests`); `tests/quota.rs`, `tests/delivery.rs` only read
  summaries. `tests/team.rs:16` is the fourth literal and gains the two `None` fields (the file is in
  scope for it and for lines 445-446).
- List-layout assertions: `rg -n "running  |ACTIVITY|\.nth\(" tests` yields the sites in section 7;
  `tests/hook.rs:250` indexes the message table and is unaffected.
- Column widths: `UPTIME` (6) and `IN-STATE` (8) are at least as wide as every value (`display_duration` yields
  at most 3 characters below 100 hours, and a longer value only widens its own column), so the header widths decide and the existing rows keep their other widths.
  Worked header for the refs_and_list fixture: `ID`(2) `NAME` padded to 10 (`agent-plan`) `HARNESS`(7)
  `STATE` padded to 7 (`running`) `UPTIME`(6) `ACTIVITY`(8) `IN-STATE`(8) `ATTACHED`(8) `PENDING`(7)
  `HELD`(4) `QUOTA`(5) `SIZE`; cells are separated by two spaces and the last
  cell is not padded, as `format_row` does.
- Worked example for D6 against D4/D5: a fresh session's first sampler tick runs immediately (the
  tokio interval's first tick), so within about a second `activity_since` is `Some`; a `list` in that
  window calls `note_activity` itself and returns 0, so `IN-STATE` shows `1s` (`display_duration` floors 0 up to
  `1s`), never `-` for a running session with `ACTIVITY` set.
- Acceptance commands need the new code and run after implementation.

## 10. Out of scope

- Cumulative idle or working totals, a history of activity changes.
- `UPTIME` or `IN-STATE` in the picker, the status bar, `list_agents`, the MCP surface or `StatusInfo`.
- A transition hook in the output path (the sampler is the accepted corner, D5), persisting clocks.
- OMP quota via `omp usage --json` and any other roadmap item.

## 11. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list, the exact
bare `cargo test` result with its disposition if the `A2AMX_BIN` contamination fails
`generic_sessions_get_no_a2amx_bin`, and a manual transcript on an isolated daemon (temp state dir):
`a2amx new --detach --name demo -- sh -c "sleep 30"`, `a2amx list` twice a few seconds apart showing
`UPTIME` and `IN-STATE` growing, `a2amx kill demo --yes`, and `a2amx list` showing `-` in both cells.

## Amendment 1 (2026-10-05): exited rows come from a natural exit, not from `Kill`

The developer reported BLOCKED (`m_429`): section 7 (the `tests/daemon.rs` case) and section 11 (the
manual transcript) say to kill a running session and then read its exited row, but
`Request::Kill` on a running session removes it from the registry (`src/daemon.rs`, the `Kill` arm,
about lines 1465 to 1473), so `List` has no row to show, and `tests/daemon.rs`
(`kill_removes_running_session_and_shutdown_disconnects_attachments`) and `tests/refs_and_list.rs`
(`kill_by_name_removes_the_named_session`) require that removal. Verified real. **Ruling:** the
`Kill` contract is unchanged; an exited row is observed after a natural exit.

- Section 7, `tests/daemon.rs`: create the generic session as `sh -c "sleep 5"` (not `sleep 30`). The
  assertions at about 0s and after the 2.2s sleep are unchanged (the session is still running at
  2.2s, so `activity_secs` is `Some` and in `2..=5`). Replace "Kill the session, wait until
  `exit_code` is set" by "wait (polling `Request::List`, at most 10 seconds) until the session's
  `exit_code` is `Some`", then both fields are `None`.
- Section 11, manual transcript: use `a2amx new --detach --name demo -- sh -c "sleep 12"`; `a2amx
  list` twice a few seconds apart showing `UPTIME` and `IN-STATE` growing; after the session has
  exited naturally, `a2amx list` shows `exited(0)` with `-` in both new cells; then `a2amx kill demo`
  (an exited session is removed without prompting).
- Nothing else changes: scope, decisions and the other tests are as in `5d21b2e`.

Gate re-run for the amendment: `rg -n "kill|Kill" docs/specs/spec-3i-list-durations.md` shows no
other place that expects an exited row from a kill; the 5-second sleep leaves 2.8 seconds between
the last running assertion (2.2s) and the exit.
