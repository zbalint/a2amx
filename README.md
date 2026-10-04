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
Claude Code sessions receive messages through a channel, OMP through an
extension, and Codex through a private app-server. Sessions have names, a status
line shows quota, and `a2amx team` starts and stops a team of sessions. Agent-initiated
launch, a prompt-submit hook safety net for Codex and OMP (their native channels already record receipts), and
cross-host support are not built yet. Everything in the design documents beyond
these slices is still a proposal unless identified as a requirement.

The implementation direction is **Rust on Linux**, with communication and session
management across multiple hosts from the outset.

## The idea

Interactive agent harnesses expose a terminal input path. A2AMX launches them
under pseudoterminals (PTYs) and lets people switch between their sessions. It
delivers messages from other agents through each harness's native channel where one
exists (Claude Code, OMP, Codex), and by typing into the terminal otherwise.

Each agent has an MCP server for sending messages. Claude Code also has a hook for
observing prompt submission; OMP and Codex report receipts through their channels. The local daemon routes and persists messages on one host and
owns local PTYs, keeping sessions running when a client disconnects. Routing across
hosts is proposed, not implemented.

For example, a planning agent on `host-a` could message a reviewing agent on
`host-b`. The recipient's host would queue the message until delivery is permitted,
deliver a clearly attributed message, and report a receipt when the harness provides
one.

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
- A small per-agent MCP interface: `list_agents` (address, state, activity, attached,
  quota, harness, and spawn `cwd`), `send_message`, `message_status`, and `reset_session`.
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
| [Backlog](docs/backlog.md) | Open known items, issues, untested behavior, and ideas |

The terminal-core, messaging-core, and Claude Code hook slices are done; their locked
specs are [spec 1](docs/specs/spec-1-terminal-core.md),
[spec 2](docs/specs/spec-2-messaging.md),
[spec 2b](docs/specs/spec-2b-hooks-receipts.md), and
[spec 2c](docs/specs/spec-2c-delivery-robustness.md). The Claude channel, OMP extension, and Codex app-server delivery profiles
followed as specs 2e to 2g and later. The next milestones are prompt-submit hooks for
Codex and OMP, then cross-host support. The checks each slice
must pass, and the results recorded so far, are in the validation plan.

## Building and trying it

```sh
cargo build
cargo test
a2amx daemon start [--foreground]  # start the daemon; detached by default
a2amx daemon status               # show daemon state and session counts
a2amx daemon stop [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx new --watch developer --heartbeat 30m -- sh # digest watched peers after idle
a2amx list [--details]
a2amx attach <id|name> [--force]
a2amx kill <id|name> [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx kill --exited              # remove every exited session without prompting
a2amx reset <id|name>               # type the configured reset sequence
a2amx screen <id|name> [--rows N]   # read visible screen text without attaching
a2amx team up [--file F] [--detach] [NAME=EXE ...]   # a2amx.toml or bare executables
a2amx team down [--file F] [--now] [NAME ...]
a2amx daemon start --host-name host-a  # names this host in addresses (name@host-a)
# --harness is inferred from the command name when omitted (claude, codex, omp)
a2amx new --name agent-plan --harness claude -- claude
a2amx new --name agent-review --harness omp -- omp
a2amx messages [--session <id>] [--state pending]
a2amx cancel <message-id>
```

`kill`, `team down`, and `daemon stop` use graceful Ctrl-D by default; pass `--now`
to skip it and use the immediate HUP/KILL path.

`a2amx kill --exited` removes every exited session and never prompts; running sessions are
untouched. It prints `removed NAME-or-ID` for each removed session, or `no exited sessions`
when there is nothing to remove. The `--exited` form cannot be combined with a session
reference, `--yes`, or `--now`.

Commands that take a session accept its name or its id.

`--harness claude` wires the MCP server and the prompt-submit hook into Claude Code
and delivers messages automatically; `--harness generic` (inferred for unrecognised commands) holds them, and
`--deliver auto` opts a generic session in to blind delivery. `a2amx mcp` is the stdio
MCP server that harnesses start; it reads `A2AMX_ADDR` and `A2AMX_TOKEN` from its
environment. `a2amx hook` is the Claude Code `UserPromptSubmit` hook, installed through
an inline `--settings` argument; it reads the harness payload on stdin, always exits 0,
and prints output only when it blocks a prompt.

Claude delivery uses a Claude Code channel by default, preserving any composer draft.
`--harness claude --no-channel` keeps terminal delivery. Channel startup auto-accepts
Claude's development-channel dialog; this research preview needs a claude.ai or Console
login. A successful write has `write_complete` evidence until the prompt hook confirms
`native_receipt`; a missing receipt never triggers a resend.

`--harness omp` writes the A2AMX extension into the state directory, loads it into OMP
with `-e` and a per-session `--config` overlay, and delivers messages natively without
typing into the terminal.

The state directory is `--home`, then `A2AMX_HOME`, then `$XDG_STATE_HOME/a2amx`,
then `$HOME/.local/state/a2amx`. The prefix key is Ctrl-B (`--prefix` or
`A2AMX_PREFIX` to change it). After the prefix: `d` detach, `w` session picker, `[`
scroll mode, `s` toggles the status line, `r` releases the session's message hold, and the prefix twice sends a
literal prefix. While a session has not turned on mouse reporting, the mouse wheel scrolls it (three lines a step) and
enters scroll mode; hold Shift to select text with the mouse. Scrollback in the real terminal after you detach is not
kept. The status line takes the terminal's last row and shows the session address,
pending-message count, and a HELD alert with the hold reason. In `a2amx list`, ATTACHED means a human client is attached, not that
the session is reachable: a detached session still receives messages. ACTIVITY is `idle`, `working`, or `busy` for running sessions, and `-` for exited sessions. The default list omits each session's working directory and command; `a2amx list --details` adds CWD and COMMAND. HELD shows the
reason a hold is stopping delivery (`-` when clear); a session that looks idle with
an empty composer may hold after you typed in it, and `r` clears it. QUOTA shows what a Claude or
Codex status line last said about remaining quota (5h and weekly percent left); for OMP it shows
`limit` when the session's last conversation rows contain a `usage_limit_reached` error. It can be stale
and is empty when no quota signal is recognised.

`a2amx reset <id|name>` types `/clear` by default, or the ordered `--reset` sequence
configured for the session. Each step is bracketed-pasted, submitted with one Enter,
and waits for the harness to become ready before the next step. A reset fails fast on
an active draft, a held or non-ready session, a write error, or a 30-second per-step
timeout; one reset runs at a time. Session-token callers need the target's
`control_from` consent.

### Teams

`a2amx team up` reads `a2amx.toml` in the current directory by default:

```toml
[[session]]
name = "architect"
command = ["claude"]
attach = true
reset = ["/clear", "/prewalk restart"]
control_from = ["developer"]
watch = ["developer"]
heartbeat = "30m" # digest watched peers after 30 minutes of idle time

[[session]]
name = "developer"
command = ["omp"]
cwd = "."
```

`reset` is optional; omitting it uses `/clear`. If present it must contain one to
eight slash commands, each at most 200 bytes and without control characters.
`control_from` lists named sessions allowed to invoke this session's reset through
the MCP tool; the CLI/admin token is always allowed. `watch` lists named sessions
whose natural exit sends this session one daemon message. A `heartbeat` such as
`"30m"` sends an additional digest after that session is ready, unheld, not resetting,
and unchanged for the interval; it requires a non-empty `watch` list and accepts
positive `s`, `m`, or `h` values from 1 second through 24 hours. The digest is skipped
when every live watched peer has `idle` activity and an empty queue. A daemon-ended
session (for example through `kill`, `team down`, or `daemon stop`) sends no exit event.

A copy to start from is in `a2amx.toml.example`; your own `a2amx.toml` is gitignored.

Use `--file` for another file; relative `cwd` values are relative to that file's
directory, while an omitted `cwd` uses the invoking directory. `up` skips and reports
running sessions. An exited session holding a requested name stops the whole team
before any spawn; remove it with `a2amx kill NAME` first. If a later spawn fails,
already-started sessions remain and no further session starts.

Without a file, `a2amx team up architect=claude developer=omp` accepts bare
executables only; commands with arguments need the file's argv arrays. The file
may select one session to attach; the flag form attaches the first. `--detach`
suppresses attachment, and nonterminal invocation never attaches.
`a2amx team down` kills the file's named sessions, running or exited; explicit
names select sessions directly. It uses the same graceful Ctrl-D default as `kill`;
pass `--now` to use the immediate HUP/KILL path. Missing names are reported without
failing.
`--file` cannot be combined with inline session items or names.

## Using it with agents

- Tell your agents how to treat peer messages. The envelope says only where a message
  came from. A line in your agent instructions saying that a peer message is a request
  from an authorized peer, not from you; that scoped, reversible work is fine; and that
  pushing, production access, deleting data, and secrets still need you, removes the
  guesswork. `--harness claude` and `--harness omp` add an operator line that tells the
  agent to act on peer messages; `--no-authorize-peers` leaves it out.
- Keep credentials out of message bodies. A prompt hook, transcript, or memory tool
  attached to the harness sees the full message, and some keep it with no way to
  delete it.
- Claude Code escapes its own paste-wrapper tag when it appears in pasted text, and
  A2AMX models that behavior on Claude Code 2.1.286, so a body containing the tag
  text remains matchable there. A sender can still avoid writing the wrapper tag in
  a message body when practical.
- A sender's `accepted` means the daemon took the message, not that it was delivered.
  Check `message_status` when the reply matters.

## Dev build versus live install

Build and test from the repository with `cargo build` and `cargo test`. Install a release
copy with `scripts/install.sh`, then run the daemon from `~/.a2amx/bin/a2amx` and point
your `amx` alias or launcher at that installed binary. `A2AMX_INSTALL_DIR` overrides the
install directory; the state directory remains separate.

Never run the daemon from `target/`: development builds replace that executable and can
break the MCP command of a later Codex session. Install freely; the atomic install leaves
a running daemon using its old image. Restart deliberately to pick up the new version,
because restarting the daemon ends all sessions. The script leaves PATH, aliases, services,
and daemon start or stop to the operator.

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
