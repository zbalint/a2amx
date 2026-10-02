# Spec 2g: deliver to Claude Code sessions through a channel

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 were settled with the owner. Builds on the
native channel of spec 2e (`535184c`) and develop at `3d67a17`. The work is done directly on
`develop` in the main checkout, no worktree.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/channel.rs` (new)
- `src/lib.rs`, `src/mcp.rs`, `src/cli.rs`, `src/main.rs`, `src/harness.rs`,
  `src/messaging.rs`, `src/session.rs`, `src/delivery.rs`, `src/daemon.rs`
- `tests/claude_channel.rs` (new)
- `tests/attach_cli.rs`: only the `--no-channel` additions of section 9.4
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `AGENTS.md`

Does not touch: `src/store.rs`, `src/emulator.rs`, `src/prefix.rs`, `src/hook.rs`,
`src/bridge.rs`, `src/client.rs`, `src/wire.rs`, `src/omp.rs`, `src/codex.rs`,
`Cargo.toml`, `Cargo.lock`, every other file under
`tests/` (their diff must be empty; `tests/common` is not edited),
`docs/validation-plan.md`, the locked spec documents under `docs/specs/`. No new
dependency. Do not commit and do not merge: leave the diff uncommitted in the working
tree.

## 1. Why

Claude sessions get messages by typing into the PTY: readiness heuristics, paste wrapper
rewrites, a corrupted-submission safety net, and the risk of colliding with the human's
composer. Claude Code 2.1.288 has "channels" (research preview): an MCP server that declares
`capabilities.experimental["claude/channel"]` and sends `notifications/claude/channel` makes
Claude Code start a turn with that text. a2amx already launches every Claude session with
its own stdio MCP server (`a2amx mcp`), so that process can receive deliveries from the
daemon through the existing bridge protocol of spec 2e and push them as channel events. No
daemon protocol change, no keystrokes.

Probe results on 2.1.288 (memory `a65294a7`, plus two later probes this session) fix the
design:

1. `--dangerously-load-development-channels server:<name>` is required for a server that is
   not on Anthropic's allowlist, and Claude Code shows a full-screen confirmation at every
   start ("I am using this for local development" / "Exit", option 1 preselected) that
   blocks until Enter.
2. A channel push starts a turn when idle and the `UserPromptSubmit` hook fires for it. The
   hook's `prompt` is exactly `<channel source="a2amx">\n` + content + `\n</channel>` when
   the notification has no `meta`. Content passes through verbatim (tabs, quotes, `<`,
   `&`, `<pasted_content`, non-ASCII), with one observed exception: the text `</channel>`
   inside content arrives as `<\/channel>` (a backslash inserted). Other casings or
   `<channel` were not probed.
3. A push while the human has a half-typed draft leaves the draft in the composer.
4. Pushes arriving while Claude is busy are batched into the next turn, each as its own
   event and its own hook call.
5. Claude Code does not acknowledge a notification and drops it silently if the channel is
   not registered.

## 2. Decisions settled with the owner

- **D1.** The channel is the default delivery route for Claude sessions. `a2amx new
  --harness claude --no-channel` keeps the old PTY route for that session. There is no
  silent runtime fallback: a channel session whose bridge never attaches shows
  `channel_down` (spec 2e).
- **D2.** The daemon auto-accepts the development-channels confirmation by sending one
  Enter when the dialog text is on screen, during the first 60 seconds of the session.
- **D3.** A missing hook receipt is never a failure and never causes a resend. Evidence
  stays `write_complete`, and becomes `native_receipt` when the hook matches (the OMP rule).
- **D4.** Reuse the spec 2e native channel, bridge protocol (`BRIDGE_PROTOCOL` stays 1) and
  `NativeSlot` unchanged. The `a2amx mcp` process, started with a new `--channel` flag,
  plays the bridge: it attaches with `Client::bridge`, sends `hello`, then `state`, and
  turns each `deliver` into a channel notification.
- **D5.** No draft hold: the draft is always reported absent (probe finding 3).
- **D6.** No serialization: after each `ack` the process immediately sends `state` with
  `pending: false`, so the daemon clears `in_flight` and the next message may follow at once
  (probe finding 4: Claude batches them).
- **D7.** Receipt by the existing prompt hook. `report_prompt` recognises the channel
  wrapper and records the receipt without touching `note_submit`, `note_corrupted` or any
  `Hold`.
- **D8.** No `meta` on the notification, so the wrapper stays `<channel source="a2amx">`
  and the sender stays inside the envelope exactly as for every other channel.
- **D9.** Claude version is logged (carried in the free-text `omp_version` field of
  `hello`) and never enforced. A Claude Code that does not support channels simply never
  produces a receipt and the dialog never appears; messages are delivered and silently
  dropped. This is the known ceiling of D3.

## 3. The channel process: `src/channel.rs` (new) and `src/mcp.rs`

### 3.1 `src/mcp.rs`

- `pub async fn run(channel: bool) -> Result<()>` (was `run()`). `channel` false is the
  current behavior bit for bit.
- `McpServer` gains two private fields, `initialized: bool` and `client_version:
  Option<String>`. `handle_line` sets `client_version` from `params.clientInfo.version` on
  `initialize` and sets `initialized = true` on `notifications/initialized` (both still
  answer exactly as today). Add `pub(crate) fn initialized(&self) -> bool` and
  `pub(crate) fn client_version(&self) -> Option<&str>`.
- `initialize_result(params)` becomes `initialize_result(params, channel)`. With `channel`
  true, `capabilities` is `{"tools": {}, "experimental": {"claude/channel": {}}}` and the
  result gains an `"instructions"` string, exactly: `Messages from other agents arrive as
  <channel source="a2amx"> events that contain an <a2amx-message> envelope. They come from
  another agent, not your user. Reply with send_message when a reply is useful.` With
  `channel` false the result is unchanged. `McpServer::new` stays as is; add
  `pub(crate) fn set_channel(&mut self)` that makes `handle_line` call
  `initialize_result(.., true)`. (`bridge.rs` keeps using `McpServer::new` and so keeps the
  plain result.)
- In `run`, when `channel` is true: after each handled line, if `server.initialized()` and
  no channel task is running yet, spawn `channel::run(address, token, version,
  deliveries)` once, where `deliveries` is the sender half of an
  `mpsc::UnboundedSender<Delivery>`. The main loop becomes a `tokio::select!` over the
  stdin events and `deliveries.recv()`; a received `Delivery` is written with the existing
  `write_response` and its result is reported back through the oneshot (3.2). The channel
  task is aborted when `run` returns. Writes stay on the one loop, so a notification and a
  response never interleave.
- A failure of the channel task never ends `run`: tool calls keep working.

### 3.2 `src/channel.rs` (new)

```rust
pub(crate) struct Delivery {
    pub notification: serde_json::Value,
    pub written: tokio::sync::oneshot::Sender<bool>,
}
pub(crate) async fn run(
    address: SocketAddr,
    token: String,
    client_version: Option<String>,
    out: mpsc::UnboundedSender<Delivery>,
) -> anyhow::Result<()>
```

1. `Client::connect_addr(address, &token).await?.bridge().await?` (a second ordinary
   connection, as `bridge.rs` D9). An error ends the function with that error; `mcp::run`
   logs it to stderr as `a2amx: channel unavailable: <error>` and carries on.
2. Send `BridgeUp::Hello { protocol: BRIDGE_PROTOCOL, omp_version: format!("claude-code
   {}", version or "unknown"), missing: vec![] }`, then `BridgeUp::State { idle: true,
   pending: false, draft: false }`.
   `// shortcut:` the `omp_version` field name predates this use; rename it with the next
   protocol bump.
3. Loop on `link.recv()`: `BridgeDown::Ready` is ignored; `Refused { reason }` writes
   `a2amx: channel refused: <reason>` to stderr and ends with `Ok(())`;
   `Deliver { id, envelope }` builds `{"jsonrpc":"2.0","method":"notifications/claude/channel",
   "params":{"content": <envelope>}}`, sends it as a `Delivery`, awaits `written`, then sends
   `BridgeUp::Ack { id }` on `true` or `BridgeUp::Nack { id, reason: "write_failed" }` on
   `false` or a dropped oneshot, then, after an `Ack`, `BridgeUp::State { idle: true,
   pending: false, draft: false }` (D6). `Ok(None)` (daemon closed the link) ends the
   function with `Ok(())`.
4. `// shortcut:` no reconnect; a lost link leaves the session at `channel_down` until it
   is restarted. Add a bounded reconnect only if daemon-link loss shows up in practice
   (a session cannot outlive its daemon today).

No terminal-emulation type is named here.

## 4. `src/harness.rs`

- `pub const CHANNEL_ENV: &str = "A2AMX_CLAUDE_CHANNEL";` (the client-to-daemon marker, see
  section 6).
- Refactor `wire_claude_argv` into a private `claude_argv(argv, exe, authorize_peers,
  channel)`. `pub fn wire_claude_argv(argv, exe, authorize_peers)` keeps its signature and
  its exact output (it calls `claude_argv(.., false)`); add `pub fn
  wire_claude_channel_argv(argv, exe, authorize_peers)` that calls `claude_argv(.., true)`.
  With `channel` true: the MCP server entry's `args` is `["mcp", "--channel"]`, and two more
  extras, `--dangerously-load-development-channels` and `server:a2amx`, are added right
  after the `--mcp-config` pair (so the extras count is 10, or 12 with peer authorization).
  The hook settings and everything else are identical to the non-channel output.
- `pub fn channel_dialog_visible(screen: &Screen) -> bool`: true when some row contains
  the text `I am using this for local development` (use the existing
  `row_contains_marker`). Not part of `ready`, `has_dialog_marker` or any readiness rule.

## 5. `src/cli.rs` and `src/main.rs`

- `Command::Mcp` becomes `Mcp { #[arg(long)] channel: bool }` with the doc comment line
  `/// Run the MCP server; --channel also delivers messages as Claude Code channel events.`
  `a2amx mcp` without the flag behaves as today.
- `Command::New` gains `#[arg(long)] no_channel: bool`, doc comment `Claude only: deliver
  by typing into the terminal instead of through a channel.`
- `src/main.rs`: `Command::Mcp { channel } => return mcp::run(channel).await`, and the late
  arm that lists `Command::Mcp` matches `Command::Mcp { .. }`. In `run_new`, `Harness::Claude`
  uses `wire_claude_channel_argv` unless `no_channel`, and when it does push
  `(a2amx::harness::CHANNEL_ENV.to_owned(), "1".to_owned())` onto `env` (next to the existing
  OMP and Codex markers). `no_channel` is ignored for other harnesses.

## 6. `src/daemon.rs`

1. **Marker.** In `create_session`, before spawning: `let channel = harness == Harness::Claude
   && env.iter().any(|(key, value)| key == harness::CHANNEL_ENV && value == "1");` then
   `env.retain(|(key, _)| key != harness::CHANNEL_ENV);` for every harness (like
   `codex::NO_AUTHORIZE_ENV`). Pass `channel` in `SessionSpec`.
2. **Dialog auto-accept.** When `channel`, after the session is registered and its delivery
   task spawned, `tokio::spawn(accept_channel_dialog(session.clone()))` and keep the handle
   in `deliveries` like the other tasks. `async fn accept_channel_dialog(session:
   Arc<Session>)`: poll every 200 ms for at most 60 s; stop when `session.exit_code()` is
   some; on the first poll where `harness::channel_dialog_visible(&screen)` is true (the
   screen from `session.lock().emulator.screen()`, scrolling ignored), write `b"\r"` with
   `session.write_input` the same way the delivery code does without blocking the runtime,
   and return. Exactly one Enter is ever sent. `// shortcut:` a fixed 60 s window and one
   string match; re-probe the dialog text when Claude Code changes it.
3. **Bridge attach.** The `BridgeAttach` arm accepts `session.harness() == Harness::Omp ||
   session.channel()`; the rejection text stays `session is not an omp session` (an
   existing test pins it).
4. **Evidence.** In `message_info`, the `native_receipt` condition becomes `matches!(
   session.harness(), Harness::Omp | Harness::Codex) || session.channel()`.
5. **Receipt for channel prompts.** At the top of `report_prompt`, after the session lookup:
   if `session.channel()` and `messaging::unwrap_channel(&prompt)` returns `Some(inner)`,
   handle it here and return `Response::PromptVerdict { verdict: "allow", reason: None }`
   in every case:
   - `messaging::envelope_ids(&inner)` yields exactly one `seq`;
   - the stored message passes the same ownership checks as the PTY path (this boot, this
     session, state `Delivering`, `Submitted` or `Unsubmitted`, not yet `observed`);
   - `inner == messaging::channel_view(&messaging::render_envelope(&format!("m_{seq}"),
     &message.sender_address, &message.subject, &message.body))`;
   then `self.store.record_receipt(seq).await?` and `session.native_received()`.
   Anything else (no id, several ids, no match, a mismatch) records nothing and changes no
   state. This path never calls `note_submit`, `note_corrupted`, `reject_attempt` or builds
   a `block` verdict. The rest of `report_prompt` is untouched. A channel session's human
   prompts (not wrapped) go through the rest of the function as today.

## 7. `src/session.rs`

- `SessionSpec` gains `pub channel: bool`; `Session` stores it (a plain field, not in
  `State`) and gains `pub(crate) fn channel(&self) -> bool`. No other change; `NativeSlot`
  and every `native_*` method are untouched.

## 8. `src/messaging.rs` and `src/delivery.rs`

`src/messaging.rs`:

- `pub fn unwrap_channel(prompt: &str) -> Option<String>`: `Some(inner)` when `prompt`
  starts with `<channel source="a2amx">\n` and ends with `\n</channel>` and is longer than
  both together; `inner` is what lies between. Otherwise `None`.
- `pub fn channel_view(envelope: &str) -> String`: the text Claude Code's hook shows for a
  channel notification's content: every `</channel>` replaced with `<\/channel>`
  (case-sensitive, nothing else changes). `// shortcut:` only the lowercase closing tag
  was probed (Claude Code 2.1.288); other forms of the wrapper tag inside content are
  unprobed, and a mismatch merely means no receipt.

`src/delivery.rs`: in `AnyChannel::for_session`, add the arm `(Harness::Claude, _) if
session.channel() => Self::Native(NativeChannel { session })` before the PTY arm and update
the doc comment. Nothing else changes: a Claude session with `channel` false keeps the PTY
channel.

## 9. Tests

All tests: real daemon on port 0 in a temp dir via `tests/common`, no real Claude Code, no
fixed ports. `tests/claude_channel.rs` has its own helpers. Test through the public
interfaces; expected values are literals.

1. **`tests/claude_channel.rs`: the process.** Create a Claude session through
   `Request::NewSession` with the env marker `(CHANNEL_ENV, "1")` and a fake command; run
   `CARGO_BIN_EXE_a2amx mcp --channel` as a child with that session's `A2AMX_ADDR` and
   `A2AMX_TOKEN` and play Claude on its stdio. Tests:
   - `initialize` answers with `capabilities.experimental["claude/channel"]` and an
     `instructions` string; without `--channel` it does not (compare with a second child).
   - After `initialize` and `notifications/initialized`, a message sent to the session
     appears on the child's stdout as a notification with method
     `notifications/claude/channel`, `params.content` equal to the full envelope literal
     (`ENVELOPE_TEXT` style) and no `meta` key; `message_status` then shows `submitted`
     with evidence `write_complete`.
   - Two messages sent back to back both arrive without any prompt report in between
     (the `pending: false` state after the ack clears `in_flight`).
   - No bridge attached (the child is not started): the message stays `pending` with
     `channel_down`.
   - A tool call (`tools/call list_agents`) still answers while a delivery is pending.
2. **`tests/claude_channel.rs`: receipts.** With a bridge attached through the child (or a
   raw `Client::bridge` link, whichever is simpler), call `Request::ReportPrompt` with the
   session token:
   - prompt `<channel source="a2amx">\n` + envelope + `\n</channel>` gives verdict `allow`
     and the message's evidence becomes `native_receipt`;
   - the same with a message whose body contains `</channel>` and a prompt containing
     `<\/channel>` in its place gives `allow` and `native_receipt`;
   - a channel-wrapped prompt whose inner text differs from the envelope gives `allow`, no
     receipt, and the session is not marked corrupted (a following human prompt is
     `allow`);
   - an unwrapped human prompt on a channel session behaves as before (`allow`).
3. **`tests/claude_channel.rs`: launch.** `wire_claude_channel_argv` includes
   `--dangerously-load-development-channels` followed by `server:a2amx` and the MCP config
   `args` `["mcp","--channel"]`; `wire_claude_argv` output is unchanged. A session created
   without the marker is not a channel session: `Client::bridge` is refused with `session
   is not an omp session`. A `Harness::Claude` session without the marker still receives
   by PTY (one existing PTY test covers it; do not duplicate).
4. **Dialog.** A channel Claude session whose command is `sh -c` printing the line
   `I am using this for local development` then `read x; echo "$x" > "$OUT"` (a file
   fixture path in the temp dir passed through the environment) results in the file
   existing and holding an empty line within 10 s, and Enter was sent once. The same
   command in a session without the marker leaves the file absent after 3 s.
5. **Existing tests.** `tests/attach_cli.rs::claude_new_wires_mcp_and_optional_peer_authorization`
   now passes `--no-channel` in both `new` invocations (add the argument right after
   `--harness`, `claude`); nothing else in that file changes, and its assertions stay
   exactly as they are. Every other existing test passes unmodified; a failure of any other
   existing test means a regression to fix in production code, not a test to edit. If one
   truly cannot pass unchanged, stop and report BLOCKED with the failing assertion.
6. **TDD.** One behavior at a time, the failing test first. Write the section 3 tests
   against a stub, watch them fail for the right reason, then implement.

## 10. Docs and `AGENTS.md`

- `docs/delivery.md`: a section "Claude Code channel" next to the OMP native channel:
  default route, `--no-channel`, the dev-flag dialog and its auto-accept, receipt through
  the prompt hook with the exact wrapper shape, the `</channel>` rewrite, the silent-drop
  ceiling (D3, D9), no draft hold and why (the probe), the batching behavior, and that
  channels are a research preview that needs a claude.ai or Console login. The status
  paragraph at the top lists this as an implemented slice.
- `docs/architecture.md`: the `mcp` and `channel` modules; the Claude route.
- `README.md`: one short paragraph and the `--no-channel` flag where `--harness claude` is
  described.
- `AGENTS.md`: module table gains a `channel` row, `The \`a2amx mcp --channel\` process
  that relays daemon deliveries to Claude Code as channel events`, and the `mcp` row stays.
- Fictional hosts and names only (`host-a`, `agent-plan`); no personal data.

## 11. Out of scope

- Mid-turn injection, permission relay (`claude/channel/permission`) and two-way channel
  reply tools: send_message already covers replies.
- Reconnecting a lost channel link; reading the Claude draft; any wire protocol change;
  renaming `omp_version`.
- Changing `hold_explanation` texts: `channel_down` still says "extension"; revisit if it
  confuses users.
- The live check against a real Claude Code session: done by the reviewer, not by tests.
- Removing the PTY route for Claude.

## 12. Acceptance

Run, from the repository root, in this order, and all must pass with no warnings:

```sh
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Then `git status` shows only the files listed in section 0, uncommitted, and `git diff`
touches no unmodified function outside the sections above.

## Amendment 1 (after the developer reported BLOCKED on the section 4 argv count)

Section 4 miscounted. `wire_claude_argv` pushes 8 extras with peer authorization and 6
without (`tests/attach_cli.rs` pins both). The two channel flags add two entries, so
`wire_claude_channel_argv` yields **10** extras with peer authorization and **8** without,
not 12 and 10. Everything else in section 4 stands: `--dangerously-load-development-channels`
and `server:a2amx` go right after the `--mcp-config` pair, the MCP entry's `args` is
`["mcp","--channel"]`, and `wire_claude_argv` output is byte-identical to today. The only
edit this amendment makes is that count; implement the argv test of section 9.3 against 10
and 8. No other section changes.
