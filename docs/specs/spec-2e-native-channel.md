# Spec 2e: the native channel, daemon side

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 were settled with the owner.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/wire.rs`, `src/client.rs`, `src/harness.rs`, `src/session.rs`, `src/delivery.rs`,
  `src/daemon.rs`, `src/messaging.rs`, `src/mcp.rs`, `src/cli.rs`, `src/main.rs`,
  `src/lib.rs`
- `src/bridge.rs` (new)
- `tests/bridge.rs` (new)
- `docs/architecture.md`, `docs/delivery.md`, `AGENTS.md`

Does not touch: `src/store.rs`, `src/emulator.rs`, `src/prefix.rs`, `src/hook.rs`,
`Cargo.toml`, `Cargo.lock`, every existing file under `tests/` (the existing test diff
must be empty; `tests/common` is not edited either, `tests/bridge.rs` has its own
helpers), `README.md`, `docs/validation-plan.md`, the locked spec documents under
`docs/specs/`. No new dependency. Do not commit and do not merge.

## 1. Why

Spec 2d put delivery behind a `Channel` seam with the PTY channel as its only
implementation. OMP lets an extension read the session's state (idle, queued messages,
the human's unsent draft) and inject a message natively, with no keystrokes. That avoids
the composer-style readiness rules, the paste rewrites, the approval-dialog collision
and the Esc and Ctrl-C hazards of PTY delivery (probes: `0b22b7cd`, `aad0ebc2`,
`b271e01a`).

This spec adds everything on the Rust side of that route: the `omp` harness, the native
channel, the daemon-to-bridge protocol, the `a2amx omp-bridge` subcommand that relays
between an OMP extension and the daemon, the `native_receipt` evidence, and the tests,
which drive a scripted fake extension against a real daemon. The extension itself (a
TypeScript file embedded in the binary), the launch wiring (`-e`, the operator line) and
the OMP-side smoke test are spec 2f. Until 2f lands an `omp` session has no extension,
so its messages wait with `channel_down`; do not release 2e alone.

## 2. Decisions settled with the owner

- **D1.** The extension and launch wiring are spec 2f. This spec is testable without OMP.
- **D2.** A session's channel follows its harness: `omp` gets the native channel, every
  other harness the PTY channel. The session stays PTY-hosted either way; the PTY remains
  the fallback for hosting and for human use, never a silent fallback for delivery.
  (Refinement of the owner's "stored in the session": the choice is made by one function
  from `session.harness()` at each use, as spec 2d already built channels at each use. The
  native state itself lives in the session.)
- **D3.** Native sessions have no `hold_reason`. Their self-clearing waiting reasons are,
  in this order: `channel_refused`, `channel_down`, `draft_present`, `in_flight`. (The
  owner approved `channel_down`, `draft_present`, `in_flight`; `channel_refused` is the
  same loud failure as `channel_down` with its own explanation, for a bridge that
  connected and was refused.) Idleness never blocks: a message arriving mid-turn is
  delivered as a `followUp`, one message in flight at a time.
- **D4.** `submit` returns when the bridge acknowledges that OMP queued the message
  (`write_complete`). A later receipt, the agent-attributed `message_end` that matches the
  envelope byte for byte, upgrades the evidence to `native_receipt`. A missing receipt is
  never a failure.
- **D5.** A negative acknowledgement ends the attempt as `failed`: the message becomes
  `undeliverable` with the reason as detail. No retry, no store change (the store requeues
  only corrupted PTY submissions).
- **D6.** Bridge death (amended by Amendment 1). When no accepted bridge is attached at
  the moment the loop considers the message, `begin` returns `None` and the message stays
  `pending` (`channel_down` or `channel_refused`); nothing is written. When the bridge
  vanishes after `begin` returned a ticket and before `native_send` queued the frame, the
  attempt ends `unsubmitted` with detail `bridge_disconnected`: visible, terminal, never
  resent. When it vanishes after the frame was queued for writing and before an
  acknowledgement, the message is `submitted` with `write_complete`, never resent. After
  the acknowledgement: unchanged. A reconnecting bridge never causes a resend. Reconnecting is the extension's job: the bridge exits when
  its daemon link ends and the extension respawns it (probe `b271e01a` (c) showed the
  extension can supervise a child). (Refinement of the owner's "Rust side owns
  reconnect": a respawned bridge needs the extension to resend `hello` and `state` anyway,
  so one respawn path replaces a replay cache plus a backoff loop.)
- **D7.** Version handling: one integer, `BRIDGE_PROTOCOL`, changes only when this repo's
  bridge and extension would disagree. The OMP version is logged and never enforced; OMP
  releases daily. Missing OMP APIs are reported by the extension in `hello.missing` and
  refuse the channel.
- **D8.** One extension per session: a second `bridge_attach` is rejected and the first
  stays. Only `ctx.agent.kind === "main"` ever attaches (a 2f rule).
- **D9.** Tool calls reach the daemon over a second, ordinary connection owned by the
  bridge, through the existing MCP server code; no multiplexing on the stream.

## 3. The protocol

Three links, one vocabulary. The extension and the bridge talk newline-delimited JSON on
the bridge's stdin and stdout (one object per line, at most `MAX_FRAME_LEN` bytes). The
bridge and the daemon talk the same objects as length-prefixed JSON frames
(`wire::encode_frame`, `FrameDecoder`), on a connection the daemon switches to bridge mode
after a `bridge_attach` request. Tool calls ride the stdio link as `mcp` lines and leave
the bridge on a second ordinary connection (section 7).

The frame size limit is the existing `MAX_FRAME_LEN` (1 MiB), not a smaller cap: a body
of 32 KiB full of control characters escapes to about 192 KiB of JSON.

### 3.1 Extension and bridge to daemon (`BridgeUp`)

```json
{"type":"hello","protocol":1,"omp_version":"18.4.5","missing":[]}
{"type":"state","idle":true,"pending":false,"draft":false}
{"type":"ack","id":"m_1"}
{"type":"nack","id":"m_1","reason":"send_failed"}
{"type":"receipt","id":"m_1"}
```

- `hello` is the first frame. `missing` lists OMP APIs the extension needs and did not
  find; `omp_version` is free text, logged only.
- `state` reports the session; the extension sends it on every change and right after
  every `ack`. `idle` is carried for diagnostics only: delivery never waits for idle.
  `draft` is true when the human has unsent text in the editor; `pending` is true while
  OMP holds a queued message that has not entered the conversation.
- `ack` says OMP queued the message in the `deliver` frame; `nack` says it could not.
  `receipt` says the agent-attributed message with exactly the delivered text entered the
  conversation.

### 3.2 Daemon to bridge and extension (`BridgeDown`)

```json
{"type":"ready"}
{"type":"refused","reason":"protocol_mismatch"}
{"type":"deliver","id":"m_1","envelope":"<a2amx-message id=\"m_1\" ...>...</a2amx-message>"}
```

- `ready` answers an accepted `hello`. `refused` answers a rejected one and the daemon
  closes the connection; the reason is `protocol_mismatch`, or `missing_apis: ` followed
  by the `missing` entries joined with `, `.
- `deliver` carries the id and the envelope text rendered by the delivery loop, the same
  text every channel gets.

### 3.3 Tool lines (stdio only)

```json
{"type":"mcp","request":{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_agents","arguments":{}}}}
{"type":"mcp","response":{"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"..."}],"isError":false}}}
```

The first goes from the extension to the bridge, the second back. The `request` is any
MCP JSON-RPC message (`initialize`, `tools/list` and `tools/call` included); the bridge
hands it to the existing MCP server code and writes whatever that code answers. A
notification gets no answer.

### 3.4 Bridge exit codes

0: done, do not respawn (stdin closed, or the daemon refused the channel, or the daemon
rejected the attach). Nonzero: error, the extension may respawn it (the daemon link was
lost, a line from the extension was invalid, an environment variable is missing). The
reason goes to stderr as `a2amx: <reason>`.

## 4. `src/wire.rs`

- `pub const BRIDGE_PROTOCOL: u32 = 1;`
- `Request::BridgeAttach` (unit variant, serializes as `{"type":"bridge_attach"}`).
  Followed, after `Response::Attached`, by a switch of the connection to bridge mode.
  Add it after `CancelMessage`.
- `BridgeUp` and `BridgeDown`, both `#[derive(Debug, Clone, PartialEq, Eq, Serialize,
  Deserialize)]` with `#[serde(tag = "type", rename_all = "snake_case")]`, with exactly
  the shapes of 3.1 and 3.2:

```rust
pub enum BridgeUp {
    Hello { protocol: u32, omp_version: String, #[serde(default)] missing: Vec<String> },
    State { idle: bool, pending: bool, draft: bool },
    Ack { id: String },
    Nack { id: String, reason: String },
    Receipt { id: String },
}

pub enum BridgeDown {
    Ready,
    Refused { reason: String },
    Deliver { id: String, envelope: String },
}
```

Existing wire JSON does not change.

## 5. `src/client.rs`

Add, mirroring `Client::attach` and `Attachment`:

```rust
impl Client {
    /// Sends `bridge_attach`; on `attached` switches this connection to bridge frames.
    pub async fn bridge(mut self) -> anyhow::Result<BridgeLink>
}
pub struct BridgeLink { connection: Framed }
impl BridgeLink {
    pub async fn send(&mut self, frame: BridgeUp) -> anyhow::Result<()>;
    /// `Ok(None)` when the daemon closed the connection. Cancellation safe.
    pub async fn recv(&mut self) -> anyhow::Result<Option<BridgeDown>>;
}
```

`Client::bridge` fails with the daemon's error message when the response is
`Response::Error { message }` (`bail!(message)`), and with `unexpected daemon bridge
response` for anything else. Frames are JSON, validated against `MAX_FRAME_LEN` exactly as
`Framed::send` already does.

## 6. `src/harness.rs`

- `Harness` gains `Omp` (serialized `omp`, a clap value `omp`). `Claude` and `Generic` are
  unchanged.
- `default_deliver`: `Omp` gives `Deliver::Auto`.
- `ready` and `paste_view`: `Omp` shares the `Generic` arm (`Harness::Generic |
  Harness::Omp`); native delivery never calls either, and no PTY delivery path is reachable
  for an `omp` session.

## 7. `src/session.rs`: the native slot

`State` gains `pub native: NativeSlot`, initialised to its default in `Session::spawn`:

```rust
#[derive(Default)]
pub(crate) struct NativeSlot {
    connected: bool,                // a bridge's hello was accepted
    refused: bool,                  // the last hello was refused; sticky until an accepted hello
    draft: bool,                    // from the last `state`
    in_flight: bool,                // an ack arrived, or the last `state` said pending
    link: Option<mpsc::UnboundedSender<BridgeDown>>,   // Some while a bridge is attached
    waiting: Option<(String, oneshot::Sender<Result<(), String>>)>,
}
```

The fields are private to the module; the contract is these `Session` methods, all
`pub(crate)`, all short critical sections under the existing `lock()`:

| Method | Behavior |
| --- | --- |
| `native_attach(self: &Arc<Self>) -> Option<NativeGuard>` | `None` when `link` is `Some`. Otherwise creates the channel pair, stores the sender in `link`, returns the guard. The guard owns the `Arc<Session>` and the receiver (`pub(crate) down: mpsc::UnboundedReceiver<BridgeDown>`). Its `Drop` clears `connected`, `link`, `waiting`, `draft`, `in_flight` (`refused` stays) and notifies `message_notify`. |
| `native_accept(&self)` | `connected = true`, `refused = false`, notify. |
| `native_refuse(&self)` | `refused = true`, `connected = false`, notify. |
| `native_state(&self, draft: bool, pending: bool)` | stores both (`in_flight = pending`), notify. |
| `native_ack(&self, id: &str, result: Result<(), String>)` | when `waiting` holds `id`: take it, send `result` to its receiver, and on `Ok` set `in_flight = true`. Otherwise ignore. |
| `native_received(&self)` | `in_flight = false`, notify. |
| `native_send(&self, id: &str, envelope: &str) -> Option<oneshot::Receiver<Result<(), String>>>` | `None` when `link` is `None` or the send fails (the receiver is gone). Otherwise stores `waiting = (id, sender)`, sends `BridgeDown::Deliver`, returns the receiver. |
| `native_reason(&self) -> Option<&'static str>` | `Some("channel_refused")` when `refused`; else `Some("channel_down")` when not `connected`; else `Some("draft_present")` when `draft`; else `Some("in_flight")` when `in_flight`; else `None`. |

"Notify" means `self.message_notify.notify_one()`; the delivery loop's one-second tick is
the backstop. `session.rs` otherwise does not change: the PTY paths, `Hold`, `Delivery`
and `DeliveryOutcome` stay as they are.

## 8. `src/delivery.rs`: `AnyChannel`, `NativeChannel`, the id in `submit`

1. **`Ticket::submit` gains the message id**: `fn submit(self, id: &str, envelope: &str)
   -> impl Future<Output = DeliveryOutcome> + Send;`. In `run`, compute
   `let id = format!("m_{}", head.seq);` once before `render_envelope` (replacing the
   inline `&format!("m_{}", head.seq)` argument) and call `ticket.submit(&id, &envelope)`.
   `PtyTicket::submit` ignores `id`. The four spec-2d unit tests change only in the
   `FakeTicket::submit` signature (`_id: &str` added); their bodies and assertions are
   untouched.
2. **`NativeChannel`**:

```rust
pub(crate) struct NativeChannel { session: Arc<Session> }
pub(crate) struct NativeTicket<'a> { session: &'a Session }
```

   `impl Channel for NativeChannel` with `type Ticket<'a> = NativeTicket<'a>`:
   `session_id`, `exited`, `deliver`, `message_notify` and `wake` are the same
   delegations as `PtyChannel`, `wake` being `self.session.notify()`; `prepare` does
   nothing; `begin` returns `Some(NativeTicket { session: &self.session })` when
   `self.session.native_reason()` is `None`, else `None`; `hold_reason` returns `None`;
   `unready_reason` is `self.session.native_reason()`.

   `impl Ticket for NativeTicket<'_>`, `submit(self, id, envelope)`:
   - `let Some(receiver) = self.session.native_send(id, envelope) else { return
     DeliveryOutcome::Unsubmitted("bridge_disconnected".into()) };` (the bridge vanished
     between `begin` and here; the frame was never queued). Leave a `// shortcut:`
     comment on it: the attempt already started, and the store cannot return a started
     attempt to `pending`, so this instant ends `unsubmitted` instead; requeue it (a
     store change) only if this window shows up in practice.
   - `match tokio::time::timeout(ACK_TIMEOUT, receiver).await`:
     `Ok(Ok(Ok(())))` gives `DeliveryOutcome::Submitted`; `Ok(Ok(Err(reason)))` gives
     `DeliveryOutcome::Failed(reason)`; `Ok(Err(_))` (the connection ended after the frame
     was queued, written or not) and `Err(_)` (no answer in time) both give
     `DeliveryOutcome::Submitted`, because the frame may have reached OMP and a resend
     could duplicate the message.
   - `const ACK_TIMEOUT: Duration = Duration::from_secs(10);` with a `// shortcut:` comment:
     a fixed timeout, make it configurable if a slow bridge needs longer.
3. **`AnyChannel` and `AnyTicket`**, enum delegation (the shape was compiled and run with
   `tokio::spawn` on rustc 1.98.1 before locking; a `dyn` channel is still impossible):

```rust
pub(crate) enum AnyChannel { Pty(PtyChannel), Native(NativeChannel) }
pub(crate) enum AnyTicket<'a> { Pty(PtyTicket<'a>), Native(NativeTicket<'a>) }

impl AnyChannel {
    /// `omp` sessions get the native channel, every other harness the PTY channel.
    pub(crate) fn for_session(session: Arc<Session>) -> Self
}
```

   `impl Channel for AnyChannel` with `type Ticket<'a> = AnyTicket<'a>` delegates every
   method by `match`, and `begin` maps the inner ticket with `AnyTicket::Pty` or
   `AnyTicket::Native`. `impl Ticket for AnyTicket<'_>` delegates `submit`.
   `PtyChannel::new` stays; `NativeChannel` has no public constructor besides
   `for_session`.

`run` itself does not otherwise change.

## 9. `src/daemon.rs`

1. **Imports**: `crate::delivery::{AnyChannel, Channel}` replaces
   `crate::delivery::{Channel, PtyChannel}`. Add the `wire` and `session` names the
   handler below uses.
2. **The four channel sites** (spawn at line 184, `send_message` at 260, `message_info`
   at 390, the `List` handler at 829): replace `PtyChannel::new(x)` with
   `AnyChannel::for_session(x)` and nothing else; `daemon.rs` no longer names
   `PtyChannel`.
3. **Role**: `Request::BridgeAttach` joins the requests a session token may send (the
   `matches!` at line 799), and an admin token gets
   `Response::Error { message: "bridge_attach needs a session token" }`, in the arm style
   of `ReportPrompt`.
4. **The attach arm**, in the style of `Request::Attach`: look up the session; its harness
   must be `Harness::Omp`, else `Response::Error { message: "session is not an omp
   session" }`; `session.native_attach()` returning `None` gives `Response::Error {
   message: "a bridge is already attached to this session" }`. Otherwise respond
   `Response::Attached` and hand the connection to a new `async fn bridge(connection:
   &mut Framed, runtime: &Arc<Runtime>, session: Arc<Session>, guard: NativeGuard) ->
   anyhow::Result<()>`, then `return Ok(())` from `serve` (the connection is spent).
5. **`bridge`**:
   - Wait up to 5 s (the hello timeout `serve` already uses) for the first frame, which
     must decode as `BridgeUp::Hello`; anything else, a timeout or a closed connection
     ends the function (the guard drop cleans up).
   - A `protocol` other than `BRIDGE_PROTOCOL`: `session.native_refuse()`, send
     `BridgeDown::Refused { reason: "protocol_mismatch" }`, return. A non-empty `missing`:
     `native_refuse()`, send `Refused { reason: format!("missing_apis: {}",
     missing.join(", ")) }`, return. Otherwise `native_accept()`, send `BridgeDown::Ready`,
     and `tracing::info!` the `omp_version` and the session id.
   - Then loop in a `tokio::select!` over: the next frame from the connection; the next
     `BridgeDown` from `guard.down`, written to the connection; a one-second interval whose
     tick ends the function when `session.exit_code().is_some()`. A closed connection, an
     undecodable frame or a second `Hello` ends the function.
   - Frame handling: `State { draft, pending, .. }` calls `native_state(draft, pending)`;
     `Ack { id }` calls `native_ack(&id, Ok(()))`; `Nack { id, reason }` calls
     `native_ack(&id, Err(reason))`; `Receipt { id }` is handled by the receipt rule below.
   - **Receipt rule** (the same ownership checks `report_prompt` makes): parse `id` with
     `parse_message_id`; fetch the message with `store.get`; act only when
     `message.boot == runtime.boot`, `message.recipient_session` is this session, the state
     is `Delivering`, `Submitted` or `Unsubmitted`, and it is not yet `observed`; then call
     `store.record_receipt(seq)`, then `session.native_received()`. Any other receipt is
     ignored. This path never touches `note_submit`, `note_corrupted` or any `Hold`.
6. **`message_info` evidence**: the `Some("submission_observed")` branch becomes
   `Some("native_receipt")` when the recipient's session is still in the registry and its
   harness is `Harness::Omp`; otherwise it stays `"submission_observed"`.
   `// shortcut:` a session removed from the registry reports `submission_observed` for a
   native receipt; persist the channel with the receipt if that matters.
7. Do not touch `report_prompt` or the other functions. The session-hold chain in
   `message_info` keeps its order (exited, `deliver_hold`, hold, `queued`, unready); a
   native session simply has no hold, so its unready reasons appear after `queued`.

## 10. `src/messaging.rs`

`hold_explanation` gains four arms (before the `_` arm), texts exactly:

- `channel_down`: `The recipient's A2AMX extension is not connected; the message waits until it connects.`
- `channel_refused`: `The recipient's A2AMX extension refused to connect (a version mismatch or an OMP feature it needs is missing); a person must update A2AMX or OMP.`
- `draft_present`: `A person has unsent text in the recipient's prompt box; delivery resumes when it is sent or cleared.`
- `in_flight`: `An earlier message is queued in the recipient and has not entered its conversation yet.`

The existing arms and strings are untouched. The MCP tool descriptions do not change (an
existing test pins the `message_status` text); `native_receipt` is documented in section 15.

## 11. `src/mcp.rs`

Visibility only, so the bridge reuses the tool and stdin code instead of copying it:
`McpServer` becomes `pub(crate)`, with `pub(crate) fn new(address: SocketAddr, token:
String) -> Self` and `pub(crate) async fn handle_line(&mut self, line: &[u8]) ->
Option<Value>`; `StdioEvent` (with its three variants) and `spawn_stdin_reader` become
`pub(crate)`. No other change, no behavior change.

## 12. `src/bridge.rs` (new): the `omp-bridge` subcommand

`pub async fn run() -> anyhow::Result<()>`. It reads the daemon address and token with
`mcp::configuration()` (the environment variables the daemon already sets for every
session; the token never appears in argv, a line or a log). Structure:

1. `Client::connect_addr(address, &token)` then `.bridge()` for the stream link. A
   `bridge()` error whose message is "a bridge is already attached to this session" or
   "session is not an omp session" is reported to the extension as
   `{"type":"refused","reason":"<message>"}` and the function returns `Ok(())` (exit 0).
   Any other error propagates (exit 1).
2. Stdin is read with `mcp::spawn_stdin_reader` (made `pub(crate)` in section 11); a
   `StdioEvent::Error` ends the function with that error and `StdioEvent::Eof` is the
   stdin EOF of step 5. Stdout is written by one task through a channel so lines never
   interleave.
3. A line from the extension is parsed as JSON. If its `type` is `mcp`, its `request` goes
   to a task that owns one `McpServer::new(address, token)` and runs
   `handle_line(request bytes)`; a `Some(value)` is written back as
   `{"type":"mcp","response":value}`. Tool calls run on that task so a slow call never
   delays state or delivery. Every other line must deserialize as `BridgeUp` and is sent
   on the stream link in arrival order. An undecodable line, or one over `MAX_FRAME_LEN`,
   ends the function with an error.
4. A `BridgeDown` frame from the daemon is written to stdout as one line. After writing
   `Refused` the function returns `Ok(())` (exit 0, do not respawn).
5. Stdin EOF returns `Ok(())`. The daemon link ending (EOF or an I/O error) returns
   `Err` (exit 1, the extension respawns the bridge). The extension is expected to send
   `hello` and then `state` again on every respawn; the bridge keeps no replay cache.
6. `// shortcut:` the bridge does not reconnect by itself; add a bounded reconnect with
   state replay only if respawning proves too slow.

No terminal-emulation types appear in this module; it names no PTY type.

## 13. `src/cli.rs`, `src/main.rs`, `src/lib.rs`

- `Command::OmpBridge` (doc comment `/// Run the OMP extension's bridge to the daemon
  (reads and writes JSON lines on stdio).`), the clap name `omp-bridge`. The
  `harness` argument of `New` gains a doc comment: `Harness profile. omp delivers
  through an OMP extension; without it, messages wait (channel_down).`
- `src/main.rs`: in `dispatch`, the early-return block gains `Command::OmpBridge =>
  return bridge::run().await,` (it needs no state directory), and the late
  `Command::Mcp | Command::Hook` arm becomes `Command::Mcp | Command::Hook |
  Command::OmpBridge`. In `message_value_rows`, the `evidence` fallback matches both
  `submission_observed` and `native_receipt` and prints the evidence text, so the table
  shows `native_receipt` where it shows `submission_observed` today.
- `src/lib.rs`: `pub mod bridge;` in alphabetical position.

## 14. `tests/bridge.rs` (new)

Integration tests only; helpers local to the file; real daemon on port 0 in a temp dir via
`tests/common`; no OMP, no fixed ports. Two kinds of fake extension: a protocol client
(`Client::connect_addr(addr, &token)` then `.bridge()`, speaking `BridgeUp`/`BridgeDown`)
and the real `a2amx omp-bridge` process (`env!("CARGO_BIN_EXE_a2amx")`, `A2AMX_ADDR` and
`A2AMX_TOKEN` set, stdin and stdout piped) driven by lines. Recipient: an `omp` session
named `agent-review` (`common::new_agent` with `Harness::Omp`, `Deliver::Auto`), sender: a
generic session named `agent-plan`; host name `host-a`. Message: subject `Parser issue`,
body `I found the regression in parser.py.`, first id `m_1`. Expected values are literals;
the envelope is the literal `ENVELOPE_TEXT` of `tests/delivery.rs` copied into the file.
Waits use `common::eventually`; no sleeps as assertions.

One test per behavior, written failing first:

1. `a_second_bridge_for_the_same_session_is_rejected`: a second `.bridge()` fails with
   `a bridge is already attached to this session`; the first still receives a `deliver`.
2. `bridge_attach_needs_an_omp_session_and_a_session_token`: the admin client's
   `.bridge()` fails with `bridge_attach needs a session token`; a session token for a
   generic session fails with `session is not an omp session`.
3. `a_protocol_mismatch_is_refused`: `hello` with `protocol: 2` answers
   `Refused { reason: "protocol_mismatch" }` and the connection closes.
4. `missing_apis_refuse_the_channel_and_explain_the_wait`: `hello` with `missing:
   ["sendUserMessage","isIdle"]` answers `Refused { reason: "missing_apis:
   sendUserMessage, isIdle" }`; with a queued message, `message_status` shows state
   `pending`, `hold_reason` `channel_refused` and the exact `hold_explanation` of
   section 10; a later accepted `hello` from a new attach clears the reason.
5. `a_message_is_delivered_once_and_stays_write_complete_without_a_receipt`: after an
   accepted `hello` and a `state` of `{idle:true,pending:false,draft:false}`, the bridge
   receives exactly one `deliver` with id `m_1` and the literal envelope; after `ack` the
   message is `submitted` with evidence `write_complete`; no second `deliver` follows.
6. `a_receipt_upgrades_the_evidence_to_native_receipt`: after the `ack`, a `receipt`
   makes `message_status` evidence `native_receipt` and `a2amx messages` (the binary,
   `--home` the temp dir) show `native_receipt` in the detail column; a `receipt` for the
   unknown id `m_999` and one for the malformed id `x1` change nothing and leave the
   connection open (send the bad ones first; the valid `receipt` afterwards still works).
7. `a_draft_delays_delivery_until_it_clears`: with `state.draft = true`, `message_status`
   is `pending` with `hold_reason` `draft_present` and no `deliver` arrives; a `state`
   with `draft = false` brings the `deliver`.
8. `a_second_message_waits_while_one_is_in_flight`: two messages queued; after the first
   `deliver` and its `ack`, a `state` with `pending = true` keeps `m_2` pending with
   reason `in_flight`; a `receipt` for `m_1` and a `state` with `pending = false` bring
   the `deliver` for `m_2`.
9. `a_nack_makes_the_message_undeliverable`: a `nack` with reason `send_failed` makes
   `m_1` `undeliverable` with detail `send_failed`, and no `deliver` for it follows.
10. `a_bridge_that_dies_after_the_ack_gets_no_resend`: after `deliver` and `ack`, drop the
    connection; `m_1` stays `submitted`; a new accepted attach never receives a `deliver`
    for `m_1`.
11. `a_bridge_that_dies_before_delivery_leaves_the_message_pending`: attach, accept, drop
    the connection, then wait until a new `.bridge()` succeeds (the first attachment is
    gone; a rejected attempt means it is not); on that new link send no `hello` yet,
    send the message: it stays `pending` with reason `channel_down`; then `hello`
    brings the `deliver`. (The message is sent only after the first attachment is gone, so
    `begin` returns `None`; the instant between a claim and the queueing is not tested,
    because no public seam can force it.)
12. `the_bridge_relays_hello_state_and_delivery_over_stdio`: through the real binary, the
    `hello` and `state` lines reach the daemon (a message sent afterwards produces a
    `{"type":"deliver",...}` line with the literal envelope on stdout; the daemon's `ready`
    answer appears as `{"type":"ready"}` on stdout first); an `ack` line moves the message
    to `submitted`.
13. `the_bridge_relays_the_three_tools`: through the real binary, `tools/list` returns the
    three tool names, `send_message` from the OMP session's token delivers a message whose
    recorded sender is that session (the sender is derived from the token), and
    `list_agents` and `message_status` return their results as `mcp` response lines.
14. `the_bridge_exits_when_stdin_closes`: closing stdin ends the process with status 0.
15. `the_bridge_exits_zero_after_a_refusal`: a `hello` with a non-empty `missing` produces
    a `{"type":"refused",...}` line and exit status 0.
16. `the_bridge_exits_nonzero_when_the_daemon_link_is_lost`: after an accepted `hello`,
    shutting the daemon down ends the process with a nonzero status and an `a2amx:` line
    on stderr.
17. `bridge_frames_serialize_to_the_documented_json`: each `BridgeUp` and `BridgeDown`
    variant serializes to exactly the JSON lines of 3.1 and 3.2 and parses back; `Request::
    BridgeAttach` serializes to `{"type":"bridge_attach"}`.

Total new tests: 17. The existing 157 and the four spec-2d unit tests stay.

## 15. Docs and `AGENTS.md`

- `AGENTS.md`, the Modules table: add the row `| \`bridge\` | The \`omp-bridge\` relay
  between an OMP extension and the daemon |` after `daemon` / `client`.
- `docs/architecture.md`, the **Delivery.** bullet: replace the clause "and the PTY channel
  is the only implementation." with "the PTY channel serves every harness, and the `omp`
  harness uses a native channel (spec 2e) fed by an OMP extension through the `a2amx
  omp-bridge` relay." Leave the scope table row as it is: the extension ships in spec 2f.
- `docs/delivery.md`:
  - the paragraph at line 209, replace "The channel is now a trait (`Channel` in
    `src/delivery.rs`, spec 2d), and the PTY channel is its only implementation." with
    "The channel is now a trait (`Channel` in `src/delivery.rs`, spec 2d); the PTY channel
    serves every harness and the native channel serves `omp` (spec 2e).";
  - the **Evidence.** paragraph at line 353: append one sentence: "A native channel
    reports `native_receipt` for a `submitted` message whose harness reported it entering
    the conversation.";
  - a new section `## Implemented: native channel (OMP)` before `## Harness adapter
    responsibilities`, with: the three links and frames of section 3 in prose (no JSON
    dump), the waiting reasons `channel_refused`, `channel_down`, `draft_present`,
    `in_flight` and what each means, the bridge-death rules of D6, `followUp` delivery with
    one message in flight, and that the extension and its launch wiring are spec 2f.
- No other doc changes. Public repository rules apply: fictional names and hosts only.

## 16. Out of scope

The extension, its embedding and its version file, `-e` and `--append-system-prompt`
wiring, the OMP operator line, `--no-authorize-peers` for OMP, and the smoke run against
OMP (all spec 2f); a Codex channel; automatic bridge reconnect; a persisted channel on the
receipt; returning a started attempt to `pending` (Amendment 1); any change to `src/store.rs`, to existing tests, to the MCP tool descriptions or
to the existing JSON of the wire types; choosing a channel anywhere except
`AnyChannel::for_session`; refactors beyond what sections 4 to 13 name.

## 17. Acceptance

All three must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

And, run from the worktree root after implementation, in a real shell:

```sh
git status --porcelain | sort
git status --porcelain -- tests
rg -n "PtyChannel|session_hold_reason" src/daemon.rs src/bridge.rs
rg -l "Harness::Omp" src | sort
rg -l "cfg\(test\)" src
git diff --check
```

Expected: the first lists exactly these lines, in this order (`sort` puts the leading
space of ` M` before `??`): ` M AGENTS.md`, ` M docs/architecture.md`,
` M docs/delivery.md`, ` M src/cli.rs`, ` M src/client.rs`, ` M src/daemon.rs`,
` M src/delivery.rs`, ` M src/harness.rs`, ` M src/lib.rs`, ` M src/main.rs`,
` M src/mcp.rs`, ` M src/messaging.rs`, ` M src/session.rs`, ` M src/wire.rs`,
`?? src/bridge.rs`, `?? tests/bridge.rs`; the second prints
exactly `?? tests/bridge.rs`; the third prints nothing (exit status 1); the fourth prints
exactly `src/daemon.rs`, `src/delivery.rs`, `src/harness.rs`; the fifth prints exactly
`src/delivery.rs`; the last prints nothing. The existing test count is 161; with the 17
new tests `cargo test` reports 178 passed. Leave the diff uncommitted and unmerged.

## Amendment 1: bridge death and the started attempt

OMP stopped with `BLOCKED — SPEC ADJUDICATION REQUIRED` before editing anything. The claim
is real; verified against the spec and the source:

- The delivery loop calls `store.begin_attempt` before `ticket.submit`
  (`src/delivery.rs`), so once `begin` returned a ticket the message is `delivering`.
- `finish_attempt` maps the outcome `unsubmitted` to the message state `unsubmitted`
  (`src/store.rs`), which `next_pending` never selects again. Only `reject_attempt`
  returns a message to `pending`, and it is hard-coded to corrupted PTY submissions.
- Section 0 forbids touching `src/store.rs` and section 8 keeps `run` otherwise unchanged.

So the first sentence of the original D6 ("before the deliver frame was written: the
message stays `pending`") could not hold for the instant between the claim and the
queueing of the frame, and the internal queue between `native_send` and the socket write
(section 9) adds a second instant that section 8 already classified as `submitted`.

**Ruling** (settled with the owner, the narrower rule over a store change): D6 in section
2 now reads as the three cases it lists. The case the owner meant, a bridge that is gone
when the loop considers the message, keeps the message `pending`; test 11 exercises
exactly that. The two instants inside a delivery attempt end `unsubmitted`
(`bridge_disconnected`) before the frame is queued and `submitted` (`write_complete`)
after, both terminal for the attempt and neither ever resent. No code path in this spec
returns a started attempt to `pending`.

**Changes**, all in this document: section 2 D6 (text replaced); section 8, the
`native_send` and timeout bullets (wording, and the `// shortcut:` comment on the
`bridge_disconnected` branch); section 14 test 11 (a note, no new test); section 16 (one
out-of-scope item). The test total stays 17 new and 178 overall. The docs bullet of
section 15 that says "the bridge-death rules of D6" now means the three cases above: the
docs section must state them in these words, including that a message claimed in the
instant the bridge vanishes ends `unsubmitted`.
