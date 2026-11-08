# Spec 3di: An enabled agent can message the human, and the gateway can fetch and report those messages

## 0. Status

LOCKED (consultant review applied; base 9fefe91).

Scope (files that may be edited): `src/messaging.rs` (envelope reply line), `src/wire.rs` (two requests, two responses), `src/daemon.rs` (the human branch of `send_message`, the two gateway requests, the direction parameter of `remote_ineligible`, the in-memory attempt map, the delivery-time recheck closure), `src/delivery.rs` (the recheck hook in `run` and its five unit-test call sites), `src/mcp.rs` (instruction and tool text), `tests/human_inbound.rs`, `tests/wire.rs`, `tests/mcp.rs` (pins the full `send_message` description, `:234`), `tests/claude_channel.rs` (pins the full instruction text, `:248`). Does not touch: `src/store.rs`, `src/session.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/codex.rs`, the docs, `CHANGELOG.md`, `Cargo.toml`. The architect writes docs and changelog in a separate commit.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: a real daemon with admin, gateway and session connections (`tests/human_inbound.rs` helpers), the MCP tool parser tests where they exist.

Fifth slice of remote human messaging (F81, `docs/permissions.md` D1, D3, D7). Builds on 3df, 3dg, 3dh. After it the whole path works except the Discord process itself and the daemon-wide rate limits (3dj).

## 1. Why

D1: an agent addresses the human with the existing `send_message` to `human:owner`. D3: the same eligibility function runs at acceptance and again at delivery, in both directions; `send` and `both` agents may message the human. D7: states report only what is known (accepted, sent, failed; the stored failure state is `undeliverable`).

Alternatives rejected:

- A separate MCP tool for the human: a second tool means a second place that must apply the same checks; the address form already exists (D1).
- Giving the gateway the message store directly: the store is the daemon's; the gateway asks for the next message and reports the result.
- Free text from the gateway in a failure report: the detail is shown to agents through `message_status`; a fixed text cannot inject.

## 2. Changes

Claims (the consultant re-checks):

| claim | where | verified |
|---|---|---|
| `send_message` looks the recipient up with `recipient(to)`, applies `visible`, refuses self, refuses an exited recipient, applies the per-sender rate limit, then inserts | `src/daemon.rs:1176-1249` (recipient lookup at `:1186` runs before the sender lookup at `:1189`; the rate-limit block is inline at `:1201-1210`, timestamp pushed only on insert success at `:1227`) | read |
| the store keys pending messages by `recipient_session` and the per-recipient queue limit counts that key, so `human:owner` as the key gets the same limit | `src/store.rs:223`, `:239`, `:520-540` | read |
| a delivery goes `pending` → `delivering` (`begin_attempt`) → `submitted`/`unsubmitted`/`failed` (`finish_attempt`) | `src/store.rs:246-290`, `:549-600` | read |
| the per-session delivery loop is `delivery::run(channel, store, boot)`, spawned once per session | `src/delivery.rs:395`, spawned in `start_session_tasks(self: &Arc<Self>)` at `src/daemon.rs:773` | read |
| `Runtime::remote_ineligible(&self, session, now)` is the single eligibility function: exited, confined, mode (3dg), then the lease check inside the `Both` arm only (3dh) | `src/daemon.rs` (grep `fn remote_ineligible`) | read |
| the Human envelope has no reply line today; the agent branch is at `:478`, the Human branch at `:482-486`; its text is pinned at `src/messaging.rs:702-706` | read |
| the MCP instruction text and the `send_message` tool description are literals | `src/mcp.rs:457`, `:476` | read |
| `request_permitted` is exhaustive per role | `src/daemon.rs` | read |
| the stored state for a `failed` attempt outcome is `undeliverable` (`MessageState::Undeliverable`, `src/store.rs:575`); no state is named `failed`; restart sets detail `daemon_restarted` (`src/store.rs:468-476`) | read |
| `lookup("human:owner")` fails at its `s` prefix strip (`src/daemon.rs:3061`), so `message_info` gives no hold reason; `message_status` visibility uses `sender_session`; `fail_open(Some(session))` is called only from a session's own loop | read |
| `gateway_other_requests` (`tests/human_inbound.rs:867`) is the list of requests the gateway is REFUSED, asserted "not permitted for this token" at `:944-948`; the admin/session refusal test is `admin_and_session_tokens_cannot_speak_as_the_human` (`:964`) | read |
| `send_message` is session-only (admin gets "send_message needs a session token", `src/daemon.rs:2302-2305`); `create_confined_with_team` (`tests/human_inbound.rs:70`) runs `sleep 30` and returns only the id, no token | read |

### 2.1 Direction

`Runtime::remote_ineligible(&self, session, now)` gains a parameter `direction: Direction` (`ToAgent`, `ToHuman`, an enum in `messaging.rs`, matched without a wildcard). The mode rule: `ToAgent` passes only `RemoteMode::Both` (3dg); `ToHuman` passes `Send` and `Both`; no setting is refused in both directions with the existing text. The lease check, now inside the `Both` arm, moves after the mode match so that it applies to every mode that passed: a `Send` session needs a valid lease to message the human too, and the no-team refusal moves with the lease check so a session with no team is refused in both directions. Exited and confined checks are unchanged and shared. Text for a `Send` session refused inbound stays `remote messages to this session are not enabled`. Callers: `human_inbound` (`ToAgent`), the human branch of `send_message` (`ToHuman`), the two delivery-time rechecks below.

### 2.2 Agent to human

In `Runtime::send_message`, a branch for `to == messaging::HUMAN_SESSION` (exact string, no host suffix). It runs after `validate_message`; the sender lookup moves above the recipient lookup so the branch has the sender (the existing path is unchanged in behaviour). It refuses with `remote_ineligible(sender, ToHuman)`, then applies the per-sender rate limit, then inserts. The rate-limit block (window prune, cap check, timestamp pushed only after a successful insert, lock held across the insert) is extracted into one helper used by both the agent path and the human branch, not copied. The branch inserts with `recipient_session` and `recipient_address` both `human:owner`, `sender_*` as for any agent message, and answers `Response::Accepted { id, recipient_hold: None, recipient_quota: None }`. A full queue answers `QUEUE_FULL` as elsewhere. The human is not a session: no `visible`, no notify. The refusal texts for an ineligible sender reuse the `remote_ineligible` texts as they are ("recipient has exited" cannot occur for a live sender; the other texts read acceptably for a sender); the tests pin the exact strings. Note that all agents share one `human:owner` queue limit (`max_pending_per_recipient`); one chatty agent can fill it for the others; per-agent outbound limits are 3dj.

### 2.3 The gateway fetches and reports

`wire.rs`: `Request::HumanOutboundNext` (gateway only), `Request::HumanOutboundDone { id: String, sent: bool }` (gateway only), `Response::HumanOutbound { message: Option<HumanOutboundMessage> }` with `HumanOutboundMessage { id, from, subject, body }` where `from` is the sender's address and `id` the usual `m_N`. `request_permitted`: Gateway true for both, Admin and Session false, no wildcards.

- **Next.** Finds the oldest `pending` message for `human:owner` in the current boot (`next_pending`). For that message it looks up the sender session by id and runs `remote_ineligible(sender, ToHuman)`. If the sender is gone or ineligible, the message is moved through `begin_attempt` and `finish_attempt("failed", detail = "sender is no longer eligible for remote messaging")` (the stored state is `undeliverable`) and the next pending message is tried, up to the queue limit. For an eligible one it calls `begin_attempt`; if that returns `None` (another call took the message) it moves on to the next pending message instead of answering `None` or an error; otherwise it remembers `seq → attempt` in a mutex-guarded map on `Runtime`, and answers the message. No pending message answers `message: None`.
- **Done.** The map entry for `id` is taken; an unknown or already-reported id is `Response::Error` ("unknown message"). `sent: true` finishes the attempt as `submitted`; `sent: false` as `failed` (stored state `undeliverable`) with the fixed detail `the gateway could not send it`. The map guard is a std mutex taken, used to remove the entry, and dropped before any await. If `finish_attempt` then errors, the entry is put back so the gateway can retry `Done`. The gateway supplies no text.
- A gateway that stops between Next and Done leaves the message `delivering` until the daemon restarts (the existing startup path fails open messages). `// shortcut:` comment in the code naming this and the ceiling: each stuck message counts against the shared `human:owner` queue limit until a restart, so once the limit is reached every enabled agent gets `QUEUE_FULL`; the trigger is a stuck-delivery timeout if the gateway proves flaky.
- The states an agent sees through `message_status` are the existing ones: `pending` (accepted), `delivering`, `submitted` (sent to Discord), `undeliverable` (the detail carries the reason).
- Structure: the request arm only maps the result of `next_eligible_human(&self) -> Result<Option<(Message, i64)>>`; a small `fail_human_bound(seq, detail)` runs `begin_attempt` then `finish_attempt` for the failure paths. Each stays under the complexity limit; the developer states the hand count. No state claims the human read it.

### 2.4 Human to agent: recheck at delivery

`delivery::run` gains one parameter `recheck: RecheckFn`, `Arc<dyn Fn(&Message) -> Option<&'static str> + Send + Sync>`, called for a head message whose `sender_session` is `human:owner`, after the `has_earlier_open` check and before `channel.begin()` (so no ticket is held while a message is failed). When it answers `Some(detail)`, the loop runs `begin_attempt` then `finish_attempt("failed", Some(detail))` (stored state `undeliverable`), calls `channel.message_notify().notify_one()` as the loop does after a delivery, and moves on. The daemon builds the closure in `start_session_tasks` from `Arc::downgrade(self)` (an `Arc` would form a cycle through `Runtime.deliveries`): it finds the recipient session and returns the text of `remote_ineligible(recipient, ToAgent)`; a dead runtime or an unknown session returns a fixed text. The five existing unit-test call sites in `src/delivery.rs` pass a closure that always answers `None`; the Codex channel's own loop (`src/codex.rs:513`) is untouched and gets no human messages because Codex sessions cannot be confined (3dg).

### 2.5 Envelope and MCP text

- `render_envelope`, `Origin::Human`: the sentence gains a final sentence "To reply: send_message(to=\"human:owner\")." (`from` is the human session, so the same form as the agent branch). The mutation and the pure test pin the exact text.
- `src/mcp.rs`: the instruction text says a message from the operator arrives the same way and can be answered with `send_message` to `human:owner` when remote messaging is enabled; the `send_message` tool description gains one sentence naming `human:owner` and saying it works only for an agent the operator enabled. The developer finds any test that pins these strings (grep the old text across `src` and `tests` before editing).

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `send_message` to `human:owner` from a session with no remote setting | an agent the operator never enabled pushes text to the phone | constrain: `remote_ineligible(ToHuman)` | refused with the 3dg/3dh text, nothing stored |
| same, from an unconfined or exited sender | n/a for unconfined (cannot have a setting); exited cannot call | constrain: shared function | unit-level through the lease: a sender outside the lease is refused |
| same, no lease or an expired lease | message to the phone after the operator stopped | constrain: lease check | refused with the lease text |
| `send_message` to `human:owner@host` or a case variant | bypass the exact-string branch into session lookup | constrain: session names cannot contain `:`; lookup returns unknown recipient | `UNKNOWN_RECIPIENT` for the variants |
| spoofed `from` in a human-bound body | an agent claims to be another | constrain: the daemon fills `from` from the authenticated sender | `from` in `HumanOutbound` equals the sender's address |
| `HumanOutboundNext`/`Done` from admin or session | read the human queue, mark results | constrain: gateway only | refused for both |
| `Done` for an id it never fetched, twice, or for an agent-to-agent id | mark someone else's message | constrain: attempt map | unknown-message error for each; message states unchanged |
| a sender that became ineligible after acceptance | message leaves after the lease ended | constrain: recheck at Next | the message is `undeliverable` with the fixed detail and is never returned |
| human inbound that waited in the queue past the lease | delivered after expiry | constrain: recheck closure | with `lease_secs` small: accepted, lease expires before delivery, message ends `undeliverable` with the fixed text |
| failure detail | gateway text injected into `message_status` | remove: no gateway text | the detail equals the fixed text |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests red first.
2. `tests/human_inbound.rs`: one test per attack-path row above; plus the happy path: an enabled agent sends to `human:owner`, the gateway's Next returns it with the sender's address, Done with `sent: true` gives `submitted` through the sender's `message_status`, with `sent: false` gives `undeliverable`. A pure test pins the new envelope sentence. `admin_and_session_tokens_cannot_speak_as_the_human` gains both new requests (they are allowed for the gateway, so `gateway_other_requests`, the gateway's refused list, does NOT gain them; the gateway test shows Next working on its connection). The sender needs its own session token: a helper creates the confined session with argv `sh -c 'printf %s "$A2AMX_TOKEN" > token; exec sleep 30'` in the fixture cwd and reads the file; if default confinement cannot write the cwd or the token does not reach the daemon from inside the sandbox, the developer stops with BLOCKED. These tests return early without bwrap like the other confined ones.
3. `tests/wire.rs`: pins for the two requests and the response. `tests/mcp.rs` and `tests/claude_channel.rs` are updated to the new strings.
4. `tests/delivery`-level: an existing delivery unit test updated for the new parameter, plus one new unit test: a head from `human:owner` with a recheck answering `Some` ends `undeliverable` and the next message is delivered; a head from an agent is not rechecked. A test with `deliver` unset (not Hold).
5. Mutation check, each restored by checksum, naming the failing test: `ToHuman` allowing nothing; `ToHuman` requiring `Both`; direction ignored at inbound; recheck at Next removed; recheck at delivery removed; Done accepted without a map entry; admin allowed `HumanOutboundNext`; `from` taken from the body (not testable: no body field; the developer names it unkillable by design).
6. `git diff --stat` shows only section 0 files; `scripts/prepush-fast.sh` (architect); Sonar after the push (hand count of cognitive complexity for the Next function; keep it split into small functions).

## 5. Out of scope

The Discord process, daemon-wide rate limits and body-size splitting (3dj), `message_status` showing the human, showing the lease or the mode in `a2amx list`, per-agent shares of the shared `human:owner` queue, docs and changelog.
