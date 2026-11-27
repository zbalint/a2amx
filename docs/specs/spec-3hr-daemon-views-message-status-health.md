# Spec 3hr: function-level code health of the message status path in `src/daemon/views.rs`

## 0. Status

LOCKED. Context `a2amx-3hr-daemon-views-message-status-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `12a4d18`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`tests/common`, `Client`: `Request::MessageStatus`, `Request::ListMessages`), the CLI over it.

Scope (edit): `src/daemon/views.rs`, only `message_status` (`:388`) and `message_info` (`:405`) and the new functions they use; tests only where Step A finds an unpinned behaviour, appended to existing files named there, never a new file. Does not touch: any other function in `views.rs` (`list_agents`, `explain_of`, `session_quota`, `list_sessions` are later specs), any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same replies, same texts, same store calls in the same order.

## 1. Why

`cs check` (`12a4d18`) reports on `views.rs`: `:395` "Complex Conditional" on `message_status` (the `if let Role::Session(sender) = role && (message.boot != self.boot || message.sender_session != *sender)` guard); `:405` `message_info` "Complex Method (cc = 12)" and "Bumpy Road Ahead (bumps = 2)" (the hold-reason `if` chain and the evidence block with its nested `if`/`else`). The other four findings of the file (`:68`, `:156`, `:275`, file-level) are out of scope here.

Rules that bind (earlier refactors 3hk-3hq): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

The developer first lists, per behaviour below, the test that already pins it and runs it. Existing pins (read by the consultant): item 1 and the admin view `tests/broker/sessions_and_naming.rs:397` `message_status_visibility_and_hold_reason` (another session, a malformed id, `m_99`, the admin, `deliver_hold` with its explanation); the earlier boot `tests/broker/limits_and_lifecycle.rs:598` `graceful_restart_marks_exited_and_continues_ids` (`:653-656`; it kills a skipped boot check only because session ids restart at `s1` every boot, `registry.rs:21`, so no redundant pin is added); `queued` and `not_ready` `limits_and_lifecycle.rs:554` `not_ready_and_queued_hold_reasons`; `channel_down` `tests/claude_channel.rs:294` `channel_without_bridge_holds_pending_message_at_channel_down`; `human_draft`, `unsubmitted_envelope` and `deliver_hold` in `tests/delivery.rs` (`:344`, `:376`, `:423`); the evidence strings in `delivery.rs`, `bridge.rs`, `codex/delivery.rs`, `hook.rs`, `mcp`. The `tests/human_inbound` "unknown message" pins are gateway-path (`HumanOutboundDone`) pins, not `MessageStatus` pins. A behaviour with no pin gets a new small test with literal expectations (whole `MessageInfo` fields), green on the unchanged code, appended to an existing file with few functions (candidates `tests/broker/sessions_and_naming.rs`, 10 functions, 648 lines; `tests/broker/limits_and_lifecycle.rs`, 10 functions, 685 lines; not `tests/delivery.rs`, 962 lines); check `cs check` on that file before and after (a long test is a new Large Method).

1. `MessageStatus` of an unparseable id, an unknown id, a message of another session (session role) and a message of an earlier daemon run (session role) each reply `UNKNOWN_MESSAGE` "unknown message"; the admin sees any message. The gateway role cannot reach `MessageStatus` (`daemon/trust.rs:25` classes it `AdminOrSession`), so no gateway pin exists.
2. Hold reason of a pending message in this priority: exited recipient none; `deliver: hold` gives `deliver_hold`; a PTY channel hold (`human_draft`, `unsubmitted_envelope`, `corrupted_submissions`, from `hold_name`); an earlier open message to the same recipient gives `queued`; otherwise the channel's unready reason, passed through unchanged (PTY `not_ready` and `cooldown`, the native reason such as `channel_down`, the Codex link reason). Each reason with its `hold_explanation` where one exists. A message that is not pending, or from an earlier run, has none. `cooldown` is a timing window with no test today; no pin is required for it (the pass-through is pinned by `not_ready` and `channel_down`).
3. Evidence of a submitted message: not observed `write_complete`; observed `native_receipt` for an OMP or Codex session or a channel session, else `submission_observed` (also when the recipient session no longer exists); any other state none.

An unreachable branch is reported, not pinned; no test hook is added.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon/views.rs`

- `fn may_read(&self, role: &Role, message: &Message) -> bool`: `match role { Role::Session(sender) => message.boot == self.boot && message.sender_session == *sender, Role::Admin | Role::Gateway => true }` (explicit arms, no wildcard, so a new role fails to compile until classified, as in `daemon/trust.rs`; this is an access check and stays the one place `message_status` passes through). One job: a session sees only its own messages of this run, the admin and gateway see all. `message_status` keeps its three `UNKNOWN_MESSAGE` returns (unparseable id, unknown id, and the third via `may_read`) and its single store read after the id parse; the guard becomes `if !self.may_read(role, &message) { return Ok(failed(code::UNKNOWN_MESSAGE, "unknown message")); }`.
- `async fn hold_reason(&self, session: &Arc<Session>, message: &Message) -> anyhow::Result<Option<&'static str>>`: today's `if let Some(session) = recipient { .. }` body, including the `has_earlier_open` store call before the exit/hold/channel checks (kept in that order), returning the same reason strings.
- `fn submission_evidence(&self, message: &Message) -> Option<&'static str>`: `None` unless the state is `Submitted`; else `write_complete`, or for an observed message `native_receipt` or `submission_observed` by today's rule, with its `shortcut:` comment. It does its own recipient lookup for the observed case: it must not reuse `message_info`'s `recipient`, which is `None` unless the message is pending (reuse would lose `native_receipt`).
- `message_info` keeps the pending-recipient lookup (`recipient`), calls `hold_reason` when there is one, builds the `MessageInfo`, and calls `submission_evidence`; every moved field keeps its source (compute both reasons before moving fields out of `message`).
- `message_status`, `list_messages` and every other function unchanged apart from the call shapes above.

If a finding remains, the developer quotes the cs text, reports the per-function cc and stops; no extra split without the architect. Hand count (cc, +1 per `if`/`if let`/`let else`/`&&`/`||`, `?` free; an operator inside a closure counts, which is what reproduces today's cc 12 of `message_info`): `may_read` 2, `message_status` 3, `hold_reason` about 5, `submission_evidence` about 5, `message_info` about 4.

## 4. Out of scope

`list_agents`, `explain_of`, `session_quota`, `list_sessions`, `session_working` and the activity helpers; `MessageInfo` and the wire types; the store; the CLI.

## 5. Trust boundary

No boundary is created or moved. The message-ownership check (`may_read`) is the one that stops a session reading another session's message or one from an earlier run; it stays in one place that `message_status` passes through and `list_messages` does not use (it lists by the caller's own filter, unchanged). Abuse-case coverage is Step A item 1 (another session's message and an earlier-run message both reply unknown).

## 6. Acceptance

Baseline (`12a4d18`): `cargo test` passes (1536 passed, 3 ignored); whole-tree cs count 851.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/views.rs
git status --short
```

- All pass; `cs check` lists no finding on `message_status` and `message_info` (the `:68`, `:156`, `:275` function findings and the file-level one remain, out of scope); the whole-tree count is below 851 (judge: the architect's `cs` run and `scripts/prepush-fast.sh`'s `cs delta`); new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources).
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hr/` (deleted before its REPORT); per mutant run `cargo test --lib` and the targets `delivery`, `claude_channel`, `bridge`, `codex`, `omp_smoke`, `human_inbound`, `attach_cli`, `mcp`, `wire`, `broker`, `daemon`, `messaging`, `quota`, `hook`, `status`, `table`. Mutants, one change each: (a) `may_read` ignores the boot check; (b) ignores the sender check; (c) returns `true` for sessions too; (d) `hold_reason` drops the exited check (expected non-kill: a pending message with an exited recipient exists only until the delivery loop's `fail_open`, and `recipient_exit_cancelling_and_filtering` asserts no hold only in that window); (e) swaps `deliver_hold` and the channel hold (expected non-kill unless the developer finds a `Deliver::Hold` session that also carries a PTY hold; every channel-hold pin uses `Deliver::Auto`); (f) drops `queued`; (g) drops the `unready_reason` fallback; (h) calls `has_earlier_open` only after the exit check (expected non-kill, an unobservable query order); (i) `submission_evidence` reports evidence for a non-submitted state; (j) drops the Omp/Codex arm; (k) drops the channel arm; (l) returns `native_receipt` for every observed message; (m) returns `write_complete` for an observed message. The tester names the killing test per mutant; an expected non-kill is reported with its reason; any other survivor in moved code is a fix request.

## 7. Claims (architect, `12a4d18`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `views.rs:395,405` | `cs check` and `cs review` run, output read |
| `message_info` has two callers (`message_status` at `views.rs:401`, `list_messages` at `:270`); `message_status` one caller | `views.rs:270,401`, `serve.rs:165` | grep over `src/` (`mcp.rs` and `codex.rs` only name the tool) |
| the hold priority and evidence rule above are today's behaviour | `views.rs:405-467` | read |
| PTY channel holds are `hold_name` strings (`human_draft`, `unsubmitted_envelope`, `corrupted_submissions`); `channel_down` is the native unready reason, not a hold; the native and Codex `hold_reason` return `None` | `delivery.rs:144,200,272,390-395`, `session/native.rs:136` | consultant read |
| the gateway cannot send `MessageStatus` | `daemon/trust.rs:25` | consultant read |
