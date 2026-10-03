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
| I1 | Exited sessions keep their name until `a2amx kill`, so a new session cannot reuse it | docs |
| I2 | Spec 2f review nits: `child.kill` and `stdin.end` in the OMP extension shutdown may be unguarded; `unreachable!` in `src/omp.rs` `render_extension`; install cleanup can mask the original error | code |
| I3 | A split escape sequence can hold a session; the paste-then-`CR` gap is one fixed constant; an unreadable PTY can hold the writer gate; message bodies are stored as plaintext | docs |
| I4 | The Codex "Update available" startup dialog leaves no thread loaded, so messages wait with `no_thread` until a person presses Esc | manual |

## Ideas and possible features

| ID | Item | Evidence |
| --- | --- | --- |
| F1 | Graceful session stop when the daemon exits (today SIGHUP, then SIGKILL). Owner design in F7 | docs |
| F2 | Named launch templates in an operator config file, launch ownership (a launcher may kill only what it launched), trusted working directory so the Claude trust dialog cannot block unattended delivery | docs |
| F3 | Scoping of `list_agents` (exchanges or workspaces) instead of every session seeing every session | docs |
| F4 | Bulk cleanup of exited sessions, for example `a2amx kill --exited`. Today an exited session keeps its name until `a2amx kill NAME`, and `team down` clears only the team file's names. Needs its own spec | reported |
| F5 | Delete `scripts/screen-probe.py` after the owner confirms `a2amx screen` works in real use | spec 2r |
| F7 | Graceful exit for `kill`, `stop` and the daemon stopping: for an idle session type Ctrl-D (once for OMP and Codex, twice for Claude), wait for exit, otherwise fall back to the current HUP, wait, KILL sequence. Default, with `--now` to skip it; timeout at least 10 s; same gates as reset (running, no hold, no human draft, ready, OMP idle), checked before each key, never Ctrl-C; Generic sessions skip it. The once-versus-twice claim is unprobed, so a tester probe on an isolated daemon comes before the spec | reported |
| F8 | Exit event: when a session in a team exits or crashes, the daemon messages the sessions that watch it at once. No timer, so it covers crashes only, not a looping peer. Smaller than F9 and built first | reported |
| F9 | Heartbeat for chosen team sessions (team file, for example `heartbeat = "30m"`): an idle orchestrator or architect only wakes on messages, so it cannot notice a stuck peer. The daemon sends a message from a system sender when the session has been idle for the interval (any activity resets the timer), through the normal delivery gates, at most one undelivered at a time. The body is a digest of the watched peers: time in the current turn, time since the last message to or from the peer, time since the screen last changed, holds, and a hint to run `a2amx screen <peer>`. Skipped when every watched peer is idle with an empty queue; no quota handling. The daemon cannot tell long work from a loop, so the recipient judges. After F8 | reported |

## Closed

| ID | Item | Closed by |
| --- | --- | --- |
| C1 | Claude Code, OMP and live multi-agent spec delivery across harnesses | reported: used live to deliver several specs through agents on different harnesses |
| C2 | Scroll in an attached OMP session | reported: wheel scrolling works after spec 2q (`0a4a9d9`) |
| C3 | Status text claiming Codex and OMP receipts or delivery profiles were not built | `94c0672` |
| C4 | Mid-turn message receiving on OMP (`aside`) and on Codex, with real harnesses | reported: tested by the owner |
| C5 | Configured session reset with consent, guarded PTY submission, CLI and MCP surfaces; completion markers remain per-harness work and Claude SessionStart after `/clear` is unverified | spec 2s |
