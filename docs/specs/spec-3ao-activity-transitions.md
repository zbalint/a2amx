# Spec 3ao: activity notices, the daemon tells a watcher when a watched peer starts or stops working

## 0. Status

**LOCKED** (2026-10-26). Architect-directed, context `a2amx-activity-transitions-2026-10-25`. Implementation goes to `a2amx-developer` only. Builds on spec 2v (exit event), 2w (heartbeat) and 2y (activity).

**Owner decisions (2026-10-25):**
- A watcher is told when a watched peer goes from idle to working and from working to idle, with a threshold: debounce each direction with two separate durations, `working_after` and `idle_after`.
- Scope is state transitions only, from the daemon's own activity calculation, the same for every harness. No hooks, no extension changes, no screen content. Backlog F6 stays parked.
- The option is called `activity`. The watcher-level form is kept; `watch` stays a list of names.
- Two sessions that watch each other with `activity` on are rejected in team plans, and the docs warn about the loop for sessions started by hand.
- `idle_after` accepts the same range as `--heartbeat` (1 s to 24 h); the docs state the extra 10 s and recommend 30 s or more.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Baseline:** develop at `6d42ef6` (spec 3an implemented; the locking commit follows it and changes only this file): `env -u A2AMX_BIN -u NO_COLOR cargo test` 574 passed, 3 ignored, 0 failed; `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean. Line numbers in this spec are from before 3an and shifted; locate code by function name. Two unrelated tests flaked once each in other runs and pass alone: `tests/daemon.rs::exited_claude_session_never_reports_needs_input` and `tests/daemon_cli.rs::team_reset_reports_missing_and_exited_before_resetting_ready`; rerun a failing one alone once and record it as pre-existing.
**Public test seams:** the new pure module `activity` (`Tracker`), the team-plan validation through `team::` as `tests/team.rs` does, and the daemon through `Client` plus `a2amx messages` as `tests/daemon.rs` and `tests/teams.rs` do.

**Scope.** May edit exactly these files and no others.

- `src/activity.rs` (new), `src/lib.rs` (module declaration)
- `src/wire.rs` (`Request::NewSession.activity` and the `ActivityWatch` type), `src/cli.rs`, `src/main.rs` (flags, request building, the team plan text), `src/team.rs` (the `TeamActivity { working_after: Option<String>, idle_after: Option<String> }` struct with `deny_unknown_fields` like `TeamSession`, validation, the cycle check, the up-plan)
- `src/session.rs` (spec field and getter next to `heartbeat`), `src/daemon.rs` (validation next to the heartbeat checks, the watcher task, `session_working`), `src/messaging.rs` (subject constant, `parse_interval` label), `src/store.rs` (open-message lookup takes a subject)
- tests: `tests/activity.rs` (new), `tests/team.rs`, `tests/daemon.rs` (only for the mechanical `activity: None` additions), `tests/daemon_cli.rs`, `tests/wire.rs`, `tests/prop_messaging.rs` (lines 254, 259, 264 and 279 call `parse_interval`, so they change with its signature), and the mechanical `activity: None` addition to every existing `Request::NewSession`, `SessionSpec`, `NewOptions` and `TeamSession` literal found with `rg -n 'NewSession \{|SessionSpec \{|NewOptions \{|TeamSession \{' src tests`
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `docs/teams.md`, `docs/cli.md`, `a2amx.toml.example`

Does not touch: `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md` (the architect adds the module-table row), `docs/backlog.md`, `docs/roadmap.md`, locked specs, `src/delivery.rs`, `src/harness.rs`, `src/mcp.rs`, `extension/`, `scripts/`. No new dependency. Leave the diff uncommitted.

## 1. Why

A watcher today learns about a peer in two ways: a message when the peer exits (spec 2v), and a heartbeat digest after the watcher itself has been idle for the interval (spec 2w). An architect that dispatched work therefore learns that a developer finished only when something else wakes it. The daemon already computes each session's activity once a second. It can tell the watcher when a peer starts working and when it stops, through the ordinary queue and gates, with a debounce so the short pauses inside a turn do not produce messages.

Facts from the code that shape the design (each in the claims table):
- Activity is `busy` while a hold or a reset is active, `working` while the harness is not ready or its screen changed in the last 10 s (`ACTIVITY_QUIET`), `idle` otherwise. `busy` means "held or resetting", not "doing work".
- Because working ends only 10 s after the last output, "now idle" arrives at least `idle_after + 10 s` after the last output.
- A notice that reaches an idle watcher usually starts a turn in it. If the peer watches the watcher back with `activity` on, the two exchange notices forever, and every lap costs quota. Heartbeat avoids this by skipping when all peers are idle; this feature needs its own guard.

## 2. Decisions

- **D1.** New optional watcher setting `activity` with two optional durations, `working_after` and `idle_after`, each 1 s to 24 h in `s`, `m` or `h` (the `parse_interval` range). At least one must be set. It requires a non-empty `watch`, like `heartbeat`. Off by default.
- **D2.** Team file: `activity = { working_after = "2m", idle_after = "30s" }` on a session entry, with unknown keys rejected. CLI: `--activity-working-after 2m` and `--activity-idle-after 30s` on `a2amx new`; giving either turns the setting on. Wire: `Request::NewSession.activity: Option<ActivityWatch>` where `ActivityWatch { working_after: Option<u64>, idle_after: Option<u64> }` holds seconds, as `heartbeat: Option<u64>` does; serde `default` and `skip_serializing_if = "Option::is_none"` on every new field so old clients and the exact JSON pinned in `tests/wire.rs` keep working.
- **D3.** "Working" for this feature is the work part of the activity function and ignores holds: working = the harness is not ready, or the screen changed within `ACTIVITY_QUIET`; idle otherwise. A hold or a reset neither starts nor stops working. Reuse the existing code: split the ready and `last_change` part of `session_activity` (`src/daemon.rs`) into `fn session_working(session: &Arc<Session>) -> Option<bool>` (`None` when the session has exited) and have `session_activity` call it, with unchanged behavior.
- **D4.** A new pure module `src/activity.rs` owns the debounce:

  ```rust
  pub struct Thresholds { pub working_after: Option<Duration>, pub idle_after: Option<Duration> }
  pub struct Tracker { /* private */ }
  impl Tracker {
      pub fn new(thresholds: Thresholds) -> Self;
      /// Feed one observation. Returns the state to announce (`true` = working) when the settled state
      /// differs from the last announced or baseline state and that direction is enabled; keeps
      /// returning it on later calls until `announced` is called.
      pub fn observe(&mut self, now: Instant, working: bool) -> Option<bool>;
      /// The caller queued the announcement for `state`.
      pub fn announced(&mut self, state: bool);
  }
  ```

  Rules:
  - A state *settles* once it has held, without interruption, for its threshold (`working_after` for working, `idle_after` for idle). A direction with no threshold settles at once.
  - The settled state persists until the other state has settled; a state still inside its debounce does not unsettle the previous one.
  - There is one *reference* state: the last state the tracker settled and either announced, recorded as the baseline, or passed over silently because its direction is disabled. The first settled state is the baseline: the reference is set to it and nothing is announced, so a booting harness or a team starting up announces nothing.
  - `observe` returns `Some(s)` while the current settled state `s` differs from the reference and the direction of `s` is enabled; it keeps returning it until `announced(s)` sets the reference to `s`. If the settled state returns to the reference first, `observe` returns `None` (a coalesced flap). A settled state different from the reference whose direction is disabled sets the reference silently and returns `None`.
  - Consequence to document: with `working_after` unset, any output blip in an idle peer (including the echo of a delivered message) produces a "now idle" notice about `idle_after + 10 s` later.
- **D5.** The daemon starts one task per watcher that has `activity`, next to the heartbeat task in `start_session_tasks`, on the same one-second tick (`HEARTBEAT_POLL`). Each tick: take `live_watched_peers`, keep one `Tracker` per live peer, keyed by session id (create on first sight, drop when the peer is no longer live, so an exited peer is forgotten silently and the exit event covers it; a restarted name is a new session and gets a new baseline), feed each `session_working(peer)` every tick, and collect the peers whose `observe` returned `Some`. The task returns when the watcher exits and shutdown aborts it before the children, as for heartbeat.
- **D6.** At most one activity message is open (pending or delivering) per watcher. Every tick feeds every tracker (D5), whether or not a message is open, so the debounce clocks never skip. Then the task asks the store whether one is open and, if so, skips the insert and `announced`. Otherwise it inserts one message listing every collected change, and calls `announced` for each only after the insert succeeded. A full queue or a failed insert is logged and retried on the next tick with the then-current state. No watcher readiness check: the delivery gates deliver the message when the watcher can take it. A message waiting for a busy watcher keeps the content it had at insert time.
- **D7.** Message: from the system sender (`a2amx-daemon@<host>`, as exit events and heartbeats), subject `activity` (constant `messaging::ACTIVITY_SUBJECT`), body one header line and one line per peer, sorted by name:

  ```
  peer activity:
  - developer: working (was idle)
  - tester: idle (was working)
  ```

  `was` is the last announced or baseline state. No timestamps, no durations, no screen content.
- **D7a.** Plan text: the team plan (`a2amx team plan`, `main.rs` next to the `heartbeat: ` line) prints `  activity: working_after 2m, idle_after 30s` after the heartbeat line, omitting an unset direction (`  activity: idle_after 30s`); nothing when `activity` is off. `tests/team.rs` pins it next to the existing `"  heartbeat: 5s\n"` check.
- **D8.** Validation, in the same places and wording style as heartbeat (`src/daemon.rs` next to the heartbeat checks, `src/main.rs` for the CLI, `src/team.rs` for the team file): `activity` without a non-empty `watch` is an error; an `activity` with neither duration is an error; a duration outside 1 s to 24 h is an error naming the field. `messaging::parse_interval` gains a label parameter so its error says `invalid activity working_after "…"`; the heartbeat message stays byte-identical (the existing test pins it).
- **D9.** Team plans reject a cycle: for every session `W` that has `activity` and every name `P` in `W.watch`, there is an edge `P -> W` (notices flow from `P` to `W`). A directed cycle among those edges is an error. The check lives in `validate_sessions` in `src/team.rs`, after `resolve_references`, so it sees the team-prefixed names. The error names the sessions on the cycle in file order, starting from the first session in the file that is on a cycle: `activity notices would loop between a and b` for two, `activity notices would loop between a, b and c` for three or more. Names outside the file (another team's session, one started by hand) contribute no edge, so cycles through them are not detected; the docs say so. A watcher with `heartbeat` (not `activity`) in a loop is not a cycle under this rule; the heartbeat interval rate-limits it. A session started with the CLI is not covered; the docs say so.
- **D10.** Compatibility: additive. An older client or file without the key behaves as today; an older binary rejects the new team-file key (unknown keys are denied). Public interface per `AGENTS.md`: compatible, patch-level. Message database schema unchanged.
- **D11.** `store`'s open-heartbeat lookup becomes a lookup of an open system message by subject (`open_system_message(boot, session, subject)`), with the heartbeat caller passing `HEARTBEAT_SUBJECT`; no second near-copy.

## 3. Worked examples

`Tracker` with `working_after = 120 s`, `idle_after = 30 s`, observations once a second, `t` in seconds from the first observation:

| # | Observations | `observe` returns |
|---|---|---|
| 1 | idle for 40 s (peer idle at start) | `None` throughout (baseline idle recorded at t=30, silent) |
| 2 | after #1: working from t=40, still working at t=160 | `None` until t=160 (working held 120 s), then `Some(true)`; `Some(true)` again on each later call until `announced(true)` |
| 3 | after #2 and `announced(true)`: idle from t=200, still idle at t=230 | `None` until t=230, then `Some(false)` |
| 4 | after #3 and `announced(false)`: working t=300..t=350, idle again | `None` (working never held 120 s; idle settles back to the announced state, nothing to announce) |
| 5 | `working_after = None`, `idle_after = 30 s`: peer working from the start for 100 s, then idle for 35 s, then (after `announced(false)`) working for 50 s, then idle for 35 s | `None` for the first 100 s (baseline working, silent); `Some(false)` once idle has held 30 s; during the second working spell `None` (working settles at once, silently, and moves the reference to working); `Some(false)` again once the second idle has held 30 s |
| 6 | `working_after = 120 s`, `idle_after = None`: idle at start, then working for 130 s, then idle for 50 s, then working for 130 s again | baseline idle at once (silent); `Some(true)` when the first spell has held 120 s (then `announced(true)`); the idle spell returns `None` (idle settles at once, silently, and moves the reference to idle); `Some(true)` again when the second spell has held 120 s |
| 7 | `120 s` / `30 s`, reference working: idle settles and the caller does not call `announced` (slot open); working begins and has held 60 s | `Some(false)` on every call, including during the 60 s of working (the settled state is still idle) |
| 8 | after #7 (reference still working): working holds 120 s before `announced` is called | `None` (settled working equals the reference), so nothing is sent |

Team plan: `a: watch=[b], activity={idle_after="30s"}` and `b: watch=[a], activity={working_after="2m"}` is rejected; the same pair with `activity` on only one of them is accepted; the three-session ring `a: watch=[b]`, `b: watch=[c]`, `c: watch=[a]`, each with `activity`, is rejected with `activity notices would loop between a, b and c`.

End to end (slow test, in `tests/daemon_cli.rs` next to the heartbeat tests at about lines 1978-2173 and using their helpers): watcher `w` is a generic session running `printf '\033[?2004h'; cat` with delivery auto, so it accepts deliveries; it watches `p` with `working_after = 1s` and `idle_after = 1s`. `p` is a generic session running `printf '\033[?2004h'; sleep 15; while :; do printf x; sleep 1; done` (bracketed paste is what makes a generic session ready). A new session counts as working for its first 10 s (its change clock starts at creation), so the baseline is working and silent. The test waits with an eventually-style poll of up to 30 s for `a2amx messages` to show, in order, `- p: idle (was working)` (about 11 s in) and then `- p: working (was idle)` (about 16 s in), and that the message list for `w` has exactly those two entries once `p` keeps printing. A third session that `w` does not watch produces nothing for `w`.

## 4. Tests

Failing test first, one behavior at a time, literals for expected values.

- `tests/activity.rs`: one test per worked-example row 1 to 8, using literal `Instant` offsets from one base `Instant`.
- `tests/team.rs`: the team-plan examples; `activity` without `watch`; neither duration; out-of-range duration; unknown key under `activity`.
- `tests/wire.rs`: `NewSession` with `activity: None` serializes exactly as before; a frame with the field round-trips.
- `tests/daemon_cli.rs`: the end-to-end example, and a second test modeled on `held_delivery_keeps_at_most_one_heartbeat_open` that a held watcher keeps at most one open activity message. `session_activity` behavior is unchanged (existing tests keep passing).
- `tests/team.rs`: the plan text of D7a.
- `src/messaging.rs` unit test: `parse_interval` error text for heartbeat unchanged, and the new label form.

## 5. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| `Activity` is `Busy` for a hold or a reset, `Working` for not-ready or changed within 10 s, `Idle` otherwise | `src/daemon.rs` `session_activity`, `ACTIVITY_QUIET` | read |
| The heartbeat task polls once a second, uses `live_watched_peers`, and returns when the watcher exits | `src/daemon.rs` `heartbeat`, `HEARTBEAT_POLL` | read |
| Heartbeat validation, request building and plan text live in these places (anchors at base c8df501; name functions, not lines, once 3an lands) | `src/daemon.rs:345-361`, `src/main.rs:374-423, 455, 537-603, 939-982`, `src/cli.rs:65`, `src/wire.rs:131`, `src/team.rs:29, 631, 727-735`, `src/session.rs:47, 66, 370, 426` | `rg -n heartbeat` |
| `open_system_heartbeat` already filters `subject` and `sender_session='daemon'`, and has one production caller | `src/store.rs:304, 811`; `src/daemon.rs:784` | `rg -n open_system_heartbeat src tests` |
| `parse_interval` callers | `src/main.rs:382, 554, 964`, `src/team.rs:734`, `src/daemon.rs:343`, tests `tests/prop_messaging.rs:254-279`, `src/messaging.rs:590, 606` | `rg -n parse_interval src tests` |
| A generic session reads as ready only with bracketed paste on | `src/harness.rs:53-54`; `tests/broker.rs:1530` prints `\033[?2004h` | read (consultant) |
| `resolve_references` prefixes watch names before `validate_sessions` runs; self-watch is rejected earlier | `src/team.rs:437-452, 674, 724` | read (consultant) |
| A notice that starts a turn in its recipient can start a loop between mutual watchers | `docs/architecture.md` (heartbeat skips when all peers idle) | read; the loop itself is an inference, covered by D9 |
| Sessions do not survive a daemon restart, so in-memory trackers are enough | `src/session.rs` (PTY children of the daemon) | inference, not re-checked |
| OMP sessions always read as ready, so "working" for OMP means recent output only | `src/harness.rs` readiness, spec 2y | inference from the spec; the developer checks with `a2amx list` on an idle OMP session and reports whether its footer redraws |

## 6. Acceptance

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Contract tests that pass on the old code: the existing heartbeat tests and the `parse_interval` tests (the signature changes; their expectations do not). The end-to-end daemon test is slow (about 15 to 20 s); a failure there is rerun once before it is reported.

## 7. Out of scope

Per-peer thresholds, a richer `watch`, hooks or extension feeds, screen lines in messages (backlog F6 and F12), a rate cap, any change to the heartbeat's behavior, and notices about `busy`.
