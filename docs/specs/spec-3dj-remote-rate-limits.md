# Spec 3dj: The daemon limits how fast the human and the agents can send each other messages

## 0. Status

DRAFT (architect, 2026-11-08). Base 37ca775.

Scope (files that may be edited): `src/messaging.rs` (two `Limits` fields, one size constant), `src/daemon.rs` (the rate helper, `human_inbound`, the human branch of `send_message`), `tests/human_inbound.rs`, `tests/messaging.rs` (the default-limits pin), `tests/broker.rs` mechanically (the `Limits { .. }` literals that name every field). Does not touch: `src/store.rs`, `src/session.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/mcp.rs`, `src/delivery.rs`, `src/wire.rs`, the docs, `CHANGELOG.md`, `Cargo.toml`.

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
| session ids look like `s<N>` and contain no `:`, so a window key containing `:` cannot collide with a session id | `src/daemon.rs` `lookup` (`strip_prefix('s')`), `src/messaging.rs` `validate_name` | read |
| `Limits` has four public fields plus the two added in 3dh; full `Limits { .. }` literals exist in `tests/broker.rs` at `:669`, `:979`, `:1041`; other uses are `..Limits::default()` | `rg "Limits \{" src tests` | read |
| `tests/messaging.rs:26` pins `Limits::default().rate_per_minute` | read |
| `validate_message` already enforces `MAX_MESSAGE_BYTES` (32 KiB) and returns `TOO_LARGE` | `src/messaging.rs:399` | read |
| `human_inbound` has no rate limit and no size limit beyond `validate_message`; a full recipient queue answers `QUEUE_FULL` and no test covers that for `human_inbound` | `src/daemon.rs`, `rg QUEUE_FULL tests/human_inbound.rs` | read, grep |

### 2.1 Limits

`messaging.rs`: `Limits` gains `human_inbound_per_minute: u32` (default 20) and `human_outbound_per_minute: u32` (default 30), and `pub const MAX_HUMAN_BODY_BYTES: usize = 8000` (a 2000-character chat message is at most 8000 bytes of UTF-8).

### 2.2 One helper, several windows

`insert_message_rate_limited` takes `windows: &[(&str, u32)]` (a window key and its cap) in place of `sender_id`. All windows must have room (prune each, then compare); on an insert that succeeds the timestamp is pushed to every window; on any failure (rate or insert) nothing is pushed. It still returns `Ok(None)` for a rate refusal, which each caller turns into its own text.

- Agent to agent: `[(sender_id, rate_per_minute)]`, unchanged behaviour.
- Agent to human: `[(sender_id, rate_per_minute), ("human:out", human_outbound_per_minute)]`; refusal text `sender rate limit reached` for the first window, `human outbound rate limit reached` for the second (the helper reports which window refused: it returns the refusing key's index or a small enum; the developer picks the shape and keeps the function under the complexity limit).
- Human to agent: `[("human:in", human_inbound_per_minute)]`; refusal code `rate_limited`, text `human inbound rate limit reached`. `human_inbound` today inserts with `self.store.insert_message` directly; it switches to the helper.

### 2.3 Inbound size

`human_inbound` refuses a body longer than `MAX_HUMAN_BODY_BYTES` with `TOO_LARGE` ("message exceeds the byte limit for a human message") before any lookup that reveals more than the existing order does: after `validate_message`, before the recipient lookup.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `HumanInbound` flood from the gateway or a hijacked phone | floods an agent's composer | constrain: `human:in` window | `human_inbound_per_minute` accepted, the next refused `rate_limited`, nothing stored for the refused one |
| `HumanInbound` oversized body | memory and composer flood | constrain: `MAX_HUMAN_BODY_BYTES` | a body of 8001 bytes refused `too_large`, 8000 accepted |
| one enabled agent floods the phone | spam, fills the shared queue | constrain: the per-sender window (unchanged) | the sender's own window refuses after `rate_per_minute` |
| several enabled agents together flood the phone | total flood | constrain: `human:out` window | with two senders and `human_outbound_per_minute = 2`, the third message from any sender refused `rate_limited` with the outbound text |
| a refused or failed insert counts against the window | a full queue locks senders out of the window | constrain: push only after success | with queue cap 1 and rate cap 2: accept, `queue_full`, cancel the first, then accept again (not `rate_limited`) |
| a window key collides with a session | an agent exhausts `human:in` | remove: keys contain `:`, session ids do not | covered by the claim; no separate test |
| `HumanInbound` into a full recipient queue | unbounded queue | constrain: existing queue limit | `queue_full`, nothing stored beyond the cap |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests red first (the QueueFull and window-recovery tests are contract tests on the old code: they pass there; say so in the report).
2. `tests/human_inbound.rs`: one test per attack-path row above through a real daemon with small limits (`start_confined_daemon_with_limits`), one daemon per scenario; confined tests return early without bwrap like the others.
3. `tests/messaging.rs`: the default-limits pin gains the two new defaults and the constant.
4. Mutation check, each restored by checksum, naming the failing test: inbound window removed; outbound total window removed; push moved before the insert result is known; body limit removed; helper ignoring the second window; limit compared with `>` instead of `>=`.
5. `git diff --stat` shows only the files in section 0 (`tests/broker.rs` only the added fields at the three literals); `scripts/prepush-fast.sh` (architect); Sonar after the push (the helper stays small; the developer states the hand count).

## 5. Out of scope

The Discord process and splitting long messages, per-agent shares of the `human:owner` queue, a stuck-delivery timeout, the failed-lease-code counter (3dh), docs and changelog.
