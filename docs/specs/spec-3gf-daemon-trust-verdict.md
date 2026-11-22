# Spec 3gf: one trust verdict per request, in src/daemon/trust.rs (daemon S5)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol (`tests/daemon/serve_protocol.rs`, `tests/human_inbound/gateway_capability.rs`, `tests/broker/sessions_and_naming.rs`, `tests/delivery.rs`, `tests/bridge.rs`, `tests/explain.rs`); the new items are private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/trust.rs`, and `tests/broker/sessions_and_naming.rs` (two pinning assertions inside the existing test, section 5).

Does not touch: every other file under `src/`, every other test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `daemon` row of `AGENTS.md` and the backlog when committing.

This spec crosses a trust boundary (which token may send which request); see section 5 for the dispositions and abuse-case tests.

## 1. Why

Who may send which request is decided in three places that must agree and that nothing forces to: `request_permitted` (`src/daemon.rs:1946-2032`, with its doc comment) is three exhaustive matches over all 25 request variants (one per role, about 85 lines), `session_only_request` (`:1770-1799`) re-derives the tool name from the request with a wildcard (`_ => "report_quota"`) and ends with a second wildcard (`_ => "unexpected request"`), and `bridge_attach` (`:2315-2346`) spells the refusal for the two non-session roles twice. Facts from reading the code: the session role's set is a strict subset of the admin role's (`SendMessage, Reset, ListAgents, MessageStatus, ReportPrompt, ReportQuota, BridgeAttach`), the gateway role's set is disjoint from the admin's (`GatewayAgents, HumanInbound, HumanOutboundNext, HumanOutboundDone, RedeemLeaseCode`), so the three matches are one three-valued classification written three times. The two wildcards mean a new session-only request routed to `session_only_request` would be labelled "report_quota" in its refusal and a request that slips through would answer "unexpected request" instead of failing to compile.

Target shape, a new module `src/daemon/trust.rs`, with no wildcard anywhere:

```rust
/// Which token roles may send a request.
enum Audience {
    /// Only the gateway token.
    Gateway,
    /// The admin token or a session token.
    AdminOrSession,
    /// Only the admin token.
    AdminOnly,
}

/// The one table: every request variant is named once, with no wildcard, so a new variant fails to compile until it is classified.
fn audience(request: &Request) -> Audience;

/// The verdict `serve` applies before dispatch.
pub(super) fn request_permitted(role: &Role, request: &Request) -> bool;

/// The sender id of a session token, or the refusal message `"{tool} needs a session token"` for any other role. It returns the message, not a `Response`: `Response` holds `MessageInfo` inline (about 250 bytes) and `clippy::result_large_err` would fail `-D warnings`.
pub(super) fn session_id<'a>(role: &'a Role, tool: &str) -> Result<&'a str, String>;
```

`request_permitted` is `matches!((audience(request), role), (Audience::Gateway, Role::Gateway) | (Audience::AdminOrSession, Role::Admin | Role::Session(_)) | (Audience::AdminOnly, Role::Admin))`.

`session_only_request` disappears: `serve` dispatches `SendMessage`, `ReportPrompt` and `ReportQuota` in three arms, each calling `session_id(&role, "send_message" | "report_prompt" | "report_quota")` and then its `Runtime` method (the bodies that `session_only_request` has now), with `Err(message) => Response::Error { message }`; `bridge_attach` uses `session_id(role, "bridge_attach")` for its two refusals. No request is named by a string derived from a wildcard any more.

Rejected: a `Role` capability bitset or a trait `Authorized` (three values, one table); making the unit `Role` carry the audience (it is used in 30 places for who the caller is); moving `Role` into `trust.rs` (optional and cheap, about two touched lines, but `authenticate`, which builds `Role`, does socket I/O and stays in `daemon.rs`, and `Role` is about who the caller is, not what is allowed; it stays, which a child module sees); a typed `SessionId` newtype for the id (`Role::Session(String)` is matched in about ten places; a newtype is a separate spec if wanted).

## 2. Mechanical changes

### 2.1 New `src/daemon/trust.rs`

`Audience`, `audience`, `request_permitted`, `session_id` as in section 1, plus `use super::Role; use crate::wire::{Request, Response};`. `audience` names every `Request` variant with no `_` arm; the grouping follows the role sets above (the five gateway variants `GatewayAgents | HumanInbound | HumanOutboundNext | HumanOutboundDone | RedeemLeaseCode` into `Gateway`; the seven `SendMessage | Reset | ListAgents | MessageStatus | ReportPrompt | ReportQuota | BridgeAttach` into `AdminOrSession`; the thirteen `Hello | Confine | NewSession | List | Version | Attach | Kill | Screen | Explain | Shutdown | IssueLeaseCode | ListMessages | CancelMessage` into `AdminOnly`: 5 + 7 + 13 = 25, the variants of `Request` at `src/wire.rs:149-267`; note `IssueLeaseCode` is admin-only and `RedeemLeaseCode` gateway-only). `session_id` returns `Err(format!("{tool} needs a session token"))` for `Role::Admin` and `Role::Gateway`, `Ok(id.as_str())` for `Role::Session(id)`.

### 2.2 `src/daemon.rs`

- Delete `request_permitted` (`:1946-2032`, with its doc comment) and `session_only_request` (`:1770-1799`) and its doc comment.
- Declare `mod trust;` and `use trust::{request_permitted, session_id};`.
- In `serve`, replace the combined arm (`:1913-1917`) with three arms: `Request::SendMessage { to, subject, message } => match session_id(&role, "send_message") { Ok(id) => runtime.send_message(id, to, subject, message).await, Err(refusal) => refusal }`, `Request::ReportPrompt { prompt } => match session_id(&role, "report_prompt") { Ok(id) => runtime.report_prompt(id, prompt).await?, Err(refusal) => refusal }`, `Request::ReportQuota { five_hour, weekly } => match session_id(&role, "report_quota") { Ok(id) => runtime.report_quota(id, five_hour, weekly).await?, Err(refusal) => refusal }`.
- In `bridge_attach` (`:2315-2346`) replace the `Role::Admin` and `Role::Gateway` arms with `let id = match session_id(role, "bridge_attach") { Ok(id) => id, Err(message) => return Ok(Some(Response::Error { message })) };` ahead of the existing session body, which is unchanged.
- Remove from `daemon.rs`'s `use` lines only the names no remaining code uses (the compiler says which).

## 3. Behaviour

No observable change: every request gets the same verdict from every role, every refusal text is the same (`"not permitted for a session token"` and `"not permitted for this token"` come from `serve` as today; `"{tool} needs a session token"` as today for the same four tools). The two wildcard arms vanish: `_ => "report_quota"` was reachable (it is how `ReportQuota` got its name) and is now explicit; `_ => "unexpected request"` was unreachable (the `serve` pattern binds only the three variants). Order of checks in `serve` is unchanged: the verdict, then dispatch, then the session-id requirement.

## 4. Out of scope

- `Role`, `authenticate`, the typed session id, and the handlers' own role checks (`reset_session`, `list_agents`, `message_status`).
- Any change to which role may send which request.

## 5. Trust-boundary check and acceptance

Attack paths and dispositions: (a) a token sends a request outside its audience: constrained by the single table with no wildcard (a new variant fails to compile until classified, which is the stricter behaviour than today's three matches that also have no wildcard but can disagree); (b) a request in `AdminOrSession` reaches a handler that needs a session id with an admin token: refused by `session_id` with the existing text, one function for all four tools; (c) a gateway token sends an admin request and the reverse: the table's `Gateway` and `AdminOnly` audiences exclude each other by construction. Two pinning gaps found by the consultant (m_8488) are closed in this spec, inside `tests/broker/sessions_and_naming.rs`'s existing test (no test added): in its refusal loop (`:84-130`) add a `Request::Confine { .. }` (a session sending `Confine` must be refused with `"not permitted for a session token"`; moving `Confine` into `AdminOrSession` would let a session create sessions), and after the admin `send_message` assertion (`:140-147`) add the admin `ReportQuota` request expecting `"report_quota needs a session token"`. Both pass on the old code (contract tests). Abuse-case tests (contract tests: they pass on the old code, as the behaviour does not change): `tests/human_inbound/gateway_capability.rs` (gateway refused outside its five; admin refused its five), `tests/daemon/serve_protocol.rs:207` and `tests/broker/sessions_and_naming.rs:133` (session refused admin requests), `tests/delivery.rs:922`, `tests/broker/sessions_and_naming.rs:147`, `tests/bridge.rs:347` (admin refused session-only tools with the three texts).

- `cargo test --no-fail-fast`: all pass; baseline at the commit before this spec (architect states it before assigning); no test added or removed.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n '\b_ =>' src/daemon/trust.rs` prints nothing; `rg -n 'session_only_request|fn request_permitted' src/daemon.rs` prints nothing; `rg -n 'needs a session token' src/daemon.rs` prints nothing; `rg -n 'format!\("\{tool\} needs a session token"' src/daemon/trust.rs` prints exactly one line.
- `git status --short` lists only `src/daemon.rs`, `src/daemon/trust.rs` and `tests/broker/sessions_and_naming.rs`; Cargo diff empty.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant: move `HumanInbound` out of `Audience::Gateway` into `AdminOnly` (gateway capability tests); move `Kill` into `AdminOrSession` (session-refusal test; tester names it or reports a survivor); `session_id` returns `Ok("")` for the admin role (the three text tests); `request_permitted` returns true for `(Audience::AdminOnly, Role::Gateway)` (the gateway capability test's first element, `Hello`); add `Confine` to `AdminOrSession` (the new `Confine` element in the session loop); change the `report_quota` literal (the new admin `ReportQuota` assertion); the `session_id` Gateway refusal is unreachable from `serve` (the verdict refuses first). The first four of the mutants named are expected killed, none is equivalent; survivors are reported.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| The session set is a subset of the admin set; the gateway set is disjoint | `request_permitted` arms `src/daemon.rs:1950-2031` | read |
| 25 request variants: 5 + 7 + 13 | `src/wire.rs:149-267`; admin match 20 true and 5 false | read (consultant m_8488) |
| `session_id` must not return `Result<_, Response>` | `MessageInfo` inline in `Response` (`src/wire.rs:364-366, 570-586`) makes the `Err` over clippy's 128-byte default; no `clippy.toml` override | computed by the consultant, not compiled |
| the three tool names and the bridge text | `src/daemon.rs:1776-1777`, `:2321-2327` | read |
| refusal texts are pinned except `report_quota needs a session token` and a session sending `Confine` | `rg -n 'not permitted for|needs a session token' tests` (ten hits); the consultant found no `report_quota` text and no `Confine` in the session loop | probed; gaps closed in section 5 |
| `session_only_request` and `request_permitted` have one caller each | `rg -n` | probed |
