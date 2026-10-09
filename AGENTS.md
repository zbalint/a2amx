# AGENTS.md

Conventions for anyone (human or agent) changing this repository. Design lives in
`docs/`; read `docs/architecture.md` before touching a module.

## Commands

Run all three before calling a change done; they must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

- Run a working session's daemon from the installed copy (`~/.a2amx/bin/a2amx`), not
  from `target/`; development builds must not replace its executable.

## Rust conventions

- No `unwrap()` or `expect()` outside tests (enforced by clippy lints). Return
  `anyhow::Result` at binary and daemon edges; use typed errors where a caller
  branches on the failure.
- Never block the async runtime. PTY reads and writes, PTY drop (it waits on the
  child), and file syscalls that can stall run on a blocking thread or
  `spawn_blocking`.
- `unsafe` only with a `// SAFETY:` comment stating the invariant. Prefer `rustix`
  over raw `libc`.
- Launch commands as argument vectors, never shell-interpolated strings.
- Terminal-emulation types (`alacritty_terminal::term`, `vte`, `grid`, `event`) are
  named only in `src/emulator.rs`; its `tty` module (PTY spawning) only in
  `src/session.rs`.
- When cutting a corner on purpose, leave a `// shortcut:` comment naming the
  ceiling and the upgrade trigger.
- Comments explain why, not what. Match the density of the surrounding code.

## Security defaults

- Bind loopback unless the operator configures otherwise. Do not add warnings or
  refusals for operator-chosen addresses.
- State files (tokens, address files, databases) are created owner-only (0600 or
  0700 for directories).
- Session tokens are random, never in argv, logs, envelopes, or terminal output.
- Derive a sender from its authenticated token, never from a caller-supplied field.
- Never send input that destroys human work. In particular, never send Ctrl-C to
  clear a composer draft.

## Tests

- Test through public interfaces at agreed seams; do not test private functions or
  mock internal collaborators.
- Write the failing test first, then the minimum code to pass it, one behavior at
  a time.
- Expected values come from literals or worked examples, never recomputed with the
  logic under test.
- Integration tests start a real daemon in a temp state dir on port 0
  (`tests/common`); no shared ports, no fixed paths.

## Modules

| Module | Owns |
| --- | --- |
| `emulator` | Screen model, query replies, snapshot serializer (the only alacritty user) |
| `prefix` | Prefix-key state machine for the attach client |
| `wire` | Framing, control JSON, stream frames |
| `session` | One PTY, its child, its emulator |
| `daemon` / `client` | TCP server and the client API |
| `bridge` | The `omp-bridge` relay between an OMP extension and the daemon |
| `channel` | The `a2amx mcp --channel` process that relays daemon deliveries to Claude Code as channel events |
| `delivery` | Ordered delivery attempts and the PTY, native, and Codex delivery channels |
| `harness` | Harness profiles, command wiring, readiness, and peer authorization |
| `hook` | The Claude Code `UserPromptSubmit` hook adapter |
| `messaging` | Message envelopes, validation, limits, states, and rendering |
| `names` | Generated session names (pure, ADJECTIVE-NOUN with a seeded picker) |
| `mcp` | The stdio MCP server and its authenticated agent tools |
| `omp` | The embedded OMP extension and its launch files |
| `picker` | The session picker's key parser and scroll window (pure) |
| `quota` | Harness quota extraction from screen, conversation state, and `omp usage` command output |
| `status` | Attach status-line formatting and status frames |
| `store` | SQLite message and delivery-attempt persistence |
| `codex` | A session's private Codex app-server, its JSON-RPC client, and the poller behind the Codex channel |
| `cli` | Command-line shape |
| `team` | The team file and flag formats and the up-plan (pure; the commands live in `main`) |
| `main` | CLI dispatch and daemon/session/team command execution |

## Public repository rules

Examples and fixtures use fictional names and hosts (`host-a`, `agent-plan`,
`example.invalid`). No personal data, no real addresses, no credentials.
