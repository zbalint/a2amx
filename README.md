# A2AMX

**Agent-to-Agent Message eXchange**

A terminal multiplexer and message exchange for communication between AI agent sessions.

## Status

The terminal core is implemented: a local daemon that owns PTY sessions, keeps
their screen state while detached, and lets one human attach, detach, switch, and
scroll from a client that redraws from the terminal model. The single-machine
messaging core is implemented too: durable SQLite messages, per-session tokens, a
stdio MCP server, and delivery into a session's composer that protects an unfinished
human draft. For Claude Code, a prompt-submit hook adds submission receipts and a
safety net that blocks a prompt mixing a peer message with a human draft.
Agent-initiated launch, hooks for Codex and OMP, their delivery profiles, and
cross-host support are not built yet. Everything in the design documents beyond
these slices is still a proposal unless identified as a requirement.

The implementation direction is **Rust on Linux**, with communication and session
management across multiple hosts from the outset.

## The idea

Interactive agent harnesses expose a terminal input path. A2AMX will launch them
under pseudoterminals (PTYs), let people switch between their sessions, and use
that same input path to deliver messages from other agents.

Each agent has an MCP server for sending messages and a hook for observing prompt
submission. A central daemon routes and persists messages across hosts. Each host
runs a launcher; the proposed architecture adds a persistent host supervisor to
own local PTYs and keep sessions running when a client disconnects.

For example, a planning agent on `host-a` could message a reviewing agent on
`host-b`. The recipient's host would queue the message until delivery is permitted,
inject a clearly attributed message, and report a matching submission-hook receipt.

Claude Code and Codex can already message parallel sessions of their own kind, but
not sessions of a different harness. A2AMX targets that gap, and also sessions on
other hosts and in containers.

Writing bytes to a PTY does not prove that the harness submitted a prompt. A hook
receipt does not prove that a model processed it. Those distinctions, and protecting
unfinished human input, are central design constraints.

## Scope

A2AMX is intended to provide:

- PTY ownership, terminal state, and human session switching across Linux hosts.
- Durable messages, routing, session identity, and submission receipts.
- Serialized input with explicit protection for human composition.
- A small per-agent MCP interface: `list_agents`, `send_message`, and
  `message_status`.
- Hosting of arbitrary interactive commands, with tested delivery profiles for
  supported harnesses. The target harnesses are Claude Code, Codex, and OMP,
  including harnesses running inside containers.
- Remote launch: a central daemon can direct the daemon on any connected host to
  start a session there.
- Mid-turn delivery: an agent in the middle of a long turn can receive a message
  without waiting for the turn to end.
- A human experience in an attached session equivalent to starting the harness
  directly, without loss of function.

It is not an agent planner, an LLM orchestration framework, a memory system, or a
container management platform. It has no planned LLM of its own. Arbitrary command
hosting does not imply safe automatic message delivery into every terminal program.

## Design documents

| Document | Read it when working on |
| --- | --- |
| [Architecture](docs/architecture.md) | Scope, topology, ownership, trust, terminal behavior, or unresolved architectural choices |
| [Delivery](docs/delivery.md) | Input arbitration, message evidence, hooks, retry, ordering, or failure recovery |
| [Validation plan](docs/validation-plan.md) | The checks each implementation slice must pass |

The terminal-core, messaging-core, and Claude Code hook slices are done; their locked
specs are [spec 1](docs/specs/spec-1-terminal-core.md),
[spec 2](docs/specs/spec-2-messaging.md), and
[spec 2b](docs/specs/spec-2b-hooks-receipts.md). The next milestones are the Codex and
OMP profiles, each with its own hook, then cross-host support. The checks each slice
must pass, and the results recorded so far, are in the validation plan.

## Building and trying it

```sh
cargo build
cargo test
a2amx daemon                        # prints "listening on <addr>"
a2amx new -- sh                     # start and attach; Ctrl-B d detaches
a2amx list
a2amx attach <id> [--force]
a2amx kill <id>
a2amx daemon --host-name host-a     # names this host in addresses (name@host-a)
a2amx new --name agent-plan --harness claude -- claude
a2amx messages [--session <id>] [--state pending]
a2amx cancel <message-id>
```

`--harness claude` wires the MCP server and the prompt-submit hook into Claude Code
and delivers messages automatically; `--harness generic` (the default) holds them, and
`--deliver auto` opts a generic session in to blind delivery. `a2amx mcp` is the stdio
MCP server that harnesses start; it reads `A2AMX_ADDR` and `A2AMX_TOKEN` from its
environment. `a2amx hook` is the Claude Code `UserPromptSubmit` hook, installed through
an inline `--settings` argument; it reads the harness payload on stdin, always exits 0,
and prints output only when it blocks a prompt.

The state directory is `--home`, then `A2AMX_HOME`, then `$XDG_STATE_HOME/a2amx`,
then `$HOME/.local/state/a2amx`. The prefix key is Ctrl-B (`--prefix` or
`A2AMX_PREFIX` to change it). After the prefix: `d` detach, `w` session picker, `[`
scroll mode, `r` releases the session's message hold, and the prefix twice sends a
literal prefix.

## Contributing

Design feedback and reproducible, sanitized compatibility findings are welcome.
Identify whether a suggestion changes a requirement, a proposal, or an open
question. Before sending a change, `cargo test`, `cargo clippy --all-targets -- -D
warnings`, and `cargo fmt --check` must pass; see [AGENTS.md](AGENTS.md).

This is a public repository. Use fictional identities and example hosts in docs
and fixtures. Keep credentials, personal infrastructure details, private agent
conversations, and local memory exports out of files, issues, commits, and pull
requests. Keep runtime databases and logs outside the checkout; review any proposed
diagnostic attachment before publishing it. Ignore rules are not a substitute for
reviewing staged content.

## License

See [LICENSE](LICENSE) for the repository's license terms.
