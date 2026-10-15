# Spec 3aj: behavior tests for the OMP extension (`extension/omp.ts`)

## 0. Status

**LOCKED** (2026-10-15; consultant review m_1819 folded in). Context `a2amx-omp-ext-tests`. Implementation goes to `a2amx-developer@desktop-aavo022` only.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. The untracked `.cargo/` belongs to the owner: do not touch it.
**Public test seam:** the extension's two boundaries only. Inbound: the OMP extension API (`pi.on(...)` handlers, `pi.sendUserMessage`, `pi.registerTool`, and the session context `ctx` with `isIdle`, `hasPendingMessages`, `ui.getEditorText`, `ui.notify`, `agent.kind`). Outbound: the `a2amx omp-bridge` child process, spoken to as JSON lines on its stdin/stdout (frame shapes in `docs/delivery.md` and `src/bridge.rs`). No test reads a closure variable of the extension.

**Scope.** May create exactly these files and no others:

- `tests/omp_extension.rs` (Rust wrapper, section 3)
- `tests/omp_extension/fake_bridge.mjs` (section 4)
- `tests/omp_extension/harness.mjs` (section 5)
- `tests/omp_extension/*.test.mjs` (section 6; any number of files, names are the developer's choice)

Does not touch: `extension/omp.ts` (must be byte-identical at handoff, see section 8), `src/**`, `Cargo.toml`, `Cargo.lock`, `.github/**`, `sonar-project.properties`, `scripts/**`, `AGENTS.md`, `CHANGELOG.md`, `docs/**` other than this spec, every other test file. No new Rust dependency, no npm package, no `package.json`. Leave the diff uncommitted.

## 1. Why

`extension/omp.ts` is 581 lines of stateful logic (receipt FIFO, late-receipt grace, backoff, keepalive, tool bridging) on the delivery path of every OMP session. It has no direct tests: `tests/omp_wiring.rs` checks how it is installed, the `#[ignore]`d `tests/omp_smoke.rs` needs a real `omp` binary and a fake model server, and `tests/bridge.rs` covers the daemon side of the same protocol. Backlog I8 (a stuck session for hours) lived in exactly this gap: a regression in the extension's pending/receipt logic is only caught by a person watching a session.

**Runner decision (owner agreed in principle, 2026-10-15).** OMP runs extensions on Bun, but Bun is not installed here or on CI. Node is. The extension uses only `node:child_process`, timers, `Buffer`, `process.env` and `Promise.withResolvers` (Node 22+), so Node runs it unchanged; this was probed before drafting: `node` v24.21.0 imported a copy of `extension/omp.ts` directly (Node strips the type annotations, of which the file has none) and the extension registered all seven event handlers. Tests use the built-in `node:test`, `node:assert` and `mock.timers`, with no npm dependency. The Rust wrapper keeps `cargo test` the single gate.

**Follow-ups owned by the architect, not this spec:** a Node setup step in `.github/workflows/ci.yml`, and a JS lcov source for the Sonar coverage report (`sonar-project.properties` and the coverage step). The harness therefore reads the extension path from an environment variable (section 5) so that the coverage run can later point it at the raw file.

**Rejected:** a separate `package.json`/npm test script (a second gate beside `cargo test`); running the real daemon behind the extension (that is `tests/bridge.rs` plus the ignored smoke tests, and it would make timer-driven cases slow and flaky); testing against Bun (not installable here).

## 2. Existing behavior under test

Every test describes behavior the extension has today, so every test passes on the unchanged `extension/omp.ts`; there is no red phase. Section 8 replaces it with a mutation check.

## 3. `tests/omp_extension.rs`

One `#[test]` (plain, not tokio) `omp_extension_behavior_tests_pass`:

1. Find `node` on `PATH` (`Command::new("node").arg("--version")`). Parse the major version. When `node` is missing or older than 22: if the environment variable `A2AMX_REQUIRE_NODE` is set to `1`, fail with a message naming what is missing; otherwise `eprintln!` one line saying the test is skipped and return. (CI will set the variable; a developer machine without Node is not blocked.)
2. Create a temp dir (reuse the temp-dir approach of `tests/common/mod.rs`; do not add a crate). Write `a2amx::omp::render_extension()?` into it as `extension.mjs` (the `.mjs` name makes Node load it as ESM without type stripping). Use `unwrap` freely, this is a test.
3. Run `node --test --test-reporter=tap tests/omp_extension/*.test.mjs` (the glob is one argv element that Node expands itself; a bare directory argument runs nothing, and a broader glob would run the helper files as tests) with the working directory `env!("CARGO_MANIFEST_DIR")`, env `A2AMX_TEST_EXTENSION=<path to extension.mjs>`, stdin null. Capture stdout and stderr.
4. Assert exit status success; on failure the assertion message is the full captured stdout and stderr.
5. Also assert that the output reports at least `MIN_TESTS` passed tests (parse the `# pass N` line of the TAP summary). `MIN_TESTS` is the number of `test(...)` cases the developer wrote, as a literal. This catches a test directory that silently stopped being discovered.

The wrapper runs `node` with an argument vector, never through a shell.

## 4. `tests/omp_extension/fake_bridge.mjs`

An executable (`#!/usr/bin/env node`; the harness `chmod 0755`s a temp copy, or the file is committed with the executable bit, the developer's choice, but the test must work from a fresh checkout) standing in for `a2amx omp-bridge`. The extension spawns `$A2AMX_BIN omp-bridge` with piped stdio. The fake: reads `FAKE_BRIDGE_PORT` from its environment, connects a TCP socket to `127.0.0.1:<port>`, copies its stdin to the socket and the socket to its stdout byte for byte, exits with code 0 when its stdin ends, and exits with code 1 when the socket closes or errors. It writes an empty file named after its own pid into the directory `FAKE_BRIDGE_PID_DIR` at start (the harness uses it to learn when a bridge process is gone, section 5). When `FAKE_BRIDGE_IGNORE_STDIN_END=1` is set it does not exit on stdin end (case 15 needs a bridge that must be killed). The test process is the TCP server, so each spawned bridge shows up as one accepted connection, and closing that connection from the test makes the extension see a bridge exit with code 1.

## 5. `tests/omp_extension/harness.mjs`

Exports helpers used by all test files; no assertions of its own beyond precondition checks.

- `loadExtension()`: dynamic `import()` of `process.env.A2AMX_TEST_EXTENSION` (fail with a clear message when unset), returns the default export. Each test needs a fresh extension instance: the default export is a factory, so call it again per test with a fresh fake `pi`.
- `makePi()`: a fake `pi` recording `on` registrations (callable by event name through `emit(name, event, ctx?)`), `sendUserMessage(text, options)` calls (with a switch to make it throw), `registerTool(tool)` calls, and `pi.pi.VERSION`.
- `makeCtx(overrides)`: a session context with mutable `idle`, `hasPending`, `draft` fields behind `isIdle`, `hasPendingMessages`, `ui.getEditorText`, `ui.notify` recording `(message, level)` calls, and `agent: { kind: "main" }`.
- `startServer()`: a `node:net` server on port 0 that exposes the accepted connections in order, a parsed-frame inbox per connection, `send(connection, frame)`, `close(connection)`, `waitForConnection(n)` and `waitForFrame(connection, predicate)` (both with a real-time 5 s failure timeout so a broken test fails instead of hanging).
- `startExtension({ env, tools })`: sets `process.env.A2AMX_BIN` to the fake bridge, `A2AMX_ADDR`/`A2AMX_TOKEN` to dummies, `FAKE_BRIDGE_PORT` to the server port, restores the environment after the test, runs the factory, emits `session_start`, and returns the handles. Tools registered in the rendered extension come from the real `tool_schemas()`; a tool-specific test picks one by name from `pi.tools`.

`waitForBridgeExit(n)`: resolves once the n-th fake bridge's pid (from `FAKE_BRIDGE_PID_DIR`) no longer exists (`process.kill(pid, 0)` throws `ESRCH`) and then waits a further 100 real ms, so the extension's asynchronous child `close` event has run `finishBridge` and armed the respawn timer. This settle step is needed because a mock-timer `tick` that lands before the timer is armed does nothing. Mark the 100 ms with `// shortcut: fixed settle delay, no signal for "close handled" in the extension; replace if the extension gains one`.

Teardown, required in every test (`afterEach` or `t.after`): emit `session_shutdown` with the main ctx, wait for its bridge to exit, `mock.timers.reset()`, close the server, then restore the environment. A leftover instance would otherwise reconnect to a later test's server (it reads `process.env` at spawn time) and inflate connection counts.

Timer subtleties (probed on Node 24.21.0): enabling `setTimeout` also mocks `node:timers` and `node:timers/promises`, so the harness's own real-time timeouts use references captured at module load (`const realSetTimeout = globalThis.setTimeout`, captured before any `mock.timers.enable`), which keep working. During one `tick(n)` every callback sees `Date.now()` equal to the final time, not its due time; write time-boundary cases as `tick(limit - 1)` expecting nothing, then `tick(1)` expecting the effect.

Timers: tests that depend on time (500 ms poll, 30 s keepalive, 120 s late-receipt grace and stale head, 300 ms to 5 s respawn backoff, 1 s shutdown kill, 60 s tool timeout) use `mock.timers.enable({ apis: ["setTimeout", "setInterval", "Date"] })` and `tick(...)`. Real socket I/O is not mocked, so after each `tick` the test awaits the frame or effect it expects through `waitForFrame` instead of sleeping. Cases that need no time use real timers.

## 6. Behaviors to cover

One `test(...)` per bullet at least; group with `describe` as the developer prefers. The worked values are literals from the current source, not recomputed.

**Startup**
1. `session_start` with `ctx.agent.kind !== "main"` spawns no bridge. With the main context and `A2AMX_BIN` unset: one `ctx.ui.notify(..., "error")` whose message contains `A2AMX_BIN is not set`, and no bridge; a second `session_start` does not notify again. Same for `A2AMX_ADDR` and `A2AMX_TOKEN`.
2. The first frame the bridge receives is `hello` with `protocol: 1`, `omp_version` equal to the fake `pi.pi.VERSION` (`"unknown"` when absent), and `missing` listing exactly the absent APIs (for example `["isIdle"]` when `ctx.isIdle` is not a function). That test must not send `ready`: `reportState` would call the missing `ctx.isIdle()`.
3. `registerTool` is called once per tool with `name`, `description` and `parameters` taken from the rendered tool list; assert literals, not the rendered text: the tool names `send_message`, `list_agents`, `message_status`, `reset_session` (from `tool_schemas()` in `src/mcp.rs`; the developer confirms the exact list there and writes it as a literal) and one schema field of one tool; a `registerTool` that throws produces one notify per tool name.

**State reporting**
4. After the bridge sends `{"type":"ready"}` the extension sends `state` with `idle`, `pending`, `draft`, `queue`, `host_pending`. Worked instances: idle ctx, no pending, empty draft gives `{idle:true,pending:false,draft:false,queue:0,host_pending:false}`; `hasPending=true` gives `pending:true,host_pending:true,queue:0` (the I8 case: the extension still reports the raw host flag, the daemon decides what it means); a whitespace-only draft gives `draft:false`; draft `"hi"` gives `draft:true`; a throwing `getEditorText` gives `draft:false`.
5. An unchanged state is not re-sent on `turn_start`/`turn_end`/`agent_start`/`agent_end`; a changed one is. 
6. The 500 ms poll sends a `state` frame when the draft changes without any event; with mock timers, after the draft changes and 500 ms tick, exactly one new state frame arrives. A keepalive repeat of an unchanged state arrives after `tick(30000)` measured from the last send and not after `tick(29999)`.

**Delivery and receipts**
7. A `deliver` frame while idle calls `sendUserMessage(envelope, { attribution: "agent" })` (exactly that options object); while not idle, `{ deliverAs: "aside", attribution: "agent" }`. An `ack` frame with the same `id` is sent, followed by a forced `state` with `queue: 1`.
8. A `message_end` event whose `message` is `{ role: "user", attribution: "agent", content: <envelope> }` (string content, and again as an array of `{type:"text"}` parts, whose texts are concatenated) sends `receipt` with that `id` and a `state` with `queue: 0`. A `message_end` with `attribution` not `"agent"`, role not `"user"`, or different text sends no receipt.
9. Two delivered messages are receipted in FIFO order: `message_end` for the first envelope sends the first receipt, then `message_end` for the second sends the second. A `message_end` for the second envelope while the first is still the head sends no receipt, and nothing is remembered for it (the extension does not buffer out-of-order receipts outside a delivery). With an empty FIFO before the delivery, a `message_end` that `sendUserMessage` itself emits synchronously for the envelope being delivered is deferred: exactly one `receipt` results, sent after the `ack`.
10. When `sendUserMessage` throws `new Error("x".repeat(300))`, a `nack` frame is sent with `reason` of exactly 200 characters and no `ack`; the FIFO stays empty: no state frame is sent by the nack path, so observe it by delivering a second message successfully and expecting `ack` and `queue: 1` (not 2).
11. Late receipt: deliver, then report `hasPending=true` (event), then `hasPending=false` (event): the head is dropped from the FIFO (state `queue: 0`) but a `message_end` for its envelope within 120 000 ms still sends the `receipt`; after `tick(119999)` it still sends the receipt and after `tick(120000)` it sends none (measured from the end of the tick that dropped the head). Stale head: with `hasPending` never true, a head older than 120 000 ms is dropped at the next state report (`tick(120001)`), and no receipt follows a `message_end` after a further `tick(120000)`.

**Bridge lifecycle**
12. Refusals: `refused` with reason `protocol_mismatch` gives exactly one notify containing `disagree on the protocol version`, no respawn after the connection closes, and stops state polling; `missing_apis: isIdle` gives one notify containing `isIdle`, no respawn; `session is not an omp session` gives one notify, no respawn; any other reason is passed through as the notify message once per distinct reason (send both distinct reasons as `refused` frames on the same connection, because any unknown reason is final and no new bridge follows); `a bridge is already attached to this session` gives no notify and a respawn after the connection closes.
13. Respawn backoff: closing the bridge connection (fake bridge exits 1) makes the extension spawn a new bridge after 300 ms, then 600 ms, 1200, 2400, then 4800 and 5000 (capped) while no `ready` arrives; a `ready` resets the delay to 300 ms. Use mock timers; after each close call `waitForBridgeExit(n)` before the `tick`, then assert on the number of accepted connections.
14. After 10 consecutive bridge starts that never reach `ready`, exactly one notify containing `cannot reach its daemon` is raised and respawn continues; an eleventh failure raises no second notify.
15. Shutdown (`session_shutdown` with the main context): the bridge's stdin is closed (the fake exits 0), no respawn follows, and a bridge that has not exited within 1 000 ms is killed (assert through the connection closing after the tick; start the bridge with `FAKE_BRIDGE_IGNORE_STDIN_END=1` for the kill part). Handlers ignore the context passed with an event and use the one stored at the main `session_start`, so there is no non-main case to test for events or shutdown.

**Tool calls**
16. A registered tool's `execute(id, params)` sends one `mcp` frame whose request is `{jsonrpc:"2.0",id:<n>,method:"tools/call",params:{name,arguments}}` with the `i` property removed from `arguments` and all other properties kept; ids increase by one per call. A matching `mcp` response frame with `result.content` resolves with `{content}`. A response with `result.isError: true` rejects with an `Error` whose message is the first content text (string or `{text}`). A response with `error: {message}` rejects with that message.
17. `execute` before `ready` throws `A2AMX is not connected` synchronously (use `assert.throws`, not `assert.rejects`). A pending call rejects with `A2AMX connection lost` when the bridge connection closes, and with `A2AMX did not answer in time` after 60 000 ms with no response.

## 7. Conventions

Follow `AGENTS.md`: tests go through the seams of the Status section, expected values are literals, no mocking of the extension's own internals. Comments explain why. Keep helpers in `harness.mjs`; do not duplicate frame-building code across test files. No `eval`, no shell strings, no reading of real secrets or the real `~/.a2amx` (the harness sets dummy `A2AMX_ADDR`/`A2AMX_TOKEN` and never launches a real `a2amx`).

## 8. Out of scope

Changing `extension/omp.ts` (a bug found while writing a test is reported to the architect, not fixed); CI, Sonar, coverage and `AGENTS.md`/`CHANGELOG.md` changes (architect follow-ups); the daemon half of the protocol (`tests/bridge.rs`); real-OMP tests (`tests/omp_smoke.rs`); TypeScript typing of the extension; testing under Bun.

## 9. Acceptance

Run after implementation, in this order:

1. Baseline before the first edit, recorded in the report: `cargo test 2>&1 | tail -n 40` result counts (the developer records the full-suite counts; a known flake is rerun once and reported as pre-existing).
2. `A2AMX_REQUIRE_NODE=1 cargo test --test omp_extension` passes, `node --version` is 22 or newer, and the output includes the pass count from step 3 of section 3.
3. `A2AMX_TEST_EXTENSION=<rendered path> node --test tests/omp_extension/*.test.mjs` run by hand passes and every case in section 6 has at least one `test(...)` (the developer lists the mapping from section 6 bullet number to test name in the report).
4. **Mutation check** (the tests cannot be red-first, so show they can fail). Apply each of these one at a time to a temporary copy or in the working tree and revert it, record the failing test names, and show `git diff --stat extension/omp.ts` empty at handoff:
   - change the nack reason cap `.slice(0, 200)` to `.slice(0, 20)`;
   - delete `delete argumentsObject.i;`;
   - change all three `120000` literals (lines 169, 206, 217) to `1`;
   - change the `deliverAs: "aside"` option to `"followUp"`;
   - change `Math.min(backoff * 2, 5000)` to `Math.min(backoff * 2, 50000)`;
   - delete the `if (hasPending && fifo.length > 0) fifo[0].sawPending = true;` line.
   Each mutation must make at least one test fail. A mutation no test catches means a missing test: add it.
5. Full gate: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all pass, with the same counts as the baseline plus the one new test. `git status --short` shows only the files of section 0 plus the untracked `.cargo/`; `git diff --stat` is empty (the new files are untracked).
6. Report the run time of `cargo test --test omp_extension`. It should be under 30 s; if it is not, say which tests dominate and why.

Known flakes: none known for this suite. The tests use real sockets: a failing `waitForFrame` is a real failure, rerun once and report if it passes.

## Amendment 1 (2026-10-15)

Section 9 step 4, third mutation: `extension/omp.ts` has three `120000` literals (lines 169, 206, 217), not two. The mutation replaces all three with `1`. Found by the developer (m_1822); no other rule changes.
