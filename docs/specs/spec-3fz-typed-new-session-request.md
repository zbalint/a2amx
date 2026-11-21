# Spec 3fz: a typed new-session request on `Client`, and one launch-validation function

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: `Client::create_session`
through the scripted fake control daemon of `tests/client_requests.rs`; the binary suites judge the launch paths unchanged.

Scope (edit): `src/client/requests.rs`, `src/client.rs` (one more name in its existing `pub use` line),
`src/commands/launch.rs`, `tests/client_requests.rs`, `tests/daemon_cli/heartbeat_and_activity.rs` (one new test). `src/commands/team.rs` is read only: `launch::create_session` keeps its signature, so `start_planned_session` needs no edit.

Does not touch: every other file under `src/` (`src/wire.rs`, `src/daemon.rs`, `src/team/validate.rs`, `src/commands/team.rs`,
`src/commands/team/dry_run.rs` included), every other test, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, `CHANGELOG.md`, `docs/` other than this spec.

## 1. Why

3fy gave `Client` a typed method per control request, but `create_session(request: Request)` still takes the raw
protocol enum. The envelope knowledge ("a confined launch is `NewSession` wrapped in `Confine`") stays in
`launch.rs`, and `Client` has to guess `confined` with `matches!` on whatever variant it is handed; the tests prove the
looseness by passing `Request::List`. The consultant's design read of 3fy named this the first thing to fix. Separately,
`launch.rs` states the watch, activity and heartbeat rules twice: `run_new` checks them before it connects (a test pins
that a bad `new --activity-working-after` fails without a daemon), and `create_session` checks them again for every
session, so the team path gets them only there. Two copies with different check orders are a trap for the next edit. The daemon enforces the same rule on its own (`src/daemon.rs:500-525`, text prefixed `session X:`) and `src/team/validate.rs:15,228` checks team files; both stay as they are, and the daemon stays the enforcing point, so the client checks only give earlier errors and this change creates no trust-boundary path. There are now three near-identical field lists (`NewOptions`, `NewSessionRequest`, `Request::NewSession`); the team-up spec is where `NewOptions` should build or collapse into `NewSessionRequest`.

Rejected, with the reason: changing `Request::NewSession` into a struct-payload variant in `wire` (the JSON stays the
same but about 30 test and daemon match sites change; a wire refactor of its own); a `Launch` type that also owns
`wired_command`, `cwd` resolution and the confine plan shared by `team up`, `--dry-run` and `validate_team_session`
(that is the team-up spec, which needs this one's result; stating its scope here would make two decisions in one spec);
leaving `create_session(Request)` as an escape hatch beside the typed method (it would keep the leak).

Prior lessons, stated here because specs cannot cite memory: a typed method that still takes the raw enum leaves
the invariant at the call site and the tests loose; a refactor that merges two copies of a rule must name which check
order survives, because the two copies disagree.

## 2. Target shape

`src/client/requests.rs` gains a mirror of the `Request::NewSession` fields (the one place that field list is
copied; it is built in exactly one place and consumed in exactly one) and `create_session` changes signature:

```rust
pub struct NewSessionRequest {
    pub argv: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub reset: Vec<String>,
    pub control_from: Vec<String>,
    pub watch: Vec<String>,
    pub activity: Option<ActivityWatch>,
    pub heartbeat: Option<u64>,
    pub name: Option<String>,
    pub harness: Harness,
    pub deliver: Option<Deliver>,
    pub team: Option<TeamScope>,
    pub role: Option<String>,
    pub remote: Option<RemoteMode>,
}

pub async fn create_session(
    &mut self,
    session: NewSessionRequest,
    confine: Option<ConfineRequest>,
) -> anyhow::Result<String>;
```

`create_session` first destructures the struct with all 16 fields named (no `..`), then writes the `Request::NewSession { .. }` literal by field name (no `From` impl, no partial defaults), so a field added to either side stops compiling or trips the unused-variable lint under `-D warnings`; it then wraps it as
`Request::Confine { confine, request: Box::new(..) }` when `confine` is `Some`, sets `confined = confine.is_some()`, and
keeps the reply semantics of 3fy's table row unchanged (`Created` gives the id; `Error` containing
`invalid control request` while `confined` gives `the running daemon does not support --confine; restart it`; any other
`Error` gives its message; any other reply is `unexpected daemon response: ...`). `NewSessionRequest` derives
`Debug, Clone, PartialEq` (every field type already derives them, because `Request` itself derives `Debug, Clone, PartialEq`). `src/client.rs` re-exports it next to `IssuedLeaseCode`.

`src/commands/launch.rs`:

```rust
/// The watch-related launch rules: a heartbeat and an activity watch both need a non-empty watch.
pub(super) fn validate_watch_options(
    watch: &[String],
    activity: Option<&ActivityWatch>,
    heartbeat: Option<u64>,
) -> anyhow::Result<()>;
```

Body: if `heartbeat` is `Some(seconds)`: the error `heartbeat needs a non-empty watch` when `watch` is empty, then
`parse_interval(&format!("{seconds}s"), "heartbeat")?`; then, if `activity` is `Some`, `activity.validate(!watch.is_empty())?`.
This is the order `create_session` has today. `run_new` calls it once after it parses the three values (replacing its two
inline checks at launch.rs:95-97 and 103-105); `create_session` calls it in place of its two inline checks (371-379).
`create_session` builds a `NewSessionRequest` (and the `Option<ConfineRequest>` it already receives in `NewOptions.confine`)
instead of `Request::NewSession` and the `match confine` wrapper, calls `client.create_session(session, confine)`, and the
`use crate::wire::Request;` import goes away. `NewOptions` is unchanged. `start_planned_session` in `team.rs` only gets its
`create_session` call re-checked (its `NewOptions` is the same); no edit is expected there.

## 3. Behaviour changes

1. When `new` is given both an invalid activity (no watch) and an invalid heartbeat (no watch) before connecting, the
   message was `activity needs a non-empty watch` (activity first in `run_new`) and is now
   `heartbeat needs a non-empty watch` (heartbeat first). Single-fault inputs, including every case a test pins, are
   unchanged. A heartbeat that fails to parse still fails in `run_new`'s parse step before this function runs.
   1b. `new` with an activity but no watch plus an unparseable heartbeat (both invalid, before connecting) said `activity needs a non-empty watch` (`run_new` checked activity before parsing the heartbeat) and now says `invalid heartbeat ...` (the heartbeat parses first, then the shared function checks). No test pins it.
2. `create_session`'s new signature is internal; no output, exit code or wire byte changes (the request sent is
   field-for-field the same `Request`).

## 4. Tests

1. `tests/client_requests.rs`: rewrite `create_session_confined_version_skew_unconfined_and_success` for the new signature.
   A helper `fn new_session(name: &str) -> NewSessionRequest` builds a fixed value (argv `["sleep", "60"]`, cols 100, rows 30,
   cwd `Some("/tmp/work")`, env `[("K","V")]`, reset `["a"]`, control_from `["c"]`, watch `["w"]`, activity
   `Some(ActivityWatch{working_after: Some(120), idle_after: Some(600)})`, heartbeat `Some(30)`, name `Some(name)`, harness
   `Harness::Claude`, deliver `Some(..)` (a non-default `Deliver`), team `Some(TeamScope{..})`, role `Some("r")`, remote `Some(..)` (a `RemoteMode`); cols 100 and rows 30; every value is non-default so that no field-copy mutant can produce it). Rows, one test each (the file's current
   spawn/connect/assert shape):
   a. `create_session(new_session("n1"), None)`, reply `Created{session:"s9"}` gives `Ok("s9")`; the recorded request is
      exactly `Request::NewSession { argv: ["sleep","60"], cols: 100, rows: 30, cwd: Some("/tmp/work"), env: [("K","V")],
      reset: ["a"], control_from: ["c"], watch: ["w"], activity: Some(..), heartbeat: Some(30), name: Some("n1"),
      harness: Harness::Claude, deliver: Some(..), team: Some(..), role: Some("r"), remote: Some(..) }` (this pins every field copy).
   b. with `Some(ConfineRequest { grants: vec!["git".into()], write: vec!["/tmp/w".into()], ..Default::default() })` (every `ConfineRequest` field is `serde(default)` and it derives `PartialEq`, so the fake's JSON round trip compares equal) the recorded request is exactly `Request::Confine { confine: <that value>,
      request: Box::new(<the request of row a>) }`, reply `Created` gives `Ok`.
   c. confined, reply `Error{message:"invalid control request: unknown variant"}` gives Err
      `the running daemon does not support --confine; restart it`.
   d. unconfined, the same reply gives Err `invalid control request: unknown variant`.
   e. confined, reply `Error{message:"other"}` gives Err `other`; unconfined, reply `Ok` gives Err `unexpected daemon response: Ok`.
2. `tests/daemon_cli/heartbeat_and_activity.rs`: new `new_heartbeat_validates_before_connecting_to_daemon`, the shape of
   `new_activity_validates_before_connecting_to_daemon` (`:661`, no daemon started): `new --detach --harness generic --heartbeat 30m -- sleep 1`
   without `--watch` fails with stderr containing `heartbeat needs a non-empty watch`. It passes on the base code (a contract test, no red phase) and is the only test that pins the client-side heartbeat check, because the daemon repeats the rule (`src/daemon.rs:511-517`) and the daemon-backed tests assert with `contains`.
3. No test for `validate_watch_options` itself (it is private); the tests of rows 1 and 2 and the mutation check judge it.

## 5. Out of scope

`wired_command`'s eight parameters and its `#[allow(clippy::too_many_arguments)]`; the three `origin.dir.join(..)` cwd
computations and the duplicated confine preflight in `validate_team_session` and `dry_run_start` (the team-up spec);
`NewOptions` fields and their two builders; `src/wire.rs`; converting `mcp.rs`, `statusline.rs`, `hook.rs`, `gateway.rs`;
the `Failed` code and `String` versus `&str` argument questions of 3fy; the test-scaffolding helper for
`tests/client_requests.rs` (a separate hardening item; this spec only edits the one test).

## 6. Acceptance

- `cargo test --test client_requests` passes (the five rows above are listed); `cargo test --no-fail-fast`: all pass.
  Baseline at the base commit `dfb986a` (architect, 2026-11-21): 66 suites, 1408 passed, 0 failed, 3 ignored; after:
  66 suites; 1413 passed (rows a to e replace one test: net +4, plus the new heartbeat test: +1); the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'Request::NewSession|Request::Confine' src/commands` prints nothing; `rg -n 'use crate::wire::Request' src/commands/launch.rs` prints nothing;
  `rg -n 'needs a non-empty watch' src/commands/launch.rs` prints exactly one match (inside `validate_watch_options`);
  `rg -n 'activity.validate' src/commands` prints exactly one match.
- `git status --short` lists only the five files of section 0 (`src/client.rs`, `src/client/requests.rs`, `src/commands/launch.rs`, `tests/client_requests.rs`, `tests/daemon_cli/heartbeat_and_activity.rs`); `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- The binary suites pass unchanged: `tests/daemon_cli` (heartbeat and activity, kill and down), `tests/team`, `tests/teams`,
  `tests/confine`, `tests/confine_show.rs`, `tests/auto_confine`, `tests/attach_cli`. Known flaky tests (rerun a failure once alone and record it as pre-existing if it passes):
  `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error`,
  `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode`,
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session`,
  `tests/presence` `viewers::viewer_attach_keeps_input_read_only_while_sizing_session`,
  `tests/attach_cli/viewer.rs::viewer_team_click_acts_on_release_and_keeps_viewer_mode`,
  `tests/attach_cli/attach_session.rs::client_file_prefix_reaches_attached_session`,
  `tests/discord/gateway_queue.rs::gateway_queue_reads_heartbeats_while_consumer_is_slow`,
  `tests/daemon_cli/reset.rs::team_reset_file_resets_sessions_in_file_order`.
- Mutation check (developer, assigned checkout, one line each, restored by sha256 before and after, non-empty edits, each killed by a named test, the killing test listed per mutant):
  in `requests.rs`: drop one field copy of `NewSessionRequest` into the request, one mutant per field class (`cols` always 80; `reset` always empty;
  `remote` always `None`; `harness: Harness::default()`; `deliver` always `None`; `team` always `None`) (row a); wrap `Confine` with `ConfineRequest::default()` always (row b);
  `confined` computed as `false` (row c); the restart text (row c); remove the `confined &&` guard (row d).
  In `launch.rs`: swap the check order of `validate_watch_options` (killed by none by design: record it as a survivor of the single-fault
  tests; it is behaviour change 1); `validate_watch_options` skipping the `activity.validate` call (killed by
  `new_activity_validates_before_connecting_to_daemon` only: `new_activity_validates_interval_and_requires_watch` runs with a daemon and the daemon repeats the rule);
  skipping the heartbeat check (killed by the new `new_heartbeat_validates_before_connecting_to_daemon` only, for the same reason); `create_session` in `launch.rs`
  passing `None` instead of `confine` (killed by `tests/daemon/confine.rs` or a `tests/confine_show.rs` test: the developer names it).
  `git diff --stat` shows only the developer's own diff afterwards.
- The tester repeats the mutation pass independently; the architect reviews the diff; a consultant reads the design (a refactor).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| `launch::create_session` (`launch.rs:437`) is the only caller of `Client::create_session` in `src`, besides `tests/client_requests.rs` (5 calls in one test) | `rg -n '\.create_session\(' src tests` | run 2026-11-21 (`src/daemon.rs:459` and `:2804-2805` are `runtime.create_session`, a different function) |
| `NewOptions` is built at `launch.rs:145-168` (`run_new`) and `team.rs:360-383` (`start_planned_session`); both call `commands::launch::create_session` | read | read |
| `run_new` checks activity and heartbeat before connecting (`launch.rs:91-105`, before `Client::connect` at 106); a test pins the pre-connect failure | `tests/daemon_cli/heartbeat_and_activity.rs:661` | read |
| The check orders differ: `run_new` is activity, then heartbeat; `create_session` is heartbeat, then activity | `launch.rs:95-105` and `371-379` | read |
| No test pins the both-invalid message | `tests/daemon_cli/heartbeat_and_activity.rs:495-560,655-680` (single-fault cases only) | read |
| `Request::NewSession` has exactly the 16 fields of section 2 | `src/wire.rs:158-185` | read |
| Every `NewSession` field type derives `Debug, Clone, PartialEq`, since `Request` derives them and tests compare requests with `assert_eq!` | `src/wire.rs` `Request`, `tests/client_requests.rs` | read; the developer confirms by compiling (a missing derive is a spec amendment, not a silent addition) |
| `Request::NewSession`/`Request::Confine` are named in 42 files of `src` and `tests`, so a wire struct refactor is far larger | `rg -c 'Request::NewSession\|Request::Confine' src tests` | run |
| `parse_interval(&format!("{seconds}s"), "heartbeat")` accepts exactly what `run_new`'s parse accepted (it re-parses a validated value) | `launch.rs:98-102,375`, `src/messaging.rs` `parse_interval` | to check by the developer |
| `src/commands/launch.rs` is 438 lines; `requests.rs` is 137; the edit grows `requests.rs` by about 60 | `wc -l` | run |
