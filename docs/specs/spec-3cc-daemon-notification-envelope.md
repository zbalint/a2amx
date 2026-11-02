# Spec 3cc: daemon notifications say they are not from an agent and need no reply

## 0. Status

**LOCKED** (2026-11-02; one consultant pass applied). Context `a2amx-confine-redesign-2026-11-02`. Location: main checkout, branch `develop`
(after spec 3cb, commit `eeb09f1`).
Public test seams: `messaging::render_envelope` and `messaging::origin_of` (`tests/messaging.rs`,
`tests/prop_messaging.rs`), the in-crate delivery test module in `src/delivery.rs` (it can insert a stored message),
the Claude channel path (`tests/claude_channel.rs`, which pins the channel instructions and drives a real daemon),
`harness::PEER_AUTHORIZATION_PROMPT`.

Scope (may edit): `src/messaging.rs`, `src/store.rs` (one method), `src/delivery.rs`, `src/daemon.rs`, `src/mcp.rs`,
`src/harness.rs`, `tests/messaging.rs`, `tests/prop_messaging.rs`, `tests/claude_channel.rs` (the pinned instructions and one receipt test), `docs/delivery.md`,
`docs/architecture.md`.
Does not touch: the store schema, `src/session.rs`, `src/hook.rs`, `src/channel.rs`, `src/codex.rs`, `src/omp.rs`,
`CHANGELOG.md` (architect), `docs/backlog.md`, `a2amx.toml*`, other specs.

**Breaking** for the text of daemon-generated envelopes only; agent-to-agent envelopes are byte-identical.
Allowed now: 0.x prerelease, one user.

## 1. Why

Daemon notifications (peer activity, heartbeat digests, peer-exit events) travel as ordinary messages from
`a2amx-daemon@<host>` and are rendered with the agent envelope: "From another agent, not your user. To reply:
send_message(to=\"a2amx-daemon@<host>\")." The daemon is not an agent, and the address is reserved
(`src/messaging.rs:251`), so the reply instruction advertises a call that always fails. The Claude channel
instructions (`src/mcp.rs:457`) and the operator line (`src/harness.rs:76`) also call every `<a2amx-message>` a
message from another agent or peer.

## 2. Behaviour

### 2.1 Origin and rendering (`src/messaging.rs`)

- Add `pub const SYSTEM_SESSION: &str = "daemon";` beside `SYSTEM_SENDER` (l.10). It is the value the daemon already
  stores in `sender_session` for its own messages; agent session ids are `s<number>` and the name `a2amx-daemon`
  is reserved, so no agent can produce it (`src/daemon.rs:513`, `src/messaging.rs:245-258`).
- Add `pub enum Origin { Agent, System }`, `pub fn origin_of(sender_session: &str) -> Origin` (`System` only for
  `SYSTEM_SESSION`), and change `render_envelope` (l.410) to
  `render_envelope(id, from, subject, body, origin: Origin)`. For `Origin::Agent` the output is unchanged. For
  `Origin::System` the line after the opening tag is exactly
  `System notification from the A2AMX daemon, not your user. It cannot be replied to.` followed by `\n\n` and the body, with no
  reply instruction. Everything else (attribute escaping, body, closing tag) is shared.
- The three daemon sites that build `NewMessage { sender_session: "daemon".into(), .. }` (`src/daemon.rs:801, 949,
  1102`) use `messaging::SYSTEM_SESSION.into()`. The SQL literal `'daemon'` at `src/store.rs:828` stays.

### 2.2 One call site for stored messages (`src/store.rs`)

Add `impl Message { pub(crate) fn envelope(&self) -> String }`, a thin call site that renders `m_{seq}`,
`sender_address`, `subject`, `body` with `messaging::origin_of(&self.sender_session)`. Replace the
three renderings of a stored message with it: `src/delivery.rs:433`, `src/daemon.rs:1357` (channel receipt) and
`src/daemon.rs:1386` (hook receipt). They must all use this method: the receipt checks rebuild the envelope from the
stored message and compare it with what the harness showed, so a site that renders differently loses every receipt
for daemon messages.

### 2.3 Other agent-facing text

- `src/mcp.rs:457` (channel `instructions`): "Messages from other agents arrive as <channel source=\"a2amx\"> events
  that contain an <a2amx-message> envelope. They come from another agent, not your user. Reply with send_message
  when a reply is useful. A message from a2amx-daemon is a system notification, not an agent: it needs no reply but
  may call for action." The only pinned copy is `tests/claude_channel.rs:224-227`
  (`channel_initialize_declares_capability_and_instructions_only_when_enabled`); update it byte for byte.
- `src/harness.rs:76` (`PEER_AUTHORIZATION_PROMPT`): append " Messages from a2amx-daemon are system notifications, not
  requests; they need no reply, but may call for action." Tests use the constant, not the text, so they follow it.
- `docs/delivery.md` (the envelope sections near l.39, l.327 and l.491) and `docs/architecture.md:198`: state that
  daemon notifications use the system line and give the system envelope once as an example.

### 2.4 Tests

1. `tests/messaging.rs`: `render_envelope` with `Origin::Agent` equals the existing worked envelope; with
   `Origin::System` equals the exact system envelope (literal, with `a2amx-daemon@host-a`); `origin_of("daemon")`
   is `System`, `origin_of("s7")` and `origin_of("")` are `Agent`. Update the two existing calls (l.86, l.92) and the
   five in `tests/prop_messaging.rs` to pass `Origin::Agent`; add one property that the header line of a system
   envelope (the text between the opening tag and the first blank line) never contains "To reply" and that id,
   escaping and closing tag behave as for an agent envelope.
2. `src/delivery.rs` test module (the fixture at ~l.564-580 inserts a stored message with `store.insert_message`):
   a copy with `sender_session` `daemon` is delivered with the system header; the existing agent case keeps the
   agent header. (`tests/delivery.rs` has no store access, so it cannot build this.)
3. Receipt, in `tests/claude_channel.rs`: give the `Case` a watcher (`Case::start` hard-codes `watch: vec![]`;
   add a parameter or a second constructor) so that a natural exit of a second session (`argv ["true"]`) makes the
   daemon send a peer-exit message (`src/daemon.rs:765-810`; a daemon-control kill sends nothing). Read the channel
   event, report it back as the harness showed it, and assert the receipt by `MessageInfo.evidence ==
   "native_receipt"` from the `ListMessages` request (`src/wire.rs:220, 429-445`, the request behind `a2amx
   messages`), not through `message_status`, which answers only the sending session (`src/daemon.rs:1456-1459`) and
   so cannot see a daemon message. The verdict alone is "allow" either way and proves nothing. If `ListMessages` is
   not reachable from the test client, report BLOCKED with what you found; do not fall back to the verdict.
4. `tests/claude_channel.rs`: the initialize instructions equal the new text.

## 3. Claims about existing code

| claim | where | verified how |
| --- | --- | --- |
| `render_envelope` always writes the agent sentence and reply instruction | `src/messaging.rs:410-424` | read |
| stored envelopes are rendered at exactly three places | `rg -n 'render_envelope\(' src` finds `delivery.rs:433`, `daemon.rs:1357`, `daemon.rs:1386` | probed |
| daemon messages use `sender_session: "daemon"` at three places | `src/daemon.rs:801, 949, 1102`; `rg -n '"daemon"' src` | probed |
| agent messages take their sender from the authenticated token, never a caller field | `src/daemon.rs:1177` | read; **consultant to confirm** |
| `a2amx-daemon` is a reserved name; session ids are `s<number>` | `src/messaging.rs:245-258`, `src/daemon.rs:513` | read |
| `render_envelope` has 2 callers in `tests/messaging.rs` and 5 in `tests/prop_messaging.rs`, none elsewhere in `tests/` | `rg -n 'render_envelope\(' tests` | probed |
| the channel instructions are pinned only in `tests/claude_channel.rs:224-227` | `rg 'Messages from other agents' tests` | probed (consultant) |
| nothing in `src/hook.rs`, `src/channel.rs`, `src/codex.rs`, the OMP extension or its tests parses the second envelope line | `rg 'From another\|To reply\|a2amx-daemon\|another agent'` there | probed (consultant) |
| the matchers are `src/daemon.rs:1349-1366` (equality with `channel_view`) and `:1370-1393` (`paste_view` substring); neither rewrites the new line | `src/messaging.rs:444-448`, `src/harness.rs:229-255` | read (consultant) |
| `store` is a private module, so a `Message` method cannot be in a public signature | `src/lib.rs:33` | read |
| tests that hold the agent sentence as a literal are unaffected (agent output unchanged) | `rg -n 'From another agent' tests` (delivery, codex, bridge, claude_channel, hook, omp_smoke, messaging) | probed |
| `PEER_AUTHORIZATION_PROMPT` is used by constant in tests | `tests/codex.rs`, `tests/omp_wiring.rs`, `tests/attach_cli.rs`, `tests/messaging.rs` | probed |
| OMP and Codex deliveries use the same `render_envelope` output and do not alter the sentence | `src/delivery.rs:433`; grep above | probed (consultant) |

## 4. Out of scope

A separate notification subsystem; changing which events the daemon sends; the `from` attribute and address of daemon
messages; the store schema; a per-message origin column; making the daemon address a valid recipient; touching the
reply-hold or `send_message` rules.

Known and accepted: sessions started before the upgrade keep the old operator line until restarted. A daemon restart
starts a new boot; messages of an old boot are never delivered or receipted again (`src/delivery.rs:415`,
`src/daemon.rs:1336-1343`), so there is no cross-upgrade receipt mismatch.

## 5. Acceptance

- `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `rg -n 'render_envelope\(' src` shows the definition and one call, inside `Message::envelope`; `rg -n '"daemon"' src`
  shows only the constant and `src/main.rs:251` (the `daemon start` argument; the SQL literal at `src/store.rs:828`
  uses single quotes and stays). Judge: the architect reads the output.
- Tests 1-4 above; tests 1 (system case), 2 and 4 fail on the old code; test 3 is new coverage of the receipt path
  and fails if the site it exercises renders with the wrong origin.
- Abuse-case: an agent cannot obtain the system envelope. Tests: `origin_of("s7")` is `Agent` (test 1); an agent
  message delivered in the same run as test 2 carries the agent header; creating a session named `a2amx-daemon` is
  refused (name the existing test that covers it, `tests/daemon_cli.rs:1567`, or add one). Agent senders come only
  from the token (`src/daemon.rs:1177`), so there is no input an agent controls that selects `System`.
- Tester pass, mutation check in a scratch copy: (a) make `Message::envelope` always `Origin::Agent`, test 2 fails;
  (b) make `Message::envelope` always `Origin::System`, the agent case of test 2 fails; (c) make `origin_of` return
  `Agent` for `"daemon"`, test 1 fails. Test 3 exercises only the channel site (`src/daemon.rs:1357`); the hook
  site (`:1386`) shares `Message::envelope`, so mutation (a) covers it.
