# Spec 3aa: split `Runtime::create_session` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14; pre-lock gate run against develop at `3aca1fd`; consultant review `m_1657`, findings 1-6 adopted; no second review pass, fixes were clarifications). Architect-directed, backlog I5 part 2, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `3aca1fd`; the developer records `cargo test --test daemon --test daemon_cli --test teams` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the `Request::NewSession` wire request through a real daemon (`tests/daemon.rs`, `tests/daemon_cli.rs`,
`tests/teams.rs`, `tests/team.rs`, `tests/codex.rs`, `tests/codex_exe.rs`, `tests/broker.rs`). New helpers are private to `src/daemon.rs`.

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: `create_session` (line 267) and new private functions placed directly after it
- `tests/teams.rs` and `tests/daemon.rs`: only the characterization tests of section 4 (`teams.rs` holds the team-scope daemon tests, `daemon.rs` the rest)

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `create_session` (complexity 78) against 30 (backlog I5). The function validates a request with about
twenty early returns, takes the new-session gate, optionally starts a Codex app-server, spawns the session and
starts up to six background tasks, all in one body.

## 2. Rule: behavior is unchanged

Same `Response` for the same request: same message text byte for byte, same CHECK ORDER (the first failing check wins),
same side effects in the same order (gate lock, id allocation, token generation, env edits, Codex start, spawn,
registry insert, task starts). Keep the existing `// shortcut:` comment with the Codex block. No `pub` change.

## 3. Changes in `src/daemon.rs`

Keep `create_session`'s signature, the destructuring of `Request::NewSession` and the `Ok(Response::Created { .. })`
and `Response::Error` shapes. Each extracted validator returns `Option<String>` (the error message) and the caller turns
`Some(message)` into `return Ok(Response::Error { message })`. Extract, preserving the order below:

1. **Field checks before the gate (daemon.rs 285-409).** Order today: (a) `argv.is_empty() || cols == 0 || rows == 0`, (b)
   `validate_reset_steps`, (c) each `control_from` entry, (d) each `watch` entry with the self-watch check in the same loop
   iteration, (e) the heartbeat parse (`parse_interval(&format!("{seconds}s"))`), (f) `session {name}: heartbeat needs a
   non-empty watch`, (g) the team scope checks, (h) `validate_role`. Keep (a), (b), (e), (f), (h) INLINE in `create_session`.
   Extract only `validate_watch_lists(control_from: &[String], watch: &[String], name: Option<&str>) -> Option<String>`
   (steps c and d, in that order) and `validate_team_scope(scope: &TeamScope, name: Option<&str>) -> Option<String>` (step g).
   No composing function: it would need ten parameters and fail `clippy::too_many_arguments` (limit 7).
2. **`validate_team_scope`**: the whole `if let Some(scope) = &team` block in the order: `validate_name(scope.name)` with the
   message prefix `team: `, trailing `-`, `!private && !allow.is_empty()`, the `allow` loop (validate with prefix `team allow
   entry`, equals the team, repeated), `!private && !agents.is_empty()`, `agents.len() > 16`, the `agents` loop (validate with
   prefix `session allow entry`, names itself, repeated). The text `allow needs a private team` appears twice, at the first
   and the second `!private` test; keep both at their places.
3. **Checks under the gate.** `let _gate = self.new_session_gate.lock().await;` stays in `create_session` as a named binding
   and lives until the function returns: it covers the team and name checks, id allocation, token generation, the env edits,
   the Codex block including `codex::start`, `Session::spawn`, the registry insert and the task starts. Do not move the guard
   into a helper, do not write `let _ = ..lock()` (drops at once) and do not scope it in a block. Extract
   `fn check_team_and_name(&self, team: Option<&TeamScope>, name: Option<&str>) -> Option<String>` as a SYNC function (the
   `sessions` mutex is `std::sync` and is never held across an await): (i) the team differs from a RUNNING session
   (`exit_code().is_none()`) of the same team name, message `team {name} is already running with different settings`, then (ii)
   `validate_name(name)` (message `error.to_string()`, no prefix) and (iii) `session name {name} is already in use`, which
   matches ANY session including exited ones. Keep that asymmetry. Name validation therefore happens AFTER the role and
   team-scope checks.
4. **Codex preparation.** `async fn prepare_codex(&self, id: &SessionId, argv: Vec<String>, cwd: Option<&str>, env: &mut Vec<(String, String)>)`
   returning `anyhow::Result<Result<(Vec<String>, codex::Link), String>>` (adapt the link type to the real one): the outer
   `Err` carries the `JoinError` of `spawn_blocking(..).await?` (propagates as an error, not a `Response::Error`); the inner
   `Err(message)` carries `executable_error`'s message and `format!("{error:#}")` from `codex::start` (become
   `Response::Error`). `create_session` calls it only when `harness == Harness::Codex`; for other harnesses `argv` passes through
   unchanged and `codex` stays `None`. The helper does the `NO_AUTHORIZE_ENV` read and retain, the `exe` clone, the `ROLE_ENV`
   read (last match, `.rev()`) and retain, `executable_error` on a blocking thread, `codex::start` and `wire_argv`. The
   `// shortcut:` comment moves with the block. The `A2AMX_TOKEN` and `A2AMX_ADDR` pushes, the `CHANNEL_ENV` detection and its
   `retain` (all harnesses) stay in `create_session` BEFORE the call.
5. **After a successful spawn.** `fn start_session_tasks(self: &Arc<Self>, session: Arc<Session>, channel: bool)` (sync; it
   calls `tokio::spawn` inside the runtime): the `store` and `boot` clones, then in this order the delivery run, `observe_exit`,
   `sample_activity`, `heartbeat` (only if `session.heartbeat()` is `Some`), `accept_channel_dialog` (only if `channel`),
   `codex::run(session, link, store, boot)` (only if `session.codex()` is `Some`; the link comes from `session.codex().cloned()`,
   not from the local variable, which moved into `SessionSpec`), then `self.deliveries.lock()..extend(tasks)`. `codex::run`
   moves `session`, `store` and `boot`, so it stays the last use. `create_session` keeps the `Arc::new(session)`, the
   `let id = session.id().0.clone()` for `Response::Created` and the `sessions.insert(number, session.clone())` BEFORE the call.

## 4. Safety net

Method: for each message literal below `rg -n '<literal>' tests/` and decide asserted or not; the CLI rejects some cases before
sending (`src/main.rs` around 386 and 552, "heartbeat needs a non-empty watch"), so a test through the CLI does not prove the
daemon message. Characterization tests send `Request::NewSession` directly to a real daemon (pattern in `tests/teams.rs` and
`tests/daemon.rs`).

Known asserted at the daemon: `watches itself` (`daemon_cli.rs`), `must not name itself` and `at most 16` and
`team a is already running with different settings` (`teams.rs`), `already in use` (`broker.rs`). Half covered: `allow needs a
private team` and `is repeated` are asserted only for the session-`agents` path (the second occurrence).

To add, each passing on the unchanged code and asserting the full message: `command and nonzero dimensions are required`; the
reset-step error; `control_from entry ...`; `watch entry ...`; the heartbeat parse error; the DAEMON `session {name}: heartbeat
needs a non-empty watch`; `team: ...` (invalid team name); `team must not end in '-'`; the first `allow needs a private team`
(public team with `allow = ["Bad_Name"]` must give `allow needs a private team`, not `team allow entry`); the three `team allow
entry` messages; `session allow entry {e:?}: ...`; the invalid-role error; the post-gate invalid-name error.

Order pairs, asserting the FIRST message: empty `argv` plus an invalid `control_from` entry (argv wins); an invalid `control_from`
entry plus a `watch` entry naming the session itself (`control_from` wins); an invalid team name plus an invalid role (`team: `
wins); a team that differs from a running one plus an in-use session name (the team message wins); an invalid session name plus an
invalid role (the role message wins, name validation is post-gate); an invalid session name plus heartbeat without `watch` (the
heartbeat message wins). Expected values are literals from the unchanged output.

## 5. Out of scope

`heartbeat`, `report_prompt`, `serve` (specs 3ab to 3ad), any other function in `daemon.rs`, `codex.rs`, wire types.

## 6. Acceptance

```sh
cargo test --test daemon --test daemon_cli --test teams --test team --test codex --test codex_exe --test broker
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/daemon.rs and, if added, tests/teams.rs and tests/daemon.rs
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
and `attach_does_not_capture_session_already_in_drag_mode` (backlog I7): rerun once, record as pre-existing. Final proof of the
complexity fix is the Sonar rescan after the owner pushes.
