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
| G1 | Agent-initiated launch (an agent starting another session). Out of scope for now (owner, 2026-10-04); revisit later | docs |
| G3 | Cross-host support: central broker, remote launch, cross-host delivery. Deferred until the owner says it is needed | docs |

## Untested or unverified

| ID | Item | Evidence |
| --- | --- | --- |
| U4 | Real-Codex smoke test as an ignored test, like the OMP ones | docs |
| U6 | A human typing exactly inside the paste-to-Enter window to force a real corrupted submission | docs |
| U7 | Block behavior on Codex in the validation plan's recorded results | docs |

## Known issues

| ID | Item | Evidence |
| --- | --- | --- |
| I3 | A split escape sequence can hold a session; the paste-then-`CR` gap is one fixed constant; an unreadable PTY can hold the writer gate; message bodies are stored as plaintext | docs |
| I4 | The Codex "Update available" startup dialog leaves no thread loaded, so messages wait with `no_thread` until a person presses Esc. Known limitation: a2amx will not press keys for a person | manual |

## Ideas and possible features

| ID | Item | Evidence |
| --- | --- | --- |
| F2 | Named launch templates in an operator config file, launch ownership (a launcher may kill only what it launched), trusted working directory so the Claude trust dialog cannot block unattended delivery. Out of scope for now with G1 (owner, 2026-10-04) | docs |
| F3 | Scoping of `list_agents` (exchanges or workspaces) instead of every session seeing every session. Accepted as is for now; revisit if it causes trouble (owner, 2026-10-04) | docs |

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
| U5 | Limits at the edges: 50 open messages per recipient, 20 sends per minute per sender, bodies near 32 KiB, recipient exiting with an open message, daemon restart with open messages | spec 3b |
| C7 | Attach rendering no longer erases a glyph written in the terminal's last column | spec 3c |
| C8 | ACTIVITY column in the Ctrl-b w session picker, including running/exited cells and CWD clipping coverage | spec 3d |
| G2 | Prompt-submit hook safety net for Codex and OMP | dropped by the owner, 2026-10-04: receipts already come from the native channels |
| G4 | Codex: restart a dead app-server | by design, 2026-10-04: a session whose app-server dies is crashed, and probing showed the Codex screen exits and the session ends with `exited(1)` |
| G5 | Codex: resume a conversation by re-attaching to an existing thread | not needed, 2026-10-04: sessions start fresh and SALTMDB carries the memory across them |
| G6 | Codex: delivery through the PTY channel | won't do while native delivery works well (owner, 2026-10-04); reopen if a problem shows up |
| I5 | Claude readiness during the Press Ctrl-D again footer | not needed, 2026-10-04: spec 2u sends both keys without a gate and graceful exit works; reopen only if a gate is added |
| F5 | Delete `scripts/screen-probe.py` | `a2amx screen` confirmed working on a live OMP session, 2026-10-04; script removed |
| C9 | Richer attach status bar | spec 3e |
| C10 | Separator row above the attach status bar | spec 3f |
| C11 | Team-file name prefix and a guard for a one-string command | spec 3g |
| U1 | `developer_instructions` taking effect in Codex | verified by the tester on an isolated daemon, 2026-10-04: the default session's model states the peer-authorization instruction and a `--no-authorize-peers` session reports none (Codex 0.160.0, one model; not a version matrix) |
| U2 | Two simultaneous Codex sessions; `/new` thread switching | verified by the tester, 2026-10-04: a message reached only the receiver's own thread with a native receipt, and after `/new` the next message landed in the new thread only |
