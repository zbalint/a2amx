# Spec 3gj: the connection loop leaves daemon.rs (daemon S6d)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file (after 3gi is pushed; the ranges below are for HEAD c143a7e: `src/daemon.rs` 879 lines). Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing protocol, trust, attach, bridge, gateway and lease tests (`tests/daemon/serve_protocol.rs`, `tests/human_inbound/`, `tests/gateway.rs`, `tests/daemon/attach_streams.rs`, `tests/bridge.rs`); the moved items are private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/serve.rs`.

Does not touch: every other file under `src/` (`attach.rs` and `bridge.rs` import `response` from the parent and keep doing so), every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md` and the backlog when committing.

## 1. Why

`src/daemon.rs` mixes three things: the `Runtime` state and its methods, the `Daemon` lifecycle (`start`, `shutdown`, 335 lines, S7), and the per-connection loop that authenticates a peer and dispatches its requests (`serve`, 144 lines, and its four helpers). The connection loop has one reader of `Runtime` (it only calls methods and free functions that already live in sibling modules) and one concern: framing, authentication, the role-scoped verdict and the dispatch of 25 request variants onto handlers. It changes when the protocol gains a request; the rest of `daemon.rs` changes when state or lifecycle changes. They belong in separate files so that a protocol change touches `serve.rs` and a state change does not.

Move: `serve` (the loop), `response` (the framed reply every handler uses), `session_has_exited` (the exited-session check run before dispatch), `authenticate_hello` (the five-second hello with the token check) and `GatewayConnectionGuard` with its `Drop` (the live gateway-connection counter `serve` holds for the connection's life). `Role` stays in `daemon.rs`: it is a type of the state (`authenticate` returns it, 30 places use it).

After 3gj `daemon.rs` is about 700 lines; S7 (lifecycle) takes `Daemon` out and leaves the state, `Role` and a handful of small shared functions.

Rejected: splitting the 25-arm dispatch in this spec (`serve` is near Sonar's complexity limit of 30: a pure move cannot raise it, but any restructuring can; a follow-up can extract the `Attach`, `Shutdown` and `BridgeAttach` arms if wanted); moving `lookup`, `failed`, `parse_message_id`, `constant_time_eq`, `random_hex` (shared by many siblings; they stay as the parent's small helpers); a `protocol.rs` name (the module is the connection loop; `serve` names it).

## 2. Mechanical changes

### 2.1 New `src/daemon/serve.rs`

Move, byte for byte except as 2.2 says, whole items with their doc comments and attributes, in source order: `struct GatewayConnectionGuard` and `impl Drop for GatewayConnectionGuard` (`:113-121`), `response` (`:693-695`), `serve` (`:697-840`), `session_has_exited` (`:842-844`), `authenticate_hello` (`:846-870`). The five items are free functions or types; the file has no `impl Runtime`.

### 2.2 Visibility, imports, call sites

- `pub(super)`: `serve` (spawned by `Daemon::start`) and `response` (called by `attach.rs`, `bridge.rs` and the moved code). Everything else is private to `serve.rs`. If the compiler asks for another `pub(super)`, stop and report.
- `daemon.rs` adds `mod serve;` (alphabetical: after `remote`/`reports`, before `trust`) and `use serve::{response, serve};` so `attach.rs` and `bridge.rs` keep their `use super::{..., response, ...}` unchanged (a child sees the parent's private imports, as with the existing `use attach::attach;`).
- The moved code reaches parent privates (`Runtime` fields `gateway_connections`, `stop_now`, `stop_requested`; `Role`, `lookup`; the parent-defined private methods `Runtime::authenticate` (called by `authenticate_hello`) and `Runtime::reset_session`) and the siblings' `pub(super)` items. `serve.rs` imports the handlers by their sibling paths, not through the parent's `use` lines: `use super::{Role, Runtime, lookup};`, `use super::admission::cancel_message;`, `use super::attach::attach;`, `use super::bridge::bridge_attach;`, `use super::launch::kill_session;`, `use super::trust::request_permitted;`, `use super::views::{explain_of, list_agents, list_messages, list_sessions, screen_of};` (all `pub(super)` today). `serve` does not call `session_id`: do not import it. No sibling private item is used.
- `daemon.rs` after the move: delete `use admission::cancel_message;`, `use attach::attach;`, `use bridge::bridge_attach;`, `use launch::kill_session;` and the `use views::{...}` line (clippy would not flag them if kept, because the children's own `use super::...` lines reach them; remove them anyway: the module declarations stay); `use trust::{request_permitted, session_id};` becomes `use trust::session_id;` (`reports.rs` and `admission.rs` import it through the parent and keep doing so); remove the `use` lines the remaining code does not use: `crate::client::Framed`, `TcpStream`, `AsyncWriteExt`, `crate::emulator::Size`, and `Request` (`use crate::wire::Response;` remains); keep `TcpListener`, `Duration`, `AtomicUsize`, `Ordering`, `Arc`.
### 2.3 `src/daemon.rs`

Delete the moved items. Nothing else changes.

## 3. Behaviour

No observable change. The order of checks in `serve` (hello, gateway guard, exited-session check, parse, verdict, dispatch) is unchanged. The moved code has no tracing call, so no log target changes (the one nearby, "client connection closed", is in `Daemon::start` and stays).

## 4. Out of scope

- Any change to the dispatch arms, verdict order, refusal texts or timeouts; splitting `serve`; `Daemon::start` and `shutdown` (S7); `Role`, `authenticate`, `lookup` (stay).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at the commit before this spec: 66 suites, 1415 passed, 0 failed, 3 ignored (architect states the commit before assigning); after: the same (no test added, removed or edited).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n '(async )?fn (serve|response|session_has_exited|authenticate_hello)\b|GatewayConnectionGuard' src/daemon.rs` prints nothing; `rg -c 'fn (serve|response|session_has_exited|authenticate_hello)\b|struct GatewayConnectionGuard|impl Drop for GatewayConnectionGuard' src/daemon/serve.rs` prints 6; `rg -n 'pub\(super\)' src/daemon/serve.rs` prints exactly `serve` and `response`.
- `git status --short` lists only `src/daemon.rs` and `src/daemon/serve.rs`; `git diff --stat -- Cargo.toml Cargo.lock src/daemon/attach.rs src/daemon/bridge.rs` is empty.
- Diff check (tester, mechanical): each moved item's body is byte-identical to its body at the base commit (`git show HEAD:src/daemon.rs`) apart from visibility, imports and `super::` paths; nothing the spec keeps moved.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant; survivors reported, not fixed: `authenticate_hello` accepts a missing hello (`serve_protocol.rs:341` `every_connection_requires_a_valid_hello`, `:4`); the hello timeout 5 s to 30 s (killed by `every_connection_requires_a_valid_hello` at `serve_protocol.rs:366-368`, whose 7 s wait fails; the lower-bound mutant 5 s to 1 s is an expected survivor: no test bounds the timeout from below); `GatewayConnectionGuard::drop` does not decrement (`tests/presence/remote_status.rs:43`, which waits for `gateway: false` after the gateway disconnects); `session_has_exited` always false (`serve_protocol.rs:214`); `request_permitted` check removed in `serve` (`gateway_capability.rs`, `serve_protocol.rs:207`); the invalid-request branch's `continue` becomes `return Ok(())` (`serve_protocol.rs:308` malformed request is recoverable, `:172`); `Request::Shutdown` stores `false` instead of `now` (the tester names a killer or reports a survivor; reordering the answer and the `stop_now` store is an equivalent mutant: `Daemon::shutdown` reads it only after `notify_one`). The tester names the test per mutant.
- Known flaky tests: those that wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| item ranges listed | brace-matched ranges of `src/daemon.rs` at c143a7e | probed with a script |
| `response` callers outside `serve`: `attach.rs`, `bridge.rs` (`use super::{..., response}`) | `rg -n 'response\(' src/daemon.rs src/daemon/*.rs` | probed |
| `serve` has one caller: `Daemon::start`'s accept loop (`daemon.rs:511`); nothing else calls `session_has_exited` or `authenticate_hello`; `Runtime::authenticate` is called only by `authenticate_hello` | `rg -n 'serve\(' src`, consultant read (m_8562) | probed |
| every handler `serve` calls is `pub(super)` in its sibling (`trust.rs:46`, `attach.rs:114`, `bridge.rs:11`, `launch.rs:518`, `admission.rs:332`, `views.rs:17/68/139/156/253`, the `Runtime` methods in `remote.rs`, `reports.rs`, `views.rs`, `admission.rs`, `launch.rs`) | consultant read | read |
| `Sonar` shows no open issue on `src/daemon.rs` at develop; serve is near the complexity limit and the move keeps boundaries and nesting | `search_sonar_issues_in_projects`, consultant | probed |
| `gateway_connections` is read by `remote.rs:213` and incremented only in `serve` | `rg -n gateway_connections src` | probed |
| `Role` stays in the parent and `serve` reaches it | `src/daemon.rs:107` | read |
| test killers per mutant | repo tests | consultant and tester to name |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer antigravity Gemini 4 Hydrogen (Medium) (the senior developer); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the mutant "invalid-request branch drops its `continue`" does not compile (the arm must yield a `Request`) | replaced by `continue` to `return Ok(())` |
| draft review | consultant | the hello-timeout mutant 5 s to 0 is not killed by the named test (it bounds the wait from above only) and is racy | replaced by 5 s to 30 s; 5 s to 1 s an expected survivor |
| draft review | consultant | the "Shutdown answers before setting `stop_now`" mutant is equivalent (`Daemon::shutdown` reads it after `notify_one`) | marked equivalent; `stop_now` stores `false` named instead |
| draft review | consultant | two import routes offered (via the parent's `use` lines or sibling paths) with the first leaving daemon.rs's imports unpruned; the spec named `session_id` as used by `serve` (it is not) and a tracing-target change (there is no tracing call in the moved code) | sibling paths chosen, the use-line list made exact, both false statements removed |
| tester pass | tester | the `stop_now` store mutant, left open in the spec, is killed by `kill_and_down::daemon_stop_now_skips_ctrl_d` | recorded |
| tester pass | tester | spec test line references name the test's first line, not the failing assertion | recorded |
