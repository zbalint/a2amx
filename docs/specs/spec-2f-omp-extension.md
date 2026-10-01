# Spec 2f: the OMP extension and launch wiring

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 were settled with the owner. This spec builds
on spec 2e (`535184c`, branch `spec/2e-native-channel`); the worktree for 2f is cut from
that commit, so 2e must be merged before 2f.

**Scope.** May edit exactly these files and no others. New files are marked.

- `extension/omp.ts` (new)
- `src/omp.rs` (new)
- `src/harness.rs`, `src/mcp.rs`, `src/main.rs`, `src/lib.rs`
- `tests/omp_wiring.rs` (new), `tests/omp_smoke.rs` (new)
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `AGENTS.md`

Does not touch: `src/store.rs`, `src/session.rs`, `src/delivery.rs`, `src/daemon.rs`,
`src/bridge.rs`, `src/client.rs`, `src/wire.rs`, `src/cli.rs`, `src/messaging.rs`,
`src/emulator.rs`, `src/prefix.rs`, `src/hook.rs`, `Cargo.toml`, `Cargo.lock`, every
existing file under `tests/` (the existing test diff must be empty; `tests/common` is not
edited), `docs/validation-plan.md`, the locked spec documents under `docs/specs/`. No new
dependency. Do not commit and do not merge.

## 1. Why

Spec 2e built the daemon side of native delivery to OMP: the `omp` harness, the channel,
the bridge protocol and the `a2amx omp-bridge` relay. Nothing yet speaks to the relay from
inside OMP, so an `omp` session's messages wait with `channel_down`. This spec adds the
OMP extension (a TypeScript file embedded in the binary), the launch wiring that installs
and loads it (`a2amx new --harness omp`), and a smoke test that runs the real OMP.

Probes on OMP 18.4.8 (memories `af5b6ca6`, `3149c1dc`) fix the design: extension tools are
`xd://` devices by default and become native tools with a per-session `--config` overlay;
`pi.registerTool` accepts a plain JSON Schema; OMP adds a required `i` ("intent") argument
to every tool; `ctx.agent.kind` is `sub` in subagents, where the extension is loaded again;
`hasPendingMessages()` is false right after a send and true about 500 ms later;
`pi.pi.VERSION` carries the OMP version; the idle and mid-turn native sends, the draft read
and the byte-exact agent-attributed `message_end` receipt work as on 18.4.5.

## 2. Decisions settled with the owner

- **D1.** Messaging tools are native tools through a per-session overlay
  (`--config <state>/omp/overlay.yml` with `tools.xdev: false`), tried first; the user's own
  OMP settings are never touched. (Side effect, accepted: those sessions also get
  `ast_edit` and `debug` as native tools.)
- **D2.** The extension finds the binary through `A2AMX_BIN`, which `a2amx new` adds to the
  session environment. The token and address come only from the environment the daemon
  already sets (`A2AMX_TOKEN`, `A2AMX_ADDR`), never from argv or the file.
- **D3.** The source is `extension/omp.ts`, embedded with `include_str!` and rendered with
  the tool schemas of `src/mcp.rs` (one source of truth, no schema copy to drift). `a2amx
  new --harness omp`, running in the user's CLI process, writes
  `<state dir>/omp/extension-<crate version>.ts` and `<state dir>/omp/overlay.yml`:
  atomically, files 0600 in a 0700 directory, rewritten whenever the content differs. It
  passes `-e`, `--config` and `--append-system-prompt` (the shared
  `PEER_AUTHORIZATION_PROMPT`, skipped with `--no-authorize-peers`) and never passes
  `--no-extensions`.
- **D4.** Tests: Rust wiring tests with the `sh -c` stand-in as for Claude, plus an
  `#[ignore]` smoke test that runs real OMP against a fake model server. The smoke test is
  a developer-time test (`cargo test -- --ignored`); it never runs in user sessions and
  skips itself when `omp` is missing. The expected `cargo test` count therefore excludes it.
- **D5.** State reporting: on load, `agent_start`, `agent_end`, `turn_start`, `turn_end`,
  every send, every matching receipt and a 500 ms poll that reports only changes. `draft` is
  `getEditorText().trim() !== ""`; `pending` is "the extension's own FIFO is non-empty or
  `hasPendingMessages()`" with the safety nets of section 3.5; a `state` sent right after an
  `ack` always carries `pending: true`.
- **D6.** Supervision: a nonzero exit respawns the bridge with backoff 300 ms doubling to
  5 s; exit 0 never respawns; every respawn sends `hello` and, after `ready`, the current `state`; ten
  consecutive failed starts show one notice and retrying continues; `session_shutdown`
  closes the bridge's stdin without respawning.
- **D7.** Loud failures show once per reason through `ctx.ui.notify(message, "error")`; the
  same reasons reach senders through `channel_down` or `channel_refused`.
- **D8.** Details applied by default: the extension strips the `i` argument before relaying
  a tool call; a tool result with `isError: true` is thrown as an error and otherwise its
  content is returned; tool names are unprefixed (`list_agents`, `send_message`,
  `message_status`); delivery uses no `deliverAs` when OMP is idle and `followUp` when not,
  never `steer`; a throwing send becomes a `nack` with the error text capped at 200
  characters; there is no status-bar indicator.
- **D9.** Only `ctx.agent.kind === "main"` ever acts; in a subagent session the extension
  registers no tools, starts no bridge and delivers nothing.

## 3. `extension/omp.ts`

Plain TypeScript with no imports except Node built-ins (`node:child_process`); OMP strips
the types, so write it so it also runs as plain JavaScript semantics. A default export
`function (pi)` registers the handlers below and does nothing else at load time. Two
markers are replaced when the file is rendered (section 4):

- `const PROTOCOL = 1;` must equal `BRIDGE_PROTOCOL` in `src/wire.rs`.
- `const TOOLS = [/* a2amx:tools */];` is rendered into a JSON array of the three tool
  schemas `{ name, description, inputSchema }` from `src/mcp.rs`.

### 3.1 Start (main session only)

On `session_start` (handler `(event, ctx)`), when `ctx.agent?.kind !== "main"` return
immediately. A later `session_start` in the same process only updates the stored `ctx`
(OMP can start another session; the bridge and tools are created once).

1. Read `A2AMX_BIN`, `A2AMX_ADDR` and `A2AMX_TOKEN` from `process.env`. If one is missing,
   notify `A2AMX could not start: <NAME> is not set. Start this session with a2amx new
   --harness omp.` and stop.
2. Compute `missing`: the names, from this fixed list in this order, whose value is not a
   function: `sendUserMessage` (`pi.sendUserMessage`), `registerTool` (`pi.registerTool`),
   `isIdle` (`ctx.isIdle`), `hasPendingMessages` (`ctx.hasPendingMessages`),
   `getEditorText` (`ctx.ui.getEditorText`).
3. Unless `registerTool` is missing, register the three tools from `TOOLS`
   (`pi.registerTool({ name, label: name, description, parameters: inputSchema, execute })`;
   the schema is a plain JSON Schema object). Registering from `session_start` (not at load)
   is what keeps subagents tool-free; it was probed to reach the first model request.
4. Start the bridge (3.2).

### 3.2 The bridge child

Spawn `process.env.A2AMX_BIN` with args `["omp-bridge"]`, `stdio: ["pipe", "pipe",
"pipe"]`, inheriting the environment (the child reads the token and address from it).
Immediately write the `hello` line (the first `state` follows the `ready` answer):

```json
{"type":"hello","protocol":1,"omp_version":"18.4.8","missing":[]}
```

`omp_version` is `String(pi.pi?.VERSION ?? "unknown")`; `missing` is the list of 3.1 step 2.
Frames are single JSON lines on the child's stdin and stdout; keep the last 2 KiB of the
child's stderr for notices.

Lines read from the child (`type`):

- `ready`: the bridge is connected; reset the failure count and backoff; send `state`.
- `refused` with `reason`: `protocol_mismatch` notify `A2AMX and this OMP extension
  disagree on the protocol version; update A2AMX.`; a reason starting `missing_apis: `
  notify `A2AMX needs OMP features this OMP version lacks (<the list after the colon>);
  update A2AMX or OMP.`; `session is not an omp session` notify `This session was not
  started with a2amx new --harness omp.`; each of these is final (never respawn).
  `a bridge is already attached to this session` is not final: treat it as a failed start
  and retry with backoff (a respawned bridge can race the daemon dropping the old one).
  Any other reason: notify it verbatim and stop.
- `deliver` with `id` and `envelope`: section 3.3.
- `mcp` with `response`: resolve the pending tool call whose JSON-RPC id equals
  `response.id`.

Child exit: exit code 0 means stop (no respawn); anything else (nonzero code, a signal)
respawns after the backoff. Pending tool calls are rejected with `A2AMX connection lost`.
A start fails when the child exits before `ready`; after 10 consecutive failed starts notify
once `A2AMX cannot reach its daemon (<last stderr line>); still retrying.` and keep trying
at the capped backoff. `session_shutdown` ends the child's stdin and, after 1 s, kills it if
still alive, and prevents respawning.

### 3.3 Delivery

For `deliver` `{ id, envelope }`:

```ts
const idle = ctx.isIdle();
try {
  pi.sendUserMessage(envelope, idle ? { attribution: "agent" }
                                    : { deliverAs: "followUp", attribution: "agent" });
} catch (error) {
  send({ type: "nack", id, reason: String(error?.message ?? error).slice(0, 200) });
  return;
}
fifo.push({ id, envelope, at: Date.now() });
send({ type: "ack", id });
reportState(true);   // forced; pending is true because the FIFO is non-empty
```

`sendUserMessage` returns undefined and only means queued. Never pass `deliverAs: "steer"`.
Send the `ack` after the call returns without throwing and before any other frame.

### 3.4 Receipts

On `message_end` (`event.message`): when `message.role === "user"` and
`message.attribution === "agent"`, build `text` as the concatenation of the `text` of every
content part with `type === "text"` (or the content itself when it is a string). If the FIFO
is non-empty and `text === fifo[0].envelope` exactly, shift it, send `{"type":"receipt",
"id": <that id>}` and report state. Any other agent-attributed message is ignored. The
probes showed the text arrives byte-identical (no trim, tab or CRLF rewrite).

### 3.5 State reporting and its safety nets

`reportState(force)` computes `{ idle: ctx.isIdle(), pending, draft }` and sends it as
`{"type":"state","idle":..,"pending":..,"draft":..}` only when the bridge is `ready` and the
value differs from the last one sent, or when `force` is true. Triggers are in D5. `draft` is
`false` when `getEditorText` is missing or throws. `pending` is `fifo.length > 0 ||
ctx.hasPendingMessages()` after the safety nets:

- If `hasPendingMessages()` was seen true for the FIFO head and is now false while the FIFO
  is non-empty, drop that entry (OMP dequeued it; a changed `message_end` shape must not
  block delivery forever).
- Drop a FIFO entry older than 120 s when `hasPendingMessages()` is false.

The poll uses `setInterval(500)` while the bridge is ready, unreferenced. Leave
`// shortcut:` comments on the polling (OMP has no editor-change event; use one if it gains
it) and on the 120 s grace (a fixed value).

### 3.6 Tools

`execute(toolCallId, params)`: remove the `i` property from `params`; if the bridge is not
`ready` throw `A2AMX is not connected`; send `{"type":"mcp","request":{"jsonrpc":"2.0",
"id":N,"method":"tools/call","params":{"name":<name>,"arguments":<params without i>}}}`
with a fresh integer `N`; wait up to 60 s for the matching `mcp` response (reject
`A2AMX did not answer in time`); `response.error` throws its `message`;
`response.result.isError === true` throws the text of the first content part; otherwise
return `{ content: response.result.content }`.

## 4. `src/omp.rs` (new)

```rust
pub struct Installed { pub extension: PathBuf, pub overlay: PathBuf }

/// The embedded extension with the tool schemas rendered in.
pub fn render_extension() -> String
/// The per-session overlay: native tools for this session only.
pub fn overlay() -> &'static str          // "tools:\n  xdev: false\n"
/// Writes both files under `<home>/omp`; blocking file syscalls, call from spawn_blocking.
pub fn install(home: &Path) -> anyhow::Result<Installed>
```

- `render_extension` replaces the exact text `[/* a2amx:tools */]` in the embedded
  `include_str!("../extension/omp.ts")` with `serde_json::to_string` of
  `crate::mcp::tool_schemas()` mapped to `{name, description, inputSchema}` objects (it is
  already that shape). It uses `str::replace`; that the marker is present in the file is
  checked by test 5 of section 8.
- `install`: `let home = std::path::absolute(home)?`; the directory is `<home>/omp`
  (`create_dir_all`, then permissions 0700 on that directory); the files are
  `extension-<CARGO_PKG_VERSION>.ts` and `overlay.yml`. A file is written when it is
  missing or its content differs, atomically: write a temporary file in the same directory
  created with mode 0600, then `rename`; afterwards the mode is 0600 whether or not the
  content was rewritten. The returned paths are absolute.

## 5. `src/harness.rs`

- Add `pub fn wire_omp_argv(argv: Vec<String>, extension: &Path, overlay: &Path,
  authorize_peers: bool) -> Vec<String>`. The extras are `-e`, the extension path,
  `--config`, the overlay path and, when `authorize_peers`, `--append-system-prompt` and
  `PEER_AUTHORIZATION_PROMPT`; they go before the first `--` in `argv`, else at the end.
- Move the four-line "insert before the separator or at the end" logic of
  `wire_claude_argv` into one private helper used by both functions (extend, do not copy);
  `wire_claude_argv` keeps its exact output.

## 6. `src/mcp.rs`

`fn tool_schemas() -> Value` becomes `pub(crate) fn tool_schemas() -> Value`. No other
change.

## 7. `src/main.rs` and `src/lib.rs`

- `src/lib.rs`: `pub mod omp;` in alphabetical position.
- `run_new`: replace the Claude-only `let command = if harness == Harness::Claude {...}`
  with a `match harness`: `Claude` as today; `Omp` runs `omp::install(&home)` on
  `tokio::task::spawn_blocking`, then `a2amx::harness::wire_omp_argv(command,
  &installed.extension, &installed.overlay, !no_authorize_peers)`; `Generic` returns the
  command unchanged. For `Omp` also add `("A2AMX_BIN", <std::env::current_exe()? as a
  string>)` to the end of the environment list (the daemon's later-wins map keeps it over an
  inherited value); `Claude` and `Generic` add nothing.

## 8. `tests/omp_wiring.rs` (new)

Integration tests, no OMP needed, in the style of the Claude wiring test in
`tests/attach_cli.rs` (the `a2amx` binary, a temp state dir, `sh -c` printing its
arguments and `A2AMX_BIN` to files, no fixed ports). One test per behavior, written failing
first:

1. `omp_new_wires_the_extension_overlay_and_operator_line`: `new --detach --harness omp --
   sh -c '<print "$@" and "$A2AMX_BIN" to files>; sleep 30' sh`; the arguments are exactly
   `-e`, `<home>/omp/extension-<version>.ts`, `--config`, `<home>/omp/overlay.yml`,
   `--append-system-prompt`, `PEER_AUTHORIZATION_PROMPT`; `A2AMX_BIN` equals the
   canonicalized `CARGO_BIN_EXE_a2amx`.
2. `no_authorize_peers_leaves_the_operator_line_out`: the same with `--no-authorize-peers`
   gives only the first four arguments.
3. `generic_sessions_get_no_a2amx_bin`: a `--harness generic` session prints the literal
   `unset` for `${A2AMX_BIN-unset}` (the test removes `A2AMX_BIN` from its own child
   environment first).
4. `install_writes_owner_only_files_and_repairs_edits`: `omp::install` returns absolute
   paths; the directory is 0700 and both files 0600; the extension equals
   `omp::render_extension()` and the overlay equals `"tools:\n  xdev: false\n"`; after the
   test overwrites the extension with `x` and chmods it 0644, a second `install` restores
   both content and mode.
5. `the_rendered_extension_embeds_the_tool_schemas_and_the_protocol`: the rendered text
   contains `"list_agents"`, `"send_message"`, `"message_status"` and the literal sentence
   `Returns a message id once the message is accepted`; does not contain
   `[/* a2amx:tools */]`; contains the line `const PROTOCOL = <BRIDGE_PROTOCOL>;`.
6. `wire_omp_argv_inserts_before_a_separator_and_honors_authorize_peers`:
   `["omp","--","hello"]` with peers authorized becomes `["omp","-e",E,"--config",O,
   "--append-system-prompt",PEER_AUTHORIZATION_PROMPT,"--","hello"]`; without it the last
   two extras are absent.

Six new tests.

## 9. `tests/omp_smoke.rs` (new)

Three tests, each `#[ignore = "needs the omp binary; run with cargo test -- --ignored"]`;
each first runs `omp --version` and, when it cannot be run, prints a skip line and returns.
They use the real OMP in print mode against a fake model server written in the test file
(tokio `TcpListener`, a hand-rolled HTTP/1.1 reader and an SSE writer; no new dependency):

- The server answers `POST` to a path ending in `/chat/completions` with an OpenAI-style
  streaming completion and records, per request, the text of the last user message, the
  names of the tools sent, and the text of any tool message.
- Script by the last user text: containing `SLOW`: eight content chunks one second apart,
  then `stop`; containing `CALL ` followed by JSON `{"name":..,"args":..}`: one tool call
  with that name and arguments, only when the last message is a user message; containing
  `SPAWN`: one `task` tool call with arguments `{"i":"Spawning probe","context":"probe
  context","tasks":[{"task":"Reply with the word ok","solutionSpace":"none"}]}`; otherwise
  the single chunk `ok`.
- A scratch agent directory (`PI_CODING_AGENT_DIR`) whose `models.yml` declares provider
  `fake` (`openai-completions`, `baseUrl` `http://127.0.0.1:<port>/v1`, `apiKey`
  `fake-key`, model `fake-1` with `contextWindow` 32000 and `maxTokens` 4096).
- A real daemon (`Daemon::start`, temp state dir, host name `host-a`) and the session
  started with the `a2amx` binary: `new --detach --name agent-review --harness omp -- sh -c
  'exec omp -p --no-prewalk --no-session --no-title --no-lsp --model fake/fake-1 "$0" <
  /dev/null' "<prompt>"` with `PI_CODING_AGENT_DIR` in the environment (stdin must be
  closed or print mode hangs).

Tests:

1. `a_native_message_reaches_real_omp_and_is_receipted`: prompt `SLOW work`; a generic
   session `agent-plan` sends `agent-review@host-a` the subject `Parser issue` and body `I
   found the regression in parser.py.` right after the launch; the message waits pending,
   then reaches `submitted` with evidence `native_receipt` (up to 60 s); the fake server
   saw a user message equal to the envelope literal of `tests/delivery.rs` (`ENVELOPE_TEXT`,
   copied into the file), and no recorded user message contains `User interjection`.
2. `real_omp_calls_the_messaging_tools_natively`: prompt `CALL {"name":"list_agents",
   "args":{"i":"Listing agents"}}`; the tool names of that request include `list_agents`,
   `send_message` and `message_status`; the next request carries a tool message containing
   `agent-review@host-a`.
3. `a_subagent_gets_no_messaging_tools`: prompt `SPAWN a subagent`; the request whose last
   user text contains `Reply with the word ok` lists no `send_message`, while the first
   request does.

These tests need the extension of section 3 and the launch wiring of sections 4 to 7. They
are slow (up to about 20 s each) and run only on request. Not executed by plain `cargo
test`; the three appear as `ignored`.

## 10. Docs and `AGENTS.md`

- `AGENTS.md`, Modules table: add `| \`omp\` | The embedded OMP extension and its launch
  files |` after the `bridge` row.
- `README.md`: in "Building and trying it", after the paragraph about `--harness claude`,
  add `a2amx new --name agent-review --harness omp -- omp` to the example block (after the
  claude line) and one sentence: "`--harness omp` writes the A2AMX extension into the state
  directory, loads it into OMP with `-e` and a per-session `--config` overlay, and delivers
  messages natively without typing into the terminal." In "Using it with agents", change
  "`--harness claude` adds an operator line" to "`--harness claude` and `--harness omp` add
  an operator line".
- `docs/architecture.md`, the scope table row at line 394: the MVP cell becomes
  `PTY delivery channel, plus a native OMP channel; submission receipts optional per
  harness profile`; the "Later" cell becomes `Other native in-harness delivery channels,
  added as adapters`.
- `docs/delivery.md`, the section `## Implemented: native channel (OMP)`: replace the
  two sentences "The extension and its launch wiring ship in spec 2f; until then, an `omp`
  session's messages wait with `channel_down`. Do not release spec 2e alone." with "The
  extension (`extension/omp.ts`, spec 2f) feeds the channel; without a connected extension
  an `omp` session's messages wait with `channel_down`." Append a subsection `### Launch
  and extension` stating: the files and flags of D3, the environment (`A2AMX_BIN`,
  `A2AMX_TOKEN`, `A2AMX_ADDR`), that tools are native through the overlay and why (OMP 18.4.8
  lists extension tools only as `xd://` devices otherwise), that the extension strips OMP's
  `i` argument, the state reporting and safety nets of 3.5, the supervision of D6, the
  notices of D7, the main-only rule, and that `cargo test -- --ignored` runs the real-OMP
  smoke tests.
- No other doc changes. Public repository rules apply: fictional names and hosts only.

## 11. Out of scope

A Codex or Claude native channel; any change to the daemon, the bridge protocol, the wire
types, the store or the 2e tests; a status-bar indicator; reconnecting inside the bridge;
persisting the channel on a receipt; changing the user's own OMP configuration;
`--no-extensions` handling; the human-visible approval-dialog behavior of OMP; refactors
beyond what sections 4 to 7 name.

## 12. Acceptance

All three must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The ignored smoke tests, run once in addition (they need `omp` and take about a minute):

```sh
cargo test --test omp_smoke -- --ignored
```

And, run from the worktree root after implementation, in a real shell:

```sh
git status --porcelain | sort
git status --porcelain -- tests
rg -l "include_str!" src
rg -n "A2AMX_TOKEN" src/omp.rs src/harness.rs
rg -l "a2amx:tools" extension src | sort
git diff --check
```

Expected: the first lists exactly these lines, in this order (` M` sorts before `??`):
` M AGENTS.md`, ` M README.md`, ` M docs/architecture.md`, ` M docs/delivery.md`,
` M src/harness.rs`, ` M src/lib.rs`, ` M src/main.rs`, ` M src/mcp.rs`,
`?? extension/omp.ts`, `?? src/omp.rs`, `?? tests/omp_smoke.rs`, `?? tests/omp_wiring.rs`;
the second prints exactly `?? tests/omp_smoke.rs` and `?? tests/omp_wiring.rs`; the third
prints exactly `src/omp.rs`; the fourth prints nothing (exit status 1: the token never
appears in the launch code; the extension only checks that `process.env.A2AMX_TOKEN` is
set and the bridge child inherits it); the fifth prints exactly `extension/omp.ts` and
`src/omp.rs`; the last prints nothing. The existing test count is 178; with the six new wiring tests
`cargo test` reports 184 passed and 3 ignored. The smoke tests all pass when run. Leave the
diff uncommitted and unmerged.
