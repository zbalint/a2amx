# Spec 3dj: The daemon limits how fast the human and the agents can send each other messages

## 0. Status

LOCKED (consultant review applied; base 37ca775).

Scope (files that may be edited): `src/messaging.rs` (two `Limits` fields, one size constant), `src/daemon.rs` (the rate helper, `human_inbound`, the human branch of `send_message`), `tests/human_inbound.rs`, `tests/messaging.rs` (the default-limits pin), `tests/broker.rs` mechanically (the three `Limits { .. }` literals that name every field; the `Default` impl in `src/messaging.rs` also changes). Does not touch: `src/store.rs`, `src/session.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/mcp.rs`, `src/delivery.rs`, `src/wire.rs`, the docs, `CHANGELOG.md`, `Cargo.toml`.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: a real daemon with admin, gateway and session connections (`tests/human_inbound.rs` helpers).

Last slice of remote human messaging before the Discord gateway (F81, `docs/permissions.md` D6).

## 1. Why

D6: only the daemon's limits count. Today the human inbound request has no rate limit at all (a compromised gateway or phone could flood an agent), all agent messages to the human share one queue limit that one chatty agent can fill, and there is no byte limit tuned to a chat message. The failed lease-code counter that D6 asks for already exists (3dh: five wrong codes per issue, daemon-wide).

Alternatives rejected:

- A separate burst mechanism: the existing limit is a sliding 60 s window, which already lets `N` messages through at once and so is the burst allowance (a long reply split into several chat messages); a token bucket would be a second mechanism for the same effect.
- A per-agent share of the `human:owner` queue: would need store changes (`src/store.rs`); the total outbound window below bounds the flood, and the per-sender window bounds each agent.
- Splitting long agent messages in the daemon: Discord's 2000-character limit is a transport concern; the gateway splits (spec for the Discord process).

## 2. Changes

Claims (the consultant re-checks):

| claim | where | verified |
|---|---|---|
| the rate helper `Runtime::insert_message_rate_limited(sender_id, new)` keeps `HashMap<String, VecDeque<Instant>>` windows in `Runtime.rate`, prunes entries older than 60 s, compares `len() >= rate_per_minute`, inserts, then pushes the timestamp (only on success) | `src/daemon.rs` (grep `fn insert_message_rate_limited`) | read |
| rate keys are session ids minted as `format!("s{number}")` and taken from `Role::Session(id)`, never from a caller field, so a key containing `:` cannot collide | `src/daemon.rs:551`, `:2476-2486` | read |
| `Limits` has four public fields plus the two added in 3dh; full `Limits { .. }` literals exist in `tests/broker.rs` at `:669`, `:981`, `:1045`; other uses (`:2472`, `:2506`, `limits_with` in `tests/human_inbound.rs`, `src/main.rs`, `src/delivery.rs`) are `..Limits::default()` or `Limits::default()` | `rg "Limits \{" src tests` | read |
| `tests/messaging.rs:26` pins `Limits::default().rate_per_minute` | read |
| `validate_message` already enforces `MAX_MESSAGE_BYTES` (32 KiB) and returns `TOO_LARGE` | `src/messaging.rs:399` | read |
| `human_inbound` has no rate limit and no size limit beyond `validate_message`; a full recipient queue answers `QUEUE_FULL` and no test covers that for `human_inbound` | `src/daemon.rs`, `rg QUEUE_FULL tests/human_inbound.rs` | read, grep |

### 2.1 Limits

`messaging.rs`: `Limits` gains `human_inbound_per_minute: u32` (default 20) and `human_outbound_per_minute: u32` (default 30), and `pub const MAX_HUMAN_BODY_BYTES: usize = 8000` (a 2000-character chat message is at most 8000 bytes of UTF-8).

### 2.2 One helper, several windows

`insert_message_rate_limited` takes `windows: &[(&str, u32, &'static str)]` (window key, cap, refusal text) in place of `sender_id`. All windows must have room (prune each, then compare); on an insert that succeeds the timestamp is pushed to every window; on any failure (rate or insert) nothing is pushed. For a rate refusal it returns `Ok(Err(text))` with the text of the first full window in list order (the sender's own window before `human:out`), and the caller answers `failed(code::RATE_LIMITED, text)`. The helper prunes and checks in a loop that reborrows `rate.entry(key)` for each key, then pushes in a second loop after the insert.

- Agent to agent: `[(sender_id, rate_per_minute)]`, unchanged behaviour.
- Agent to human: `[(sender_id, rate_per_minute), ("human:out", human_outbound_per_minute)]`; refusal text `sender rate limit reached` for the first window, `human outbound rate limit reached` for the second (carried in the window tuple).
- Human to agent: `[("human:in", human_inbound_per_minute)]`; refusal code `rate_limited`, text `human inbound rate limit reached`. `human_inbound` today inserts with `self.store.insert_message` directly; it switches to the helper.

### 2.3 Inbound size

`human_inbound` refuses a body longer than `MAX_HUMAN_BODY_BYTES` with `TOO_LARGE` ("message exceeds the byte limit for a human message") immediately after `validate_message` and before `self.recipient(&to)`; an oversized body to an unknown recipient therefore answers `too_large`. Refused requests (unknown recipient, ineligible recipient, oversized body) do not count against `human:in`: the window counts accepted inserts only.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `HumanInbound` flood from the gateway or a hijacked phone | floods an agent's composer | constrain: `human:in` window | with `human_inbound_per_minute = 2` and `max_pending_per_recipient >= 3`: two accepted, the third refused `rate_limited` with the inbound text, nothing stored for it |
| `HumanInbound` oversized body | memory and composer flood | constrain: `MAX_HUMAN_BODY_BYTES` | a body of 8001 bytes refused `too_large`, 8000 accepted |
| one enabled agent floods the phone | spam, fills the shared queue | constrain: the per-sender window (unchanged) | with `rate_per_minute = 2`, the default `human_outbound_per_minute` (30) and `max_pending_per_recipient >= 3`: the third from one sender is refused with the sender text |
| several enabled agents together flood the phone | total flood | constrain: `human:out` window | with `rate_per_minute >= 2`, `human_outbound_per_minute = 2`, `max_pending_per_recipient >= 3` and two senders (both team members before the lease is redeemed, because membership is fixed at redeem): the third message from any sender is refused with the outbound text; when both windows are full the sender text is reported |
| a refused or failed insert counts against the window | a full queue locks senders out of the window | constrain: push only after success | inbound: `human_inbound_per_minute = 2`, `max_pending_per_recipient = 1`, recipient delivery on hold: accept, `queue_full`, the admin cancels the first message (`CancelMessage` is admin only and works on a pending message), then accept again (not `rate_limited`); an outbound copy only if the gateway has not fetched the message (a fetched message leaves `pending` and cannot be cancelled) |
| a window key collides with a session | an agent exhausts `human:in` | remove: keys contain `:`, session ids do not | covered by the claim; no separate test |
| `HumanInbound` into a full recipient queue | unbounded queue | constrain: existing queue limit | `queue_full`, nothing stored beyond the cap |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests red first (the QueueFull and window-recovery tests are contract tests on the old code: they pass there; say so in the report).
2. `tests/human_inbound.rs`: one test per attack-path row above through a real daemon with small limits (`start_confined_daemon_with_limits`), one daemon per scenario, with the limit values named in each row; test bodies contain no control characters other than newline and tab and no envelope tag; senders get tokens from `create_confined_with_token` with `deliver: Some(Hold)`; confined tests return early without bwrap like the others.
3. `tests/messaging.rs`: the default-limits pin gains the two new defaults and the constant.
4. Mutation check, each restored by checksum, naming the failing test: inbound window removed; outbound total window removed; push moved before the insert result is known; body limit removed; helper ignoring the second window; limit compared with `>` instead of `>=`.
5. `git diff --stat` shows only the files in section 0 (`tests/broker.rs` only the added fields at the three literals); `scripts/prepush-fast.sh` (architect); Sonar after the push (the helper stays small; the developer states the hand count).

## 5. Out of scope

The Discord process and splitting long messages, per-agent shares of the `human:owner` queue, a stuck-delivery timeout, the failed-lease-code counter (3dh), counting refused requests (a hijacked gateway can probe agent names through `unknown_recipient` and `not_permitted` answers at an unlimited rate), docs and changelog.

## Review record

Models: architect and spec author Claude Sonnet 5.5; consultant (pre-lock review) model not reported; developer and tester openai-codex/gpt-5.6-luna (planning and working), advisor slot model not exposed.

BLOCKED/amend cycles: 0.

| found at | by | problem | resolution |
|---|---|---|---|
| draft review | consultant | spec drafting findings (window tuple shape, refusal text carried per window, order of the size check, test limit values) | spec amended before lock |
| draft (architect) | architect | a stale draft was committed as 96bd207 after a failed edit script; the locked text is 6ee4f19 | superseded by 6ee4f19, both pushed together |
| tester pass | tester | no test fills both outbound windows, so a reversed window order survived | test `outbound_refusal_prefers_sender_when_both_windows_are_full` added by the developer, mutant killed |
| tester pass | tester | no test showed a rate refusal does not push a timestamp, so a pushing mutant survived | test `outbound_rate_refusal_does_not_push_a_timestamp` added (about 61 s of real time), mutant killed; the confined sender fixture sleeps 90 s instead of 30 s so it survives that test |
| tester pass | tester | key isolation of `human:out` is caught only by the per-sender sibling test | accepted: killed by a sibling |
| tester pass | tester | no multibyte test at the 8000-byte boundary; no 60 s expiry probe beyond the new test | accepted gaps: the check uses `body.len()` (bytes) |
