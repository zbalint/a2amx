# CLI reference

Reference for operators looking up commands, attachment controls, and status fields.

## Commands

The brackets below describe optional arguments; angle brackets name required values.

```text
a2amx daemon start [--foreground]  # start the daemon; detached by default
a2amx daemon status               # show daemon state, build version and session counts
a2amx daemon stop [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx new --watch developer --heartbeat 30m -- sh # digest watched peers after idle
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

## State directory and attachment

The state directory is `--home`, then `A2AMX_HOME`, then `$XDG_STATE_HOME/a2amx`,
then `$HOME/.local/state/a2amx`. The prefix key is Ctrl-B (`--prefix` or
`A2AMX_PREFIX` to change it). After the prefix: `d` detach, `w` session picker, `[`
scroll mode, `s` toggles the status line, `r` releases the session's message hold, and the prefix twice sends a
literal prefix. The picker accepts PageUp/PageDown and Home/End and scrolls when sessions do not fit. When teams
exist it groups sessions into foldable sections: `Space`/`Enter` fold or unfold a selected header, `h`/`l` and
`Left`/`Right` fold or unfold its section, and `/` starts a case-insensitive filter that `Esc` clears and exits.
When more than two teams exist, other teams start folded; the current session's team starts open.
While a session has not turned on mouse reporting, the mouse wheel scrolls it (three lines a step) and
enters scroll mode; hold Shift to select text with the mouse. Scrollback in the real terminal after you detach is not
kept. The status line takes the terminal's last row, keeps a thin rule above it, and shows the session address, optional activity,
low-quota, role and team, and harness details, pending-message count, and a HELD alert with the hold reason and (for
releasable holds) a prefix-key hint for `r`; `s` hides both the status line and separator.

## List columns and quota

In `a2amx list`, ATTACHED means a human client is attached, not that the session is reachable: a detached session still receives messages. ACTIVITY is `idle`, `working`, or `busy` for running sessions, and `-` for exited sessions. UPTIME is how long the session has existed (`-` when exited), and IN-STATE is how long its current ACTIVITY has lasted (`-` when ACTIVITY is `-`). Both are floored and shown as `Ns`, `Nm` or `Nh`, at one-second sampling resolution, starting when the daemon starts. The default list omits each session's working directory and command; `a2amx list --details` adds CWD and COMMAND. HELD shows the reason a real hold is stopping delivery (`-` when clear); a sustained Codex `no_thread` or `waiting_on_approval` signal is shown as `needs_input`, while a sustained Claude screen heuristic is shown as `input?`. Both labels are display-only warnings: A2AMX never sends a key to dismiss a dialog. `input?` is deliberately hedged because the same two markers can appear in ordinary menus or search results, and the label appears only after ten seconds of a continuous signal. A session that looks idle with an empty composer may hold after you typed in it, and `r` clears it. QUOTA shows what a Claude or Codex status line last said about remaining quota (5h and weekly percent left). For OMP it shows the 5-hour and weekly percentages left when `omp usage --json` works; the daemon polls about once every five minutes from the running session's own executable only when its file name is `omp`. Values older than 15 minutes are dropped. These account-level values use the smallest value across providers and accounts, so that is what is shown. The poll may refresh provider credentials as the interactive `omp usage` does; a bare `omp` uses the daemon's `PATH`, and an OMP session running nothing costs nothing because no OMP session means no poll. The screen's `usage_limit_reached` error appends `limit`. It shows `-` when no quota signal is recognised.

TEAM, `--team`, `--as`, and private-team visibility are described in [Teams](teams.md#visibility).

## Reset

`a2amx reset <id|name>` types `/clear` by default, or the ordered `--reset` sequence
configured for the session. Each step is bracketed-pasted, submitted with one Enter,
and waits for the harness to become ready before the next step. A reset fails fast on
an active draft, a held or non-ready session, a write error, or a 30-second per-step
timeout; one reset runs at a time. Session-token callers need the target's
`control_from` consent.
