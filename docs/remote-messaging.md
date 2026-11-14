# Remote messaging

> **Status:** The maintainers have tested remote messaging live against a real Discord server: pairing, the lease code, messages in both directions, the background gateway and the status line. Not yet tested live: a well-formed wrong lease code, an unpaired Discord account, and a daemon outage while the gateway runs.

Remote messaging lets an agent on a long task message its paired human in Discord. With the
`both` mode, the human can write back to that agent. It is messaging only: it does not stream
terminal output, provide remote control, carry attachments, or support direct messages with
agents. Discord text is not end-to-end encrypted; Discord and everyone with access to the server
can read it.

## When you are at the computer

Presence is an interactive attach. The daemon keeps the owner present for the `owner_away`
duration from `daemon.toml` (`60s` / `1m` by default) after the last interactive attachment
ends; `a2amx daemon start --owner-away-secs SECONDS` (or `A2AMX_OWNER_AWAY_SECS`) changes that
grace period. `owner_present` refuses an agent's
`send_message(to="human:owner", ...)` while the owner is present. `list_agents` reports `owner`
as `present` or `away` for remote-enabled sessions, and the daemon emits one
“The owner is away from the computer...” notice when the state changes to away. Human inbound
messages are accepted regardless of that state.

Remote messaging is off by default, enabled per agent, and available only to confined sessions.
A human can watch an attached session without claiming owner presence by using viewer mode (`v` after
the attach prefix). Viewer mode is read-only for terminal input: input and release frames are
suppressed, while resize frames keep the session sized to the client's usable terminal. Local picker,
scroll, and team-line controls remain available.
A complete mouse report never displays the viewer notice; ignored keys and pastes do.
A timer can enter viewer mode automatically with the global `--viewer-after SECONDS` option;
`0` disables the timer.
A message from the phone is less trusted than input typed at the computer; it is not approval to
change the default branch, use production, handle secrets, or delete data.

## What you need

- Linux with `bwrap` (bubblewrap) available. `a2amx gateway up` refuses to start the gateway
  without it and launches the relay confined.
- A running A2AMX daemon and sessions started under confinement. Codex sessions may be confined,
  so Codex agents can use remote messaging.
- A Discord account and a private server you own. Keep the server without an invite link and
  keep the bot as the only other member.
- In the server's `@everyone` role, remove Create Invite, Mention @everyone, and other defaults
  the agents do not need. Check the bot's effective permissions on the server's Integrations
  page. The gateway needs only the permissions described below.

The gateway checks Discord's **approximate** member count against `max_members` (2 by default:
you and the bot). If the count is above the limit at startup, it exits. A later check pauses
traffic while the count is too high; the check runs every 60 seconds by default, and a failed
check changes nothing.

## Set up Discord

These portal steps follow Discord's documented setup flow. Discord changes its portal; if a screen
differs, follow Discord's own documentation:

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

The a2amx root directory is selected in this order: `--home`, `A2AMX_HOME`, then `$HOME/.a2amx`.
The examples below use `<root>` for that directory.

First store the Discord bot token. Type or paste the token and press Enter. The command prints
nothing; when standard input is a terminal, A2AMX disables echo while reading it:

```sh
a2amx gateway token
```

It stores the value as `<root>/config/secrets/discord.token` with mode `0600` and creates a
commented `<root>/config/gateway.toml` template once. Set `guild_id` in that file:

```toml
guild_id = "123456789012345678"
```

The template is owner-only (`0600`) and is never rewritten. The gateway file must be a regular,
owner-only file no larger than 64 KiB; symlinks and broad permissions are refused.

| Key | Default | Meaning |
| --- | --- | --- |
| `guild_id` | required | Discord server ID: 1–20 ASCII digits |
| `control_category` | `a2amx` | Category containing the control channel |
| `control_channel` | `control` | Channel for pairing and lease codes |
| `max_members` | `2` | Maximum approximate Discord member count; at least 1 |
| `sync` | `15s` | Agent/channel synchronization interval |
| `idle` | `1s` | Outbound queue polling interval |
| `member_check` | `1m` | Member-count check interval |
| `reconnect` | `5s` | Daemon reconnect interval after a later outage |

Intervals are duration strings using `ms`, `s`, `m`, `h`, or `d`, from `1ms` through `3650d`.
Unknown keys and malformed values are rejected. `api_base` and `gateway_url` are accepted as
test-only overrides and are hidden from the template and `config show`; leave them unset for a
real Discord server.

## Pair your Discord account

Start the daemon first, then run:

```sh
a2amx gateway up --pair
```

`up --pair` starts the gateway in its confined sandbox. Pairing is a one-time setup step: it
creates the `a2amx` control category and `control` channel (or the configured names), prints a
pairing code, and waits for that code to appear in the control channel. Post the code from the
Discord account that should be paired. The terminal then asks whether to bind that Discord
account; answer `y` or `yes`. In the default flow a decline ends with `pairing not confirmed`, followed by
`pairing did not complete; the gateway was not started`. The code expires after 10 minutes,
and a successful pairing is saved in `<root>/state/gateway/pairing.json` with owner-only permissions.

If a pairing already exists, `--pair` first warns which Discord account and server are currently
paired, then asks `replace Discord account <old-id> with <new-id>? [y/N]`. Confirming writes the
new pairing, prints `paired with Discord account <new-id>; starting the gateway in the background`,
and then hands the lock to the background gateway. `up --pair` returns after this hand-over.
Moving to a different Discord server requires deleting `<root>/state/gateway/pairing.json` first,
because the gateway refuses a pairing whose server differs from `gateway.toml`.

The pairing confirmation is read from standard input; answer `y` or `yes` at the terminal. The
gateway accepts control and agent-channel messages only from the paired Discord account. After
pairing, later runs use:

```sh
a2amx gateway up
```

## Start the gateway

The daemon must be running before `gateway up`. `up` checks that `<root>/run/link/addr` and
`<root>/run/link/gateway.token` exist as regular files. If either is missing, it reports:

```text
the a2amx daemon is not running in this a2amx directory; start it first
```

Those files can be stale after a daemon crash. The relay's first daemon connection is not retried,
so a stale address causes the relay to exit with a connection error at startup. Once the first
connection succeeds, a later daemon outage leaves the relay running: it answers the paired
account with `a2amx is not reachable right now` and reconnects through `<root>/run/link/` every
`reconnect`.

The control category and channel appear at startup. Team categories and agent channels appear
only after the lease has started; synchronization lists only agents that have `remote` set, are
confined, and are covered by a valid lease. They appear within `sync` (15 seconds by default).
A category represents a team, and each enabled agent gets one channel in that category.

`a2amx gateway up` starts the confined gateway in the background and prints its pid and log path.
Use `--foreground` to keep it in this terminal until Ctrl-C. With `--pair`, the launcher keeps
the pairing step in the foreground, then hands over to the same background launch; use
`--pair --foreground` to keep both pairing and message delivery in this terminal. A declined or
timed-out pairing leaves no gateway running. `gateway status` reports whether the lock is held,
and `gateway stop` performs a graceful background stop. The log is
`<root>/log/gateway.log`, defaults to level `warn`, and follows `RUST_LOG` when set. There is
no rotation; one gateway per a2amx root directory. A foreground gateway's stop is hard, so one message may be relayed twice after a
restart.

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

When standard input is a terminal, `team up` pauses after the lease output with
`press Enter to attach to the session`. This keeps the code visible before attachment switches
to the alternate screen. Press Enter to attach. Ctrl-C at the prompt ends `team up` but leaves
the sessions running; use `a2amx attach NAME` to attach later. With `--detach`, `team up` does
not pause or attach. Running `a2amx team up --detach` again prints a new code and leaves running
sessions alone.

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

### Remote state in the attach status line

Only remote-enabled sessions show a remote cell in the attach status line:

- `remote: no lease` means the team has not redeemed a lease for this session. Run
  `a2amx team up --detach` and post the printed code.
- `remote on` means the lease is active and a gateway process is attached to the daemon.
- `remote: gateway off` means the lease is active but no gateway process is currently attached.
  Start one with `a2amx gateway up`.
- `remote: lease ended` means the lease expired. Run `a2amx team up --detach` again.

“Gateway” here means that the gateway process is attached to the daemon; it does not report
whether Discord is reachable or whether messages are currently relaying.

## What the sandbox does

`a2amx gateway up` runs the gateway relay inside bubblewrap. The sandbox shares the host network
so it can reach Discord and the daemon's loopback address; A2AMX does not filter its network
traffic.

| The gateway can see | The gateway cannot see |
| --- | --- |
| Read and write `<root>/config/gateway.toml`, `<root>/config/secrets/discord.token`, and `<root>/state/gateway/` | `<root>/run/admin.token` |
| Read `<root>/run/link/` (the daemon address and gateway token) | The rest of the a2amx directory |
| A 1 GiB temporary `/tmp` | The a2amx directory, `/var`, `/opt`, and the rest of `/run` |
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
- The foreground sandbox does not create a new terminal session; on a kernel that still permits
  `TIOCSTI`, a compromised foreground gateway could push input into the terminal from which it was
  started. Background launches use `setsid`; start a foreground gateway from a terminal whose
  input you do not mind exposing.
- A background `gateway stop` is graceful and lets the relay finish its shutdown; stopping a
  foreground gateway is hard and can leave one message to be relayed twice after restart.
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
| `the a2amx daemon is not running in this a2amx directory; start it first` | `<root>/run/link/` or one of its required regular files is missing. | Start the daemon for this a2amx home, then run `gateway up` again. |
| `not paired; run with --pair` | No `pairing.json` exists. If it exists but is malformed, insecure, or mismatched, its own error names the bad file. | Run `a2amx gateway up --pair` interactively and redeem a pairing code. |
| `server has N members, the limit is M` | The approximate Discord member count is above `max_members`; at startup the relay exits. | Remove extra members or raise `max_members` deliberately, then restart the gateway. During a later check, wait for the count to fall; the relay pauses while it is above the limit. |
| `that code is not valid` | The lease code is wrong, expired, already used, or all pending codes were cleared after five wrong attempts. | Run `a2amx team up` at the computer and post its new code. |
| `send the code that a2amx team up printed` | The control-channel text is not a 12-character lowercase hexadecimal lease code. | Copy the exact code printed by `a2amx team up`. |
| `a2amx is not reachable right now` | The gateway was connected, but the daemon is currently unreachable. | Check the daemon and `<root>/run/link/`; the relay retries through `link/` every `reconnect`. |
| `cannot resolve /etc/resolv.conf; the gateway needs DNS` | The resolver path has another stat error, or it is a symlink whose target cannot be resolved or stat-ed or is not a regular file. A missing or non-symlink path starts without a resolver bind; targets under `/etc` or `/usr` need no extra bind. | Repair the path or target, then run `gateway up` again. |
| `secure file permissions are too broad` | A token, config, or pairing file has group or other permission bits. | Make the file a regular, owner-only `0600` file; do not replace it with a symlink. |
| `pairing not confirmed` | The interactive pairing prompt was declined or did not receive `y`/`yes`. | Run `a2amx gateway up --pair` in a terminal and confirm the paired account. |
| `pairing timed out` | No matching pairing code arrived within 10 minutes. | Run `gateway up --pair` again and post its newly printed code promptly. |
| `pairing did not complete; the gateway was not started` | Pairing declined, timed out, or the pairing sandbox exited before confirmation. | Retry `a2amx gateway up --pair`; no background gateway was started. |
| `gateway close 4013: invalid intents` or `gateway close 4014: disallowed intents` | Discord rejected the requested gateway intents. | Enable the guilds, guild messages, and Message Content intents for the bot, then pair/start again. |
| `the gateway is already running` | Another gateway holds the state-directory lock. | Run `a2amx gateway stop`, then retry. |
| `the gateway exited during startup` | The relay failed before printing its readiness line. | Read the sanitized startup tail in `<root>/log/gateway.log`, fix the cause, and retry. |
| `the gateway did not become ready within 10s` | Startup is slow or the log was replaced before readiness. | Check `gateway status` and the log; stop it if needed, then retry. |
| `the gateway holds the lock but its process N is gone` | The sandbox outer process was killed and its child is orphaned. | Run the printed `kill -TERM -- -N` recovery command, then retry. |

## Files

The gateway reads `discord.token`, `gateway.toml`, and `pairing.json` only when each is a regular
file, not a symlink, owned by the current user, with no group or other permission bits, and no
larger than 64 KiB. This `read_secure` check applies to all three files. The gateway token and
pairing writes use mode `0600`; the daemon's state files and dedup file use the same mode.

| Path | Owner and mode | Purpose |
| --- | --- | --- |
| `<root>/run/gateway.lock` | Current user, `0600` | Advisory lock for one gateway per a2amx directory; outside the sandbox bind |
| `<root>/run/gateway.pid` | Current user, `0600` | Launcher-owned pid target for `gateway stop`; outside the sandbox bind |
| `<root>/log/gateway.log` | Current user, `0600` | Background startup and relay log; opened hardened by the launcher |
| `<root>/config/secrets/discord.token` | Checked by `read_secure` above | Discord bot token entered with `gateway token` |
| `<root>/config/gateway.toml` | Checked by `read_secure` above | Gateway configuration |
| `<root>/state/gateway/pairing.json` | Checked by `read_secure` above | Paired Discord server/user IDs |
| `<root>/state/gateway/seen.json` | Current user, `0600` | Last handled Discord message ID per channel for deduplication |
| `<root>/run/link/addr` | Daemon user, `0600` | Daemon address exposed to the gateway |
| `<root>/run/link/gateway.token` | Daemon user, `0600` | Gateway-authentication token exposed to the gateway |
| `<root>/run/gateway.token` | Daemon user, `0600` | Daemon-side copy of the gateway-authentication token |
| `<root>/run/addr` | Daemon user, `0600` | Daemon address for local clients |

Do not share `admin.token`, `gateway.token`, `link/gateway.token`, or the Discord bot token. The
sandbox is designed to expose only the gateway-specific files listed above.
