# Spec 3hq: function-level code health of `src/daemon/reports.rs`

## 0. Status

LOCKED (amendment 1: section 2.1 session setup). Context `a2amx-3hq-daemon-reports-prompt-verdict-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `5302421`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`tests/common`, `Client`), the hook adapter (`tests/hook.rs`), the Claude channel (`tests/claude_channel.rs`).

Scope (edit): `src/daemon/reports.rs` (no `#[cfg(test)]` module), and tests only in `tests/delivery.rs` and `tests/claude_channel.rs` (Step A: new tests appended, one existing test strengthened in place), never a new file. Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same verdicts, reasons and texts, same store writes in the same order, same `note_*` and notify calls.

## 1. Why

`cs check` (`5302421`) reports on `reports.rs`: file-level "Overall Code Complexity"; `:121` "Complex Conditional" on `channel_prompt_verdict` (3 complex conditional expressions: the `if let` chain `let [seq] = .. && let Some(message) = .. && awaiting_receipt(..) && inner == ..`; `cs review` names that function; `awaiting_receipt` and the `draft` expression are not flagged today); `:153` `block_corrupted_submission` "Bumpy Road Ahead" (2 bumps: the envelope-stripping loop with its `complete` flag, and the reject loop with its `undeliverable` flag).

Rules that bind (earlier refactors 3hk-3hp): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

Existing pins (read by the architect and consultant; the developer confirms each by running it): mutant-killing tests that exist today are `mismatched_channel_prompts_allow_without_receipt_or_corruption` (`tests/claude_channel.rs:466`, killing a skipped channel-view comparison), `interleaved_corruption_redelivers_without_a_draft_hold` (`tests/delivery.rs:746`, a wrong `complete`), `two_rejections_retire_one_message_and_allow_the_next` (`delivery.rs:758`, the undeliverable reason), `two_known_envelopes_with_a_draft_block_and_requeue_both_messages` (`delivery.rs:481`, rejecting every known message), `wrapped_receipt_upgrades_evidence_and_duplicate_is_allowed`, `receipt_of_unsubmitted_envelope_clears_hold`, `human_prompt_clears_the_human_hold` (single envelope and no envelope paths), `leftover_wrapper_tags_are_never_restored_as_a_draft` (`delivery.rs:625`, which leaves both wrapper fragments at once). `report_quota` is unchanged and needs no pin.

Unpinned today; each gets a pin with literal expectations, green on the unchanged code, and small (a long test is a new cs Large Method; check `cs check` on the file before and after):

1. Skipped `awaiting_receipt` in `channel_prompt_verdict` (the store's `record_receipt` is a no-op on an observed or held message, so only a message in flight to another session shows it): new test in `tests/claude_channel.rs` (its Low Cohesion warning exists already, so a test adds none): a second Generic session with `Deliver::Auto`, started in the test with `Request::NewSession` and an `argv` that enables bracketed paste (`sh -c "printf '\\033[?2004h'; sleep 30"`; `common::new_agent` never enables it, so a Generic session made through it is never ready and its message stays `pending`, `harness.rs:436-438`), receives a message and reaches `delivering` or `submitted`; the channel recipient reports the exact channel view of that message; the verdict is `allow` and the message's evidence is unchanged.
2. Only the first envelope removed (`strip_envelopes`) and a draft that is not sanitised: strengthen `two_known_envelopes_with_a_draft_block_and_requeue_both_messages` in place: its human text gets a control byte (for example `my\x1bdraft`) and the test waits for the restored draft `\x1b[200~mydraft\x1b[201~` with `wait_rest`, as the other draft tests do. If that makes the test a cs Large Method, the new assertion goes into a second small test instead.
3. Each wrapper check alone: two small new tests in `delivery.rs` beside `leftover_wrapper_tags_are_never_restored_as_a_draft`: an open fragment only and a close fragment only left in the remainder; each asserts `block()`, the message state, and `!held`. They cannot share one case (`MAX_MESSAGE_REJECTIONS` is 2 and `MAX_CORRUPTED_SUBMISSIONS` is 3, so a third block holds the session). The developer first checks what `messaging::unwrap_pastes` does with each lone tag; a shape that cannot be built through the public seam is reported as an equivalent mutant, not forced.
4. `delivery.rs` is 925 lines; the three additions stay under about 60 lines so the file stays under 1,000. If they do not fit, the developer stops and asks; no new test file.

Pins that stay absent on purpose: clearing in-flight on the channel path (the channel does not wait on receipts, `claude_channel.rs:273`) and the multi-id case (it cannot match a channel view unless a body itself contains an envelope id): mutants (c) and (d) below are expected non-kills.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon/reports.rs`

- `channel_prompt_verdict`: early returns instead of the four-condition chain: `let [seq] = ids.as_slice() else { return Ok(()) };`, `let Some(message) = self.store.get(*seq).await? else { return Ok(()) };`, then `if awaiting_receipt(..) && inner == messaging::channel_view(&message.envelope()) { record_receipt; native_received }`. Same checks, same order, same store calls (`get` only for the single-id case, before the comparison).
- `fn strip_envelopes(text: String, known: &[(i64, String)]) -> Option<String>`: removes each known envelope from `text` in order; `None` as soon as one is missing (today's first loop and its `complete` flag, with the stop at the first missing envelope). No bool flag.
- `fn recoverable_draft(remainder: &str) -> Option<String>`: `None` when a `<pasted_content` or `</pasted_content` fragment remains, else `Some(messaging::sanitize_draft(remainder.trim()))`; the leftover-tag why-comment moves in with it (the `shortcut:` comment stays in `block_corrupted_submission`).
- `async fn reject_known(&self, known: Vec<(i64, String)>) -> anyhow::Result<bool>`: one `reject_attempt(seq, MAX_MESSAGE_REJECTIONS)` for every known message in order, never stopping early (no `any()` and no return on the first `Undeliverable`), `true` when any outcome was `Undeliverable`.
- `block_corrupted_submission`: `strip_envelopes(text, &known).and_then(|rest| recoverable_draft(&rest))`, `note_corrupted`, `reject_known`, the notify, and the `PromptVerdict` with the reason choice, in that order.
- `record_receipt`, `awaiting_receipt`, `report_prompt`, `report_quota`, `known_envelopes` unchanged.

If a finding remains (the file-level "Overall Code Complexity" may stay: `report_prompt` and `report_quota` carry most of the file's complexity and are out of scope), the developer quotes the cs text, reports the per-function cc and stops; no extra split without the architect. Hand count: `channel_prompt_verdict` cc 5, `block_corrupted_submission` 2, `strip_envelopes` 3, `recoverable_draft` 2, `reject_known` 3. Every new function has at most three parameters counting `self`.

## 4. Out of scope

Other `daemon/` files; the store and message states; `messaging` helpers; the hook binary; `report_quota`'s validation.

## 5. Trust boundary

No boundary is created or moved. The prompt is untrusted text from the session's own hook: the same checks (open message, addressed to this session, not yet observed, exact channel view) run before a receipt is stored, in the same order; the existing `shortcut:` comment about a forgeable prompt stays. Abuse-case coverage is the Step A pins (a prompt naming another session's message, a prompt with extra text, and a wrapper fragment never produce a receipt or a restored draft).

## 6. Acceptance

Baseline (`5302421`): `cargo test` passes (1533 passed, 3 ignored); whole-tree cs count 853.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/reports.rs
git status --short
```

- All pass; `cs check` lists no function finding on `reports.rs` (a remaining file-level finding is reported with the per-function cc); the whole-tree count is below 853 (judge: the architect's `cs` run); new tests add no cs warning (the architect runs `cs delta` through `scripts/prepush-fast.sh` too).
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources).
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hq/` (deleted before its REPORT); per mutant run `cargo test --lib` and the targets `hook`, `claude_channel`, `delivery`, `messaging`, `status`, `wire`, `human_inbound`, `quota`, `daemon`, `broker`, `presence`, `bridge`. Mutants, one change each: (a) `channel_prompt_verdict` skips the `awaiting_receipt` check; (b) skips the `inner == channel_view` comparison; (c) skips `native_received` (expected non-kill: the channel path does not wait on receipts); (d) accepts a prompt naming more than one id (expected non-kill: equivalent unless a body contains an envelope id); (e) `strip_envelopes` returns `Some` when an envelope was missing; (f) removes only the first envelope; (g) `recoverable_draft` ignores the `<pasted_content` check; (h) ignores the `</pasted_content` check; (i) skips `sanitize_draft`; (j) `reject_known` returns `false` always; (k) `reject_known` stops after the first message; (l) `block_corrupted_submission` skips the notify (the tester establishes from `delivery.rs` whether the delivery loop wakes on a tick; if so, expected non-kill). The tester names the killing test per mutant; an expected non-kill is reported with its reason; any other survivor in moved code is a fix request.

## 7. Claims (architect, `5302421`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `reports.rs:1,121,153` | `cs check` run, output read (line numbers: `channel_prompt_verdict` is `:114`, `block_corrupted_submission` `:153`; the Complex Conditional sits at `:121`) |
| `channel_prompt_verdict`, `known_envelopes` and `block_corrupted_submission` each have one caller (`report_prompt`) | `reports.rs` | grep |
| `record_receipt` has two callers outside this file (`bridge.rs`, `codex.rs`) | `daemon/bridge.rs`, `codex.rs` | grep |
| the two reason constants are used only in this function | `messaging.rs:53,55`, `reports.rs:194,196` | grep |
| `cs review` puts the Complex Conditional on `channel_prompt_verdict`; `awaiting_receipt` and the `draft` expression are not flagged | `cs review src/daemon/reports.rs` | run, output read |
| `delivery.rs` is 925 lines, `claude_channel.rs` already has a Low Cohesion warning, `delivery.rs` has none | `cs check` on both | run, output read |
| the existing killers listed in section 2 exist and cover the mutants named | `delivery.rs:481,625,746,758`, `claude_channel.rs:466` | consultant read; developer reruns |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-senior-developer` on the agy harness, Gemini 4 Hydrogen (Medium); tester `a2amx-tester` on the omp harness, `opencode-go/deepseek-v4.1-flash`.

BLOCKED/amend cycles: 1 (amendment 1).

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The spec claimed a third complex conditional that cs does not report; the `then(..)` form of the draft helper would keep two operators; the `complete` bool flag | Section 1 corrected from `cs review`; `strip_envelopes` returns `Option<String>`, `recoverable_draft` takes no flag |
| Draft review | consultant | Mutants (a), (f), (g), (h), (i) had no killing test; (c) and (d) unobservable; the quota pin item pointed at the wrong files | Pins specified (channel test, in-place strengthening, two wrapper tests); (c), (d) expected non-kills; the quota item dropped |
| Implementation | developer (BLOCKED) | Section 2.1 said `common::new_agent` makes a session that can reach `delivering`; it never enables bracketed paste, so a Generic session stays `pending` (`harness.rs:436-438`) | Amendment 1: the test starts the session itself with a bracketed-paste argv |
| Design read | consultant | The spec said both comments are `shortcut:`; only one is | Wording corrected here; the other comment is a why-comment inside `recoverable_draft` |
| Design read | consultant | The two wrapper tests lacked a why-comment | One-line comments added by the architect |
| Housekeeping | architect | 26 GB of finished specs' scratch (`target/scratch/3hm`, `3ho`) left by earlier passes | Deleted |

Tester mutants (12): (a), (b), (e) to (k) killed; (c), (d), (l) expected non-kills with reasons (the channel path does not wait on receipts; an envelope id in a body is needed; the delivery loop ticks every second). Old-source pins pass on the old code. Fresh checks by the architect: `cs` on `reports.rs` 9.38, Complex Conditional and Bumpy Road cleared, only the file-level finding left; whole-tree cs warnings 853 to 851.
