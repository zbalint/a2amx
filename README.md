# A2AMX

[![CI (master)](https://img.shields.io/github/actions/workflow/status/zbalint/a2amx/ci.yml?branch=master&label=CI%20(master))](https://github.com/zbalint/a2amx/actions/workflows/ci.yml)
[![CI (develop)](https://img.shields.io/github/actions/workflow/status/zbalint/a2amx/ci.yml?branch=develop&label=CI%20(develop))](https://github.com/zbalint/a2amx/actions/workflows/ci.yml)
[![Quality gate (develop)](https://sonarcloud.io/api/project_badges/measure?project=zbalint_a2amx&branch=develop&metric=alert_status)](https://sonarcloud.io/summary/new_code?id=zbalint_a2amx&branch=develop)
[![Coverage (develop)](https://sonarcloud.io/api/project_badges/measure?project=zbalint_a2amx&branch=develop&metric=coverage)](https://sonarcloud.io/summary/new_code?id=zbalint_a2amx&branch=develop)
[![Security rating (develop)](https://sonarcloud.io/api/project_badges/measure?project=zbalint_a2amx&branch=develop&metric=security_rating)](https://sonarcloud.io/summary/new_code?id=zbalint_a2amx&branch=develop)
[![Rust 1.88+](https://img.shields.io/badge/Rust-1.88%2B-orange?logo=rust)](https://github.com/zbalint/a2amx/blob/develop/Cargo.toml)
[![Platform Linux](https://img.shields.io/badge/platform-Linux-blue?logo=linux)](https://github.com/zbalint/a2amx)
[![License AGPL-3.0-or-later](https://img.shields.io/badge/license-AGPL--3.0--or--later-blue)](https://spdx.org/licenses/AGPL-3.0-or-later.html)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/zbalint/a2amx/badge)](https://securityscorecards.dev/viewer/?uri=github.com/zbalint/a2amx)

A2AMX hosts Claude Code, OMP, and Codex sessions under PTYs and lets the agents message one another through native harness channels.

**Status:** alpha. The Linux single-machine daemon, terminal sessions, durable messaging, team management, and three native delivery profiles work today; cross-host support and agent-initiated launch are not built.

> **Not the A2A protocol.** A2AMX is independent of the [Agent2Agent (A2A) protocol](https://github.com/a2aproject/A2A). It is a local terminal multiplexer and message exchange for agent sessions.

## The problem

Agents running in different harnesses cannot use each harness's built-in peer messaging. Typing a message into a terminal can destroy a human's half-written draft, and a session dies when its terminal closes.

A2AMX owns the sessions under PTYs, delivers through each supported harness's native channel, and keeps the human's draft protected. The daemon keeps detached sessions running and persists accepted messages on the local machine.

## Example

This run used a temporary state directory and a freshly installed build.

Start the daemon in one terminal:

```sh
$ a2amx daemon start
listening on 127.0.0.1:34613
```

Start each session in its own terminal; `a2amx new` attaches the terminal, so use two more terminals:

```sh
$ a2amx new --name planner -- claude
```

```sh
$ a2amx new --name reviewer -- omp
```

Detach each with Ctrl-B d, then inspect the sessions from any terminal:

```text
$ a2amx list
ID  NAME      HARNESS  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA          SIZE
s1  planner   claude   running  3s      working   3s        no        0        -     -              80x24
s2  reviewer  omp      running  3s      working   3s        no        0        -     5h 81% wk 54%  80x24
```

The planner agent calls `send_message` to `reviewer`; the reviewer receives it as an attributed peer message.

## What you get

- **Persistent sessions:** detach and reattach to PTY-backed Claude Code, OMP, Codex, or generic commands.
- **Cross-harness messaging:** agents send durable messages through an authenticated MCP interface.
- **Native delivery:** Claude Code uses a channel, OMP an extension, and Codex a private app-server.
- **Draft protection:** delivery holds when automatic input could mix with a human's unfinished composition.
- **Operator controls:** attach, switch, scroll, inspect, reset, and stop sessions from the CLI.
- **Teams and status:** group sessions with visibility rules, roles, watches, heartbeats, and quota status.

## Quick start

Install the release copy, put its `bin` directory on `PATH`, and start the daemon:

```sh
scripts/install.sh
export PATH="$HOME/.a2amx/bin:$PATH"
a2amx daemon start
```

Then start the two sessions from [the example above](#example). See [Installation](docs/install.md) for install details.

## Compatibility and limits

- Linux only; build requires Rust 1.88 or newer.
- Claude Code, OMP, and Codex have native delivery profiles. Other commands are held unless explicitly opted into blind delivery.
- The daemon currently owns one machine; cross-host routing and remote launch are not built.
- Agents cannot launch other sessions. Teams provide visibility and coordination, not OS-user or process isolation.
- PTY bytes do not prove prompt submission. Receipts report harness evidence, not model processing.
- A2AMX is not an agent planner, an LLM orchestration framework, a memory system, or a container management platform, and has no LLM of its own.

## How this project is built

All code, tests and documentation are written by AI agents; no human writes or hand-edits them (apart from a GitHub-generated initial commit and the OpenSSF Scorecard workflow template). Specifications, implementation, testing and review are done by separate agent roles on different models. Every push is checked in CI by `cargo test`, `clippy` and `rustfmt`, and CI fails when the SonarQube quality gate fails; the architect also runs the same gate locally before pushing, because CI runs after the push and does not stop it. A human sets the project's direction and decides what is released, but does not review every change line by line, and there has been no independent security audit. See [How A2AMX is built](docs/how-this-is-built.md).

## Documentation
- [CLI reference](docs/cli.md) — commands, attachment controls, list columns, quota, and reset.
- [Teams](docs/teams.md) — configure team files, visibility, watches, heartbeats, and team commands.
- [Harnesses and delivery](docs/harnesses.md) — choose profiles and interpret channels, hooks, and receipts.
- [Installation](docs/install.md) — separate development builds from the live installed binary.
- [Releasing](docs/releasing.md) — how releases are cut, signed and verified.
- [Using with agents](docs/using-with-agents.md) — peer-message instructions and credential hygiene.
- [Roadmap](docs/roadmap.md) — what is implemented, the planned milestones, and what is not planned.
- [How A2AMX is built](docs/how-this-is-built.md) — who writes and checks the code, and what that does and does not show.
- [Architecture](docs/architecture.md) — topology, scope, authority, and terminal behavior.
- [Delivery](docs/delivery.md) — input arbitration, evidence, retries, ordering, and recovery.
- [Validation plan](docs/validation-plan.md) — implementation-slice checks and recorded results.
- [Backlog](docs/backlog.md) — open gaps, issues, untested behavior, and ideas.
- [Contributing](CONTRIBUTING.md) — setup, checks, and public-repository rules.
- [Security](SECURITY.md) — private vulnerability reporting and security scope.
- [Changelog](CHANGELOG.md) — user-visible changes.
- [License](LICENSE) — AGPL-3.0-or-later terms.
