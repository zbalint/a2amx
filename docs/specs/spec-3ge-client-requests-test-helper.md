# Spec 3ge: one exchange helper for the typed-client tests

## 0. Status

LOCKED. Location: a git worktree at `target/scratch/3ge-worktree` (detached at the commit that adds this file; the main checkout holds other in-flight daemon work), built with `CARGO_TARGET_DIR=/home/zbalint/workspace/a2amx/target` so dependencies are shared and only the crate rebuilds; the developer removes the worktree (`git worktree remove --force`) after reporting, once the architect has taken the diff. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: `Client`'s typed methods against the scripted fake daemon (`tests/client_requests.rs`).

Scope (edit): `tests/client_requests.rs` only.

Does not touch: every file under `src/`, every other test, `tests/common/`, `Cargo.toml`, `Cargo.lock`, `docs/` other than this spec.

## 1. Why

`tests/client_requests.rs` is 749 lines and repeats one shape 39 times (40 `spawn_fake` calls; only `transport_failure_after_hello_returns_err` is not a one-reply case): `spawn_fake(vec![reply])`, `Client::connect_addr(addr, "token")`, one typed call, an assertion on the result, an assertion on the recorded request, `handle.await.unwrap()`. Each of the ten per-method tests (`sessions_`, `kill_`, `reset_`, `shutdown_`, `version_`, `explain_`, `screen_`, `messages_`, `cancel_message_` and `issue_lease_code_` `success...`) spells it three or four times, with only the reply, the call and the assertion differing; the five `create_session_*` tests spell it once each. The repetition hides which request and which reply each case pins, and CodeScene/Sonar count it as duplication (spec 3fz needed a rewrite of its expected side for the 3% new-code duplication gate). It is the open test-hardening item "`tests/client_requests.rs` repeats spawn/connect/call/assert about 45 times (a helper halves it)" in backlog F94.

Target shape, one helper in the file:

```rust
/// Run one typed call against a fake daemon that answers with `reply`; return the call's result and the requests the daemon recorded.
async fn exchange<T>(
    reply: Response,
    call: impl AsyncFnOnce(&mut Client) -> T,
) -> (T, Vec<Request>)
```

It does what the 41 copies do: `spawn_fake(vec![reply])`, `Client::connect_addr(addr, "token")`, `call(&mut client)`, `handle.await` and the recorded requests read out of the mutex. The tests become `let (result, requests) = exchange(Response::Ok, async |c| c.kill("s1".into(), true).await).await;` followed by the assertions. The helper's order is: call, `drop(client)`, `handle.await.unwrap()`, then clone the recorded requests (the fake blocks on receive until the client sends or closes, so the client must be gone before the join). No test in the file sends more than one request; `transport_failure_after_hello_returns_err` (`spawn_fake(vec![])`) stays unchanged and is the only remaining direct `spawn_fake` and `Client::connect_addr` user besides the helper.

Rejected: a macro (hides the call under test); a table-driven test over closures (the calls have ten result types); changing `spawn_fake` (the transport test uses it with no reply).

## 2. Changes

`tests/client_requests.rs` only:

1. Add `exchange` after `spawn_fake` (`:14-62`).
2. Rewrite every case of the ten per-method tests and the five `create_session_*` tests to use `exchange`: one `exchange` per case (do not merge cases onto one connection). Keep every assertion's expected value and every test function name; keep the count of tests unchanged. Change `recorded_json` (`:601`) to take `&[Request]` instead of `&Mutex<Vec<Request>>`; that is the only other signature change allowed.
3. Keep the asserted wire shapes exactly (the `Request` values, the JSON of `create_session`, the `to_string()` texts of errors). A case that asserted the recorded request keeps asserting it; a case that did not keep not asserting it (do not add assertions: that is a separate change).
4. `#![allow(clippy::expect_used, clippy::unwrap_used)]` stays.

No new test, no deleted test. The file shrinks (expect roughly 450-550 lines; no line count is acceptance).

## 3. Out of scope

- New assertions, new cases, or the missing `role` observation (backlog F94 lists it as a separate follow-up).
- Any change to `tests/common/` or `src/`.

## 4. Acceptance

- `cargo test --test client_requests` passes with 16 tests before and after (the developer runs it first on the base and states both).
- `rg -c 'assert' tests/client_requests.rs` prints 49 before and after (the helper adds none); the architect reads the diff for the expected literals.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -c 'spawn_fake\(' tests/client_requests.rs` prints exactly 3 (the definition, the call inside `exchange`, the transport test); `rg -c 'Client::connect_addr' tests/client_requests.rs` prints exactly 2 (`exchange` and the transport test).
- `git status --short` in the worktree lists only `tests/client_requests.rs`.
- Mutation check (assigned tester or developer, in the worktree, restoring by checksum; the test file is the deliverable, so mutate `src/client/requests.rs`): change the request built by `kill` (swap `now`), by `reset` (change the session), by `shutdown` (swap `now`) and by `explain` (change the session) and show a `client_requests` test fails on each; change the mapping of `Response::Error` to the result in `version` (the old-daemon `None`) and show a test fails. Survivors are reported. (Pre-lock: these are contract tests that pass on the old code; the mutation check shows the rewrite kept their power.)
- The whole suite is not required in the worktree beyond `cargo test --test client_requests`; the architect runs the full suite on the applied diff in the main checkout.

## 5. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| 41 `spawn_fake(` matches (the definition and 40 calls), 40 `connect_addr`, 16 tests and 6 helpers | `rg -c` on both patterns, `rg -n '^async fn |^fn '` | probed |
| every typed method returns an owned result, so `exchange`'s `T` borrows nothing from the client | `src/client/requests.rs:44-145`; `Request` derives `Clone` (`src/wire.rs:146`) | read (consultant m_8465) |
| `rg -c 'assert' tests/client_requests.rs` is 49 | consultant m_8465 | probed by the consultant |
| `AsyncFnOnce` is available | `rust-toolchain.toml` channel `1.98.1` (stable since 1.85) | read |
| the file already allows unwrap and expect | `tests/client_requests.rs:1` | read |
| the other users of `spawn_fake` are in this file only | `rg -n 'spawn_fake' tests` prints nothing outside `tests/client_requests.rs` | probed |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer OMP-external senior slot (antigravity, Gemini 4 Hydrogen, Medium); no separate tester (the developer ran the mutation check, as the spec assigned). BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the spec's picture of the file was wrong: "eleven `*_success_error_and_unexpected` tests" (six have that suffix; ten per-method tests plus five `create_session_*`), "multi-reply tests" (none exist), counts 41 and "at most 8" | corrected to 39 one-reply cases, exact counts 3 and 2 |
| draft review | consultant | the helper must drop the client before awaiting the fake's join handle (the fake blocks on receive); `recorded_json` takes the mutex, so its signature must change; a worktree with its own target dir is a second full build | order and signature stated; shared `CARGO_TARGET_DIR` |
| implementation | developer | none beyond the spec | n/a |
| architect review | architect | the spec said 16 tests before and after; `cargo test --test client_requests` runs 17 (the target also runs `common::fake_discord::tests`) | developer reported 17 before and after; spec text left as locked |
