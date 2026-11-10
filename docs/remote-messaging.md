# Remote messaging

> **Status:** The Discord setup steps and the gateway against a real Discord server are not yet tested with A2AMX.

Remote messaging lets an agent on a long task message its paired human in Discord. With the
`both` mode, the human can write back to that agent. It is messaging only: it does not stream
terminal output, provide remote control, carry attachments, or support direct messages with
agents. Discord text is not end-to-end encrypted; Discord and everyone with access to the server
can read it.

## When you are at the computer

Presence is an interactive attach. The daemon keeps the owner present for 60 seconds after the
last interactive attachment ends by default; `a2amx daemon start --owner-away-secs SECONDS` (or
`A2AMX_OWNER_AWAY_SECS`) changes that grace period. `owner_present` refuses an agent's
`send_message(to="human:owner", ...)` while the owner is present. `list_agents` reports `owner`
as `present` or `away` for remote-enabled sessions, and the daemon emits one
“The owner is away from the computer...” notice when the state changes to away. Human inbound
messages are accepted regardless of that state.

Remote messaging is off by default, enabled per agent, and available only to confined sessions.
A human can watch an attached session without claiming owner presence by using viewer mode (`v` after
the attach prefix). Viewer mode is read-only for the session: terminal input, resize, and release
frames are suppressed while local picker, scroll, and team-line controls remain available.
A complete mouse report never displays the viewer notice; ignored keys and pastes do.
A timer can enter viewer mode automatically with the global `--viewer-after SECONDS` option;
`0` disables the timer.
A message from the phone is less trusted than input typed at the computer; it is not approval to
change the default branch, use production, handle secrets, or delete data.

## What you need

- Linux with `bwrap` (bubblewrap) available. `a2amx gateway up` refuses to start the gateway
  without it and launches the relay confined.
- A running A2AMX daemon and sessions started under confinement. Codex sessions cannot be
  confined, so Codex agents cannot use remote messaging.
- A Discord account and a private server you own. Keep the server without an invite link and
  keep the bot as the only other member.
- In the server's `@everyone` role, remove Create Invite, Mention @everyone, and other defaults
  the agents do not need. Check the bot's effective permissions on the server's Integrations
  page. The gateway needs only the permissions described below.

The gateway checks Discord's **approximate** member count against `max_members` (2 by default:
you and the bot). If the count is above the limit at startup, it exits. A later check pauses
traffic while the count is too high; the check runs every 60 seconds by default, and a failed
check changes nothing.

## Set up Discord (not yet tested)

These portal steps are from Discord's documented setup flow and have not been run against the
live Discord portal for A2AMX:

1. Create an application and a bot in the Discord developer portal.
2. Enable the **Message Content** privileged intent for the bot. A2AMX requests intents 1
   (guilds), 512 (guild messages), and 32768 (message content).
3. Copy the bot token. Treat it like a password; never put it in a command argument, an
   environment variable, a log, or a message. Reset it in the developer portal if it leaks.
4. Create a private server with no invite link. Enable developer mode in Discord and copy the
   server ID (the `guild_id`).
5. Invite the bot to that server with **Manage Channels**, **View Channels**, and **Send
   Messages**. Manage Channels lets A2AMX create team categories and agent channels. A2AMX does
   not read message history: inbound messages arrive through the Discord websocket, so Read
   Message History is not required by the gateway's REST calls.
6. Trim the server defaults as described in [What you need](#what-you-need), then check the
   bot's granted permissions on the server's Integrations page.

The gateway accepts messages only from the paired account in the configured server. It ignores
other authors, other servers, and direct messages.

## Configure the gateway

The state directory is selected in this order: `--home`, `A2AMX_HOME`,
`$XDG_STATE_HOME/a2amx`, then `$HOME/.local/state/a2amx`. The examples below use `<state>` for
that directory.

First store the Discord bot token. Type or paste the token and press Enter. The command prints
nothing; when standard input is a terminal, A2AMX disables echo while reading it:

```sh
a2amx gateway token
```

It stores the value as `<state>/gateway/discord.token` with mode `0600`.
Create `<state>/gateway/config.toml` yourself. The gateway state directory is created by the
token command. Replace `<state>` with the directory selected above; when invoking A2AMX with an
explicit `--home`, use that same directory:

```sh
install -d -m 700 "<state>/gateway"
install -m 600 /dev/null "<state>/gateway/config.toml"
```

Put the server ID and, optionally, names and timing overrides in the file:

| Key | Default | Meaning |
| --- | --- | --- |
| `guild_id` | required | Discord server ID: 1–20 ASCII digits |
| `control_category` | `a2amx` | Category containing the control channel |
| `control_channel` | `control` | Channel for pairing and lease codes |
| `max_members` | `2` | Maximum approximate Discord member count |
| `sync_ms` | `15000` | Agent/channel synchronization interval |
| `idle_ms` | `1000` | Outbound queue polling interval |
| `member_check_ms` | `60000` | Member-count check interval |
| `reconnect_ms` | `5000` | Daemon reconnect interval after a later outage |
| `api_base` | `https://discord.com/api/v10` | Discord REST base; leave unset for a real server |
| `gateway_url` | `wss://gateway.discord.gg` | Discord websocket base; leave unset for a real server |

`control_category` and `control_channel` must be 1–100 characters, start with a lowercase
letter or digit, and then contain only lowercase letters, digits, or `-`. Every `*_ms` value
must be at least 1. Unknown keys are rejected. The token and config files must be regular files,
owned by the current user, without group or other permission bits, and no larger than 64 KiB;
symlinks are refused. Create them with mode `0600`.

`api_base` and `gateway_url` are test overrides. Leave both unset when connecting to Discord;
the defaults in the table are used.

## Pair your Discord account

Start the daemon first, then run:

```sh
a2amx gateway up --pair
```

`up --pair` starts the gateway in its confined sandbox. It creates the `a2amx` control category
and `control` channel (or the configured names), prints a pairing code, and waits for that code
to appear in the control channel. Post the code from the Discord account that should be paired.
The terminal then asks whether to bind that Discord account; answer `y` or `yes`. A decline ends
with `pairing not confirmed`. The code expires after 10 minutes, and a successful pairing is
saved in `<state>/gateway/pairing.json` with mode `0600`.

The pairing confirmation is read from standard input; answer `y` or `yes` at the terminal. The
gateway accepts control and agent-channel messages only from the paired Discord account. After
pairing, later runs use:

```sh
a2amx gateway up
```

## Start the gateway

The daemon must be running before `gateway up`. `up` checks that `<state>/link/addr` and
`<state>/link/gateway.token` exist as regular files. If either is missing, it reports:

```text
the a2amx daemon is not running in this state directory; start it first
```

Those files can be stale after a daemon crash. The relay's first daemon connection is not retried,
so a stale address causes the relay to exit with a connection error at startup. Once the first
connection succeeds, a later daemon outage leaves the relay running: it answers the paired
account with `a2amx is not reachable right now` and reconnects through `<state>/link/` every
`reconnect_ms`.

The control category and channel appear at startup. Team categories and agent channels appear
only after the lease has started; synchronization lists only agents that have `remote` set, are
confined, and are covered by a valid lease. They appear within `sync_ms` (15 seconds by default).
A category represents a team, and each enabled agent gets one channel in that category.

`a2amx gateway up` starts the gateway relay inside bubblewrap; the hidden internal `gateway run`
relay is unconfined if invoked directly and must not be run by hand. Stop the relay with Ctrl-C
for a clean stop. SIGTERM stops the gateway immediately without a clean close.

## Turn it on for agents and start the lease

In a team file, set `remote` for each session that should use remote messaging:

```toml
team = "myproject"

[[session]]
name = "architect"
command = ["claude"]
cwd = "."
remote = "both"       # "send" or "both"
[session.confine]
enabled = true
```

`send` allows the agent to message the human. `both` additionally allows the paired human to
message that agent. Remote-enabled sessions must be confined. The team file must have a team
name, because leases and the gateway's agent list are per team.

The standalone command-line form is `a2amx new --confine --remote <send|both> -- <command>`;
remote messaging requires `--confine`. It marks a session but does not put an ungrouped session
in the gateway. A session outside a team never reaches remote messaging, and `new` has no team
option.

Bring the team up from the directory containing its team file:

```sh
a2amx team up
```

When remote messaging is configured, `team up` prints the enabled agents and a single-use code,
for example:

```text
remote messaging for: architect (both)
post this code in the control channel within 10m: <12-hex-code>
```

Post that 12-hex code in the Discord control channel. A successful redemption gets the reply
`remote messaging is on`. The code is valid for 10 minutes. Five wrong codes, counted across the
whole daemon, clear every pending code; run `a2amx team up` again to print a new code.

The redeemed lease lasts at most 24 hours, is held in daemon memory, and covers the team members
running with remote messaging enabled at the moment of redemption. The lease record remains until
it expires or the daemon stops. An agent restarted or added after redemption is not covered until
a new code is posted. Exiting the member sessions (including `team down`) leaves no running
recipients, and stopping the daemon loses the in-memory lease. A daemon restart therefore requires
a new lease code.

An agent sends to the paired human with the existing message tool:

```text
send_message(to="human:owner", subject="…", message="…")
```

## What the sandbox does

`a2amx gateway up` runs the gateway relay inside bubblewrap. The sandbox shares the host network
so it can reach Discord and the daemon's loopback address; A2AMX does not filter its network
traffic.

| The gateway can see | The gateway cannot see |
| --- | --- |
| Read and write `<state>/gateway/` (including its bot token, config, pairing and dedup state) | `<state>/admin.token` |
| Read `<state>/link/` (the daemon address and gateway token) | The rest of the state directory |
| A 1 GiB temporary `/tmp` | The home directory, `/var`, `/opt`, and the rest of `/run` |
| Read-only `/usr`, `/bin`, `/sbin`, `/lib`, `/lib64`, `/etc`, and its own executable | Other users' files |
| The file targeted by `/etc/resolv.conf` when that target is outside `/etc` and `/usr` | A host-wide process view; `/proc` is its own PID namespace |
| A minimal `/dev` and its own processes | Environment variables other than the configured `PATH` and optional `RUST_LOG` (bubblewrap also supplies `PWD` for the gateway working directory) |

The sandbox's network namespace is shared. If a host firewall or proxy is in use, allow the
Discord REST host `discord.com` and `gateway.discord.gg`; Discord may redirect the websocket to
another `*.discord.gg` host. That redirect behavior is from Discord's documentation and is not
yet tested with A2AMX.

## Limits and residual risks

- Human-to-agent traffic is limited to 20 messages per minute.
- All agent-to-human traffic shares a 30-message-per-minute limit, and each agent has its own
  20-message-per-minute limit.
- A human message is limited to 8000 bytes. Agent messages are split into pieces of at most 2000
  characters before the gateway posts them to Discord.
- When the approximate server member count exceeds `max_members`, the gateway pauses traffic;
  a failed periodic count check changes nothing.
- Abstract Unix sockets and loopback TCP remain reachable from the sandbox because its network is
  shared.
- The sandbox does not create a new terminal session; on a kernel that still permits `TIOCSTI`, a
  compromised gateway could push input into the terminal from which it was started. Start it from
  a terminal whose input you do not mind exposing, or use `setsid`.
- Ctrl-C stops the relay cleanly, while SIGTERM stops the gateway immediately without a clean close.
- If `link/` is deleted and recreated by hand while the gateway is running, its directory mount
  remains the old, stale directory until the gateway restarts. The daemon itself does not remove
  or recreate `link/` while running; a daemon restart rewrites the files inside the existing
  directory, and the mount shows those files.
- A message from Discord is recorded as seen before the daemon accepts it, and it is not retried.
  If the daemon cannot be reached at that moment, the gateway posts `a2amx is not reachable right
  now` (a best-effort reply that can itself fail) and the message is not delivered; send it again.
  If the daemon accepted a message but its reply was lost, sending it again delivers it twice.
- The bot token and pairing record are files on disk with mode `0600`.
- A message from the phone carries less authority than input typed at the computer.

## Troubleshooting

| Message | Cause | Fix |
| --- | --- | --- |
| `bubblewrap is required to run the gateway; it never runs unconfined` | Bubblewrap is missing or its preflight failed. | Install a working `bwrap` and make it available on the daemon/gateway launch `PATH`; there is no unconfined fallback. |
| `the a2amx daemon is not running in this state directory; start it first` | `<state>/link/` or one of its required regular files is missing. | Start the daemon for this state directory, then run `gateway up` again. |
| `not paired; run with --pair` | No `pairing.json` exists. If it exists but is malformed, insecure, or mismatched, its own error names the bad file. | Run `a2amx gateway up --pair` interactively and redeem a pairing code. |
| `server has N members, the limit is M` | The approximate Discord member count is above `max_members`; at startup the relay exits. | Remove extra members or raise `max_members` deliberately, then restart the gateway. During a later check, wait for the count to fall; the relay pauses while it is above the limit. |
| `that code is not valid` | The lease code is wrong, expired, already used, or all pending codes were cleared after five wrong attempts. | Run `a2amx team up` at the computer and post its new code. |
| `send the code that a2amx team up printed` | The control-channel text is not a 12-character lowercase hexadecimal lease code. | Copy the exact code printed by `a2amx team up`. |
| `a2amx is not reachable right now` | The gateway was connected, but the daemon is currently unreachable. | Check the daemon and `<state>/link/`; the relay retries through `link/` every `reconnect_ms`. |
| `cannot resolve /etc/resolv.conf; the gateway needs DNS` | The resolver symlink is dangling, unreadable, or points to a non-regular file. | Repair `/etc/resolv.conf` or its target, then run `gateway up` again. |
| `secure file permissions are too broad` | A token, config, or pairing file has group or other permission bits. | Make the file a regular, owner-only `0600` file; do not replace it with a symlink. |
| `pairing not confirmed` | The interactive pairing prompt was declined or did not receive `y`/`yes`. | Run `a2amx gateway up --pair` in a terminal and confirm the paired account. |
| `pairing timed out` | No matching pairing code arrived within 10 minutes. | Run `gateway up --pair` again and post its newly printed code promptly. |
| `gateway close 4013: invalid intents` or `gateway close 4014: disallowed intents` | Discord rejected the requested gateway intents. | Enable the guilds, guild messages, and Message Content intents for the bot, then pair/start again. |

## Files

The daemon state directory and its files are owner-only. The gateway token and pairing writes use
mode `0600`; the daemon's state files and the dedup file use the same mode.

| Path | Owner and mode | Purpose |
| --- | --- | --- |
| `<state>/gateway/discord.token` | Current user, `0600` | Discord bot token entered with `gateway token` |
| `<state>/gateway/config.toml` | Current user, `0600` | Gateway configuration |
| `<state>/gateway/pairing.json` | Current user, `0600` | Paired Discord server/user IDs |
| `<state>/gateway/seen.json` | Current user, `0600` | Last handled Discord message ID per channel for deduplication |
| `<state>/link/addr` | Daemon user, `0600` | Daemon address exposed to the gateway |
| `<state>/link/gateway.token` | Daemon user, `0600` | Gateway-authentication token exposed to the gateway |
| `<state>/gateway.token` | Daemon user, `0600` | Daemon-side copy of the gateway-authentication token |
| `<state>/addr` | Daemon user, `0600` | Daemon address for local clients |

Do not share `admin.token`, `gateway.token`, `link/gateway.token`, or the Discord bot token. The
sandbox is designed to expose only the gateway-specific files listed above.
