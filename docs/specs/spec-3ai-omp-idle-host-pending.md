# Spec 3ai: an idle OMP with queued host messages and an empty extension queue is not in flight (backlog I8 fix)

## 0. Status

**LOCKED** (2026-10-15; consultant review m_1792 folded in). Owner-directed fix of backlog I8, context `a2amx-i8-fix-2026-10-15`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `76905c0`; `cargo test --test bridge --test broker` is 17 + 30 passed, 0 failed. The developer records full `cargo test` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. The untracked `.cargo/` directory belongs to the owner: do not touch, stage or delete it.
**Public test seams:** the bridge wire protocol through a real daemon (`tests/bridge.rs`: `BridgeUp::State` over a `BridgeLink`, then a queued message that is delivered or stays `pending/in_flight`).

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: the `BridgeUp::State` arm of the bridge loop (about line 2166)
- `tests/bridge.rs`: the new tests of section 4
- `docs/delivery.md`: the paragraph "State is reported after connection ..." (about line 579)

Does not touch: `extension/omp.ts`, `src/session.rs`, `src/wire.rs`, `src/channel.rs`, `BRIDGE_PROTOCOL`, `tests/broker.rs`, `docs/backlog.md` and `CHANGELOG.md` (the architect updates them), `Cargo.toml`, every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

Backlog I8 (cause in docs/backlog.md I8). After a failed `todo` tool call OMP queues a `todo-error-reminder` custom message with `deliverAs: "nextTurn"`. OMP holds it until a new turn starts, and it counts in `ctx.hasPendingMessages()`. The extension reports `pending: fifo.length > 0 || hasPending` (`extension/omp.ts:223`), so the idle OMP reports `idle=true pending=true queue=0 host_pending=true` for hours. The daemon stores `pending` as `native.in_flight`, so every message to that session stays `pending` with hold reason `in_flight` and `reset_session` is rejected `busy`, until someone types into the composer. Captured: `native busy: idle=true in_flight=true waiting=false idle_for=1056s reported=4s ago ext_queue=0 ext_pending=true`.

`idle && queue == 0 && host_pending` means "OMP is waiting for the next turn", not "a delivery is in flight". The extension's own FIFO holds every message a2amx delivered until its receipt or a drop, so `queue == 0` proves a2amx has nothing in flight. Delivering to an idle harness calls `sendUserMessage`, which starts a turn; a started turn flushes OMP's next-turn queue, and the stuck item is released exactly as when the owner types.

**Why the daemon, not the extension.** The CLI writes the extension and the daemon is a separate, possibly older or newer process. A daemon-side rule fixes already-written extensions at the next daemon restart, and it keys off the diagnostic fields spec 3af already put on the wire. An extension with no diagnostic fields (`queue` or `host_pending` `None`) keeps today's behavior.

**Races.** If OMP starts its own stream between the extension's `isIdle()` check and `prompt()`, the delivery enters as a steer; that already holds for every idle delivery, and a next-turn item never starts a stream by itself, so the rule does not widen it. When a delivery's turn flushes the reminder, the FIFO head can move to the late-receipt path before its `message_end`; the receipt still arrives through that existing path.

**Rejected alternatives.** Change the extension to `fifo.length > 0 || (hasPending && !idle)` (needs the extension rewritten on every installation, and the daemon cannot tell the difference between the two behaviors). Suppress OMP's todo reminder (outside this repository). Reset on `busy` (hides the symptom).

## 2. Rule

In the `BridgeUp::State` arm, compute `waiting_for_turn = idle && queue == Some(0) && host_pending == Some(true)` and pass `pending && !waiting_for_turn` as the pending argument of `session.native_state(idle, draft, pending)`. Nothing else changes: `session.native_diagnostics(queue, host_pending)` still receives the raw values, so a busy rejection from another cause still shows `ext_queue=0 ext_pending=true`. The `draft`, `idle`, receipt, acknowledgement and FIFO behavior are untouched.

Worked instances (each is a test row in section 4):

| idle | pending | queue | host_pending | stored `in_flight` | why |
| --- | --- | --- | --- | --- | --- |
| true | true | Some(0) | Some(true) | false | the I8 case: waiting for the next turn |
| true | true | Some(1) | Some(true) | true | a2amx's own message is still in the extension FIFO |
| false | true | Some(0) | Some(true) | true | OMP is mid-turn |
| true | true | None | None | true | old extension: old behavior |
| true | true | Some(0) | Some(false) | true | inconsistent report, keep the old behavior |

**Interaction with the acknowledgement.** `native_ack` sets `in_flight = true` on a successful acknowledgement and the next state frame overwrites it. After an acknowledgement the extension holds the message in its FIFO, so its next state has `queue >= 1` and the rule does not apply. The window before OMP shows the delivered message as queued is covered by `queue >= 1`, not by the rule. This is the reason the rule requires `queue == Some(0)` and not only `host_pending`.

## 3. Changes

1. `src/daemon.rs`, the `BridgeUp::State` arm: replace `session.native_state(idle, draft, pending);` with a call whose third argument is `pending && !(idle && queue == Some(0) && host_pending == Some(true))`. Add one `//` comment on the why (OMP holds its own next-turn reminders while idle; a2amx has nothing in flight when the extension queue is empty). A local `let waiting_for_turn` is fine; no helper function, no new type.
2. `docs/delivery.md`: in the paragraph "State is reported after connection ...", after the sentence about `queue` and `host_pending`, add that a state with `idle`, `queue` 0 and `host_pending` true does not count as in flight on the daemon, because OMP is waiting for its next turn (for example a queued reminder after a failed `todo` call), and that the next delivery starts that turn. A reset is no longer rejected `busy` in that state.

## 4. Safety net

Tests that pass on the unchanged code and must keep passing: all 17 of `tests/bridge.rs`, all 30 of `tests/broker.rs` (in particular `reset_diagnostic_extension_queue_tracks_state_and_reconnect`, which never sends `idle: true, queue: Some(0), host_pending: Some(true)`), and `a_second_message_waits_while_one_is_in_flight` (its `state(false, true)` has `queue: None`, which is the "old extension" row).

New tests in `tests/bridge.rs`, one function per row of the section 2 table, with the shape below (consultant review m_1792, Amendment 1). Only the first is red before the change; the other four are regression guards that pass on today's code and must keep passing. The report says so.

Shape of every test (Amendment 1; the draft barrier of the first lock raced the separate sender connection and timed out once under load): `Case::start`, `let mut link = case.ready().await`, then `case.send()` (m_1), `first_delivery(&mut link)`, `link.send(BridgeUp::Ack { id: "m_1".into() })`, `case.wait_state("m_1", "submitted")`, then `case.send()` (m_2) and only then the row frame (Amendment 2: m_2 is sent after the acknowledgement, so it is accepted before the row frame is written). This is the shape of `a_second_message_waits_while_one_is_in_flight`: the acknowledgement sets `in_flight` and m_2 waits behind it. Then send the row frame and assert about m_2:

- `an_idle_host_with_queued_next_turn_messages_does_not_block_delivery` (row 1: `idle: true, pending: true, draft: false, queue: Some(0), host_pending: Some(true)`): `assert_eq!(next(&mut link).await, BridgeDown::Deliver { id: "m_2".into(), envelope: <the m_2 envelope literal used in a_second_message_waits_while_one_is_in_flight> })`. Red before the change: m_2 stays `in_flight` and `next()` times out after 10 s.
- `a_non_empty_extension_queue_still_blocks_delivery` (row 2), `a_busy_host_with_queued_messages_still_blocks_delivery` (row 3), `an_extension_without_diagnostics_keeps_blocking_delivery` (row 4), `an_inconsistent_state_report_keeps_blocking_delivery` (row 5): `case.wait_reason("m_2", "in_flight")`, then `no_delivery(&mut link)`.

The tests use the table's values as literals, not derived from the production expression. `cargo test --test bridge` after the change has 17 + 5 = 22 tests; `--test broker` stays 30.

Behavior change this implies, with no code in `src/session.rs`: in the I8 state `reset_session` is no longer rejected `busy` (`in_flight` is false, nothing waits), so the reset proceeds. No new broker test (scope: `tests/broker.rs` is not edited); the real-OMP check in section 6 covers it.

## 5. Out of scope

`extension/omp.ts` (including the candidate `hasPending && !idle` change), `reset_native_reason` and `native_reason` policy (the reset now succeeds in the I8 state as a consequence, section 4), the FIFO nets, protocol version, OMP-side todo behavior, `docs/backlog.md`, `CHANGELOG.md`, the cargo-mutants work, backlog I11 to I14.

## 6. Acceptance

```sh
cargo test --test bridge --test broker
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/daemon.rs, tests/bridge.rs, docs/delivery.md, plus the untracked .cargo/ (the spec itself is committed at lock)
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs` drag-mode and wheel tests (backlog I7, I14) and `tests/daemon.rs::exited_claude_session_never_reports_needs_input` (I9): rerun once, record as pre-existing. The tests of section 4 are post-implementation checks; the baseline above was run before locking. The final proof is a real OMP session: after a failed `todo` call, a message sent to the idle session is delivered without anyone typing.

## Amendment 1 (2026-10-15)

The developer's report m_1802 showed one full-suite run where `an_inconsistent_state_report_keeps_blocking_delivery` timed out in `wait_reason("m_1", "draft_present")`: the draft-barrier frame travels on the bridge link and `case.send()` on a separate connection, so under load the message can be processed before the barrier. Not a defect in the developer's work; the locked shape was the architect's. Section 4 now uses the acknowledgement of m_1 as the barrier (same shape as the existing `a_second_message_waits_while_one_is_in_flight`); in rows 2 to 5 either processing order of the row frame and m_2 yields `in_flight`, and in row 1 either order ends in delivery, so no test depends on cross-connection ordering. Scope, rule, table, acceptance and counts (22 bridge tests) are unchanged. Gate step 7: the old wording ("draft_present", "barrier") now appears only in this paragraph.

## Amendment 2 (2026-10-15)

The tester's BLOCKED (m_1809) found that section 4 said both messages are sent before the first delivery while the developer's five tests send m_2 after the acknowledgement of m_1. Both orders are deterministic, and the developer's order is the stronger one: `case.send()` returns only after the daemon accepted the message, and the row frame is written after that, so m_2 is always in the queue (held `in_flight` by the acknowledgement) before the row frame can be read. In row 1 the frame then releases m_2; in rows 2 to 5 `in_flight` holds before and after the frame, so rows 2 to 5 are regression guards, as stated. Section 4 now describes the order the tests use; scope, rule, table and acceptance are unchanged. The developer's tests already conform and need no change.
