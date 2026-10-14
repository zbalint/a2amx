# Spec 3ac: split `Runtime::report_prompt` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14). Architect-directed, backlog I5 part 4, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Sequencing:** spec 3ab (same file) is accepted and committed, so `src/daemon.rs` has one writer. At the baseline below `report_prompt` starts at line 1065; the developer re-reads the anchors if the tree moved.

**Baseline:** develop at `28cf1b8` (the locking commit follows it and changes only this file); at that commit `cargo test --test delivery --test hook --test claude_channel` passes 23, 8 and 13 tests (44 of 44). The developer records `cargo test --test delivery --test hook --test claude_channel` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the `report_prompt` wire request through a real daemon (`tests/delivery.rs`, `tests/hook.rs`,
`tests/claude_channel.rs`). New helpers are private to `src/daemon.rs`.

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: `report_prompt` and new private `&self` helpers directly after it (inside `impl Runtime`)
- `tests/delivery.rs`: only characterization tests of section 4

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `report_prompt` (42) against 30 (backlog I5). It handles a channel-delivered prompt, collects the known
envelopes in a prompt, and either records a receipt or blocks a corrupted submission, in one function with a three-level
nested condition repeated twice.

## 2. Rule: behavior is unchanged

Same `Response::PromptVerdict` (verdict and reason) for the same prompt and store state; same store calls in the same order
(`get`, `record_receipt`, `reject_attempt` per known message in order); same session notifications (`native_received`,
`note_submit`, `note_corrupted`, `message_notify`). Keep both `// shortcut:` comments with the code they annotate. No
`pub` change.

## 3. Changes in `src/daemon.rs`

Keep `report_prompt`'s signature, the `lookup` of the session and its error text `session is unavailable`. Today's order, which
must not change: (1) channel branch: only if `session.channel()` and `unwrap_channel(&prompt)` is `Some(inner)`: if exactly one
envelope id, fetch the message, and when it is deliverable AND `inner` equals the channel view of the re-rendered envelope,
record the receipt and call `session.native_received()`; then ALWAYS return `allow` (even when nothing matched); (2)
otherwise `unwrap_pastes`, then collect known envelopes: each id's message that is for this boot and session, in state
`Delivering`/`Submitted`/`Unsubmitted`, not observed, rendered through `harness::paste_view`; (3) no known: `note_submit(false)`,
verdict `allow`; (4) exactly one known and the text equals it: `record_receipt`, `note_submit(true)`, `allow`; (5) otherwise strip
each known envelope from the text (first occurrence, in order, stopping at the first one not found), compute `draft`, call
`note_corrupted(draft)`, `reject_attempt` for each known message in order, `message_notify`, and return `block` with
`UNMATCHABLE_SUBMISSION_REASON` if any rejection was `Undeliverable`, else `CORRUPTED_SUBMISSION_REASON`.

Extract:

1. `fn is_awaiting_receipt(&self, message: &Message, session_id: &str) -> bool`: the condition both branches repeat (same
   boot, same recipient session, state one of the three, not observed). The channel branch adds its `inner == channel_view(..)`
   test AFTER this one; keep that extra test at the call site. `Message` is `store::Message`, already imported in `daemon.rs`; fields `boot`, `recipient_session`, `state`, `observed`. The four tests run in this order: boot, recipient, state, not observed; at the channel call site `is_awaiting_receipt(..) && inner == channel_view(..)` is exact.
2. `async fn channel_prompt_verdict(&self, session: &Arc<Session>, session_id: &str, inner: &str) -> anyhow::Result<()>`: step 1's
   body (single-id check, `get`, receipt, `native_received`). The caller returns the `allow` verdict afterwards.
3. `async fn known_envelopes(&self, session: &Arc<Session>, session_id: &str, text: &str) -> anyhow::Result<Vec<(i64, String)>>`:
   step 2's loop (`envelope_ids`, `get`, `is_awaiting_receipt`, `paste_view`). `?` on a store error propagates as today.
4. `async fn block_corrupted_submission(&self, session: &Arc<Session>, text: String, known: Vec<(i64, String)>) -> anyhow::Result<Response>`:
   step 5. `text` is compared with `known[0].1` BEFORE it moves into this helper. The `draft` condition is `complete` AND the remainder contains neither the
   opening NOR the closing pasted-content wrapper tag (two separate `contains` tests; the closing tag does not contain the opening one, so both are
   required), then `sanitize_draft(remainder.trim())`. Order inside: strip loop over `&known`, `draft`, `note_corrupted(draft)`, `reject_attempt` for EVERY known
   message in order with no early exit when one is `Undeliverable`, `message_notify().notify_one()`, return the block verdict. `record_receipt` returns a
   bool today that is discarded with `?;`; keep discarding it.

Comments: the forgery `// shortcut:` comment (a session-token holder can forge prompts) sits directly above the step-2 loop and moves into `known_envelopes`;
the plain comment about the hook seeing the envelope as the harness transformed it on paste moves with the `paste_view` call; the interleaved-envelopes
`// shortcut:` comment moves into `block_corrupted_submission`.

## 4. Safety net

Already covered (checked at `0ce2ee4`): the channel branch (`tests/claude_channel.rs`: `exact_channel_prompt_records_native_receipt_and_allows`,
`mismatched_channel_prompts_allow_without_receipt_or_corruption` for non-matching, zero-id, unknown-id and two-id prompts,
`human_prompt_on_channel_session_allows_without_receipt`); step 3 (`tests/delivery.rs`: `unknown_envelope_is_allowed_without_changing_message`,
`human_prompt_clears_the_human_hold`); step 4 (`wrapped_receipt_*`, `trimmed_wrapper`, `receipt_of_unsubmitted_envelope_clears_hold`,
`tests/hook.rs::hook_exact_wrapped_envelope_records_receipt_and_cli_detail`); step 5 with one known envelope (`corruption_restores_draft_without_cr_and_release_redelivers`,
`interleaved_corruption_redelivers_without_a_draft_hold`, `leftover_wrapper_tags_are_never_restored_as_a_draft`,
`two_rejections_retire_one_message_and_allow_the_next` for the `Undeliverable` reason, `tests/hook.rs::hook_corrupted_submission_blocks_with_fixed_reason`);
`foreign_session_cannot_forge_receipt` for a recipient mismatch.

Not covered, and the one test to add (in `tests/delivery.rs`, passing on the unchanged code, with the exact verdict and reason literals): step 5 with TWO known
envelopes in one non-channel prompt (two messages Submitted at once, see `two_messages_submit_in_acceptance_order`): report both wrapped envelopes
concatenated plus extra text; expect `block` with `CORRUPTED_SUBMISSION_REASON` and both messages requeued. A boot mismatch and store errors need store states the
helpers cannot reach; they rely on reading the diff, which the report must say.

## 5. Out of scope

Everything else in `daemon.rs`, `messaging.rs`, `store.rs`, the hook adapter.

## 6. Acceptance

```sh
cargo test --test delivery --test hook --test claude_channel
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/daemon.rs and, if added, tests/delivery.rs
```

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
and `attach_does_not_capture_session_already_in_drag_mode` (backlog I7): rerun once, record as pre-existing. Final proof of the
complexity fix is the Sonar rescan after the owner pushes.
