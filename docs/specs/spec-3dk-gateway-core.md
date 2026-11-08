# Spec 3dk: The gateway core, a transport-free library that carries messages between the daemon and a chat

## 0. Status

LOCKED (consultant review applied; base 7ab0e72; amendment 1: `NeedChannel` carries the message id).

Scope (files that may be edited): `src/gateway.rs` (new), `src/lib.rs` (one `pub mod gateway;` line), `src/wire.rs` (one request, one response, one summary type), `src/daemon.rs` (the three role rows, the handler), `tests/gateway.rs` (new), `tests/common/mod.rs` (only helpers moved out of `tests/human_inbound.rs`, see section 4), `tests/human_inbound.rs` (those moved helpers removed, and `gateway_other_requests` (:1378) and the admin and session lists (about :1519 and :1574) updated for the new request), `tests/wire.rs` (pins for the new types). Amendment 1 (tester pass): `Next::NeedChannel` gains the held message's `id`, so a caller can give up; the developer also merges the two near-identical resolve paths in `outbound` (a held message and a newly fetched one) into one path and removes the one-field `Held` wrapper.

Does not touch: `src/main.rs`, `src/cli.rs`, `src/client.rs`, `src/store.rs`, `src/messaging.rs`, `src/mcp.rs`, `src/delivery.rs`, `Cargo.toml` (no new dependency), the docs, `CHANGELOG.md`, `AGENTS.md`.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: a real daemon with admin, gateway and session connections, and the new `a2amx::gateway` API.

First of four gateway specs: 3dk core (this), 3dl the Discord connection and `a2amx gateway up`, 3dm the gateway's confinement, 3dn the setup guide.

## 1. Why

The daemon side of remote messaging is built (3dg to 3dj), but nothing speaks to a chat. The gateway has to do five things that do not depend on which chat it is: keep only messages from the paired person, send each message to the right agent from the right channel, forget nothing twice (a reconnect must not deliver a message again), cut long agent messages into pieces a chat accepts, and turn every daemon refusal into a short fixed line. Doing this first, with no network code, lets it be tested end to end against a real daemon with a plain struct standing in for Discord.

The gateway also cannot see which agents are enabled: its role may not list agents, and redeeming a lease code answers only "ok". Without that it cannot know which channels to make. This spec adds one gateway-only request that lists exactly the agents that may exchange messages right now.

Alternatives rejected:

- A `Transport` trait the core calls: async methods in a public trait trip `clippy::async_fn_in_trait` under `-D warnings`, and a suppression is not allowed. The core does the daemon requests and the dedup file, and takes and returns plain values on the chat side; the caller (3dl) does the chat network.
- Letting the gateway use `ListAgents`: that gives it every session, cwd, quota and harness, far more than it needs (D2: own fail-closed allowlist).
- Storing the last-seen ids in `messages.db`: changes the public schema; a small 0600 file in the gateway's own state is enough (D8 allows either).
- Splitting at a fixed length only: a cut in the middle of a word is hard to read on a phone; prefer a newline, then a space.

## 2. Changes

Claims (the consultant re-checks):

| claim | where | verified |
|---|---|---|
| the gateway role's allowlist is `HumanInbound`, `HumanOutboundNext`, `HumanOutboundDone`, `RedeemLeaseCode`; every other request is refused, with no wildcard | `src/daemon.rs` `request_permitted` (grep `Role::Gateway =>`) | read |
| `Request` variants listed per role in three exhaustive matches (Admin, Gateway, Session) | `src/daemon.rs` near `request_permitted` | read |
| `RedeemLeaseCode` answers `Response::Ok` or `Response::Error`, with no team name | `src/daemon.rs` `redeem_lease_code` | read |
| `HumanOutboundMessage` has `id`, `from` (the sender's address), `subject`, `body` | `src/wire.rs` | read |
| `HumanInbound { to, body }` takes the recipient's name or address, answers `Accepted` or `Failed { code, message }` | `src/wire.rs`, `src/daemon.rs` `human_inbound` | read |
| `Runtime::lease_valid(team, id, now)` and session accessors `remote()`, `team()`, `confined()`, `exit_code()`, `name()`, `id()` exist; `messaging::address(name, id, host)` builds an address | `src/daemon.rs`, `src/messaging.rs:371` | read |
| `Client::connect_addr(addr, token)` gives a connection of a given token; `Client::request` sends one request | `src/client.rs:67`, `:89` | read |
| gateway token is `gateway.token` in the state directory, `addr` holds the address | `src/daemon.rs` (grep `gateway.token`), `src/client.rs:46` | read |
| `tests/human_inbound.rs` holds the daemon and session helpers: `new_session_request` (:18), `new_session_request_with_remote` (:22), `start_confined_daemon_with_limits` (:47), `create_confined_with_team`, `create_confined_with_token`, `gateway` (:213), `error_message`, `failed_code`, `team_scope`, `issue_code`, `issue_and_redeem`, `limits_with` (to :284) | `tests/human_inbound.rs:18-284` | read |
| `tests/common/mod.rs` already allows dead code, so a helper used by only one test file does not trip clippy | `tests/common/mod.rs:4` | read |
| the store maps a gateway `sent: false` report to the stored state `undeliverable` | `src/store.rs:575` | read |
| `team up` already maps the modes to the labels `send` and `both` | `src/daemon.rs:1363-1367` (`issue_lease_code`) | read |
| `gateway_other_requests()` is the gateway's refused list | `tests/human_inbound.rs:1378` | read |
| the codes `unknown_recipient`, `recipient_exited`, `too_large`, `queue_full`, `rate_limited`, `not_permitted` are constants in `messaging::code` | `src/messaging.rs:210-218` | read |
| the human subject constant is `messaging::HUMAN_SUBJECT` (`remote message`) | `src/messaging.rs:13` | read |
| `tests/common/mod.rs` holds `start_daemon_in`, `new_agent`, `new_team_agent` and the process helpers, none of the confined-session helpers listed above | `tests/common/mod.rs` | read |

### 2.1 Daemon: `GatewayAgents`

`wire.rs`: `Request::GatewayAgents` (no fields), `Response::GatewayAgents { agents: Vec<GatewayAgent> }`, and `GatewayAgent { address: String, name: String, team: String, mode: String }` where `mode` is `send` or `both`. `name` is the local part of the address. The new request and response are additive (older clients ignore what they do not send).

`daemon.rs`: allowed for `Role::Gateway` only; `false` for `Role::Admin` and `Role::Session` (all three matches stay exhaustive, no wildcard). `mode` reuses the label mapping `issue_lease_code` already has. The handler checks `exit_code` itself (a lease keeps an exited member's id valid). It returns one entry for each session that is running, confined, has a remote mode, has a team and has a valid lease (`lease_valid`) now. It returns nothing else: not the cwd, harness, quota or state.

### 2.2 `src/gateway.rs`

Owns: the chat-independent rules of the gateway. It names no chat service and does no network or file IO other than the dedup file (section 2.3).

```rust
pub struct Event { pub channel: String, pub message_id: u64, pub author: String, pub text: String }
pub struct Outbound { pub id: String, pub channel: String, pub pieces: Vec<String> }
pub struct Gateway { /* paired author, control channel, channel map, seen ids, state path */ }

impl Gateway {
    pub fn new(state_path: PathBuf, paired_author: String, control_channel: String) -> anyhow::Result<Self>;
    pub async fn agents(&self, daemon: &mut Client) -> anyhow::Result<Vec<GatewayAgent>>;
    pub fn bind_channel(&mut self, channel: String, address: String) -> anyhow::Result<()>;
    pub fn unbind_channel(&mut self, channel: &str);
    pub fn channel_for(&self, address: &str) -> Option<&str>;
    pub async fn inbound(&mut self, daemon: &mut Client, event: Event) -> anyhow::Result<Option<String>>;
    pub async fn outbound(&mut self, daemon: &mut Client) -> anyhow::Result<Next>;
    pub async fn outbound_done(&mut self, daemon: &mut Client, id: &str, sent: bool) -> anyhow::Result<()>;
}
pub enum Next { Idle, Send(Outbound), NeedChannel { id: String, agent: GatewayAgent } }
pub const UNREACHABLE_LINE: &str = "a2amx is not reachable right now";
pub fn split_message(text: &str, max_chars: usize) -> Vec<String>;
pub const MAX_CHAT_CHARS: usize = 2000;
```

`bind_channel` adds or replaces a binding and refuses (`Err`) the control channel, which is never bound. Bindings are kept in memory only; the caller rebuilds them after a restart. `inbound` returns `Some(line)` when the caller should post a fixed line in the event's channel, `None` when nothing is to be posted. Rules, in this order:

1. `event.author != paired_author`: drop it, return `None`. Nothing is recorded and nothing is sent to the daemon.
2. `event.message_id <=` the stored last id for `event.channel`: drop it, return `None` (the caller contract: events of one channel arrive in increasing id order; an older event handled after a newer one is dropped, which loses a message and never repeats one). Otherwise store `message_id` and write the file before anything else happens (at most once: a crash can lose a message but never repeat one).
3. The control channel: trim the text; if it is exactly 12 characters of `0-9a-f` send `RedeemLeaseCode` and answer `remote messaging is on` for `Ok` or `that code is not valid` for any other `Response`; any other text answers `send the code that a2amx team up printed`.
4. A bound channel: send `HumanInbound { to: address, body: text }`. `Accepted` returns `None`. A `Failed` code is mapped to a fixed line: `not_permitted` to `that agent is not taking messages from you`; `rate_limited` to `too many messages, wait a minute`; `too_large` to `that message is too long`; `unknown_recipient` and `recipient_exited` to `that agent is not running`; `queue_full` to `that agent has too many unread messages`; any other code, and any other `Response` (such as `Error`), to `the message could not be delivered`. The daemon's own text is never copied into a line. A transport error talking to the daemon (rules 3 and 4) is returned as `Err` after the id is recorded: the caller must not replay the event and posts `UNREACHABLE_LINE` instead. To keep `inbound` small each rule step is its own private function (`control_line`, `deliver_line`, `refusal_line(code)`).
5. Any other channel: return `that channel is not linked to a running agent`.

`outbound` fetches with `HumanOutboundNext` (or first re-examines the one message it is holding, below). A message whose `from` has a bound channel is returned as `Next::Send(Outbound)` with its text cut by `split_message(.., MAX_CHAT_CHARS)`. A message whose `from` has no bound channel is looked up in `GatewayAgents`: if the sender is listed, the gateway holds the message (it stays fetched at the daemon) and returns `Next::NeedChannel { id, agent }` (`id` is the held message's id); the caller creates the channel, calls `bind_channel` and calls `outbound` again, or gives up with `outbound_done(id, false)`, which also drops the held message. If the sender is not listed the message is reported `HumanOutboundDone { sent: false }` at once and the loop continues. `Next::Idle` when the daemon has nothing and nothing is held. A gateway crash while holding leaves the message fetched at the daemon (recovery of that is out of scope). The text is `body` when `subject` is empty or equal to `messaging::HUMAN_SUBJECT`, otherwise `**subject**` on its own line, then the body. The caller sends every piece and then calls `outbound_done`.

`split_message` cuts at the last newline in the window, else the last space, else at the character limit, never inside a character; every piece has at most `max_chars` characters (Unicode scalar values; 3dl checks whether the chat counts differently), no piece is empty, and joining the pieces (with the cut whitespace) loses no non-whitespace character.

### 2.3 The dedup file

`state_path` holds a JSON object from channel id to the last handled message id (both decimal strings). A write flushes the temp file (`sync_all`) before the rename and the directory after it; a file that is empty or invalid after a crash makes the next start fail, which is the intended fail-closed outcome. `new` reads it when it exists (at most 1 MiB; a larger, unreadable or invalid file is an error and the gateway does not start) and treats a missing file as empty. A write goes to a temp file next to it with mode 0600, then renames over it; on Unix the final file is 0600.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| a chat message from someone other than the paired author | speaks to agents or starts a lease | constrain: author check first | a message from another author sends nothing to the daemon, posts nothing and is not recorded |
| a replayed or redelivered event | the same prompt twice, a lease code replayed | constrain: stored last id, written before the daemon is called, kept across a restart | the same event twice is sent once; a new `Gateway` from the same file still drops it |
| a corrupt or oversized dedup file | resets the ids so old messages replay | constrain: refuse to start | an invalid file and a file over 1 MiB make `new` fail |
| a message in channel A | reaches agent B | constrain: the recipient comes only from the channel map | with A and B bound, a message in B reaches only B's session and a message in A only A's (the other queue stays empty each time) |
| a message in an unbound channel | reaches an agent | remove: fixed line, nothing sent | nothing reaches the daemon |
| the daemon's refusal text | text with markup or secrets reaches the chat | constrain: fixed lines only | a refusal's own text is not in the returned line, for each mapped code and for an unmapped one |
| `GatewayAgents` | the gateway or another role learns more than it should | constrain: gateway only, enabled and leased agents only, four fields | admin and session tokens are refused it; the list omits an agent without a lease, one with no remote mode, an unconfined one and an exited one |
| a very long agent message | refused by the chat or truncated silently | constrain: split into pieces of at most 2000 characters | pieces are within the limit and lose no text |
| an agent message with no channel yet | lost, or stuck in `delivering` | constrain: a listed sender gets `NeedChannel` and a retry, an unlisted one is reported failed | a message from a listed agent with no channel returns `NeedChannel`, and after `bind_channel` is returned as `Send` and ends `submitted`; a listed agent whose caller gives up with `outbound_done(id, false)` ends `undeliverable` and a following `outbound` does not return it again; one from an unlisted sender ends `undeliverable` |
| the daemon unreachable during `inbound` | the event is replayed or silently lost | constrain: the id is recorded first, the error is returned | after the daemon call fails, `inbound` returns `Err`; a new `Gateway` from the same file drops the same event without calling the daemon |
| binding the control channel | an agent channel hides the lease code path | remove: `bind_channel` refuses it | `bind_channel(control, ..)` is `Err` |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. Every new test is red first: all of them need the new request or module, so none is a contract test on the old code.
2. `tests/gateway.rs`: one test per row in section 3, through a real daemon (admin, gateway and session connections) and a plain test-side fake chat (a vector of events in, a vector of posted lines out); no network. The shared daemon and session helpers are moved out of `tests/human_inbound.rs` into `tests/common/mod.rs` (moved, not copied) and used by both files; behaviour of the existing tests does not change. Confined tests return early without bwrap like the others. A test body contains no control characters other than newline and tab and no envelope tag.
3. `split_message` is pinned by example (including exactly `max_chars + 1` characters with no whitespace, two pieces) and by `proptest` (properties: every piece within the limit; no empty piece; no loss of non-whitespace characters).
4. `tests/wire.rs` pins the request and response JSON. `request_permitted` exhaustiveness is enforced by the compiler.
5. Mutation check, each restored by checksum, naming the failing test: the author check removed; the dedup compare `<=` changed to `<`; the file written after the daemon call (killed by the daemon-unreachable test); the dedup file ignored at `new`; the recipient taken from the first bound channel (killed by the B-channel test); a daemon refusal text copied into the line; the lease filter removed from `GatewayAgents`; `GatewayAgents` allowed for admin; splitting at `max_chars + 1`.
6. `git diff --stat` shows only section 0 files; `scripts/prepush-fast.sh` (architect); Sonar after the push (the developer states a hand count for `inbound`; the three rule functions named in section 2.2; the consultant's hand count of the inline form was about 17, the split form about 4).

## 5. Out of scope

The Discord connection, creating channels and categories, `a2amx gateway up`, pairing the first Discord account, the gateway's confinement and launch, reconnect and retry timing, the member-count check, the setup guide, docs and changelog, a mutex or shared state between several gateways, bot-side rate limits, recovery of a message the gateway fetched and then crashed on, and the points 3dl must handle: send with an empty allowed-mentions list so agent text cannot ping, a possible UTF-16 length limit, cancel-safety of the daemon calls (do not drop an `inbound` or `outbound` future mid-request), and rebuilding the bindings from the chat after a restart.
