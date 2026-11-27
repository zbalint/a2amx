# Spec 3hn: function-level code health of `src/daemon/serve.rs` and `src/daemon/bridge.rs`

## 0. Status

LOCKED after consultant review m_9618 (guard signature, `presence` pin and target, mutant (a) dropped, (d)/(e) expected non-kills, anchors, `bridge_frame` takes bytes). Context `a2amx-3hn-daemon-serve-bridge-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `24ad74c`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`tests/common`, `Client`/`Framed`), the omp bridge link (`tests/bridge.rs`).

Scope (edit): `src/daemon/serve.rs`, `src/daemon/bridge.rs` (production code; neither has a `#[cfg(test)]` module), and, only if Step A finds an unpinned behaviour, one new test file `tests/daemon/serve_dispatch_pins.rs` with one `mod` line in `tests/daemon/main.rs`. Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same responses and texts, same order of checks (exited session, then parse, then permission), same close and continue decisions, same connection-count accounting, same bridge frames.

## 1. Why

`cs check` (`24ad74c`) reports: `serve.rs:34` `serve` (Bumpy Road 3, cc 15, Large Method 143 lines); `bridge.rs:37` `bridge` (cc 16, Large Method 90 lines). Each function is one loop doing several jobs: `serve` authenticates, counts gateway connections, gates a request (exited, parse, permit), then dispatches about thirty request kinds of which three own the connection; `bridge` does the handshake with its refusal rule, then the frame loop with the state-merging rule.

Rules that bind (earlier refactors 3hk-3hm): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function; split along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

Existing pins: `tests/daemon/serve_protocol.rs` (authentication errors and close, control validation messages and attachment precedence, session-token parse before permission, exited and removed sessions closing before parsing, malformed request recoverable, failed attach keeps the connection, version), `tests/bridge.rs` (protocol mismatch, missing apis, already attached, not an omp session, the state frames incl. `host_pending`/`queue` at `:482-535`), `tests/human_inbound/gateway_capability.rs` (gateway token role). The developer first lists, per behaviour below, the test that already pins it (file and name). A behaviour with no pin gets a new test in `tests/daemon/serve_dispatch_pins.rs` (literal expectations, green on the unchanged code):

1. Shutdown answers exactly one `Ok` frame on the same connection and the connection then ends (EOF with no second frame). The notify-before-reply and double-reply mutants race the daemon's own close, so a kill is not guaranteed; the pin asserts the one-frame-then-EOF shape and the tester reports (d) and (e) as expected non-kills if timing hides them.
2. The gateway connection count rises on a gateway-token hello and falls when that connection closes: already pinned by `tests/presence/remote_status.rs:60-108` (read through `RemoteStatus`, `daemon/remote.rs:213`); the developer confirms and adds nothing.
3. `BridgeAttach` ends the control connection after the bridge ends (no second reply) and replies a normal `Error` for a non-omp session (existing: `tests/bridge.rs`).
4. Bridge: a `State` frame with `idle`, `queue == Some(0)`, `host_pending == Some(true)` does not mark the session pending (the `waiting_for_turn` rule, `bridge.rs:95-96`); `Hello` mid-stream closes the bridge; a session exit ends the bridge within the one-second tick (existing: `tests/bridge.rs:482-535` and the exit test, to confirm).
5. `bridge_attach` for a session that disappeared between lookup and attach bails (`"bridge session disappeared"`, no test names it; reachable only by a race, report as unreachable, no pin).

An unreachable branch is reported, not pinned; no test hook is added.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon/serve.rs`

- `enum Step { Reply(Response), Continue, Close }`: what a request handler tells the loop (answer and read on; read on without answering; end the connection).
- `fn enter<'a>(runtime: &'a Runtime, role: &Role) -> Option<GatewayConnectionGuard<'a>>` on `GatewayConnectionGuard` (the explicit lifetime is required: two reference inputs, no `self`): the `matches!(role, Role::Gateway)` count increment and guard creation of `serve`.
- `fn admit(role: &Role, bytes: &[u8]) -> Result<Request, &'static str>`: parse the frame (`"invalid control request"`), then `request_permitted` (`"not permitted for a session token"` for a session role, else `"not permitted for this token"`). `serve` replies the error and continues, as before; the exited-session check stays first in the loop.
- `async fn dispatch(connection: &mut Framed, runtime: &Arc<Runtime>, role: &Role, request: Request) -> anyhow::Result<Step>`: the three requests that own the connection, `Shutdown`, `Attach` (dimension check, lookup, `attach(...)`, with `Close` when `attach` says so, `Continue` otherwise, a `Reply(Error)` for zero dimensions or an unknown session) and `BridgeAttach` (`Close` when the bridge ran, `Reply` for its refusal); every other request is `Step::Reply(answer(runtime, role, request).await?)` through the catch-all arm after the three explicit ones. `Shutdown` sends its own `Ok`, then notifies, then propagates the send error, and returns `Continue` (the notify must come after the send).
- `async fn answer(runtime: &Arc<Runtime>, role: &Role, request: Request) -> anyhow::Result<Response>`: the plain request-to-response arms (`Hello`, `List`, `Version`, `Confine`, `NewSession`, `Kill`, `Reset`, `Screen`, `Explain`, `ListAgents`, `SendMessage`, `ReportPrompt`, `ReportQuota`, `MessageStatus`, `HumanInbound`, `GatewayAgents`, `HumanOutboundNext`, `HumanOutboundDone`, `IssueLeaseCode`, `RedeemLeaseCode`, `ListMessages`, `CancelMessage`) with the texts and calls of today. The three connection-owning kinds, which `dispatch` never forwards, share one arm that returns an `Err` naming the routing mistake; the developer states why that arm cannot be reached.
- `serve` keeps: hello, guard, the `Ok` reply, the loop (`recv`, exited check, `admit`, `dispatch`, send `Reply`). `authenticate_hello`, `session_has_exited` and `response` are unchanged.

### `src/daemon/bridge.rs`

- `fn hello_refusal(protocol: u32, missing: &[String]) -> Option<String>`: `"protocol_mismatch"` when the protocol differs from `BRIDGE_PROTOCOL`, else `"missing_apis: a, b"`, else `None` (the type of `protocol` as in `BridgeUp::Hello`).
- `async fn bridge_handshake(connection: &mut Framed, session: &Session) -> anyhow::Result<Option<String>>`: the first frame under the five-second timeout, the refusal (`native_refuse`, `Refused` frame) or the accept (`native_accept`, `Ready` frame); returns the omp version when accepted, `None` when the bridge must end.
- `bridge_frame` (signature below): one frame from the extension (`State` with the `waiting_for_turn` rule, `Ack`, `Nack`, `Receipt`); `Ok(false)` ends the bridge (a mid-stream `Hello`).
- `bridge` keeps: the handshake, the `tracing::info!` line, the one-second interval, and the `select!` with three short arms.
- `bridge_attach` unchanged. `bridge_frame` takes the raw bytes instead of a parsed frame: `async fn bridge_frame(runtime: &Arc<Runtime>, session: &Session, bytes: &[u8]) -> anyhow::Result<bool>`; a frame that fails to parse ends the bridge (`Ok(false)`), as today. Keep `waiting_for_turn` as a `let` binding (inlined it would be a two-operator conditional). The routing arm of `answer` cannot be reached, so its mutant is reported as unkillable.

If a finding remains after this shape (for example the `select!` loop's Bumpy Road), the developer reports the per-function cc and stops; no extra split without the architect.

## 4. Out of scope

`attach.rs`, `launch.rs`, `lifecycle.rs`, `daemon.rs`, `trust.rs`; changing the `Request` enum or the wire format; splitting `Runtime`; any change to `request_permitted`.

## 5. Trust boundary

No boundary is created or moved: the same permission verdict (`trust.rs`) runs at the same point for every request kind, and `answer`'s routing arm cannot be reached by a wire request. Abuse-case coverage is the existing pins: a session token on an admin request, a gateway token on a session request, a malformed frame, a request before hello (`serve_protocol.rs`, `gateway_capability.rs`).

## 6. Acceptance

Baseline (before any edit, on `24ad74c`): `cargo test` passes (1522 passed, 3 ignored); whole-tree cs count 867.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/serve.rs
cs check src/daemon/bridge.rs
git status --short
```

- All pass; `cs check` lists no finding on `serve.rs` and `bridge.rs` (both are at 10.00) and, if the new test file exists, none on it; the whole-tree count is below 867 (judge: the architect's `cs` run).
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources); the whole suite passes on both trees.
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hn/` (both deleted before its REPORT); per mutant run `cargo test --lib` and the test targets `daemon`, `bridge`, `broker`, `human_inbound`, `presence`, `client_requests`, `explain`, `teams`, `gateway_up`, `gateway_background`. Mutants, one change each: (a) dropped: permission needs the parsed request, so the order is forced by the types; (b) `admit` uses the session text for every role; (c) the exited-session check moves after `admit`; (d) `Shutdown` stops before replying (reply after `stop_requested.notify_one()`; expected non-kill if the race hides it); (e) `Shutdown` replies twice (`Reply(Ok)`; expected non-kill, a second frame sent just before the close is never read); (f) `Attach` with zero columns attaches; (g) a failed `attach` returns `Close`; (h) `GatewayConnectionGuard::enter` counts every role; (i) the guard does not decrement; (j) `dispatch` replies after `BridgeAttach` ends; (k) `hello_refusal` swaps the two reasons; (l) `hello_refusal` ignores `missing`; (m) the `waiting_for_turn` rule is dropped; (n) `bridge_frame` returns `Ok(true)` for a mid-stream `Hello`; (o) a `Receipt` never calls `native_received`; (p) `bridge_handshake` does not call `native_refuse`. Expected kills are named by the tester from its own run; an expected non-kill is reported with its reason. Any survivor in moved code is a fix request.

## 7. Claims (architect, `24ad74c`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `serve.rs:34`, `bridge.rs:37` | `cs check` run, output read |
| three request kinds own the connection: `Shutdown`, `Attach`, `BridgeAttach` | `serve.rs:88-173` | read |
| the `Shutdown` arm replies, then notifies, and continues without a second reply | `serve.rs:98-105` | read |
| check order today: exited session, parse, permission | `serve.rs:48-87` | read |
| `bridge_attach` has one caller | `serve.rs` | grep |
| the serve and bridge messages are pinned | `tests/daemon/serve_protocol.rs`, `tests/bridge.rs`, `tests/human_inbound/gateway_capability.rs` | grep of each message text |
| `"bridge session disappeared"` is not named in any test | `bridge.rs:21` | grep `tests/` |
| `gateway_connections` is read by `daemon/remote.rs:213` | `remote.rs` | grep |
