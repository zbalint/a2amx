# Backlog

Status: the running list of open known items, issues, untested behavior, ideas, and
features. It is a tracker, not a design: a decision belongs in [architecture](architecture.md)
or [delivery](delivery.md), and an implementation slice gets a locked spec under
`docs/specs/`. Check an item against the code before acting on it, because entries can
lag the code.

## How to use this file

- One row per item, newest at the bottom of its section. Give each item a stable ID.
- Evidence column: say how the state was learned (`code`, `docs`, `manual`, `reported`).
  `reported` means the owner said so and it was not re-run.
- When an item is done, move it to [Closed](#closed) with the commit or spec that closed it.
- `// shortcut:` comments in the code are the ledger of deliberate corners; this file
  lists only the ones worth scheduling.

## Known gaps (documented as not built)

| ID | Item | Evidence |
| --- | --- | --- |
| G1 | Agent-initiated launch (an agent starting another session) | docs |
| G2 | Prompt-submit hook safety net for Codex and OMP. Receipts already come from the native channels, so first decide whether it is needed | docs |
| G3 | Cross-host support: central broker, remote launch, cross-host delivery | docs |
| G4 | Codex: restart a dead app-server (`app_server_down` is permanent for that session) | docs |
| G5 | Codex: resume a conversation by re-attaching to an existing thread | docs |
| G6 | Codex: delivery through the PTY channel (deliberately absent today) | docs |

## Untested or unverified

| ID | Item | Evidence |
| --- | --- | --- |
| U1 | `developer_instructions` taking effect in Codex, separate from the model acting on a peer request anyway | code |
| U2 | Two simultaneous Codex sessions; `/new` thread switching (the poller takes the most recently updated loaded thread) | code |
| U4 | Real-Codex smoke test as an ignored test, like the OMP ones | docs |
| U5 | Limits at the edges: 50 open messages per recipient, 20 sends per minute per sender, bodies near 32 KiB, recipient exiting with an open message, daemon restart with open messages | docs |
| U6 | A human typing exactly inside the paste-to-Enter window to force a real corrupted submission | docs |
| U7 | Block behavior on Codex in the validation plan's recorded results | docs |

## Known issues

| ID | Item | Evidence |
| --- | --- | --- |
| I3 | A split escape sequence can hold a session; the paste-then-`CR` gap is one fixed constant; an unreadable PTY can hold the writer gate; message bodies are stored as plaintext | docs |
| I4 | The Codex "Update available" startup dialog leaves no thread loaded, so messages wait with `no_thread` until a person presses Esc | manual |
| I5 | Claude readiness during the Press Ctrl-D again footer is unprobed (spec 2u sends both keys without a gate between them) | spec 2u |

## Ideas and possible features

| ID | Item | Evidence |
| --- | --- | --- |
| F2 | Named launch templates in an operator config file, launch ownership (a launcher may kill only what it launched), trusted working directory so the Claude trust dialog cannot block unattended delivery | docs |
| F3 | Scoping of `list_agents` (exchanges or workspaces) instead of every session seeing every session | docs |
| F5 | Delete `scripts/screen-probe.py` after the owner confirms `a2amx screen` works in real use | spec 2r |

## Closed

| ID | Item | Closed by |
| --- | --- | --- |
| C1 | Claude Code, OMP and live multi-agent spec delivery across harnesses | reported: used live to deliver several specs through agents on different harnesses |
| C2 | Scroll in an attached OMP session | reported: wheel scrolling works after spec 2q (`0a4a9d9`) |
| C3 | Status text claiming Codex and OMP receipts or delivery profiles were not built | `94c0672` |
| C4 | Mid-turn message receiving on OMP (`aside`) and on Codex, with real harnesses | reported: tested by the owner |
| C5 | Configured session reset with consent, guarded PTY submission, CLI and MCP surfaces; completion markers remain per-harness work and Claude SessionStart after `/clear` is unverified | spec 2s |
| C6 | Agent activity (`idle`, `working`, `busy`) in `list` and `list_agents`, and activity-aware heartbeat busy detection | spec 2y |
| I1 | Exited sessions keep their name until `a2amx kill`, so a new session cannot reuse it | spec 2z |
| I2 | OMP install and extension cleanup/error handling nits | spec 3a |
| F4 | Bulk cleanup of exited sessions, for example `a2amx kill --exited` | spec 2z |
| C7 | Attach rendering no longer erases a glyph written in the terminal's last column | spec 3c |
