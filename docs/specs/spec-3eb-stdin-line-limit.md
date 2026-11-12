# Spec 3eb: bound the MCP and bridge stdin reader (I21)

## 0. Status

LOCKED.

Scope: `src/mcp.rs` (`spawn_stdin_reader`, `StdioEvent`, the `run` loop), `src/bridge.rs` (the `relay`
loop's `StdioEvent` match), `tests/mcp.rs`, `tests/bridge.rs`. Does not touch `src/main.rs` (its own
private `spawn_stdin_reader` at line 3392 is the attach client's and is a different function),
`src/wire.rs`, `src/channel.rs`, `CHANGELOG.md`, `docs/backlog.md`, or any other file.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id: `a2amx-0-3-1-stdin-line-limit`.
Public test seams: the `a2amx mcp` and `a2amx omp-bridge` processes through their stdin and stdout, as
`tests/mcp.rs` (`McpProcess`) and `tests/bridge.rs` already drive them.

## 1. Why

`mcp::spawn_stdin_reader` (`src/mcp.rs:105-128`) reads with `read_until(b'\n')` into a fresh `Vec` with no
size limit and hands each line to the async side through an unbounded channel. A producer that sends bytes
without a newline grows the process without bound; a producer faster than the handler grows the channel
without bound. Both the `a2amx mcp` server and the `a2amx omp-bridge` relay use it (`src/mcp.rs:33`,
`src/bridge.rs:67`).

The input comes from the agent's own harness, so this is not a trust-boundary problem; it is a robustness one
(AGENTS.md and the project rules ask every untrusted-size input to be bounded, and a runaway harness is
the case that does not need an attacker). No attack path is created or closed: the harness already holds the
session token.

Rejected while designing:

- A separate, smaller MCP line limit. The bridge already refuses any frame above `MAX_FRAME_LEN` (1 MiB,
  `src/wire.rs:17`, `src/bridge.rs:154`) and the daemon's wire frames use it too, so one existing constant
  is the limit for both users; a `send_message` body is at most `MAX_MESSAGE_BYTES` (32 KiB, even when every
  byte is JSON-escaped to six bytes it is about 192 KiB), far below it.
- Truncating an oversized line and parsing the prefix. A cut JSON document is garbage; the line is discarded.

## 2. `src/mcp.rs`

1. `StdioEvent` gains a variant `Oversized`: a line whose content (the bytes before the newline) is longer
   than `MAX_FRAME_LEN` bytes. Import `MAX_FRAME_LEN` from `crate::wire`. A line of exactly `MAX_FRAME_LEN`
   bytes is accepted.
2. `spawn_stdin_reader` returns a bounded `mpsc::Receiver<StdioEvent>` with capacity 16, and the reader thread
   sends with `blocking_send`, so a handler that stalls stops the thread from reading stdin (the pipe's own
   buffer then pushes back on the producer). A send that fails because the receiver is gone ends the thread
   as it does today.
3. The reader never holds more than `MAX_FRAME_LEN + 1` bytes of a line: each `read_until(b'\n')` runs on
   `stdin.by_ref().take(MAX_FRAME_LEN as u64 + 1)`. Three outcomes, decided by that read alone:
   - the read ends in `\n`: a `Line` (its content is at most `MAX_FRAME_LEN` bytes; the newline is the
     last of at most `MAX_FRAME_LEN + 1` bytes);
   - the read returned exactly `MAX_FRAME_LEN + 1` bytes and none is `\n`: the line is too long. Send one
     `StdioEvent::Oversized`, then discard input up to and including the next `\n` (or EOF) in chunks
     without keeping it (`fill_buf` / `consume`), then resume normal reading. An oversized line that ends
     at EOF gives `Oversized`, then `Eof`. An I/O error in the discard loop sends `StdioEvent::Error` and
     ends the thread, as the main loop does;
   - any other result (fewer bytes, no `\n`, so EOF): the final unterminated line within the limit is
     delivered as a `Line`, as today; a read of 0 bytes is EOF.
4. `run` (the `select!` at `src/mcp.rs:42`): on `Some(StdioEvent::Oversized)` write the JSON-RPC error
   `error_response(Value::Null, -32600, "request too large")` through `write_response` and continue the loop;
   a write failure breaks with the error exactly like a normal response.

## 3. `src/bridge.rs`

The `relay` loop's match on `StdioEvent` (`src/bridge.rs:72-95`) gains
`Some(StdioEvent::Oversized) => break Err(anyhow!("bridge input line exceeds {MAX_FRAME_LEN} bytes"))`.
This matches how the loop already ends the relay on a line it cannot parse (`parse_extension_line` error at
line 76). `MAX_FRAME_LEN` is already imported there (line 14). Nothing else in `bridge.rs` changes.

## 4. Tests

Written first, each failing on the old code except the two named contract tests. Write them against process
behaviour; a compile failure from the missing variant is not the red phase. Payload byte counts below are
content bytes before the newline. The old bridge already rejects a complete oversized line after reading it
(`parse_extension_line`, `src/bridge.rs:170-176` and `validate_payload_len`, `:195-199`, "frame exceeds
maximum length"), so the bridge test below sends no newline: the old code then waits for ever and the new
code exits.

- `tests/mcp.rs`, test 1 (`McpProcess` and the existing daemon fixture): send `MAX_FRAME_LEN + 1` bytes of
  `x` and a `\n`; the next response is `-32600`, `id` null, message `request too large`; then send a valid
  `ping` and read its normal response (the process survived and resumed).
- `tests/mcp.rs`, test 2 (the discard path): send `2 * MAX_FRAME_LEN` bytes of `x` in several writes, then `\n`,
  then a valid `ping`; the next two responses are exactly `-32600` and then the `ping` result. The input is
  well over one read, so the discard loop runs; without it the leftover would parse as a second line and
  answer `-32700` first.
- `tests/mcp.rs`, test 3 (contract test, passes on the old code; the limit is inclusive): `MAX_FRAME_LEN` bytes
  of `x` and a `\n` answer `-32700` (parse error, not `-32600`).
- `tests/mcp.rs`, test 4: `MAX_FRAME_LEN + 1` bytes of `x`, no newline, then close stdin: the first response is
  `-32600`, and the process then exits with status 0 (oversized line ended by EOF). Read stdout until EOF.
- `tests/bridge.rs`, attached through the existing fixture (`Case::start`, `BridgeProcess::spawn`, `ready()`,
  `tests/bridge.rs:186-245`): write exactly `MAX_FRAME_LEN + 1` bytes of `x` with no newline and keep stdin
  open; the bridge exits within a bounded wait with a non-zero status and stderr containing
  `bridge input line exceeds 1048576 bytes`. Write exactly that many bytes: more risks the bridge exiting
  while the test is still writing (`BridgeProcess::send` unwraps the write, `tests/bridge.rs:220-224`).
- Contract tests (pass on the old code and stay green unchanged): `mcp_protocol_errors_notifications_and_argument_validation`
  in `tests/mcp.rs`, the other tests in `tests/bridge.rs`, and `tests/claude_channel.rs` (it runs
  `a2amx mcp --channel` through the changed loop).

The bounded channel and the `blocking_send` backpressure are not observable through the processes without
a memory measurement and are not given a black-box test; the tester reads them in the diff and the mutation
pass checks that the other tests still pass with them changed (a known limit, named here so it is not read as
a gap).

## 5. Out of scope

- A limit on what the daemon, the channel (`src/channel.rs`) or the attach client read.
- Changing `MAX_FRAME_LEN`, `MAX_MESSAGE_BYTES` or any message limit.
- Making the output side bounded (`write_response`, `Output` in `bridge.rs`).
- The `omp-bridge` unbounded `requests` channel in `src/bridge.rs:65` (it carries parsed MCP requests from the
  already bounded reader, one per line; revisit if the bridge handler can stall).
- Docs: `docs/delivery.md` does not state a line limit today (checked: lines 531-572); the architect adds the
  changelog line.

## 6. Acceptance

Run in the main checkout, as an ordinary user (AGENTS.md "Commands"):

```sh
cargo test --test mcp --test bridge
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

- The new `tests/mcp.rs` and `tests/bridge.rs` tests fail on the old code and pass on the new (the developer
  shows the red run before implementing).
- `git status --short` lists only the four files in section 0.
- Known failures in a confined session: `tests/gateway_up.rs::sandbox_hides_state_host_and_limits_writes_to_gateway`
  and `tests/human_inbound.rs::queued_human_message_rechecks_recipient_eligibility_before_delivery` fail with
  `Read-only file system` (backlog I24); CI runs unconfined and passes them. Record them as pre-existing.
- Mutation check (tester, on the assigned checkout, restored by checksum afterwards). Each mutant and the test
  that must kill it: the `take` size raised by one (`MAX_FRAME_LEN + 2`), test 1; the limit made exclusive
  (`take(MAX_FRAME_LEN)`), test 3; the discard loop dropped, test 2; `Oversized` not sent on EOF, test 4;
  the bridge arm ignoring `Oversized` (continue), the bridge test. Two more mutants, the channel capacity
  and `blocking_send` replaced by an unbounded send, are judged by reading (section 4).

## 7. Claims

| Claim | Where | Verified |
| --- | --- | --- |
| `spawn_stdin_reader` has no limit and an unbounded channel | `src/mcp.rs:105-128` | read |
| Two callers only, `mcp::run` and `bridge::relay` | `src/mcp.rs:33`, `src/bridge.rs:67`; `rg spawn_stdin_reader src tests` also hits an unrelated private fn in `src/main.rs:3392` | grepped |
| The bridge refuses frames above `MAX_FRAME_LEN`, a line it cannot parse ends the relay | `src/bridge.rs:154`, `:74-77` | read |
| A parse error answers `-32700` with null id and the process continues | `tests/mcp.rs:553-571` | read |
| No test covers an oversized line | `rg -n "MAX_FRAME_LEN\|oversize\|too large" tests/mcp.rs tests/bridge.rs` matches nothing in `tests/mcp.rs`; `tests/bridge.rs` matches nothing | grepped |
| `StdioEvent` is matched exhaustively only in `mcp.rs` and `bridge.rs` | `rg StdioEvent src tests` | grepped |
| The old bridge already rejects a complete oversized line after reading it, so only a newline-free flood is new | `src/bridge.rs:170-176`, `:195-199` | read (consultant and architect) |
| `tests/claude_channel.rs` runs `a2amx mcp --channel` through the changed loop | `tests/claude_channel.rs:174-240` | read (consultant) |
| Baseline: `cargo test --test mcp --test bridge` passes before the change | run at base 777fa96, 2026-11-12 | ran |
