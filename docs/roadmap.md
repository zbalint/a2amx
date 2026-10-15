# Roadmap and implementation status

Explanation for readers distinguishing implemented behavior from proposed work.

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

## Deferred and proposed

[Backlog](backlog.md#known-gaps-documented-as-not-built) G1 tracks agent-initiated
launch, which is out of scope for now. G3 tracks cross-host support: a central broker,
remote launch, and cross-host delivery are deferred until the owner needs them.
There is no committed schedule for either.

For example, a planning agent on `host-a` could message a reviewing agent on
`host-b`. The recipient's host would queue the message until delivery is permitted,
deliver a clearly attributed message, and report a receipt when the harness provides
one. **This is proposed, not implemented.**

Backlog G2, the prompt-submit hook safety net for Codex and OMP, was dropped by the
owner: their native channels already record receipts. It is not a planned milestone.
Current follow-on candidates include exposing OMP native state in `list --details`
(G7), stale-human-draft clearing on OMP delivery (G8), and showing OMP's native draft
hold (G9). These are backlog items, not promises or an ordered release plan.

