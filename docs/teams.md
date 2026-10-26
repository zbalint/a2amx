# Teams

How-to and reference for operators configuring and managing a group of sessions.

## Team file

`a2amx team status`, `team up` (file form), `team down`, and `team reset` use
`a2amx.toml` in the invoking directory when it exists. Otherwise they use
`<state home>/team.toml` as a global fallback. A global file gets its team name
from the invoking directory: the final directory component is lowercased,
non-alphanumeric runs become `-`, and the result is limited to 40 bytes. A
global file must not set `team`; it may set `private` and `allow`, which apply
to the derived team. Relative session `cwd` values (including `cwd = "."`) are
resolved against the invoking directory for a global file. The fallback refuses
the filesystem root, home directories, hidden or user-data directories under
home, the daemon state and system trees, and top-level/container or Windows
home directories. A local `a2amx.toml` or explicit `--file` always overrides
the fallback and the guard. Two project directories with the same final
component therefore share a derived team name and its sessions; this is a
known limitation.
A team file looks like:

```toml
team = "myproject"
# private = false # teams are private by default
# allow = ["other-team"] # file-level allow: mutual private-team consent by team name
[[session]]
name = "architect"
command = ["claude"]
attach = true
role = "architect"
allow = ["other-team/architect"] # session-level allow: mutual consent by final agent name
reset = ["/clear", "/prewalk restart"]
control_from = ["developer"]
watch = ["developer"]
heartbeat = "30m" # digest watched peers after 30 minutes of idle time
activity = { working_after = "2m", idle_after = "30s" } # notices on peer transitions
# Reference forms: bare local "developer", explicit "other-team/architect", global "/hand".

[[session]]
name = "developer"
command = ["omp"]
cwd = "."
role = "developer"
```

The old `prefix` team-file key was renamed to `team`.

## Visibility

The file-level `team` prefixes every session name (`architect` becomes `myproject-architect`).
Teams are private by default; file-level `allow` names teams, and naming a public team changes nothing.
Private teams can see ungrouped sessions and public teams, but two private teams need mutual
file-level `allow` entries. A session-level `allow` uses the same key with `team/agent`
entries such as `other-team/architect`; both named sessions must list each other. It adds
that pair to the team-level visibility rule, still needs a private team, and does not expose
the rest of either team. Changing a running session's list changes its scope and is a
`team-mismatch`; take the team down with `a2amx team down` before the next `team up`.
Sessions outside a private visibility boundary are omitted from
`list_agents`/`list` and return `unknown recipient` when addressed by an agent. Team metadata
is not OS-user or process isolation. The flag form has no team. Each `command` element is one
argument, so use `["claude", "--model",
"opus"]`, not `["claude --model opus"]`; an executable path containing a space
is not accepted.

When any session has a team, `a2amx list` inserts a TEAM column after NAME; a private
team is shown as `NAME (private)`, and ungrouped sessions show `-`. `--team NAME` filters
by team; `--as NAME` keeps sessions visible to that named session and the named session
itself. The filters combine with AND and do not affect the picker, which remains the unfiltered
operator view; picker `/` filtering is local to the open picker.
Ungrouped (hand-started) sessions and `private = false` teams see everything and are seen
by everyone, so a team is protected only from other private teams.

## Session settings

`reset` is optional; omitting it uses `/clear`. If present it must contain one to
eight slash commands, each at most 200 bytes and without control characters.
`control_from` lists named sessions allowed to invoke this session's reset through
the MCP tool; the CLI/admin token is always allowed. `watch` lists named sessions
whose natural exit sends this session one daemon message. A `heartbeat` such as
`"30m"` sends an additional digest after that session is ready, unheld, not resetting,
and unchanged for the interval; it requires a non-empty `watch` list and accepts
positive `s`, `m`, or `h` values from 1 second through 24 hours. The digest is skipped
when every live watched peer has `idle` activity and an empty queue. A daemon-ended
session (for example through `kill`, `team down`, or `daemon stop`) sends no exit event.

`activity = { working_after = "2m", idle_after = "30s" }` enables transition notices
for watched peers. Either direction may be omitted, but at least one is required. A
direction with no threshold settles silently and advances the baseline. The first
settled state is always silent; later transitions are debounced and enter the ordinary
message queue, with at most one open activity message per watcher. The message body is
sorted by peer name and keeps the state from the last announcement. With `working_after`
unset, even a short output echo can eventually produce an idle notice; because working
ends ten seconds after output stops, use `idle_after = "30s"` or longer. Team plans reject
mutual or larger activity-watch cycles (`activity notices would loop ...`); manually
started sessions are not checked, so avoid enabling activity in both directions.

A cross-team `control_from` entry such as `other-team/agent` works only when the
two sessions can already see each other. Mutual session-level `allow` lets one
architect reset the other when the target lists that architect in `control_from`.

`role` is an optional label of at most 64 characters, shown only to that session in
its system prompt for Claude, OMP, and Codex. It grants no authority, is ignored for
other commands, and is not visible to peers or in `list`. When set, it is appended
to the same injected operator text as the peer-authorization line, including when
`--no-authorize-peers` is used.

Within a `[[session]]` table, `name` may be omitted when `role` is set; the role is then used as the name and must itself be a valid name (otherwise set `name` explicitly). An explicit `name` always wins.

A copy to start from is in [a2amx.toml.example](../a2amx.toml.example); your own `a2amx.toml` is gitignored.

Use `--file` for another file; relative `cwd` values are relative to that file's
directory, while an omitted `cwd` uses the invoking directory. `up` skips and reports
running sessions. An exited session holding a requested name stops the whole team
before any spawn; remove it with `a2amx kill NAME` first. A live team name with different
private/allow settings (file-level or session-level) is a conflict; run `a2amx team down` before changing those settings.
If a later spawn fails, already-started sessions remain and no further session starts.

## Team commands

Without a file, `a2amx team up architect=claude developer=omp` accepts bare
executables only and has no team; commands with arguments need the file's argv
arrays. The file may select one session to attach; the flag form attaches the
first. `--detach` suppresses attachment, and nonterminal invocation never attaches.
`a2amx team down` kills the file's named sessions, running or exited; explicit
names select full session names directly, including any team-prefixed name. It uses the
same graceful Ctrl-D default as `kill`; pass `--now` to use the immediate HUP/KILL
path. Missing names are reported without failing; explicit missing names also print
a stderr hint to use full names from `a2amx list`.

`a2amx team reset [--file F | NAME ...] [--except NAME]... [--include-attached] [--yes]`
runs the configured reset sequence (default `/clear`) for each selected session, in file order or NAME
argument order. It asks for confirmation before the first reset; pass `--yes` for
noninteractive use. Use `--except` to leave selected sessions untouched (an excepted name
that was not selected is ignored). Sessions with a human attachment are skipped by default
when selected from the file, because reset clears the conversation that person is viewing;
name a session explicitly or pass `--include-attached` to opt in. Missing sessions print
`no session`, exited sessions print a skip line, and a busy or drafting session is skipped
by the daemon, reported as `failed`, and followed by the remaining sessions.
exits 1 if any reset failed and 0 when every running session reset or was legitimately
skipped.

Resets are sequential and preserve selection order. Each reset step settles for at least
1.5 seconds, and a step that never becomes ready can take up to the 30-second timeout, so
a five-session team can take roughly ten to twenty seconds when all sessions are ready.
The command does not detect whether its caller is one of the selected agents; the state
directory is the trust boundary. Use `--except` for the session running the command.

`a2amx team status` also needs a running daemon and prints one `NAME TEAM STATE NOTE`
row per wanted session in file order. `TEAM` is the configured team name, with a
` (private)` suffix for private teams, or `-` when ungrouped. `STATE` is `missing`,
`running`, `exited`, or `team-mismatch`; `running` means only that the name and team
scope match, not that the command, cwd, role, or other settings are current. It exits
0 when `team up` would proceed (including missing sessions) and 1 when `team up` would
refuse because of a conflict. A conflict note uses the same remediation text as
`team up`; status returns that exit code directly without an `a2amx:` error line.

`a2amx team up --dry-run` uses the same daemon list and conflict preflight, then prints
`would start`/`already running` rows and the resolved harness, cwd, team visibility and
allow list, role, attachment, watch, controller, heartbeat, and redacted command for
each session that would start. It needs a running daemon, exits 1 on the same conflicts,
and never starts or attaches a session; OMP extension installation is explicitly skipped.
The command keeps `argv[0]`, flag names, and UTF-8 byte counts, but redacts every value
and every non-flag argument (including everything after `--`) because settings and
injected prompts can contain secrets. `--dry-run` cannot be combined with `--detach` or
inline `NAME=EXECUTABLE` items.

After `team up`, `team status`, or a dry run, the command may print
`note: N message(s) to this team were lost in a daemon restart; see a2amx messages
--state undeliverable` when retained undeliverable messages have detail
`daemon_restarted` for one of the wanted names. The note is best-effort and does not
change the command's result; messages are retained for the existing seven-day window.
`--file` cannot be combined with inline session items or names.

For individual session commands and attachment controls, see [CLI reference](cli.md).
