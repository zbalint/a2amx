# CLI reference

Reference for operators looking up commands, attachment controls, and status fields.

## Commands

The brackets below describe optional arguments; angle brackets name required values.

```text
a2amx daemon start [--foreground]  # start the daemon; detached by default
a2amx daemon status               # show daemon state, build version and session counts
a2amx daemon stop [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx new --watch developer --heartbeat 30m -- sh # digest watched peers after idle
a2amx new --watch developer --activity-working-after 2m --activity-idle-after 30s -- sh # notices on peer transitions
a2amx list [--details]
a2amx attach <id|name> [--force]
a2amx kill <id|name> [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx kill --exited              # remove every exited session without prompting
a2amx reset <id|name>               # type the configured reset sequence
a2amx screen <id|name> [--rows N]   # read visible screen text without attaching
a2amx daemon start --host-name host-a  # names this host in addresses (name@host-a)
# --harness is inferred from the command name when omitted (claude, codex, omp)
a2amx new --name agent-plan --harness claude -- claude
a2amx new --name agent-review --harness omp -- omp
a2amx messages [--session <id>] [--state pending]
a2amx cancel <message-id>
```

For team commands and their selection rules, see [Teams](teams.md#team-commands).

`kill`, `team down`, and `daemon stop` use graceful Ctrl-D by default; pass `--now`
to skip it and use the immediate HUP/KILL path.

`a2amx kill --exited` removes every exited session and never prompts; running sessions are
untouched. It prints `removed NAME-or-ID` for each removed session, or `no exited sessions`
when there is nothing to remove. The `--exited` form cannot be combined with a session
reference, `--yes`, or `--now`.

Commands that take a session accept its name or its id.
Without `--name`, `new` generates a name such as `soggy-walrus`, prints it on stderr, and treats it like an explicit name.

`--activity-working-after` and `--activity-idle-after` enable daemon activity notices for
the named `--watch` peers. Each value accepts `s`, `m`, or `h` from 1 second through
24 hours; at least one direction and a non-empty watch list are required. Working means
the peer is not ready or changed within ten seconds; idle notices therefore arrive at
least ten seconds after the peer's last output. The first settled state is silent, and
each direction is debounced independently. Prefer 30 seconds or more for `idle-after`.

## Confinement

`a2amx new --confine -- <command>` runs the PTY child under bubblewrap. The daemon
resolves `bwrap` from its startup `PATH` and refuses the request when the executable
or its `--version` probe is unavailable; confinement is opt-in and unconfined sessions
retain their existing launch behavior.

Confinement is a safety net for a trusted but fallible agent, not a security boundary.
It does not isolate a malicious agent or dependency; use a container or a VM for that.
The residuals are documented in [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries).

Confined Generic commands receive Ctrl-C as they would when unconfined: the sandbox
wrapper ignores SIGINT and the command gets the default disposition back. Claude and
OMP are unaffected because they run the terminal in raw mode and receive Ctrl-C as the
byte 0x03. Confinement of a Generic session needs `env` with `--ignore-signal` and
`--default-signal` (GNU coreutils 8.31 or newer, or uutils).

`a2amx confine list` prints the valid grant names, scope and one-line exposure
description without contacting a daemon or reading `HOME`.

The repeatable path flags are `--confine-write PATH`, `--confine-hide PATH`, and
`--confine-unhide PATH`; `--confine-grant NAME` opens one named runtime capability.
The valid values and what each one exposes are listed by `a2amx confine list`.
Relative paths are resolved against the session cwd. `--confine-no-default-hides`
disables the optional home hides but never removes the mandatory hides: the state
directory, the runtime directory, `/run/WSL` and `/mnt/wslg`. The private `/proc` and
PID namespace are base options, and descriptor restrictions are a daemon property.
Rootless runtime sockets under `$XDG_RUNTIME_DIR` remain hidden. These flags require
`--confine`; path lists accept at most 64 entries and each entry is capped at 4096 bytes.

The session status line adds a dim `[confined]` tag immediately after the session name.
`a2amx team up --dry-run` reports confinement, writable/protected paths, the final hidden
paths, and missing paths skipped from the launch plan. A socket or file created on the host
after launch is visible when it is outside every launch-time hide; start container daemons
before a confined session if their socket must be hidden.

See [Confinement](confinement.md) for recipes by ecosystem, credentials and Docker.

## State directory and attachment

The state directory is `--home`, then `A2AMX_HOME`, then `$XDG_STATE_HOME/a2amx`,
then `$HOME/.local/state/a2amx`. The prefix key is Ctrl-B (`--prefix` or
`A2AMX_PREFIX` to change it). After the prefix: `d` detach, `w` session picker, `[`
scroll mode, `s` toggles the status and team lines, `r` releases the session's message hold,
`1`–`9` select a team member, `n` and `p` select the next or previous non-exited member,
and the prefix twice sends a literal prefix. The picker accepts PageUp/PageDown and Home/End and scrolls when sessions do not fit. When teams
exist it groups sessions into foldable sections: `Space`/`Enter` fold or unfold a selected header, `h`/`l` and
`Left`/`Right` fold or unfold its section, and `/` starts a case-insensitive filter that `Esc` clears and exits.
When more than two teams exist, other teams start folded; the current session's team starts open.
While a session has not turned on mouse reporting, the mouse wheel scrolls it (three lines a step) and
enters scroll mode; hold Shift to select text with the mouse. Scrollback in the real terminal after you detach is not
kept. The status line takes the terminal's last row, keeps a thin rule above it, and shows the session address, optional activity,
low-quota, role and team, and harness details, pending-message count, and a HELD alert with the hold reason and (for
releasable holds) a prefix-key hint for `r`. Team sessions with a visible status line and at least five rows also show
the clickable member line at the top; terminals with at least six rows add a dim separator rule underneath. `s` hides
both top rows, the status line, and both separator rules.

## Selecting and copying text

To copy text out of an attached session, hold Shift while you drag with the mouse (in most terminals, including
Windows Terminal, Shift bypasses mouse reporting and selects text in the terminal itself). This works whatever the
session runs, and your terminal's own copy applies.

A plain drag does not select, for two reasons. The attach client takes the mouse for wheel scrolling while the session
has not turned on mouse reporting (OMP), and a session that has turned it on (Claude Code) gets the drag for its own
selection. Such a harness may report "copied to clipboard", but a2amx does not forward clipboard writes (OSC 52) to your
terminal, so nothing reaches the clipboard. Use Shift+drag instead.

## List columns and quota

In `a2amx list`, ATTACHED means a human client is attached, not that the session is reachable: a detached session still receives messages. ACTIVITY is `idle`, `working`, or `busy` for running sessions, and `-` for exited sessions. UPTIME is how long the session has existed (`-` when exited), and IN-STATE is how long its current ACTIVITY has lasted (`-` when ACTIVITY is `-`). Both are floored and shown as `Ns`, `Nm` or `Nh`, at one-second sampling resolution, starting when the daemon starts. The default list omits each session's working directory and command; `a2amx list --details` adds CWD and COMMAND. HELD shows the reason a real hold is stopping delivery (`-` when clear); a sustained Codex `no_thread` or `waiting_on_approval` signal is shown as `needs_input`, while a sustained Claude screen heuristic is shown as `input?`. Both labels are display-only warnings: A2AMX never sends a key to dismiss a dialog. `input?` is deliberately hedged because the same two markers can appear in ordinary menus or search results, and the label appears only after ten seconds of a continuous signal. A session that looks idle with an empty composer may hold after you typed in it, and `r` clears it. QUOTA shows what a Claude or Codex status line last said about remaining quota (5h and weekly percent left). For OMP it shows the 5-hour and weekly percentages left for the provider of the session's current model when `omp usage --json` works; the extension samples that provider continuously, so changing providers changes the next displayed quota. The daemon polls about once every five minutes from the running session's own executable only when its file name is `omp`. Values older than 15 minutes are dropped. A missing provider or a provider absent from the usage report shows `-`; there is no fallback to another provider's quota. The poll may refresh provider credentials as the interactive `omp usage` does; a bare `omp` uses the daemon's `PATH`, and an OMP session running nothing costs nothing because no OMP session means no poll. The screen's `usage_limit_reached` error appends `limit`. It shows `-` when no quota signal is recognised.

TEAM, `--team`, `--as`, and private-team visibility are described in [Teams](teams.md#visibility).

## Reset

`a2amx reset <id|name>` types `/clear` by default, or the ordered `--reset` sequence
configured for the session. Each step is bracketed-pasted, submitted with one Enter,
and waits for the harness to become ready before the next step. A reset fails fast on
an active draft, a held or non-ready session, a write error, or a 30-second per-step
timeout; one reset runs at a time. Session-token callers need the target's
`control_from` consent.
