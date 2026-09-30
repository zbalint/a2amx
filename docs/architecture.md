# Architecture

Status: design draft, with one implemented slice: the terminal core (see
[Implemented: terminal core](#implemented-terminal-core)). Everything else here is
a proposal, and no compatibility results are implied.

This document owns topology and scope. [Delivery](delivery.md) owns input and
receipt semantics; [validation](validation-plan.md) defines how to test them.

## Decision status

- **Requirement:** established product intent; changing it requires an explicit
  product decision.
- **Proposal:** a recommended design to review and validate.
- **Open:** a choice or feasibility question that is not resolved.

## Requirements

- Target Linux and implement in Rust.
- Support multiple hosts through one central daemon/server and a daemon on each
  host. The central daemon can direct any connected host daemon to launch a session.
- Give each agent session an MCP server and a prompt-submission hook (or the
  harness's native equivalent, such as an extension).
- Provide a client/launcher on each host and human management across hosts.
- Launch interactive commands under PTYs; support attachment, switching,
  detachment, and continued execution of inactive sessions.
- Target Claude Code, Codex, and OMP as hosted harnesses, including harnesses
  running inside containers.
- Deliver messages to a recipient during its active turn. Long turns are expected,
  so waiting for the turn to end is not an acceptable default.
- Give an attached human the experience of starting the harness directly, without
  loss of function.
- Persist messages and delivery evidence in SQLite.
- Derive sender identity from the authenticated session, not a `from` tool argument.
- Preserve ordinary terminal input except a small configurable mux prefix.
- Serialize PTY input and protect unfinished human composition.
- Keep the MCP interface small and independent of PTY implementation details.
- Keep the core independent of container engines and model providers.

## Proposed topology

```text
                       Human CLI / TUI
                              |
                       Central daemon
                     directory + broker
                         central SQLite
                         /           \
               Host supervisor    Host supervisor
                    host-a             host-b
                   /      \               |
              Session    Session       Session
              runtime    runtime       runtime
                 |          |             |
              PTY +      PTY +         PTY +
              harness    harness       harness
              MCP/hook   MCP/hook      MCP/hook
```

The central daemon and a supervisor may run on the same machine. “Central” is a
logical role, not a requirement for a dedicated physical host.

| Module/process | Owns | Does not own |
| --- | --- | --- |
| Central daemon | Enrollment, directory, authorization, durable message acceptance, routing, global status | Local PTY handles or interpretation of harness keystrokes |
| Host supervisor | Local processes, session runtimes, reconnect reconciliation, local delivery journal | Global acceptance while disconnected from the central daemon |
| Session runtime | PTY lifecycle, screen model, input arbiter, writer, resize, delivery execution | MCP tool parsing or global routing |
| Per-agent MCP server | Small tool interface and authenticated session binding | Direct PTY access or human management authority |
| Prompt-submit hook | Reporting submitted prompts and applying the supervisor's allow or block verdict | Deciding that the model processed a message |
| Human CLI/TUI | Session selection, attachment, management requests, pending-message display | Persistent ownership of a running PTY |

The host supervisor is the per-host daemon. It accepts launch and management
requests from the central daemon as well as from local clients. A host supervisor
must remain alive after its launching CLI exits. Hosting PTYs
centrally over remote shell connections is not the proposed ownership model.

Proposal: use one executable with role-specific modes and internal Rust modules.
Do not commit to a crate hierarchy or public plugin interface before validation.

## Connections and interfaces

Proposal: each host supervisor establishes an authenticated outbound connection
to the central daemon. Per-agent MCP servers use local IPC to their supervisor;
hooks report receipts through local IPC as well. This concentrates network
reconnection and host credentials in the supervisor.

Proposal: local IPC is TCP, not a Unix socket, so that a containerized harness can
reach it without a mounted socket. The host daemon listens on loopback by default;
the listen setting accepts a list of addresses, for example loopback plus a
Tailscale address, which lets containers on the host connect without host
networking. Every connection presents a per-session token that is generated at launch and
passed in the environment. The token identifies the session, which is how the sender
is derived. A2AMX binds wherever the operator configures and does not warn about or
refuse the choice: what the network exposes and who may use it is the operator's
responsibility. The protocol is
length-prefixed JSON for messages, with terminal streams on separate connections.

Proposal: the per-agent MCP server is a stdio subcommand of the same executable. The
harness starts it, it reads the session token from its environment, and it forwards
calls to the daemon over TCP. It runs wherever the harness runs, so a containerized
harness has its MCP server and hook inside the container, needing only a TCP route
to the daemon and the executable, either installed in the image or mounted. The
daemon stays outside the container. An MCP endpoint served by the daemon over HTTP
is a later option for harnesses where providing the executable is awkward.

Human clients connect to the central daemon for management and remote attachment.
Initially relay remote terminal traffic through the daemon, with independently
bounded scheduling for terminal streams, messages, and control traffic. A noisy
terminal must not starve receipt processing or another session's input.

The network transport, framing, protocol version negotiation, credential format,
and revocation mechanism are open decisions. Every host must be authenticated and
explicitly authorized. Confidentiality is the operator's responsibility: A2AMX does
not encrypt traffic in the MVP, and the documentation tells operators to run it over
a trusted network such as a Tailscale overlay. Authentication uses challenge-response,
so a credential is never sent over the wire.

Proposal: enroll a host by pairing. The host daemon prints a one-time code carrying
its public key, and the operator enters the code at the central daemon to authorize
that host. The central daemon then issues a long-lived credential.

Proposed agent-facing tools:

```text
list_agents()
send_message(to, subject, message)
```

The directory returns authorized recipients and their availability. A successful
send returns a durable message identifier and acceptance status, not a promise of
model processing. An optional acceptance-idempotency key remains under review.
Human management uses a separate interface and authority.

## Identity and authority

Distinguish a host ID, a session ID, a process incarnation, and a display alias.
An alias such as `agent-review` is for discovery; resolve it to an immutable target
before accepting a message. Restarting an MCP bridge does not itself create a new
PTY incarnation. Reusing an alias does not transfer an old incarnation's messages.

Proposal: session-scoped credentials bind the MCP bridge and hook to the session
runtime. The supervisor derives the sender and validates receipt ownership. The
central daemon binds session registrations to the enrolled host. Ordinary agents
cannot supply another identity or issue privileged terminal-control requests.

Proposal: the initial trust model is an explicitly enrolled set of hosts under one operator.
It is not hostile multi-tenant isolation. A compromised enrolled host can expose
its local sessions; a compromised central daemon can affect routing and relayed
terminal traffic. Restrict local sockets, credentials, and database permissions.
Keep credentials out of arguments, logs, message envelopes, and terminal output.

Security posture: A2AMX ships secure and safe defaults and leaves the rest to the
operator. The defaults are a loopback bind, owner-only permissions on the database,
configuration, and credential files, random per-session tokens, a sender always
derived from the token, escaped agent payloads, and no destructive input without a
human. A2AMX does not solve transport encryption, what a configured bind address
exposes, a hostile process running as the same OS user, or multi-tenant isolation.
The documentation names these limits so that operators can decide.

Open: exact exchange/workspace visibility rules and management permissions.
Agents should only discover and message recipients authorized for their scope.

## Terminal behavior

The supervisor maintains terminal state continuously, including while detached.
The supported virtual terminal contract must cover the capabilities used by the
selected harnesses: primary/alternate screens, cursor and attributes, scrolling
regions, wrap state, Unicode cell widths, and relevant input modes.

Terminal queries need responses while no human client is attached. The virtual
terminal and outer terminal must not both answer the same query. Advertised
capabilities must match the emulator rather than blindly inheriting the outer
terminal's identity.

Attachment uses a consistent snapshot followed by ordered updates. Switching
restores the selected session's screen and supported input modes. Historical
output replay must not repeat clipboard or other terminal side effects. Decide
explicit policies for clipboard, hyperlinks, and other non-screen sequences;
terminal graphics are outside the proposed MVP.

Preserve input bytes where possible. Prefix recognition must tolerate split
sequences and avoid consuming prefix bytes inside paste or protocol frames.
Provide a literal-prefix escape. The default prefix is Ctrl-Space, chosen because
it exists on every keyboard layout, and it is configurable. Extended keyboard protocols and nested muxes are
compatibility cases, not automatically supported features.

Proposal: one controlling human attachment per session. The controller determines
its dimensions; detached sessions retain the last size. Serialize resize with
screen updates, notify the PTY, and redraw. Restore the client terminal on normal
exit and recoverable errors. A crashed client must not leave persistent ownership
of the hosted PTY.

Forward terminal control characters through the input path; do not additionally
send an OS signal for the same key. Explicit terminate requests are separate and
must account for process groups and descendants.

Bound scrollback, terminal frames, and input queues. Drain PTY output independently
of database operations and slow clients. A lagging display may be resynchronized;
input and durable message evidence must never be silently discarded.

## Implemented: terminal core

Built from [spec 1](specs/spec-1-terminal-core.md); the spec is the contract, this
section is the summary.

- One `a2amx daemon` process owns sessions; clients are thin. Sessions live as long
  as the daemon (no persistence across restarts). Each session is one PTY, its
  child, and an `Emulator` (the only user of `alacritty_terminal`'s terminal types).
- The attach client redraws from the emulator, like tmux: one full render, then
  damage-based updates. The daemon never forwards raw PTY bytes. No graphics,
  hyperlinks, clipboard writes, application titles, or bell reach the client.
- Query replies come only from the emulator and are written to the PTY, so they are
  answered while detached. `TERM=xterm-256color`, `COLORTERM=truecolor`, kitty
  keyboard protocol off.
- PTY output is drained by a per-session thread that never waits on a client. A
  slow client only delays its own attachment task.
- Input is never dropped: a bounded queue (256 chunks) feeds one writer thread, and
  enqueueing applies backpressure to that client's connection.
- One controlling attachment per session. `--force` takeover is ordered: the old
  attachment receives `Detached` and closes before the new one registers. There is
  deliberately no deadline, so a stalled old client delays a takeover of that
  session (other sessions and control requests stay responsive).
- The child's environment is the daemon's own overlaid with the client's
  environment, with `TERM` and `COLORTERM` last. `DESKTOP_STARTUP_ID` and
  `XDG_ACTIVATION_TOKEN` are always removed by the pinned library.
- Local IPC is TCP (default `127.0.0.1:0`; the bound addresses are written to
  `addr`), authenticated by an `admin.token` file. The state directory is 0700 and
  its files 0600. Framing is a `u32` big-endian length plus payload, with JSON on the
  control channel and tagged binary frames on attach streams.
- The client has a prefix state machine (bracketed-paste aware), a scroll mode, and
  a session picker.

Not yet built: per-session tokens, the MCP server, hooks, envelopes, the dirty
flag, SQLite, and everything cross-host.

## Wrappers and containers

The launched command is an argument vector, not a shell-interpolated string.
Hosting a container CLI or an SSH client does not make that tool a core dependency.
No container-engine socket is required merely to host an interactive command.

Terminal connectivity and MCP/hook connectivity are separate concerns. A harness
inside a container needs a deliberate route to its session bridge and credentials.
Nested PTYs add mode, resize, escape, and lifetime behavior to validate. A lost
wrapper process does not prove that a remote process exited.

Requirement: harnesses running inside containers must be supported. The route
from a container to its session bridge and credentials is therefore a design item
for the first prototype, not a later certification.

Proposal: run a supervisor where the harness executes when possible. Certify
specific wrapper arrangements individually; arbitrary command launch is not a
blanket integration guarantee.

## Persistence and lifecycle

The central SQLite database owns durable message acceptance and global evidence.
Proposal: a small local SQLite journal records attempts and receipts before
reporting them, supporting reconnect reconciliation. This is not a replicated
database or an offline central broker.

Use short transactions and a single database owner per process. Keep databases on
local storage. Proposed durability baseline: WAL with `synchronous=FULL`, subject
to filesystem and hardware guarantees. Handle disk-full errors before promising
acceptance. Plan checkpoints and backups with the live WAL in mind.

| Failure | Intended behavior |
| --- | --- |
| Human client disconnect | Hosted processes continue; remote control authority expires; draft state remains protected |
| Network partition | Local sessions continue; directory presence becomes stale; accepted messages remain pending |
| Central daemon restart | Hosts reconnect and reconcile; local PTYs remain owned by supervisors |
| Host supervisor crash | Recover journal evidence; mark affected sessions lost; no promise of reclaiming live PTYs |
| Harness exit | End that incarnation; retain pending messages and require an explicit disposition |

Proposal: a new send is successful only after the central commit. If the commit
response is lost, acceptance is uncertain and must be reconciled; an unreachable
server must not produce a false success. Offline outgoing acceptance is deferred.
See [delivery recovery](delivery.md#recovery-and-retry) for side-effect ambiguity.

## Scope and candidate stack

| MVP proposal | Later | Out of scope |
| --- | --- | --- |
| One central server, multiple enrolled Linux hosts | High availability and federation | Native Windows support |
| Three tested harness profiles (Claude Code, Codex, OMP) with submission observation | More harnesses and wrapper certifications | Universal safe injection into arbitrary TUIs |
| One human controller per session | Multiple viewers and richer layouts | LLM planning and orchestration |
| PTY delivery channel only; submission receipts optional per harness profile | Native in-harness delivery channels, added as adapters | |
| Durable messages, receipts, and visible uncertainty | Offline outgoing queues | Exactly-once model processing |
| Remote launch, attach, switch, detach, resize | Session survival through supervisor crash | Container management and model inference |

Candidates, not pinned dependencies: rusqlite, rmcp, crossterm, and ratatui.
Already in use by the terminal core: Tokio, serde/serde_json, clap, tracing,
rustix, and `alacritty_terminal` (including its `tty` module for PTYs, so
portable-pty is not used). A UI framework is not a terminal emulator.

Decision: the terminal emulator is `alacritty_terminal`, kept behind a small
interface of A2AMX's own so it can be replaced. It is maintained, tracks the modes
the harnesses use (bracketed paste, focus, alternate screen, mouse, and the kitty
keyboard flags), and emits the replies to terminal queries that must be answered
while no client is attached. It has no reattach snapshot, so A2AMX writes its own
serializer from the screen state to escape sequences. Alternatives considered:
`vt100` (ready-made snapshots, but not updated since 2025-07 and it leaves query
replies to the host), `libghostty-vt` (API not yet stable), and `wezterm-term` (not
published on crates.io). Isolate blocking PTY/database work from asynchronous network handling.

## Open decisions

1. What observable state permits safe automatic submission for each harness?
2. What explicit action relinquishes human composition ownership?
3. Which harness versions (Claude Code, Codex, OMP) and terminal capabilities
   define initial support?
4. How are hosts revoked and assigned messaging/management scopes?
5. Which transport handles streams, reconnects, fencing, and flow control?
6. How are stale recipients, message expiry, and deliberate retries presented?
7. What retention, payload, queue, and receipt-journal limits are appropriate?

Resolve these through design review and the first implementation slices, checked
against the [validation plan](validation-plan.md).
Detailed schemas, wire formats, and command syntax follow those decisions.
