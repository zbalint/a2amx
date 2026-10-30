# Harnesses and delivery

Explanation for operators choosing a harness profile and interpreting delivery evidence.

## Hosting and channels

Interactive agent harnesses expose a terminal input path. A2AMX launches them
under pseudoterminals (PTYs) and lets people switch between their sessions. It
delivers messages from other agents through each harness's native channel where one
exists (Claude Code, OMP, Codex), and by typing into the terminal otherwise.

Each agent has an MCP server for sending messages. Claude Code also has a hook for
observing prompt submission; OMP and Codex report receipts through their channels. The local daemon routes and persists messages on one host and
owns local PTYs, keeping sessions running when a client disconnects. Routing across
hosts is proposed, not implemented.

Claude Code and Codex can already message parallel sessions of their own kind, but
not sessions of a different harness. A2AMX targets that gap; cross-host sessions
and routing are proposed, not implemented.

Writing bytes to a PTY does not prove that the harness submitted a prompt. A hook
receipt does not prove that a model processed it. Those distinctions, and protecting
unfinished human input, are central design constraints.

## Harness profiles

`--harness claude` wires the MCP server and the prompt-submit hook into Claude Code
and delivers messages automatically; `--harness generic` (inferred for unrecognised commands) holds them, and
`--deliver auto` opts a generic session in to blind delivery. `a2amx mcp` is the stdio
MCP server that harnesses start; it reads `A2AMX_ADDR` and `A2AMX_TOKEN` from its
environment. `a2amx hook` is the Claude Code `UserPromptSubmit` hook, installed through
an inline `--settings` argument; a user `--settings` in the command is merged with this
hook rather than replaced. It reads the harness payload on stdin, always exits 0, and
prints output only when it blocks a prompt.

Claude delivery uses a Claude Code channel by default, preserving any composer draft.
`--harness claude --no-channel` keeps terminal delivery. Channel startup auto-accepts
Claude's development-channel dialog; this research preview needs a claude.ai or Console
login. A successful write has `write_complete` evidence until the prompt hook confirms
`native_receipt`; a missing receipt never triggers a resend.

`--harness omp` writes the A2AMX extension into the state directory, loads it into OMP
with `-e` and a per-session `--config` overlay, and delivers messages natively without
typing into the terminal.

`--harness codex` hosts a private app-server and delivers through it, rather than typing into the terminal.

For input arbitration and failure semantics, see [Delivery](delivery.md).

## Confinement and harness boundaries

`--confine` is an opt-in bubblewrap boundary around the PTY child. Generic, Claude,
and OMP sessions may be confined; Codex is refused because its app-server tools run
outside the PTY process. The plan keeps harness data directories read-only except for
the documented data allow-list, hides the daemon state and runtime directory, and
unsets host/container environment variables unless `--confine-allow-containers` is
selected. That switch exposes only the configured system container sockets; rootless
sockets under `$XDG_RUNTIME_DIR` remain hidden.

The Claude and OMP confined launch notices are appended to an existing system prompt.
Claude refuses `--append-system-prompt-file`; OMP refuses an existing file used as the
append-prompt value. Generic sessions receive no notice. A socket created after launch
outside every launch-time hide is intentionally visible, so start container daemons
before the confined session when their host socket must be restricted.

Confined Generic commands receive Ctrl-C as they would when unconfined: the sandbox
wrapper ignores SIGINT and the command gets the default disposition back. Claude and
OMP are unaffected because they run the terminal in raw mode and receive Ctrl-C as the
byte 0x03. Confinement of a Generic session needs `env` with `--ignore-signal` and
`--default-signal` (GNU coreutils 8.31 or newer, or uutils).
