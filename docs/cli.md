# CLI reference

Reference for operators looking up commands, attachment controls, and status fields.

## Commands

The brackets below describe optional arguments; angle brackets name required values.

```text
a2amx daemon start [--foreground] [--owner-away-secs SECONDS] # start the daemon; detached by default
a2amx config show                    # show daemon configuration and value sources
a2amx daemon status                  # show daemon state, build version and session counts
a2amx daemon stop [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx new --watch developer --heartbeat 30m -- sh # digest watched peers after idle
a2amx new --watch developer --activity-working-after 2m --activity-idle-after 30s -- sh # notices on peer transitions
a2amx list [--details]
a2amx [--viewer-after SECONDS] attach <id|name> [--force]
a2amx kill <id|name> [--yes] [--now] # graceful by default; --now skips Ctrl-D
a2amx kill --exited              # remove every exited session without prompting
a2amx reset <id|name>               # type the configured reset sequence
a2amx statusline [--report]         # Claude status-line command, injected by a2amx; not run by hand
a2amx screen <id|name> [--rows N]   # read visible screen text without attaching
a2amx explain <id|name>              # explain state, readiness, holds and reset gate
a2amx confine show [<id|name>]  # show recorded confinement mode and grants
a2amx daemon start --host-name host-a  # names this host in addresses (name@host-a)
# --harness is inferred from the command name when omitted (claude, codex, omp)
a2amx new --name agent-plan --harness claude -- claude
a2amx new --name agent-review --harness omp -- omp
a2amx messages [--session <id>] [--state pending]
a2amx cancel <message-id>
```

## Explain

`a2amx explain SESSION` is a read-only, admin-only view; a session token is refused. It reports
the daemon's evidence from today and does not record or reconstruct whether past activity came
from a native channel, prompt hook or screen.

For a confined session, running or exited, it ends with the confinement: the grants and the writable and
read-only paths, the paths no option opens, the hidden paths present on this host, and for each denied path the
grant, `--confine-unhide` flag or `write` line that opens it.

```text
s1 (worker@host-a): generic, running
activity: busy (held: human_draft)
ready: yes
needs input: no
hold: human_draft - A person typed in the recipient session. Delivery resumes when they submit or release it (prefix, then r).
queued: 2
reset: refused (draft_present): hold human_draft
```

- The first line identifies the session, address, harness and whether its child is running or
  exited.
- `activity` reports the current idle, working or busy classification and the evidence that
  produced it.
- `ready` reports the harness readiness check; `no (starting)` means the readiness latch has not
  been set yet.
- `needs input` reports the harness input-needed signal, or `no`.
- `hold` reports the delivery hold and its explanation, or `none`.
- `queued` reports the number of open messages for the session.
- `reset` reports the reset gate's answer at this instant; `gate open` is not a promise that a
  later reset will succeed.

Looking can latch `starting` as `list` does because the readiness check observes the live screen.


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
description, and a warning line under each grant that can escape the sandbox, without
contacting a daemon or reading `HOME`.

`a2amx confine suggest [DIR]` checks project markers and walks upward from the
project directory, following `.git` file pointers. It reads supported git remote forms
only when the common directory's `config` is a regular file of at most 65,536 bytes.
It defaults to the current directory and does not need a daemon; credential probes use
`HOME` when it is available.

The repeatable path flags are `--confine-write PATH`, `--confine-hide PATH`, and
`--confine-unhide PATH`; `--confine-grant NAME` opens one named runtime capability.
The valid values and what each one exposes are listed by `a2amx confine list`.
`--confine-keep-env NAME` returns one listed credential environment variable to this session;
repeat it for multiple names. It requires `--confine` and rejects names that are not in the
closed credential list.

Relative paths are resolved against the session cwd. `--confine-default-hides=false` shows the optional home
credential locations and `/run/dbus` visible instead of hidden, but never removes the mandatory hides: the state
directory, the runtime directory, container sockets without a grant, `/run/WSL` and `/mnt/wslg`. Claude and
OMP harness configuration roots are writable and persistent by default; `harness-config` names the same
capability explicitly. There is no build-path switch. The private `/proc` and PID namespace are base options,
and descriptor restrictions are a daemon property.
Rootless runtime sockets under `$XDG_RUNTIME_DIR` remain hidden. These flags require
`--confine`; path lists accept at most 64 entries and each entry is capped at 4096 bytes.

The session status line adds a dim `[confined]` tag immediately after the session name,
or `[confined auto]` when global auto detection added grants. `a2amx list` and
`a2amx list --details` add `CONFINE` immediately after `HARNESS`; it is `off`, `manual`,
or `auto`, including for exited sessions.
`a2amx team up --dry-run` reports confinement, writable/protected paths, the final hidden
paths, and missing paths skipped from the launch plan. A socket or file created on the host
after launch is visible when it is outside every launch-time hide; start container daemons
before a confined session if their socket must be hidden.

`a2amx confine show [<id|name>]` reads the daemon's recorded plan. Omit the reference to
show every session. It prints the mode, effective grants, each grant's source, and
explicit writable paths. Automatic grants include their detector reason:

```text
$ a2amx confine show agent-plan
agent-plan s1 generic confine manual
GRANT  SOURCE
cargo  set
write: /tmp/a2amx-doc-project
```

For `a2amx team up --dry-run`, each session that would start includes
`confinement: off`, `confinement: manual`, or `confinement: auto`; automatic sessions
also print one `  <grant>: <reason>` line per detected grant, such as
`  cache: always added by auto`. An already-running session is reported with its mode,
for example `already running agent-plan s1 (confine manual)`.

When global automatic detection finds a grant not yet accepted, `a2amx team up` asks
before starting. Pass `a2amx team up --yes` to accept those grants for that run without
writing `<root>/state/auto-grants`; use a terminal to answer and save acceptance for
future projects.

See [Confinement](confinement.md) for recipes by ecosystem, credentials and Docker.

## Remote messaging

Remote messaging lets a confined, remote-enabled agent message its paired human in Discord.
With `both`, the human can write back. It is off by default; setup, lease and sandbox details
are in [Remote messaging](remote-messaging.md).

```text
a2amx gateway token                         # store the Discord bot token without echoing it
a2amx gateway up [--pair] [--foreground]   # start the confined gateway
a2amx gateway stop                          # gracefully stop the background gateway
a2amx gateway status                        # show whether the gateway is running
a2amx new --confine --remote <send|both> -- <command> # mark a confined session for remote messaging
```

`a2amx gateway up` is the supported launcher: the hidden internal relay should not be run directly,
because direct invocation is unconfined. Remote-enabled sessions must be confined; to reach the
gateway they must also belong to a named team.
With `--pair`, pairing stays in the foreground until confirmation and then hands the lock to the
background gateway; `--pair --foreground` keeps the gateway in this terminal until Ctrl-C.

## State directory and attachment

The a2amx home directory is `--home`, then `A2AMX_HOME`, then `$HOME/.a2amx`.

The global `--viewer-after SECONDS` option (or `A2AMX_VIEWER_AFTER`) switches an interactive
attachment to read-only viewer mode after that many seconds without input from the owner at the computer (keys, paste, mouse, prefix, picker, or scroll input; agent output does not reset it).
The default is 300 seconds; `0` disables the timer. Viewer mode can also be toggled with the prefix
key followed by `v`.
Viewer mode does not send terminal input or release frames to the session; it still sends resize
frames so the session stays sized to the client's usable terminal. It still updates the local
screen, lets prefix commands open the picker or scroll mode, and routes team-line clicks on their
release. A complete mouse report never displays the viewer notice; an ignored key or paste displays
it briefly. Switching back with prefix `v` sends the normal resize and redraw sequence. Viewer
attachment does not count as owner presence.

The attach status line shows a yellow `VIEW` marker when the client is in viewer mode; the overlay says
`VIEW MODE` and `typing is off, press the prefix key then v`.

The prefix key is Ctrl-B (`--prefix` or `A2AMX_PREFIX` to change it). After the prefix: `d` detach,
`v` toggle viewer mode, `w` session picker, `[` scroll mode, `s` toggles the status and team lines,
`r` releases the session's message hold, `1`–`9` select a team member, `n` and `p` select the next
or previous non-exited member, and the prefix twice sends a literal prefix. The picker accepts
PageUp/PageDown and Home/End and scrolls when sessions do not fit. When teams exist it groups
sessions into foldable sections: `Space`/`Enter` fold or unfold a selected header, `h`/`l` and
`Left`/`Right` fold or unfold its section, and `/` starts a case-insensitive filter that `Esc`
clears and exits. When more than two teams exist, other teams start folded; the current session's
team starts open.
While a session has not turned on mouse reporting, the mouse wheel scrolls it (three lines a step)
and enters scroll mode; hold Shift to select text with the mouse. Scrollback in the real terminal
after you detach is not kept. The status line takes the terminal's last row, keeps a thin rule
above it, and shows the session address, optional activity, low-quota, role and team, and harness
details, pending-message count, and a HELD alert with the hold reason and (for releasable holds) a
prefix-key hint for `r`. Team sessions with a visible status line and at least five rows also show
the clickable member line at the top; terminals with at least six rows add a dim separator rule
underneath. `s` hides both top rows, the status line, and both separator rules.

## Selecting and copying text

To copy text out of an attached session, hold Shift while you drag with the mouse (in most terminals, including
Windows Terminal, Shift bypasses mouse reporting and selects text in the terminal itself). This works whatever the
session runs, and your terminal's own copy applies.

A plain drag does not select, for two reasons. The attach client takes the mouse for wheel scrolling while the session
has not turned on mouse reporting (OMP), and a session that has turned it on (Claude Code) gets the drag for its own
selection. Such a harness may report "copied to clipboard", but a2amx does not forward clipboard writes (OSC 52) to your
terminal, so nothing reaches the clipboard. Use Shift+drag instead.

## List columns and quota

In `a2amx list`, ATTACHED means a human client is attached, not that the session is reachable: a detached session still receives messages. ACTIVITY is `idle`, `working`, or `busy` for running sessions, and `-` for exited sessions. UPTIME is how long the session has existed (`-` when exited), and IN-STATE is how long its current ACTIVITY has lasted (`-` when ACTIVITY is `-`). Both are floored and shown as `Ns`, `Nm` or `Nh`, at one-second sampling resolution, starting when the daemon starts. The default list omits each session's working directory and command; `a2amx list --details` adds CWD and COMMAND. HELD shows the reason a real hold is stopping delivery (`-` when clear); a sustained Codex `no_thread` or `waiting_on_approval` signal is shown as `needs_input`, while a sustained Claude screen heuristic is shown as `input?`. Both labels are display-only warnings: A2AMX never sends a key to dismiss a dialog. `input?` is deliberately hedged because the same two markers can appear in ordinary menus or search results, and the label appears only after ten seconds of a continuous signal. A session that looks idle with an empty composer may hold after you typed in it, and `r` clears it. `a2amx reset` also clears that hold on its own for a Claude or OMP session when the composer is empty (a grey prompt suggestion counts as empty) and two seconds have passed since you typed; a draft, a pasted-text chip or an image chip keeps the hold. QUOTA shows, for Claude, the 5-hour and weekly percent left that Claude Code reports to a2amx through the status-line command a2amx injects when it starts the session (subscription accounts only, after the first reply; a status line of your own keeps working and is called with the same input), and, for Codex, what its status line last said (5h and weekly percent left). Without a report a Claude session falls back to the percentages on its own status line. For OMP it shows the 5-hour and weekly percentages left for the provider of the session's current model when `omp usage --json` works; the extension samples that provider continuously, so changing providers changes the next displayed quota. The daemon polls about once every five minutes from the running session's own executable only when its file name is `omp`. Values older than 15 minutes are dropped. A missing provider or a provider absent from the usage report shows `-`; there is no fallback to another provider's quota. The poll may refresh provider credentials as the interactive `omp usage` does; a bare `omp` uses the daemon's `PATH`, and an OMP session running nothing costs nothing because no OMP session means no poll. The screen's `usage_limit_reached` error appends `limit`. It shows `-` when no quota signal is recognised.
In that column, `viewer` means the client is in viewer mode.

TEAM, `--team`, `--as`, and private-team visibility are described in [Teams](teams.md#visibility).

## Reset

`a2amx reset <id|name>` types `/clear` by default, or the ordered `--reset` sequence
configured for the session. Each step is bracketed-pasted, submitted with one Enter,
and waits for the harness to become ready before the next step. A reset fails fast on
an active draft, a held or non-ready session, a write error, or a 30-second per-step
timeout; one reset runs at a time. Session-token callers need the target's
`control_from` consent.
