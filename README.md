# A2AMX

**Agent-to-Agent Message eXchange**

A terminal multiplexer and message exchange for communication between AI agent sessions.

## Status

A2AMX is in the design stage. This repository contains design documents, not a
working application. Commands, interfaces, and dependencies described here are
proposals unless identified as requirements. No harness compatibility is certified.

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

Writing bytes to a PTY does not prove that the harness submitted a prompt. A hook
receipt does not prove that a model processed it. Those distinctions, and protecting
unfinished human input, are central design constraints.

## Scope

A2AMX is intended to provide:

- PTY ownership, terminal state, and human session switching across Linux hosts.
- Durable messages, routing, session identity, and submission receipts.
- Serialized input with explicit protection for human composition.
- A small per-agent MCP interface, initially `list_agents` and `send_message`.
- Hosting of arbitrary interactive commands, with tested delivery profiles for
  supported harnesses.

It is not an agent planner, an LLM orchestration framework, a memory system, or a
container management platform. It has no planned LLM of its own. Arbitrary command
hosting does not imply safe automatic message delivery into every terminal program.

## Design documents

| Document | Read it when working on |
| --- | --- |
| [Architecture](docs/architecture.md) | Scope, topology, ownership, trust, terminal behavior, or unresolved architectural choices |
| [Delivery](docs/delivery.md) | Input arbitration, message evidence, hooks, retry, ordering, or failure recovery |
| [Validation plan](docs/validation-plan.md) | Feasibility experiments, compatibility evidence, or implementation go/no-go decisions |

The next milestone is a feasibility prototype exercising two harnesses across two
Linux hosts. Its acceptance criteria are in the validation plan. The prototype
must resolve delivery risks before the design is treated as an implementation contract.

## Contributing during the design stage

Design feedback and reproducible, sanitized compatibility findings are welcome.
Identify whether a suggestion changes a requirement, a proposal, or an open
question. There is no build or test command yet because no implementation exists.

This is a public repository. Use fictional identities and example hosts in docs
and fixtures. Keep credentials, personal infrastructure details, private agent
conversations, and local memory exports out of files, issues, commits, and pull
requests. Keep runtime databases and logs outside the checkout; review any proposed
diagnostic attachment before publishing it. Ignore rules are not a substitute for
reviewing staged content.

## License

See [LICENSE](LICENSE) for the repository's license terms.
