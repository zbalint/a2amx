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

- Inside an a2amx-hosted session the environment carries `A2AMX_BIN`, which breaks
  `tests/omp_wiring.rs::generic_sessions_get_no_a2amx_bin`. Run the suite as
  `env -u A2AMX_BIN -u NO_COLOR cargo test`; a spec's baseline and acceptance commands use that form.
- The OMP extension's behavior tests (`tests/omp_extension/`) run on Node 22 or newer through
  `cargo test --test omp_extension`. Without Node the test skips; `A2AMX_REQUIRE_NODE=1` (set in CI)
  makes a missing Node a failure.
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

## Versioning

The project follows [Semantic Versioning 2.0.0](https://semver.org/). `version` in
`Cargo.toml` is the single source of truth, and `CHANGELOG.md` records user-visible changes.

- The public interface is what an operator or a harness depends on: CLI commands and flags,
  the config and team file formats, the control and wire protocol between client and daemon,
  the message database schema, and the MCP tools with their arguments. Internal modules are
  not public.
- While the major version is 0, a breaking change raises the minor version and a compatible
  change or fix raises the patch version. From 1.0.0, a breaking change raises the major
  version.
- Only the architect changes `version`, when cutting a release, in the commit that moves
  the changelog's Unreleased entries under the new heading. A feature or fix does not bump it.
  After a release, `develop` carries the next version.
- A change that breaks the public interface says so in its spec and in its changelog entry.
  The architect adds a line under Unreleased for each user-visible change when committing
  it; a developer's spec scope does not include the changelog.
- Release tags are `vMAJOR.MINOR.PATCH`, created on the default branch.

## Tests

- Test through public interfaces at agreed seams; do not test private functions or
  mock internal collaborators.
- Write the failing test first, then the minimum code to pass it, one behavior at
  a time.
- Expected values come from literals or worked examples, never recomputed with the
  logic under test.
- Integration tests start a real daemon in a temp state dir on port 0
  (`tests/common`); no shared ports, no fixed paths.

## Review record

The project measures how well its review process works, so every accepted spec carries a
record of what the review found. The architect appends a `## Review record` section to the
spec in the commit that accepts the implementation:

- A line naming the models used: architect, consultant, and for the developer and tester the
  planning, working and advisor models. The developer and tester state theirs in their
  report; the architect states its own and the consultant's.
- The number of BLOCKED/amend cycles.
- One table row per problem found after the first draft: where it was found (draft review,
  implementation, tester pass, architect review, or after the push), by which role and model,
  the problem in one line, and how it was resolved. A problem is anything that changed the
  spec or the diff, or a claim in the spec that turned out false.
- The text "no problems found" when there were none. An empty record is not the same as a
  missing one.

A problem found after the push is added to its spec's record in a later commit, with the
commit that fixed it. The record describes the process, not individuals: do not rank agents
by it, and do not use it to decide that a check can be dropped without a measured run of
specs showing it found nothing.

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
| `picker` | The session picker's key parser, view model (team sections, folds, filter) and scroll window (pure) |
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
