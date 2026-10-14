# Spec 3af: the OMP extension reports its queue, and a busy reset rejection shows it (backlog I8 diagnostics)

## 0. Status

**LOCKED** (2026-10-14; consultant reviews m_1732 and m_1735 folded in). Owner-directed investigation, context `a2amx-i8-omp-stall-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `7913a91` (spec 3ae, 518 passed, 0 failed, 3 ignored); the locking commit follows it and changes only this file. The developer records `cargo test` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the bridge wire protocol through a real daemon (`tests/broker.rs`: `BridgeUp` over `Client::attach`-style `BridgeLink`, reset rejection text), frame serialization (`tests/bridge.rs`), the rendered extension text (`tests/omp_wiring.rs`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs`: two optional fields on `BridgeUp::State`
- `src/daemon.rs`: the `BridgeUp::State` arm of the bridge loop
- `src/channel.rs`: its two `BridgeUp::State` literals (add `queue: None, host_pending: None`)
- `src/session.rs`: `NativeSlot`, one new method, `NativeGuard::drop`, the busy branch of `reset_native_reason`
- `extension/omp.ts`: `reportState` and one constant
- `tests/bridge.rs`, `tests/broker.rs`, `tests/omp_wiring.rs`: the new tests of section 4 and the mechanical `queue: None, host_pending: None` added to existing `BridgeUp::State` literals
- `docs/delivery.md`: the `state` sentence of the frame list ("`state` reports idle, queued-message and human-draft state") and the paragraph "State is reported after connection ..."

Does not touch: `docs/backlog.md` (the architect updates it), `src/bridge.rs`, `src/client.rs`, `BRIDGE_PROTOCOL`, `tests/wire.rs`, every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

Backlog I8: an OMP session sat with the daemon showing `idle=true in_flight=true waiting=false` for 3786 s and then 3987 s (`reported=` equal to `idle_for=`), so `reset_session` was rejected as `busy`. The extension's own safety nets (`extension/omp.ts:211-217`) clear its FIFO within 120 s and then report `pending=false`, and OMP's `hasPendingMessages()` does not count asides (OMP `queuedMessageCount` covers steering, follow-up and next-turn messages only). So either OMP kept reporting queued messages, or the extension stopped reporting; the daemon cannot tell which from `idle`, `pending` and `reported` alone. This spec adds the missing evidence; it does not change delivery behavior. The fix comes in a later spec once one more occurrence shows which cause holds.

## 2. Rule: delivery behavior is unchanged

The Rust serialization of the state frame with both new fields `None` is byte for byte unchanged (the extension's frame gains the two fields), and acknowledgements, receipts, the FIFO and its 120 s nets, and every existing rejection text prefix stay byte for byte. Only additions: two optional fields on the state frame, a keepalive resend of that frame, and a suffix on one rejection message.

**Version skew.** The CLI writes the extension and the daemon is a separate, possibly older process (consultant review m_1732). An old daemon ends a bridge on a frame it cannot parse, so a new frame type would make a new extension flap against it. The two fields therefore ride on the existing `state` frame: serde ignores unknown fields, so an old daemon accepts the new extension, and a new daemon reading an old extension sees `None`. `BRIDGE_PROTOCOL` stays 1. A frame that fails to parse still ends the bridge, as today.

## 3. Changes

1. `src/wire.rs`: `BridgeUp::State` gains `#[serde(default, skip_serializing_if = "Option::is_none")] queue: Option<u32>` and the same attributes on `host_pending: Option<bool>` (`queue` is the extension's FIFO length, `host_pending` is OMP's `ctx.hasPendingMessages()` as a real boolean). With both `None` the serialized frame is byte for byte what it is today; with values it is `{"type":"state","idle":true,"pending":true,"draft":false,"queue":2,"host_pending":true}` (field order as declared: `idle`, `pending`, `draft`, `queue`, `host_pending`). Every existing `BridgeUp::State { idle, pending, draft }` literal in `src`, `tests/broker.rs` (13) and `tests/bridge.rs` (1, the `state` helper) gets `queue: None, host_pending: None`.
2. `src/daemon.rs`: the arm becomes `BridgeUp::State { idle, draft, pending, queue, host_pending } => { session.native_state(idle, draft, pending); session.native_diagnostics(queue, host_pending); }`.
3. `src/session.rs`: `NativeSlot` gains `diagnostics: Option<(u32, bool)>`. `pub(crate) fn native_diagnostics(&self, queue: Option<u32>, host_pending: Option<bool>)` stores `Some((q, h))` only when both are `Some`, and otherwise sets `None` (an old extension clears it); it does not notify `message_notify` (`native_state` already does). `NativeGuard::drop` sets it to `None` with the other resets. In `reset_native_reason`, the `busy` message gets the suffix ` ext_queue={queue} ext_pending={host_pending}` appended AFTER the existing `idle_for=... reported=...` text, only when `diagnostics` is `Some`; with `None` the message is unchanged. Example: `native busy: idle=true in_flight=true waiting=false idle_for=3s reported=3s ago ext_queue=1 ext_pending=true`.
4. `extension/omp.ts`: add `const KEEPALIVE_MS = 30000;` beside the other module constants and `let lastSentAt = 0;` beside `lastState`. In `reportState`, `value` becomes `{ idle, pending: fifo.length > 0 || hasPending, draft: safeDraft(), queue: fifo.length, host_pending: hasPending === true }` (the comparison at `extension/omp.ts:220-226` names `idle`, `pending` and `draft` explicitly, so it must also require `lastState.queue === value.queue` and `lastState.host_pending === value.host_pending`; otherwise a queue change is sent only at the next keepalive), and the unchanged-state early return additionally requires `now - lastSentAt < KEEPALIVE_MS`, so an unchanged state is re-sent at least every 30 s. After a successful `sendFrame` set `lastSentAt = now` next to `lastState = value`. The frame is still `{ type: "state", ...value }`, one frame, so there is no partial-send case. Everything above the early return in `reportState`, and `handleDeliver`, are unchanged. The keepalive means a live extension re-sends its state at least every 30 s, so `reported=` growing past about 30 s on a connected session proves the extension stopped reporting.
5. `docs/delivery.md`: in the "State is reported ..." paragraph add that the `state` frame also carries `queue` (the FIFO length) and `host_pending` (OMP's queued-message flag), that state is re-sent every 30 s when nothing changed, and that a busy reset rejection ends with `ext_queue=N ext_pending=B` when the extension has reported them. The frame-list sentence says `state` reports idle, queued-message and human-draft state plus those two diagnostic fields.

## 4. Safety net

Tests that pass on the unchanged code and must keep passing: the existing `reset_diagnostic_*` tests in `tests/broker.rs` (they use `contains`, and the suffix is appended last), `tests/bridge.rs`, `tests/omp_wiring.rs`.

New tests, each failing before the change:

- `tests/bridge.rs`, in `bridge_frames_serialize_to_the_documented_json`: a row `BridgeUp::State { idle: true, pending: true, draft: false, queue: Some(2), host_pending: Some(true) }` with the literal `{"type":"state","idle":true,"pending":true,"draft":false,"queue":2,"host_pending":true}` (both directions, like the other rows), and a check that the old literal `{"type":"state","idle":true,"pending":false,"draft":false}` still deserializes to a state with `queue: None, host_pending: None` (the existing row already pins the old serialization).
- `tests/broker.rs` (reuse `omp_reset_target`, `reset_rejection`): (a) after only a `State { idle: true, pending: true, .. }` the busy rejection does NOT contain `ext_`; (b) after `State { idle: true, pending: true, draft: false, queue: Some(2), host_pending: Some(true) }` it contains `ext_queue=2 ext_pending=true`, and still starts with `reset gate rejected step 1 of 1: native busy: idle=true in_flight=true waiting=false`; (c) a later state with `queue: Some(0), host_pending: Some(false)` and `pending: true` changes it to `ext_queue=0 ext_pending=false`; (d) a later state with both `None` removes the suffix; (e) after a bridge reconnect (use the reconnect step of `reset_diagnostic_native_timing_survives_reconnect`) with a state that has both `None` the suffix is absent.
- `tests/omp_wiring.rs`: `the_rendered_extension_embeds_the_tool_schemas_and_the_protocol` (or a sibling test) asserts the rendered extension contains `host_pending` and `KEEPALIVE_MS`.

The keepalive timing and the `reportState` condition are JavaScript inside the embedded extension and have no cheap unit seam; they rely on reading the diff, which the report must say. The developer also runs the existing real-OMP smoke tests before and after (`cargo test --test omp_smoke -- --ignored --test-threads=1`) and reports both results; if they cannot run (`omp` unavailable) the report says so.

## 5. Out of scope

Any fix for the stall itself, the reset gate policy, `native_reason`, delivery ordering, the FIFO nets, `docs/backlog.md`, `Cargo.toml`, other lint cleanups.

## 6. Acceptance

```sh
cargo test --test broker --test omp_wiring --test bridge --test claude_channel
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo test --test omp_smoke -- --ignored --test-threads=1   # before and after; report both
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
rg -n 'host_pending' src extension tests docs/delivery.md   # only scope files
git status --short   # only the files of section 0
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs` drag-mode and wheel tests (backlog I7) and `tests/daemon.rs::exited_claude_session_never_reports_needs_input` (I9): rerun once, record as pre-existing. The final proof is the next occurrence of the stall: the rejection then shows `ext_queue`, `ext_pending` and a small `reported=` (extension alive) or a `reported=` past 30 s (extension silent), which picks the fix.
