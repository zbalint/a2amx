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

Confinement is a safety net for a trusted but fallible agent. It lowers the damage an
honest mistake can do, such as `rm -rf`, `chmod`, or `chown`; it is not a security
boundary or isolation from a malicious agent or dependency. The session shares the
kernel, the network, and the operator's user account with the host: files are read and
written as you. It does not share the process list; that list is private. Anyone who
needs that isolation uses a container or a VM.

`--confine` is an opt-in bubblewrap confinement of the PTY child. Generic, Claude,
and OMP sessions may be confined; Codex is refused because its app-server tools run
outside the PTY process. The plan keeps harness data directories read-only except for
the documented data allow-list, hides the daemon state and runtime directory, and
always unsets `DBUS_SESSION_BUS_ADDRESS`, `SSH_AUTH_SOCK`, `GPG_AGENT_INFO`,
`WSL_INTEROP`, `DISPLAY`, and `WAYLAND_DISPLAY`. `DOCKER_HOST`, `DOCKER_CONTEXT`,
`CONTAINER_HOST`, `CONTAINER_CONNECTION`, `CONTAINERD_ADDRESS`, `BUILDKIT_HOST`,
and `LIBVIRT_DEFAULT_URI` stay set only with the `containers` grant. That grant
exposes only the configured system container sockets; rootless sockets under
`$XDG_RUNTIME_DIR` remain hidden. For an OMP session, the `omp` directory inside
the state directory stays visible, read-only; the rest of the state directory is
hidden.

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

### What confinement does not cover

- Network access remains available, including services listening on localhost such as
  an editor's MCP server, Jupyter, dev servers, and databases; on WSL2 this includes
  the Windows host's forwarded ports. A confined process can ask such a service to
  act for it. Only a network namespace would stop that, and it would also stop the
  agent's own API access.
- Abstract Unix sockets remain reachable.
- Messages to peer sessions through `send_message` remain possible.
- Kernel keyrings are not covered.
- CPU and memory outside `/tmp` are not limited, and disk exhaustion is possible; there
  are no cgroup limits.
- The Windows drives under `/mnt` on WSL remain readable.
- A sandbox started inside the session, such as a harness's own, does not revoke a
  descriptor opened before it: masking a path leaves an already-open descriptor usable.
  The a2amx child holds no descriptor above 2, so this concerns descriptors the agent
  or its harness opens itself.
- `XDG_RUNTIME_DIR` stays set although the directory is hidden.
- There is no syscall filter.
- Everything the agent may write is writable, including files that run later outside the
  sandbox: `package.json` scripts, `Makefile`, `.pre-commit-config.yaml`, `lefthook.yml`,
  `.husky/`, `.vscode/tasks.json`, `.envrc`, and the repository's
  `.claude/settings*.json` and `.mcp.json`. Instructions planted in data the next
  unconfined run reads (a harness's per-project memory, todos and plans, `CLAUDE.md`,
  `AGENTS.md`, and OMP's agent databases) are likewise not protected. Confinement
  protects files and the listed code-carrying paths; it does not make the writable set
  trustworthy.
- **Review the workspace diff before building, testing or committing it outside a
  sandbox.**

### Background processes

- Everything a confined session starts (file tools, hooks, MCP servers, shell commands)
  runs in the session's own PID and mount namespaces. Inside, process ids are small
  numbers; the host shows other ids, so a process id recorded by a helper started in
  one session is wrong in every other session and in the host. A liveness check that
  tests a recorded process id from another session tests the wrong process; check a
  port or a socket instead.
- Sessions still share the host network and any directory that is writable in each of
  them (for example one named with `--confine-write`), so a helper started in one session
  can be found by the others through a loopback port or a file in such a directory.
  In these docs, the state directory is a2amx's own and is hidden. `/tmp` is private to
  each session and `$XDG_RUNTIME_DIR` is hidden, so a helper whose socket, lock, or pid
  file lives in either cannot be found by the other sessions.
- When the session's command exits, or the sandbox is killed, every process in it ends,
  including a helper started by an MCP server or hook; other sessions that use that
  helper lose it.
- To share a helper across sessions, start a shared helper daemon by hand, outside any
  sandbox, before the team, and let the sessions find it by port.

See [Confinement](confinement.md) for recipes by ecosystem, credentials and Docker.
