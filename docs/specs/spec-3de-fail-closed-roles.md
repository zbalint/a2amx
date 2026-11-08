# Spec 3de: The daemon decides each request for each role in one exhaustive match, so a new request or a new role cannot pass by default

## 0. Status

LOCKED (architect, 2026-11-08), after the consultant's pre-lock review (`m_5981`, one round, all findings applied).

Scope (files that may be edited): `src/daemon.rs` (`session_request_denied`, `session_token_permitted` and their one call in `serve`), `tests/broker.rs` (three requests added to one existing loop). Does not touch: `src/wire.rs` (the `Request` enum), `src/client.rs`, `src/mcp.rs`, any other test file, the docs, `CHANGELOG.md`, `Cargo.toml`.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seam: a real daemon connection made with an admin token and with a session token (`tests/common`, as `tests/broker.rs` does). This is the first slice of remote human messaging (backlog F81, `docs/permissions.md` D2). It adds no feature and changes no behavior.

## 1. Why

`docs/permissions.md` D2 adds a third daemon role, the Discord gateway, whose requests must be a short list. Today the check cannot carry that:

- `session_request_denied` (`src/daemon.rs:2177`) is `matches!(role, Role::Session(_)) && !session_token_permitted(request)`. It refuses a request only for a session. Any other role is allowed every request, so a new `Role::Gateway` added beside `Admin` and `Session` would pass every request, including `ListMessages`, `CancelMessage`, `Kill`, `NewSession` and `Shutdown`, with no compile error and no test failing.
- `ListMessages` and `CancelMessage` have no role check of their own; they rely entirely on this one function.

The fix is to make each role's list explicit and make the compiler require a decision. One `match` on the role, and for each role one `match` on the request with no wildcard arm: adding a `Request` variant, or a `Role`, fails to compile until the verdict is written for every role. A new variant already fails to compile in `serve`'s dispatch `match` today; what is new is that no permission verdict can be skipped. That is fail-closed by construction, for `match`es only: an `if let Role::Session(..)` or a `let ... else` still gives a new role a default path (see section 5).

Alternative rejected: a test that lists every request and role. A test cannot enumerate variants it does not know about, so it cannot catch the added variant; the compiler can.

Alternative rejected: changing the denial text or adding a role-specific message now. The gateway slice (3df) adds its own role and decides its message; here the three existing tests that assert `not permitted for a session token` must keep passing unchanged.

## 2. Changes

Claims this spec relies on (read at `74ddfe4` plus the doc commits, `src/daemon.rs` 3057 lines):

| claim | where | verified |
|---|---|---|
| `Role` has exactly `Admin` and `Session(String)` | `src/daemon.rs:199-202` | read |
| `session_request_denied` is called once, in `serve`, after the request parses and before the dispatch `match`, and its refusal text is `not permitted for a session token` | `src/daemon.rs:2085-2093` | read |
| `session_token_permitted` is called only from `session_request_denied` | `rg -n "session_token_permitted" src tests` shows `:2178` and the definition `:2207` only | probed |
| the session list is `SendMessage, Reset, ListAgents, MessageStatus, ReportPrompt, ReportQuota, BridgeAttach` | `src/daemon.rs:2207-2218` | read |
| `Request` has 19 variants: `Hello, Confine, NewSession, List, Version, Attach, Kill, Reset, Screen, Explain, Shutdown, SendMessage, ListAgents, MessageStatus, ReportPrompt, ReportQuota, ListMessages, CancelMessage, BridgeAttach` | `src/wire.rs:150-` | read, the developer recounts |
| an admin token sent `Hello` again gets `already authenticated`; admin sent `SendMessage`, `ReportPrompt` or `ReportQuota` gets `<tool> needs a session token` from `session_only_request` | `src/daemon.rs:2023-2058`, `:2093-2100` | read |
| the admin claims are pinned by `tests/daemon.rs:1174` (`Hello` again gives `already authenticated`), `tests/delivery.rs:921` (`report_prompt needs a session token`), `tests/bridge.rs:315` (`bridge_attach needs a session token`) | those lines | read by the consultant |
| tests that already pin the refusal for a session: `List`, `NewSession`, `Attach`, `Kill`, `Screen`, `ListMessages`, `CancelMessage` in `tests/broker.rs:285-330`; `Confine` in `tests/daemon.rs:257`; `List` raw in `tests/daemon.rs:1338`; `Explain` in `tests/explain.rs:420` | those lines | read |
| nothing pins `Hello`, `Version` or `Shutdown` refused for a session token | `rg -n "Shutdown" tests/*.rs` shows no session-token case | probed |
| baseline: `cargo test --test broker session_tokens_roles` passes | run 2026-11-08, `1 passed` | probed |

### 2.1 `src/daemon.rs`

Replace `session_request_denied` and `session_token_permitted` with one function:

```rust
/// One verdict per role and request. Neither match has a wildcard arm: a new `Request`
/// variant or a new `Role` does not compile until its verdict is written here.
fn request_permitted(role: &Role, request: &Request) -> bool {
    match role {
        Role::Admin => match request {
            Request::Hello { .. } | /* every variant, listed */ Request::BridgeAttach => true,
        },
        Role::Session(_) => match request {
            Request::SendMessage { .. }
            | Request::Reset { .. }
            | Request::ListAgents
            | Request::MessageStatus { .. }
            | Request::ReportPrompt { .. }
            | Request::ReportQuota { .. }
            | Request::BridgeAttach => true,
            Request::Hello { .. }
            | Request::Confine { .. }
            | Request::NewSession { .. }
            | Request::List
            | Request::Version
            | Request::Attach { .. }
            | Request::Kill { .. }
            | Request::Screen { .. }
            | Request::Explain { .. }
            | Request::Shutdown { .. }
            | Request::ListMessages { .. }
            | Request::CancelMessage { .. } => false,
        },
    }
}
```

The admin arm lists all 19 variants explicitly (not `_ => true`); the developer writes the real list. The caller in `serve` becomes `if !request_permitted(&role, &request)` with the same response text and `continue`. The set of requests a session may make and the set an admin may make are exactly today's: this is a no-behavior-change refactor. `session_only_request` and every other `Role` match stay as they are.

The boolean `match` is deliberate. Do not "simplify" it to `matches!(...)`: `matches!` puts a wildcard arm back, and with it the default. Say so in a one-line comment above the function (the doc comment above already states the rule; keep it). If clippy reports `match_like_matches_macro`, allow that lint on this function with a reason in the attribute.

Placement: `Role` and the check live in `src/daemon.rs`; the change adds no responsibility to the module and the file is large only because the daemon is.

### 2.2 `tests/broker.rs`

In `session_tokens_roles_permissions_and_child_identity`, add to the `for request in [ ... ]` loop at lines 285-330, which asserts `not permitted for a session token` for each: `Request::Version`, `Request::Hello { token: sender_token.clone() }` (the loop takes ownership and `sender_token` is used again at line 351) and `Request::Shutdown { now: true }`. These are contract tests: they pass on the unchanged code. A refused request does not close the connection, so the loop and the assertions after it keep running; a mutant that permits `Shutdown` stops the daemon and fails the loop's assertion. After the loop the existing assertions still run, which shows the daemon was not stopped by the `Shutdown`.

## 3. Attack paths (trust-boundary check)

The boundary is the daemon's control port: a caller holding a session token is less trusted than the admin.

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| any request on an authenticated connection | a session token sends a request outside the session list (`Shutdown`, `Kill`, `NewSession`, `ListMessages`, `CancelMessage`, `Explain`, `Hello`, `Version`, `Confine`, `Screen`, `Attach`, `List`) | constrain: one `request_permitted` check every request passes through, exhaustive per role | each refused by a real daemon with `not permitted for a session token`: the existing tests above plus the three new ones |
| a future `Request` variant or `Role` | passes by default | remove: no wildcard arm, so it does not compile | the developer's compile probe in section 4 |
| `Confine` carries a nested `Request` | `request_permitted` sees only the outer request, so a role allowed `Confine` would also allow the inner one | constrain, not by this function: `split_confine` refuses a nested `Confine` and `create_session` accepts only `NewSession` inside it (`src/daemon.rs:346`, `:421`). `Confine` stays admin-only here; a later spec that permits it for a new role must keep that inner check | the existing `tests/daemon.rs:257` (session refused) |
| `Attach` and `BridgeAttach` | after the answer the connection carries stream frames, not control requests | no gap: `serve` returns after them (`return Ok`) | none needed |

## 4. Acceptance

All run from `/home/zbalint/workspace/a2amx` on `develop`, as an ordinary user.

1. `rg -n "session_request_denied|session_token_permitted" src tests` prints nothing.
2. `cargo test` passes with no failures (943 tests at `de46a83`; the count does not change except the three added requests in an existing test), `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean.
3. `cargo test --test broker session_tokens_roles` passes, and the new `Version`, `Hello` and `Shutdown` requests are in its loop. These are contract tests, green on the old code.
4. Compile probe (a probe, not part of the diff; checked by the tester and by the architect with `git diff`): in the checkout, add a dummy variant `ProbeOnly` to `Request` in `src/wire.rs` and run `cargo check` (library and binary only, not `--all-targets`). It must report at least one non-exhaustive-patterns error (E0004) inside each of `request_permitted`'s two inner matches. The existing dispatch `match` in `serve` (about `src/daemon.rs:2095-2163`) also errors, so more than two errors is expected. The developer pastes the `request_permitted` error lines in the report and restores `src/wire.rs` by checksum, so `git diff` shows no change to it (the "does not touch" list applies to the final diff).
5. Mutation check: the developer changes the `Role::Session` arm one at a time to permit `Shutdown`, then `ListMessages`, then `Kill`, and names the repo test that fails on each mutant; each mutant is restored by checksum. A mutant that no test kills is reported.
6. `git diff --stat` shows only `src/daemon.rs` and `tests/broker.rs`. No `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` or docs change.
7. `scripts/prepush-fast.sh` passes (the architect runs it before the push).
8. Cognitive complexity (Sonar `rust:S3776`, limit 15): by hand, the outer `match` costs 1 and each of the two inner matches costs 2, so 5 in total; `if !request_permitted(..)` in `serve` adds nothing over the old call. The Sonar gate after the push is the check; no local tool measures it.

Known flaky tests: none expected; a failure in an unrelated PTY test is rerun once and reported as pre-existing if it recurs (see the backlog flake family F37).

## 5. Out of scope

- The gateway role itself, the human origin, the pairing record: spec 3df.
- A role-specific denial message: spec 3df.
- Note for 3df: "every other `Role` match stays" does not mean they are fail-closed. `Role` is matched exhaustively at `src/daemon.rs:1240`, `:1269`, `:2274` and `:2472`, so a new role fails to compile there. But `if let Role::Session(..)` at `:1251` (the visibility check in `reset_session`), at `:1472` (the sender filter in `message_status`) and the `let ... else` in `session_only_request` (`:2033`) give a new role the admin view or a denial by default. 3df must not permit a new role `Reset` or `MessageStatus` without changing those sites.
- The one reference to `session_token_permitted` in `docs/permissions.md` (line 17): the architect updates it in the acceptance commit.
- Changing which requests an admin or a session may make.
- Changing `session_only_request`, `session_has_exited` or any other `Role` match.
- Splitting `src/daemon.rs`.
- The changelog: no user-visible change.
