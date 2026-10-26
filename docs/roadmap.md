# Roadmap and implementation status

Explanation for readers distinguishing implemented behavior from planned work.

A2AMX aims to be reliable at connecting, observing, controlling, and recovering agent
sessions. It does not decide how agents do their work: roles, review rules, and
workflows belong to the operator's own instructions.

## Built today

The local terminal core owns PTY sessions and maintains their screen state while detached.
A human can attach, detach, switch, and scroll. Sessions last as long as the daemon;
they do not survive its restart.

The single-machine messaging core persists messages in SQLite, authenticates sessions
with per-session tokens, and exposes a stdio MCP server. Claude Code receives messages
through a channel, OMP through an extension, and Codex through a private app-server.
The Claude prompt-submit hook records receipts and blocks terminal submissions that
mix a peer envelope with a human draft. Native channels provide their own receipts.
Names, quota reporting, the attach status line, and team management are implemented.
See [Architecture](architecture.md#implemented-messaging-core) and
[Delivery](delivery.md) for the implementation boundaries.

## Milestones

Milestones are versions, in the order they are planned. They carry no dates. Each item
gets its own spec before it is built, and an item can still move between milestones or
be dropped. The IDs refer to rows in the [backlog](backlog.md). While the major version
is 0, a milestone may change the public interface; the [changelog](../CHANGELOG.md) says
when one does.

### 0.2.0: Reliability and observability

Make what exists on one machine dependable and explainable before adding more.

- A command that explains a session's state: where the state came from (the harness's
  native channel, the prompt hook, or the screen), why a message is held, and why a
  reset would be refused (F18, absorbing G7 and G9).
- `team status` distinguishes a session that is running from one that is ready to work,
  for example one stopped at a startup dialog (F19, with I16).
- A stable test suite: the flaky PTY tests fixed (I7, I9, I14) and tests isolated from
  the environment of the session that runs them (I13).
- A decision on whether a message body may contain an opening envelope tag (I11).
- The heartbeat shows what a busy peer is doing, not only how long it has been busy (F12).

### 0.3.0: Worktrees

- An authorized agent or operator can move a session into a git worktree. A2AMX stops
  the session's process and starts it again in the worktree, with the same name and
  settings and a fresh conversation, as after a reset. Several sessions can share one
  worktree, so an implementer and a reviewer can work in the same tree while another
  pair, or a separate team, works in parallel on the same repository (F20).
- A2AMX creates the worktrees it needs and never removes one; cleaning up stays with
  the operator.

### 0.4.0: Multiple hosts, messaging and management

- A daemon on each host connects to one central daemon, which keeps the directory and
  accepts messages. Hosts join by pairing, and every connection between hosts is
  authenticated and encrypted with the keys exchanged at pairing, so A2AMX does not
  depend on the operator providing a private network (G3).
- Agents list, message, and check messages for agents on other hosts. A team can span
  hosts, with the same visibility rules as on one host.
- The operator lists, kills, and resets sessions and reads team status across hosts.
- Sessions keep running while the central daemon restarts, and open messages survive a
  restart instead of becoming undeliverable (F22).
- The design keeps a later move to federated daemons possible: identifiers are unique
  across hosts, every record names its host, and the protocol is versioned from its
  first frame.

For example, a planning agent on `host-a` could message a reviewing agent on `host-b`.
The recipient's host would queue the message until delivery is permitted, deliver a
clearly attributed message, and report a receipt when the harness provides one.

### 0.5.0: Multiple hosts, attach and launch

- Attach to a session on another host, with terminal traffic relayed through the
  central daemon and scheduled separately from messages and control traffic (G3).
- Start a session on another host, and team files that place sessions on hosts (G3).
- A trusted working directory, so a startup dialog cannot block an unattended launch
  on a remote host (part of F2).

## Design work ahead of the milestones

- **Permissions.** Today several separate rules decide who may do what, among them
  admin and session tokens, private-team visibility, per-session `control_from` for
  resets, `watch` lists, and the peer-authorization line in the session prompt. One
  design will cover who may see, message, reset, move, manage, and launch which
  sessions, on one host and across hosts. It is written as an architecture decision before 0.3.0 starts, and each
  milestone builds the part it needs (F21).

## Engineering track

Work on quality and security that changes no user-visible behavior. It has no version
and is picked up between milestone items, except where a condition is given.

- A weekly non-blocking broad Semgrep scan, before 0.2.0 starts (F16). The blocking gate with the
  repository's own rules is done (F15, spec 3aq).
- Fuzzing of wire framing and message parsing with `cargo-fuzz`, before 0.2.0 starts;
  the property tests for the pure modules already exist (F10).
- A check of what a malicious dependency's build script could reach in CI, such as the
  release signing permission, before 0.2.0 starts (F17).
- A mutation-testing baseline on the pure modules (F11).
- The OpenSSF Best Practices criteria and Scorecard findings (F14).
- Coverage of subprocess tests that end their child with a signal (I15).
- A real-Codex smoke test and the unprobed harness flag behaviors (U4, U7, U8).

## Not planned now

These wait for the trigger named with them.

- A general event-stream API, when an external consumer such as a monitoring tool or a
  web interface needs more than watches and heartbeats delivered as messages.
- Structured task handoffs (task identifiers, handoff metadata), if assignments get
  lost or duplicated in real use.
- A session activity feed for watchers (F6), when an operator wants an advisor-style
  watcher that heartbeats cannot serve.
- Agents starting other sessions (G1 and the rest of F2): it makes A2AMX a process
  authority and needs its own threat model and a concrete use.
- A recorded two-agent demo (F13), once I16 is understood.
- Clearing a stale human draft on OMP message delivery (G8), if it causes trouble.
- Sessions surviving a restart of their own host's daemon, if such restarts lose work
  in real use.

Backlog G2, the prompt-submit hook safety net for Codex and OMP, was dropped by the
owner: their native channels already record receipts.
