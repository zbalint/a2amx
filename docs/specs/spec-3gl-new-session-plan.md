# Spec 3gl: `a2amx new` plans before it connects, and one function checks an interval in seconds (S5e)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file (after 3gk is pushed). Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the `a2amx new` command through the existing CLI tests (`tests/team/parse_and_guards.rs:184`, `tests/confine_show.rs:339`, `tests/daemon_cli/heartbeat_and_activity.rs:503`, `tests/daemon/session_creation.rs`, `tests/attach_cli/`) and the library function `a2amx::messaging::interval_from_seconds` through `tests/prop_messaging.rs`.

Scope (edit): `src/commands/launch.rs`, `src/messaging.rs`, `src/wire.rs`, `src/daemon/launch.rs`, `src/team.rs` (visibility of one function), `tests/prop_messaging.rs`.

Does not touch: every other file, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the backlog when committing.

## 1. Why

`run_new` (`src/commands/launch.rs:57-197`, 140 lines) does eight jobs in one function: destructure the clap command, de-duplicate the grants, check that `--confine-*` flags come with `--confine`, validate the grants, parse `--remote`, `--activity-*` and `--heartbeat`, validate the watch options, connect to the daemon and pick a free generated name, build the `ConfineRequest`, create the session, and finally print the name and attach or detach. The pure checks (everything before the connection) cannot be read apart from the I/O, and the order that decides which error a user sees first is buried in the middle of the function. Splitting along that seam gives a pure `plan_new` and a small async `run_new`.

Separately, a heartbeat or an activity window held as a number of seconds is validated by formatting it back to text and parsing the text again: `format!("{seconds}s")` followed by `parse_interval` at three places (`src/commands/launch.rs:46`, `src/daemon/launch.rs:137`, `src/wire.rs:133`), a detour whose only purpose is the 1 s to 24 h range check and the error text. One function states the range check on the number.

Target shape:

```rust
// src/messaging.rs
/// The range check of `parse_interval` for a value already in seconds; the error text is the one `parse_interval` gives for `"{seconds}s"`.
pub fn interval_from_seconds(seconds: u64, label: &str) -> anyhow::Result<Duration>;

// src/commands/launch.rs
/// What `a2amx new` decided from its flags before it touches the daemon.
struct NewPlan {
    remote: Option<RemoteMode>,
    activity: Option<ActivityWatch>,
    heartbeat: Option<u64>,
    confine: Option<ConfineRequest>,   // flags only; relative paths are resolved against the cwd in `run_new`
}
fn plan_new(command: Command) -> anyhow::Result<NewPlan>;   // pure: no I/O, no clock, no daemon
```

`plan_new` takes the whole `Command` (the consultant's attack on a 12-field `NewFlags`, a single-use parameter object that would also have forced a borrow of `watch`, which `create_session` needs afterwards): it does the `let Command::New { .. } = command else { bail!("expected new session options") }` itself (that was the first check of `run_new`, so the order holds), runs the checks and returns a `NewPlan` that carries the checked values (`remote`, `activity`, `heartbeat`, `confine`) plus every pass-through field of `Command::New` that `run_new` still needs (`detach`, `name`, `harness`, `deliver`, `no_authorize_peers`, `no_channel`, `reset`, `control_from`, `watch`, `command`); the developer names the fields from `src/cli.rs` and keeps their types. `run_new` becomes: `plan_new`, connect, pick the name (a private `pick_session_name(client, name)` that returns the chosen name and whether it was generated), resolve the confine paths, build the `NewOptions`, `create_session`, print, attach.

Rejected: merging the four copies of "a heartbeat needs a non-empty watch" (`commands/launch.rs:44`, `daemon/launch.rs:150`, `team/validate.rs:228`, `commands/team.rs`): the daemon checks the range first and the watch second, the CLI the reverse, and the messages carry different prefixes, so a merge would change which error a user with two mistakes sees; a `NewOptions`/`NewSessionRequest` merge (a separate shape change); moving `parse_activity_watch` (used by `team.rs`).

## 2. Changes

1. `src/messaging.rs`: add `interval_from_seconds` after `parse_interval`. It returns `Duration::from_secs(seconds)` when `(1..=HEARTBEAT_MAX.as_secs()).contains(&seconds)`, else the error `parse_interval` returns for the text `"{seconds}s"` with the same `label` (`invalid {label} "{seconds}s": expected a number followed by s, m or h, from 1s to 24h`; the text is built only on the error path). `parse_interval` itself is unchanged.
2. Replace the three `parse_interval(&format!("{seconds}s"), label)` calls with `interval_from_seconds(seconds, label)`: `src/commands/launch.rs:46`, `src/daemon/launch.rs:137` (the `.map(|seconds| ...)` there keeps its `.transpose()` shape), `src/wire.rs:133`.
3. `src/team.rs`: `deduplicated_values` becomes `pub(crate)`; `plan_new` uses `crate::team::deduplicated_values(&[], &confine_grant)` for the grants instead of the hand loop (`launch.rs:92-98`). No change to its body or its other callers.
4. `src/commands/launch.rs`: add `NewPlan`, `plan_new`, `pick_session_name` as above; move the pure checks of `run_new` into `plan_new` in their present order: (a) grants de-duplicated, (b) `has_confine_options && !confine` refuses with `--confine-* flags require --confine`, (c) `crate::grant::validate` when `confine`, (d) `RemoteMode::parse`, (e) `parse_activity_watch`, (f) the heartbeat parse, (g) `validate_watch_options`. The `ConfineRequest` literal is built in `plan_new`; `resolved_confine_request` (cwd resolution) stays in `run_new`. The order of the errors is the order of the old code.
5. `tests/prop_messaging.rs`: add two tests first (red before step 1: they do not compile before the function exists): a `proptest!` case `seconds_interval_checks_accept_exactly_the_range` (`n in 1..=86400u64`: `interval_from_seconds(n, "heartbeat")` equals `Duration::from_secs(n)`, next to `seconds_intervals_parse_exactly`) and a plain test `interval_from_seconds_refuses_zero_and_over_a_day_with_the_parse_text` asserting the exact text `invalid heartbeat "0s": expected a number followed by s, m or h, from 1s to 24h` for `0` and `invalid heartbeat "86401s": ...` for `86401`; the expected values are literals, not recomputed with `parse_interval`.

## 3. Behaviour

No observable change: the same accepted and refused inputs, the same texts, the same order of errors for `a2amx new`. The three seconds-to-interval checks keep their messages (the error text for a number is the text for `"{n}s"`, as before).

## 4. Out of scope

- The four duplicated watch/heartbeat checks (section 1); `NewOptions`; `wired_command`; `create_session`'s shape (`create_session` is 80 lines of request building: a separate spec if wanted); `team up` (`start_planned_session`); any change to clap definitions in `src/cli.rs`.

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass; the new test fails to compile or run before step 1 and passes after; baseline at the commit before this spec (architect states it): 66 suites, 1416 passed, 3 ignored; after: 1418 (a `proptest!` function and a plain test are two tests). The red phase of step 5 is a compile failure (the function does not exist yet); that counts as red.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'parse_interval\(&format!' src` prints nothing (the pattern `format!("{seconds}s")` alone also matches `src/config.rs:729`, a display of a duration that stays); `rg -n 'interval_from_seconds' src` prints four lines when every call is fully qualified (the definition and three call sites; a `use` import would add lines); `rg -n 'fn (plan_new|pick_session_name)\b' src/commands/launch.rs` prints 2 lines; `run_new`'s length is reported (about 45 to 60 lines expected; a report item, not a gate).
- `git status --short` lists only the six scope files; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Diff check (tester, mechanical): `plan_new`'s checks appear in the old order with the same texts; `run_new`'s remaining steps are the old steps in the old order; `deduplicated_values` body unchanged.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant; survivors reported: `plan_new` skips the `--confine-* flags require --confine` check (`tests/team/parse_and_guards.rs:184`, `tests/confine_show.rs:339`); skips `grant::validate` (expected survivor or the error moves to the daemon: no CLI test runs `new --confine --confine-grant <unknown>`; backlog); does not de-duplicate grants (expected survivor: no CLI test runs `new --confine` with a repeated `--confine-grant`; `grant::validate` refuses a repeat at `src/grant.rs:823`, so the dedupe is what lets it through; backlog); `interval_from_seconds` accepts `0` (the new test); accepts `HEARTBEAT_MAX + 1` (the new test); the daemon-side call uses the wrong label (a `session_creation` test pinning the text); `pick_session_name` ignores the taken names (expected survivor: the generated-name seed is random, a collision never happens in tests). The `interval_from_seconds` call at `commands/launch.rs:46` has no CLI killer (both callers already range-checked the number from text at `:112-116` and `commands/team.rs:338`); only the new tests and the daemon-side test `tests/daemon/session_creation.rs:90` pin the range and the text.
- Known flaky tests: those that wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| the re-parse `parse_interval(&format!("{seconds}s"), ..)` is at `commands/launch.rs:46`, `daemon/launch.rs:137` and `wire.rs:133` | `rg -n 'parse_interval\(&format!' src` | probed (three hits) |
| `parse_interval`'s error text and range | `src/messaging.rs:473-496` | read |
| the pattern `format!("{seconds}s")` alone has a fourth hit, `src/config.rs:729` (display only, out of scope); the acceptance grep is `parse_interval\(&format!` | consultant probe | probed |
| `deduplicated_values` is private to `src/team.rs` and also called with `&[]` as its first argument (`:465`) | `src/team.rs:367-375`, `:449-465` | read |
| tests pin `--confine-* flags require --confine` and `heartbeat needs a non-empty watch` | `rg -n` in `tests/` | probed (the five hits listed in section 0) |
| the check order in `run_new` is the one listed in step 4 | `src/commands/launch.rs:57-135` | read |
| `ActivityWatch::validate` (`wire.rs:121-136`) re-parses `format!("{seconds}s")` once, inside a loop over `working_after` and `idle_after` with labels `activity working_after` and `activity idle_after`; the call is the only one in `wire.rs` | `src/wire.rs:121-136` | read |
| `tests/prop_messaging.rs` has a `proptest!` block with `seconds_intervals_parse_exactly` (`:298-300`, `n in 1..=86400u64`) and a plain test `malformed_or_out_of_range_intervals_are_rejected` (`:318-330`); the added tests follow the same two shapes | `tests/prop_messaging.rs:290-330` | read |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer antigravity Gemini 4 Hydrogen (Medium) (the senior developer); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the acceptance grep `format!("{seconds}s")` has a fourth hit (`src/config.rs:729`, display only) and could not pass; the claims row said three hits | grep changed to `parse_interval\(&format!`, row corrected |
| draft review | consultant | the test count was 1417 for a proptest and a plain test (two tests) | 1418 stated |
| draft review | consultant | `NewFlags` was a single-use 12-field parameter object that left `watch` ownership unspecified and run_new long | `plan_new` takes the whole `Command` and returns a wider `NewPlan` |
| draft review | consultant | the grant dedupe, `grant::validate` and generated-name mutants have no killers; the CLI call at `launch.rs:46` has no CLI killer | named as expected survivors |
| tester pass | tester | the order of the `--confine-*` guard against `--remote` is pinned by no test (mutant survived) | recorded, backlog |
| tester pass | tester | expected survivors confirmed by probe: `grant::validate` skip, no dedupe, pick ignores taken names | recorded, backlog |
