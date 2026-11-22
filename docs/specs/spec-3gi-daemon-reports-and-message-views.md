# Spec 3gi: session reports, message status and cancel leave daemon.rs (daemon S6c)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file, which is made after 3gh is accepted and pushed (the ranges below are for HEAD 8391903, which is the tree 3gh left: `src/daemon.rs` 1,169 lines). Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing prompt-report, receipt, quota, message-status, cancel and codex tests (`tests/hook.rs` or wherever `report_prompt` is driven, `tests/delivery.rs`, `tests/quota/`, `tests/messaging/`, `tests/codex/`, `tests/daemon/`); the moved items are private to `daemon`, except `record_receipt`, which has two outside callers: `src/codex.rs:756` as `crate::daemon::record_receipt` and `src/daemon/bridge.rs:5,:108` as `super::record_receipt` (the OMP native receipt path).

Scope (edit): `src/daemon.rs`, `src/daemon/views.rs`, `src/daemon/admission.rs`, new `src/daemon/reports.rs`.

Does not touch: every other file under `src/` (including `src/codex.rs`), every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md` and the backlog when committing.

## 1. Why

After 3gh `src/daemon.rs` is 1,169 lines. Three groups still live in `impl Runtime` and beside it that each have one job and one reader of their state:

- **reports**: what a session tells the daemon about itself: `report_prompt` (the `UserPromptSubmit` hook's verdict), the helpers it uses to match a submitted prompt against the open message envelopes (`channel_prompt_verdict`, `known_envelopes`, `block_corrupted_submission`), the receipt recording they share with the native channel (`record_receipt`, `is_awaiting_receipt`), and `report_quota`. About 190 lines.
- **message views**: `message_status` and `message_info`, the read-only answer to "what happened to my message" (78 lines), which belong with the other read-only views in `views.rs`.
- **message cancel**: `cancel_message` (16 lines), the owner's control of a message's life, which belongs with admission (how a message enters and leaves the queue).

One genuine duplication goes away in the move: `record_receipt` (`:150-157`) and `Runtime::is_awaiting_receipt` (`:384-392`) spell the same five-part condition (same boot, addressed to this session, state `Delivering`/`Submitted`/`Unsubmitted`, not yet observed). They become one free function `awaiting_receipt(message: &Message, boot: &str, session_id: &str) -> bool` in `reports.rs`, called by `record_receipt` and by the two `Runtime` methods that call `is_awaiting_receipt` today (`channel_prompt_verdict` at `:403`, `known_envelopes` at `:423`). Same behaviour; one place says what "awaiting a receipt" means.

After 3gi `daemon.rs` is about 880 lines, under the 1,000 goal. S6d (`serve.rs`) and S7 (lifecycle) follow.

Rejected: leaving `report_quota` in `daemon.rs` (it is a session report: same shape as `report_prompt`); putting `message_status` in `reports.rs` (a query is not a report); a `receipts.rs` module separate from `reports.rs` (the receipt predicate and recording are used by the prompt report, the OMP bridge and the Codex poller: three callers, but the two helpers are 50 lines and share `awaiting_receipt` with the prompt matching).

## 2. Mechanical changes

### 2.1 New `src/daemon/reports.rs`

Move, byte for byte except as 2.2 says, whole items, in source order, inside one `impl Runtime` block where they are methods: the free function `record_receipt` (`:137-163`, with its doc comment; `pub(crate)` stays), `report_prompt` (`:316-347`), `report_quota` (`:349-382`), `channel_prompt_verdict` (`:394-410`), `known_envelopes` (`:412-431`), `block_corrupted_submission` (`:433-481`). `is_awaiting_receipt` (`:384-392`) is not moved but replaced, see 2.2.

### 2.2 The one deliberate change: `awaiting_receipt`

In `reports.rs`, add `fn awaiting_receipt(message: &Message, boot: &str, session_id: &str) -> bool { message.boot == boot && message.recipient_session == session_id && matches!(message.state, MessageState::Delivering | MessageState::Submitted | MessageState::Unsubmitted) && !message.observed }`. In `record_receipt` the inline condition (the negation of the five-part test at `:150-157`, written as `if message.boot != boot || ... { return Ok(false); }`) becomes `if !awaiting_receipt(&message, boot, &session.id().0) { return Ok(false); }`. In `channel_prompt_verdict` and `known_envelopes` the calls `self.is_awaiting_receipt(&message, session_id)` become `awaiting_receipt(&message, &self.boot, session_id)`. `Runtime::is_awaiting_receipt` is deleted. This is the only line of logic that changes; the condition is identical.

### 2.3 Move into existing files

- `src/daemon/views.rs`: `message_status` (`:483-498`) and `message_info` (`:500-561`) as methods in an `impl Runtime` block at the end of the file, `message_status` `pub(super)` (called from `serve`), `message_info` private to `views.rs` (called by `message_status` and by `list_messages` in the same file, `views.rs:270`; no other caller: `rg -n 'message_info' src`).
- `src/daemon/admission.rs`: the free function `cancel_message` (`:1145-1160`), `pub(super)` (called from `serve`), at the end of the file.

### 2.4 Visibility, imports, call sites

- `pub(super)`: `report_prompt`, `report_quota` (called from `serve`), `message_status`, `cancel_message`. `record_receipt` stays `pub(crate)` and `daemon.rs` re-exports it: `pub(crate) use reports::record_receipt;` so `crate::daemon::record_receipt` (`src/codex.rs:756`) and `super::record_receipt` (`src/daemon/bridge.rs:5`) keep resolving; `bridge.rs` and `codex.rs` are not edited and a developer must not repoint them at `reports::`. Everything else is private to its new file (`channel_prompt_verdict`, `known_envelopes`, `block_corrupted_submission`, `awaiting_receipt`, `message_info`). If the compiler asks for another `pub(super)`, stop and report.
- The moved code reaches parent privates (`Runtime` fields `boot`, `store`, `sessions`; `failed`, `lookup`, `parse_message_id`, `code`) and siblings' `pub(super)` items (`trust::session_id`, `views::{session_quota, session_activity}`); no sibling private item. Each new or extended file gets the `use` lines for what it uses; `daemon.rs` loses the `use` lines no remaining code uses (clippy says which; expect `CancelResult`, `QuotaWindow`, `MessageInfo` and some `messaging` names to move).
- `daemon.rs` declares `mod reports;` (alphabetical between `registry` and `remote`) and the re-export.

### 2.5 `src/daemon.rs`

Delete the moved items and `is_awaiting_receipt`. Nothing else changes.

## 3. Behaviour

No observable change. The `awaiting_receipt` predicate is the same boolean in all three callers. Tracing targets of the moved log events change to the child module path. The `UserPromptSubmit` hook, channel prompt and native receipt paths are unchanged.

## 4. Out of scope

- Any behaviour change other than 2.2; the wording of verdicts and reasons; `serve`, `authenticate_hello`, `GatewayConnectionGuard`, `response` (S6d); `Daemon::start` and `shutdown` (S7); `reset_*`, `owner_cause`, `authenticate`, `omp_usage_command` (stay).
- Splitting `report_prompt` or `block_corrupted_submission` (49 lines; follow-up if Sonar or CodeScene flags them).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at the commit before this spec: 66 suites, 1415 passed, 0 failed, 3 ignored (architect states the commit before assigning); after: the same (no test added, removed or edited).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'fn (record_receipt|report_prompt|report_quota|is_awaiting_receipt|channel_prompt_verdict|known_envelopes|block_corrupted_submission|message_status|message_info|cancel_message)\b' src/daemon.rs` prints nothing; `rg -n 'is_awaiting_receipt' src` prints nothing; `rg -c 'fn awaiting_receipt' src/daemon/reports.rs` prints 1 and `rg -n 'awaiting_receipt\(' src` prints four lines (the definition and the three callers: `record_receipt`, `channel_prompt_verdict`, `known_envelopes`).
- `rg -n 'pub\(super\)' src/daemon/reports.rs` prints exactly `report_prompt` and `report_quota`; `pub(crate)` appears once there (`record_receipt`); `rg -n 'record_receipt' src/daemon.rs` prints only the re-export.
- `git status --short` lists only `src/daemon.rs`, `src/daemon/views.rs`, `src/daemon/admission.rs`, `src/daemon/reports.rs`; `git diff --stat -- Cargo.toml Cargo.lock src/codex.rs src/daemon/bridge.rs` is empty.
- Diff check (tester, mechanical): each moved item's body is byte-identical to its body at the base commit (`git show HEAD:src/daemon.rs`) apart from visibility, imports, `super::` paths and the three `awaiting_receipt` call forms of 2.2; the inline condition in `record_receipt` matches `awaiting_receipt`'s body clause for clause (negated).
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant; survivors reported, not fixed: `awaiting_receipt` drops the recipient clause (candidate `tests/delivery.rs:673` `foreign_session_cannot_forge_receipt`); drops the `!message.observed` clause (expected survivor: `Store::record_receipt` is idempotent, `store.rs:617` `receipt_at IS NULL`; a duplicate bridge receipt test would kill it); drops the boot clause (expected survivor without a restart-over-the-same-root test: `fail_open` leaves Submitted and Unsubmitted rows of an old boot open and session ids restart at `s1`); adds `MessageState::Pending` to the state list (the tester names the killer or reports a survivor); `report_prompt` skips `note_submit(false)` for an unknown prompt (candidate `tests/delivery.rs` `human_prompt_clears_the_human_hold`); `block_corrupted_submission` drops the `reject_attempt` loop (candidates `tests/delivery.rs:846-875`) and, separately, drops `note_corrupted` (`tests/hook.rs:267` covers only the verdict); `report_quota` accepts a window more than eight days ahead (`tests/broker/quota.rs:14`); `message_status` shows a message of another boot to a session sender (expected survivor for the same restart reason); `message_info` loses the hold reason for a pending message (`tests/delivery.rs:423,:344,:376`); `cancel_message` answers `Ok` for `CancelResult::NotCancellable` (`tests/delivery.rs:220`, `tests/daemon/serve_protocol.rs:165`). The tester names the test per mutant and states which survivors match the expected ones.
- Known flaky tests: those that wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| item ranges listed | brace-matched ranges of `src/daemon.rs` at 8391903 (1,169 lines), doc comment to closing brace | probed with a script, twice |
| `record_receipt`'s outside callers are `src/codex.rs:756` and `src/daemon/bridge.rs:5,:108` (`use super::{..., record_receipt, ...}`); the store has its own `Store::record_receipt` and a private `record_receipt` in `store.rs:611` (different items) | `rg -n 'record_receipt' src tests` and the consultant's read of `bridge.rs` (m_8544) | probed; the first draft missed `bridge.rs` because the grep looked for `crate::daemon::` paths only |
| the `record_receipt` condition and `is_awaiting_receipt` are the same boolean | `src/daemon.rs:150-157` and `:384-392` | read |
| `parse_message_id` and `constant_time_eq` stay in `daemon.rs` (used by `authenticate`, `message_status` moving, `admission.rs`, `remote.rs`) | `rg -n` | probed |
| `message_info`'s callers are `message_status` (`daemon.rs:496`) and `views::list_messages` (`views.rs:270`); after the move both are in `views.rs` | `rg -n 'message_info' src` | probed |
| `MessageState` has no `Delivered` variant (Pending, Delivering, Submitted, Unsubmitted, Cancelled, Undeliverable) | `src/messaging.rs:256-263` | read (consultant) |
| test killers per mutant | repo tests | consultant named candidates (m_8544); the tester confirms |
