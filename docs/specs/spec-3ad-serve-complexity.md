# Spec 3ad: split `daemon::serve` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14). Architect-directed, backlog I5 part 5, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Sequencing:** spec 3ac (same file) is accepted and committed, so `src/daemon.rs` has one writer. At the baseline below `serve` starts at line 1742; the developer re-reads the anchors if the tree moved.

**Baseline:** develop at `7ee110b` (the locking commit follows it and changes only this file); at that commit `cargo test --test daemon --test daemon_cli --test delivery --test bridge --test broker` passes 33, 59, 24, 17 and 29 tests (162 of 162). The developer records `cargo test --test daemon --test daemon_cli --test delivery --test bridge --test broker` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the daemon's TCP control protocol through real daemons (`tests/daemon.rs`, `tests/daemon_cli.rs`,
`tests/delivery.rs`, `tests/bridge.rs`, `tests/broker.rs`; `serve` is also exercised by `refs_and_list.rs`, `attach_cli.rs`, `claude_channel.rs`, `teams.rs`, which the full suite runs). New helpers are private to `src/daemon.rs`.

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: `serve` and new private functions placed directly after it at module level
- `tests/daemon.rs`: only characterization tests of section 4

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `serve` (88) against 30 (backlog I5). It authenticates a connection, then dispatches about sixteen request kinds in one
`match`, three of which also end or continue the connection loop.

## 2. Rule: behavior is unchanged

Same `Response` for each request, same message text byte for byte, same order of checks (authentication, then the exited-session check
for session tokens, then request parsing, then the session-token permission list, then dispatch), same connection lifecycle (which arms
`continue`, which `return`, which write a response and then loop). No `pub` change, no wire change.

## 3. Changes in `src/daemon.rs`

Keep `serve`'s signature, the `Framed` connection, the loop and the final `response(&mut connection, result).await?`. Keep these arms
INLINE because they change the loop's control flow: `Hello`, `Version`, `NewSession`, `Shutdown` (answers first, then `continue`),
`Reset`, `SendMessage`, `ReportPrompt`, `MessageStatus`, and `Attach` (calls `attach(..)`, `return Ok(())` or `continue`). Extract the
rest into private async functions at module level, each taking what it uses from `runtime`/`role`/`connection`:

1. `async fn authenticate_hello(connection: &mut Framed, runtime: &Runtime) -> anyhow::Result<Option<Role>>`: the 5 second hello read,
   the `Hello` parse and `authenticate`, and on failure the `authentication required; check admin.token` error response plus
   `shutdown()`. The caller returns `Ok(())` on `None`. The `response(Response::Ok)` after success stays in `serve`.
2. `fn session_token_permitted(request: &Request) -> bool`: the `matches!` on `SendMessage | Reset | ListAgents | MessageStatus |
   ReportPrompt | BridgeAttach`. The caller keeps the `not permitted for a session token` response and `continue`.
3. `async fn list_sessions(runtime: &Runtime) -> anyhow::Result<Response>`: the `Request::List` arm.
4. `fn list_agents(runtime: &Runtime, role: &Role) -> Response`: the `Request::ListAgents` arm. It holds the `sessions` std mutex guard
   throughout and must stay SYNC (no await while the guard is alive).
5. `async fn kill_session(runtime: &Runtime, id: &str, now: bool) -> Response` and `fn screen_of(runtime: &Runtime, id: &str) -> Response`.
6. `async fn list_messages(runtime: &Runtime, session: Option<String>, state: Option<String>) -> anyhow::Result<Response>` and
   `async fn cancel_message(runtime: &Runtime, id: &str) -> anyhow::Result<Response>`.
7. `BridgeAttach`: `async fn bridge_attach(connection: &mut Framed, runtime: &Arc<Runtime>, role: &Role) -> anyhow::Result<Option<Response>>`
   where `Some(response)` is a normal error to send and `None` means the bridge ran to completion and the caller must `return Ok(())`.
   The `bridge session disappeared` bail, the OMP/channel check, `native_attach()`, the `Attached` response and `bridge(..)` keep their order.

Besides the arms, three steps STAY in the loop in this order: the per-request exited-session check (a session token whose session is gone from the registry or has exited gets `session has exited`, then `shutdown()` and `return Ok(())`), the parse with its `invalid control request` `continue`, and the session-token permission `continue`. `kill_session` maps the request's `now` to `session.kill(!now)` (`kill(graceful)`); keep the inversion and name the helper parameter `now`. `authenticate_hello` propagates `response(..)?` and `shutdown()?` errors as `Err`; `Ok(None)` means the error was sent and the stream shut down. `bridge_attach` keeps `anyhow::bail!("bridge session disappeared")` as an `Err`. Only `bridge_attach` needs `&Arc<Runtime>` (`bridge` takes it); every other helper takes `&Runtime`. `list_sessions` awaits `open_counts` BEFORE taking the `sessions` lock; keep that. `list_agents` calls `omp_usage()` while holding the `sessions` guard (nesting sessions then the omp cache); keep it so.

Exact type names (`Framed` is `crate::client::Framed`, `Runtime`, `Role`) follow the source. If a helper needs more than 7 parameters, group them; none should.

## 4. Safety net

Asserted already (checked at `0ce2ee4`): `invalid control request` (`daemon.rs`), `not permitted for a session token` (`broker.rs`), `a bridge is already
attached to this session` and `session is not an omp session` and `bridge_attach needs a session token` (`bridge.rs`, `claude_channel.rs`), `send_message needs a
session token` (`broker.rs`), `report_prompt needs a session token` (`delivery.rs`), `unknown session` for Kill (`refs_and_list.rs`), `unknown message` for the
store-Unknown path (`attach_cli.rs`, `a2amx cancel m_99`).

Not asserted anywhere, to add in `tests/daemon.rs` over a real daemon connection, each passing on the unchanged code, with the full message: `already authenticated`
(an admin token sending Hello again); `authentication required; check admin.token`; `session has exited` (and that the connection then closes; also the case where
the session is gone from the registry after Kill); `attachment dimensions must be nonzero` (Attach with zero cols or rows); `unknown message state` (ListMessages with a bad state filter); `message {id} is {state} and cannot be
cancelled`; `unknown message {id}` for an id `parse_message_id` rejects (for example `abc`); `unknown session {id}` for Screen and for Attach. The `bridge session
disappeared` race and the `sender session is unavailable` race (Amendment 1) have no cheap test; they rely on reading the diff, which the report must say.

Order pairs, asserting the first message: a session token sending unparsable bytes gets `invalid control request`, not `not permitted for a session token` (the parse
precedes the permission check); an exited session's token gets `session has exited` before any parse (a valid or invalid body does not matter). Expected values are
literals from the unchanged output.

## 5. Out of scope

`create_session`, `heartbeat`, `report_prompt` (specs 3aa to 3ac), `attach`, `bridge`, `response`, wire types.

## 6. Acceptance

```sh
cargo test --test daemon --test daemon_cli --test delivery --test bridge --test broker
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/daemon.rs and, if added, tests/daemon.rs
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
and `attach_does_not_capture_session_already_in_drag_mode` (backlog I7): rerun once, record as pre-existing. Final proof of the
complexity fix is the Sonar rescan after the owner pushes.

## Amendment 1 (2026-10-14): `sender session is unavailable` is race-only

The developer reported BLOCKED (m_1699): section 4 required a test for `sender session is unavailable` from `ListAgents` with a session token whose
session is gone, passing on the unchanged code. Verified: the loop's exited-session check (`serve`, line 1765 at `7ee110b`) answers `session has exited` and
shuts the connection down before any request is parsed, and `authenticate` (line 294) refuses new connections for gone or exited sessions, so the
`ListAgents` branch is reachable only when the session disappears between that check and the `sessions` lock. A deterministic public-seam test cannot reach it.

Decision: the `sender session is unavailable` test is removed from the section 4 list and the message joins the `bridge session disappeared` race as diff-review-only;
the report must say so. The `session has exited` tests (including the gone-from-registry-after-Kill case) stay and cover the precheck. Nothing else in the spec changes; scope, section 3 and acceptance
are unaffected. Pre-lock gate re-run on this amendment: the old wording of the removed list entry no longer appears in the spec outside this amendment and the race sentence.
