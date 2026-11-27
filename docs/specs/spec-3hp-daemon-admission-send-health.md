# Spec 3hp: function-level code health of `src/daemon/admission.rs`

## 0. Status

LOCKED after consultant review m_9686 (argument-count finding handled with `Outgoing` and a one-line `serve.rs` change, `quota` target added, notify mutants expected non-kills, owner-before-remote pin named, concrete existing pins listed, hand counts corrected). Context `a2amx-3hp-daemon-admission-send-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `169bbf9`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`tests/common`, `Client`).

Scope (edit): `src/daemon/admission.rs` (no `#[cfg(test)]` module), the one call site `src/daemon/serve.rs:149` (the `SendMessage` arm of `answer`) and, only if Step A finds an unpinned behaviour, tests appended to existing files named in Step A (`tests/broker/limits_and_lifecycle.rs`, `tests/human_inbound/rate_limits.rs`, `tests/presence/owner_presence.rs`, `tests/teams/team_visibility.rs`), never a new file. Does not touch: any other source or test file (`serve.rs` only at the one arm named), `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same responses and texts, same check order, same rate-limit windows, same notify calls, except the one log text named in section 3.

## 1. Why

`cs check` (`169bbf9`) reports on `admission.rs`: file-level "Overall Code Complexity"; `:23` `send_message` (Bumpy Road 3, cc 15, Large Method 124 lines) and, as a separate finding on the same function, "Excess Number of Function Arguments (Arguments = 5)" (`self`, `role`, `to`, `subject`, `body`). `send_message` does five jobs: resolve the sender, the human branch (remote eligibility, owner presence, two rate windows), the agent branch (recipient checks), the insert result mapping (written three times in the file) and the agent acceptance reply.

Rules that bind (earlier refactors 3hk-3ho): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

The developer first lists, per behaviour below, the test that already pins it (file and name). Existing pins (consultant grep, the developer confirms each): self send `tests/broker/limits_and_lifecycle.rs:40-45` (`unknown_recipient`); exited agent recipient `limits_and_lifecycle.rs:546-549` (`recipient_exited`); invisible recipient `tests/teams/team_visibility.rs:81,159,358`; `deliver_hold` on accept `limits_and_lifecycle.rs:61`; queue full on the agent path `limits_and_lifecycle.rs:70` and on inbound `tests/human_inbound/rate_limits.rs:177`; human windows `rate_limits.rs:323,393,448,501` (the sender window is preferred when both are full); the two owner-present texts `tests/presence/owner_presence.rs:49,238`; quota text on the accept reply `tests/quota/daemon_limits.rs:197`; inbound `recipient_exited`/`not_permitted` `tests/human_inbound/inbound_admission.rs:505-518`. Pinning by code is enough for self and exited (no listed mutant changes the text). No test names "a session cannot message itself", "recipient has exited", "sender session is unavailable" or "message storage failed" by text (grep of `tests/`); the developer checks whether a test pins their codes. A behaviour with no pin gets a new test with literal expectations, green on the unchanged code, appended to the existing file that owns the behaviour.

1. Agent send refusals in order: unknown recipient; not visible (same text); self ("a session cannot message itself", code `unknown_recipient`); exited recipient (`recipient_exited`, "recipient has exited"); a sender rate limit on the agent path ("sender rate limit reached", `rate_limited`).
2. Human send: remote ineligible verdict comes before the owner-presence verdict (no test combines a remote-ineligible sender with an owner present: new pin in `tests/presence/owner_presence.rs`, owner attached plus a remote-off sender, expecting the remote refusal code and not `owner_present`; those tests return early without `bwrap`, so the tester's host needs it for this pin and for mutants (d) and (f)); `Attached` and `Grace` texts; both rate windows (sender window first); queue full.
3. Agent acceptance reply: `recipient_hold` is `deliver_hold` for a `Hold` recipient, else the channel's hold reason; `recipient_quota` is "<window> quota exhausted" when exhausted. The `notify_one` calls are not pinned: the delivery loop wakes every second anyway (`delivery.rs`), so a removed notify only delays delivery and a timing pin would be flaky.
4. `human_inbound`: too large, unknown recipient, remote ineligible, window "human inbound rate limit reached", accepted reply with no hold or quota.
5. Unreachable by the public protocol, reported not pinned: "sender session is unavailable" (the serve loop closes exited sessions first), "message storage failed" (needs a failing store).

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon/admission.rs`

- `fn insert_outcome(result: Result<Result<i64, &'static str>, InsertError>) -> Result<i64, Response>`: the one mapping of the rate-limited insert result: accepted sequence, `rate_limited` with the window text, `queue_full` ("message queue is full"), internal ("message storage failed", logged with `tracing::error!`). Used by `send_to_human`, `send_message` and `human_inbound`, replacing three copies. One log-only change: `human_inbound` logged "human message acceptance failed"; all three now log "message acceptance failed".
- `fn owner_present_message(cause: messaging::PresenceCause, away_secs: u64) -> String`: the two refusal texts of today's `match cause`.
- `pub(super) struct Outgoing { pub(super) to: String, pub(super) subject: String, pub(super) body: String }`: what a session asked to send, built by `serve.rs` from the `SendMessage` request (`body` takes the request's `message`). `send_message(&self, role: &Role, outgoing: Outgoing)` replaces the four-value signature, which clears the argument-count finding; it destructures `Outgoing` first. The `serve.rs` arm becomes `Request::SendMessage { to, subject, message } => runtime.send_message(role, Outgoing { to, subject, body: message }).await` (`Outgoing` imported from `admission`).
- `async fn send_to_human(&self, sender: &Session, subject: String, body: String) -> Response`: the human branch of today's `send_message`, same order: `remote_ineligible`, owner presence (`owner_cause`, `OWNER_PRESENT` with `owner_present_message`), the `NewMessage`, the two windows (sender, then `human:out`), insert through `insert_outcome`, `Accepted` with no hold or quota. The sender id is `sender.id().0`, equal to the caller's `sender_id` because `lookup` matches on it.
- `fn routable_recipient(&self, sender: &Session, to: &str) -> Result<Arc<Session>, Response>`: `recipient`, visibility, self, exited, in today's order and with today's codes and texts.
- `fn agent_accepted(&self, recipient: &Arc<Session>, seq: i64) -> Response`: `recipient.message_notify().notify_one()` and the `Accepted` reply with hold and quota, exactly today's expressions.
- `send_message` keeps: `session_id`, `validate_message`, sender lookup, the human early return, `routable_recipient`, the agent `NewMessage` and window, `insert_message_rate_limited`, `insert_outcome` into `agent_accepted`.
- `human_inbound` replaces its result match with `insert_outcome` and keeps its notify and `Accepted` reply.
- `recipient`, `insert_message_rate_limited`, `fail_human_bound`, `next_eligible_human`, `human_outbound_next`, `human_outbound_done`, `cancel_message` unchanged.

If a finding remains (the file-level "Overall Code Complexity" may stay: it likely reflects the mean complexity per function, which smaller functions lower only slightly), the developer quotes the finding text, reports the per-function cc and stops; no extra split without the architect. Hand count: `send_message` cc about 7 (about 45 lines, no nesting), `send_to_human` about 4, `routable_recipient` 5, `agent_accepted` 2, `insert_outcome` about 4.

## 4. Out of scope

Other `daemon/` files; the rate-limit data structure; message states and the store; the wire `Response`; the human outbound side.

## 5. Trust boundary

No boundary is created or moved. The sender is still derived from the authenticated token (`session_id(role, ..)`), the recipient visibility check (`messaging::visible`) and the owner-presence and remote-eligibility refusals stay in the same order before any insert. Abuse-case coverage is the existing pins plus the Step A pins above (a session cannot reach an invisible recipient, nor itself, nor an exited one).

## 6. Acceptance

Baseline (`169bbf9`): `cargo test` passes (1531 passed, 3 ignored); whole-tree cs count 858.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/admission.rs
git status --short
```

- All pass; `cs check` lists no function finding on `admission.rs` (a remaining file-level finding is reported with the per-function cc); the whole-tree count is below 858 (judge: the architect's `cs` run); new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources).
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hp/` (deleted before its REPORT); per mutant run `cargo test --lib` and the targets `broker`, `messaging`, `human_inbound`, `presence`, `quota`, `teams`, `team`, `daemon`, `bridge`, `client_requests`, `gateway_up`, `delivery`, `mcp`. Mutants, one change each: (a) `routable_recipient` skips the visibility check; (b) skips the self check; (c) skips the exited check; (d) `send_to_human` checks owner presence before remote eligibility; (e) `send_to_human` drops the `human:out` window; (f) `send_to_human` swaps the `Attached` and `Grace` texts; (g) `insert_outcome` maps `QueueFull` to `rate_limited`; (h) `agent_accepted` omits `notify_one` (expected non-kill, see Step A item 3); (i) `agent_accepted` drops the `deliver_hold` rule; (j) `agent_accepted` drops the quota; (k) `human_inbound` no longer notifies the recipient (expected non-kill, same reason). The tester names the killing test per mutant; an expected non-kill is reported with its reason; any survivor in moved code is a fix request.

## 7. Claims (architect, `169bbf9`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `admission.rs:1,23` | `cs check` run, output read |
| the insert-result mapping occurs three times | `admission.rs:` the `send_message` human tail, its agent tail (about 122-145) and the `human_inbound` tail | read |
| `lookup` returns a session whose id equals the argument | `daemon.rs` `lookup` | read (`filter(|s| s.id().0 == id)`) |
| the texts "a session cannot message itself", "recipient has exited", "sender session is unavailable", "message storage failed" appear in no test | `tests/` | grep |
| `send_message` has one caller, `serve.rs:149` | `daemon/serve.rs` | grep `.send_message(` in `src` |
| `insert_message_rate_limited` has three callers (`send_message` twice, `human_inbound`) | `admission.rs` | read |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-senior-developer` on the agy harness, planning and working model Gemini 4 Hydrogen (Medium), no advisor (single-slot harness); tester `a2amx-tester` on the omp harness, planning and working model `opencode-go/deepseek-v4.1-flash` (advisor model not exposed).

BLOCKED/amend cycles: none. No developer fix round; the architect applied three changes after the REPORT.

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The draft kept `send_message`'s five-argument signature, so the argument-count finding could not clear and the one caller (`serve.rs:149`) was out of scope | `Outgoing` struct and the `serve.rs` arm added to scope |
| Draft review | consultant | Mutant (j) needed the `quota` target; (h) and (k) (`notify_one`) are unobservable (the delivery loop wakes every second); the owner-before-remote order was unpinned and needs `bwrap`; Overall Code Complexity was misdescribed | `quota` and `mcp` targets added, (h)/(k) expected non-kills, new pin named, existing pins listed by code, wording fixed |
| Implementation | architect | The developer's human-send queue-full pin added a new Low Cohesion warning to `tests/human_inbound/rate_limits.rs` | Test removed there, then re-added in `tests/human_inbound/outbound.rs` after the tester found its absence a survivor (no new warning) |
| Tester pass | tester | Mutant (m), `send_to_human` mapping `QueueFull` to `rate_limited` at its own site, survived: no test pinned the human-send queue-full reply | `outbound_respects_the_human_pending_queue_limit` in `outbound.rs` |
| Pre-push | architect (`cs delta` in `prepush-fast`) | The new owner-presence pin pushed `tests/presence/owner_presence.rs` over the Low Cohesion threshold (8 functions) | Pin moved to `tests/presence/remote_status.rs` (4 functions, no new warning); outside the test files named in Scope, a placement deviation |
| Tester pass | tester | Suggested asserting owner presence in the new presence pin | Not applicable: `list_agents` reports no owner for a remote-off sender (the assertion failed when tried); reverted |
| Design read | consultant | `routable_recipient` returned a boxed `Response` and `insert_outcome` likewise, boxing only to satisfy `clippy::result_large_err`; the spec claimed `Result<i64, Response>` | Both now return a `(code, message)` tuple like `remote_ineligible`; no `Box` remains; the spec's claim is corrected here |
| Design read | consultant | Nits left: `agent_accepted` could be named for its notify side effect; `owner_present_message` would sit better on `PresenceCause`; four copies of the address expression; the human queue-full test does not check stored count | Left: out of scope or cosmetic |

Tester mutants (scratch with its own `CARGO_TARGET_DIR`, deleted afterwards): 16 runs; (a) to (g), (i), (j), (l), (n) killed; (h) and (k) expected non-kills; (o) unreachable through the protocol; (p) equivalent (an exited session cannot be the sender); (m) survived and is fixed above. Fresh checks by the architect: `cargo test` 1533 passed, 3 ignored, `clippy -D warnings` and `fmt --check` clean, `cs` 10.00 on `admission.rs`, no new warning on the touched test files, whole-tree cs warnings 858 to 853.
