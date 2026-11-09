# Spec 3dm: `a2amx gateway run`, the process that connects the daemon, the gateway core and Discord

## 0. Status

LOCKED (base: 75fd8d2, which accepts spec 3dl).

Amendment 1: section 2.5 item 5 no longer says a daemon `Response::Error` breaks the connection through `inbound` (the core maps it to a fixed line and `src/gateway.rs` stays out of scope).

Scope (files that may be edited): `src/relay.rs` and `src/relay_files.rs` (new modules), `src/lib.rs` (two `pub mod` lines), `src/cli.rs` (the `gateway` command), `src/main.rs` (dispatch, signal handling, the pairing prompt, the log filter in section 2.6), `src/client.rs` (section 2.2), `tests/relay.rs` (new), `tests/common/fake_discord.rs` (new, section 2.7), `tests/common/mod.rs` (one `mod` line), `tests/discord.rs` (the moved helpers removed). Does not touch: `src/gateway.rs`, `src/discord.rs`, `src/daemon.rs`, `src/wire.rs`, `Cargo.toml`, `Cargo.lock`, `deny.toml`, the docs, `CHANGELOG.md`, `AGENTS.md` (the architect adds the module-table rows when committing).

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: `a2amx::relay::run` against a real daemon and the fake Discord server; the `a2amx` binary for the command-line, pairing-prompt and log tests.

Third of four gateway specs after 3dk (core) and 3dl (Discord client). Next: 3dn confinement (`a2amx gateway up` starts `gateway run` inside its own sandbox); 3do the setup guide. Until 3dn, `gateway run` runs unconfined.

## 1. Why

3dk and 3dl are libraries. Nothing yet connects them to a running daemon, a bot token on disk, and an operator who has to pair a Discord account. This spec writes that program: it reads its files, pairs once with a terminal confirmation, keeps the Discord channels in step with the agents the daemon says may talk right now, carries messages both ways, and stops sending when the server has more members than the operator allowed.

Alternatives rejected:

- Bot token from an environment variable or argv: both leak; a 0600 file and a stdin command are enough.
- The daemon holding the pairing record: it never needs Discord ids; the gateway keeps the record.
- One spawned task per step: the core's calls hold `&mut Client` across awaits, and dropping such a future inside `select!` leaves the daemon connection half-way through a request. All work runs after `select!` has chosen a wake-up; only cancel-safe waits (the event receiver, interval ticks, the stop future) are `select!` arms.
- A pairing prompt with a timeout: a blocking stdin read cannot be cancelled and would hang the runtime at exit. The prompt has no timeout; the signal handlers are installed only after pairing, so Ctrl-C ends the process by default while the prompt waits.
- Making `gateway up` now: it needs the sandbox (3dn).

## 2. Changes

Claims (the consultant re-checked the anchors):

| claim | where | verified |
|---|---|---|
| the state directory is `--home` / `A2AMX_HOME` / `XDG_STATE_HOME/a2amx` / `$HOME/.local/state/a2amx`; `daemon::ensure_state_dir` creates missing directories 0700 and chmods an existing leaf to 0700 | `src/main.rs:230-245`, `src/daemon.rs:2139`, `2163-2165` | read |
| the daemon writes `gateway.token` (at each start, regenerated) and `addr` into the state directory; `Client::connect_addr(addr, token)` connects and `Client::connect(state_dir)` reads `addr` and `admin.token`; its error texts name `admin.token` | `src/daemon.rs:2208`, `2222`, `src/client.rs:46-77` | read |
| `random_hex::<N>()` is `pub` and reads `/dev/urandom` | `src/daemon.rs:2506` | read |
| `tracing_subscriber::fmt().with_env_filter(EnvFilter::from_default_env())` is installed for every command | `src/main.rs:90-93` | read |
| team and agent names are DNS-label names (`[a-z0-9][a-z0-9-]{0,62}`) | `src/messaging.rs:300` | read |
| `rustix` with the `termios` feature is a dependency; a terminal guard exists | `Cargo.toml:16`, `src/main.rs:3490` | read |
| `tracing-subscriber` has the `env-filter` and `registry` features, so a per-layer filter is possible | `Cargo.toml:22` | read |
| `Gateway::inbound` checks the author first (`gateway.rs:95`), records the id, then answers; `outbound_done` clears the held message only on `Response::Ok` (`gateway.rs:159`); a daemon `Response::Error` is returned as `Err` (`gateway.rs:67`, `164`) | `src/gateway.rs` | read |
| `Discord::ensure_category` and `ensure_text_channel` do a GET then a POST on every call; the watch does not filter the bot's own posts | `src/discord.rs:208-245`, `908-926` | read |
| `Watch::recv` is cancel-safe; the watch task keeps heartbeating while the consumer is slow | `src/discord.rs:471-489`, `789`, `tests/discord.rs:1279` | read |
| the fake Discord helpers (`http_fake`, `ws_fake`, `send_json`, `next_json`, `hello`, `discord_for_http`, `discord_for_ws`, `HttpReply`) live in `tests/discord.rs` and nothing outside it uses them; `tests/common/mod.rs` allows dead code | `tests/discord.rs:57-221`, `507-592`; `tests/common/mod.rs:4` | read |
| `http_fake` serves a fixed reply vector by index, so it cannot serve a relay that polls | `tests/discord.rs:135` | read |
| a stopped daemon makes a `Client` request return `Err`; the daemon sessions and leases do not survive a restart | `src/client.rs:89` | the developer confirms by test |
| tungstenite and ureq print the token or `Authorization` at trace level | 3dl notes | not verified; the log test must be red first (section 4) |

### 2.1 Command line

`a2amx gateway token` reads the bot token from standard input (one line of at most 64 KiB, trimmed; when standard input is a terminal the echo is turned off while reading and restored by a guard), refuses an empty one, and writes `<home>/gateway/discord.token` (directory 0700, file 0600, temp file then rename). If the process is killed by a signal mid-read the echo stays off; accepted. The token is never an argument and is never printed.

`a2amx gateway run [--pair]` runs the relay in the foreground until `SIGINT` or `SIGTERM`.

### 2.2 Files and the client (`src/relay_files.rs`, `src/client.rs`)

All under `<home>/gateway/` (created 0700).

- `config.toml`, written by the operator: `guild_id` (a TOML string of 1 to 20 digits, required), `control_category` (default `a2amx`), `control_channel` (default `control`), `max_members` (default 2: the operator and the bot), `sync_ms` (default 15000), `idle_ms` (default 1000), `member_check_ms` (default 60000), `reconnect_ms` (default 5000, the gap between daemon reconnects), `api_base` and `gateway_url` (optional, default the Discord URLs). Unknown keys are an error; every `_ms` value must be at least 1; names must match `[a-z0-9][a-z0-9-]{0,99}`.
- `discord.token`, `config.toml`, `pairing.json`: each must be a regular file, opened with `O_NOFOLLOW` and checked with `fstat` on the open descriptor, owned by the process's uid, with no group or other permission bits; otherwise the run fails naming the path (never the content). The token is trimmed; empty is an error.
- `pairing.json`: `{"guild_id": "...", "user_id": "..."}` (digit strings), written by pairing 0600 through a temp file and rename. `run` without it and without `--pair` fails with "not paired; run with --pair". A record whose `guild_id` differs from the config's is an error.
- `seen.json`: the core's dedup file (3dk).
- Every read is bounded at 64 KiB.

`src/client.rs`: `Client::connect` gains an inner `connect_with(state_dir, token_file)`; `connect` calls it with `admin.token`, a new `Client::connect_gateway(state_dir)` calls it with `gateway.token`. The error text names the file actually read. Both read `addr` and the token file on every call, so a reconnect picks up a restarted daemon's new token.

### 2.3 Entry point (`src/relay.rs`)

`pub fn load(home) -> Result<Loaded>` reads and checks the files (token, config, pairing when present) and builds nothing else. `pub async fn run(loaded: Loaded, discord: DiscordConfig, io: Io) -> Result<()>`, where `Io { stop: Pin<Box<dyn Future<Output = ()> + Send>>, pair: Option<PairIo> }` and `PairIo { announce: Box<dyn Fn(&str) + Send>, confirm: Box<dyn FnMut(&str) -> bool + Send> }`. `main.rs` builds `DiscordConfig` from the token, `api_base` and `gateway_url`, maps SIGINT and SIGTERM to `stop` (created once, `recv()` is cancel-safe), and implements `announce` as a line on standard error and `confirm` as one line read from standard input (called on a blocking thread). Tests call `run` directly with tiny `_ms` values, a oneshot as `stop` and closures for `PairIo`; the fake Discord's address comes through `DiscordConfig`.

### 2.4 Pairing (`--pair`)

After `watch()` has started (the start order in 2.5), call `announce("post this code in the control channel: <12 hex characters>")` (from `random_hex::<6>`). Wait up to 10 minutes for an event in the control channel whose text, trimmed, equals the code, from any author; other texts and channels are ignored. Then call `confirm("bind Discord account <author id>? [y/N]")`; `true` (the real prompt accepts `y` or `yes`) writes the record, anything else (and the timeout) writes nothing and fails. A new pairing replaces an existing record only after the same confirmation. Pairing runs before the `Gateway` exists. The signal handlers are installed only after pairing (section 1).

### 2.5 The run loop

Order at start: connect to the daemon (`connect_gateway`); `Discord::new`; the member check must pass or the run fails with `server has <n> members, the limit is <max>`; ensure the control category and channel; start `watch()`; pair when asked; create the `Gateway`; run one sync; enter the loop.

The loop is `loop { let wake = select! { ... }; match wake { ... } }` over: `Watch::recv` (an `Err` ends the run with the fixed error text), three intervals (`sync_ms`, `idle_ms`, `member_check_ms`, each with `MissedTickBehavior::Delay`), and `stop`. No work happens inside a `select!` branch expression. A small `Relay` struct owns the daemon client, the `Discord`, the `Gateway`, the id caches, the `paused` and `broken` flags; setup, pairing, event handling, sync, pump (with `send_pieces`) and member check are separate functions.

1. Event: when paused, drop it without a reply. When the daemon is broken or the call fails: reply `UNREACHABLE_LINE` only if the event's author is the paired user (the relay compares the ids itself, as the core does) and mark the daemon broken; a bot or other author gets nothing, so the bot never answers its own posts. Otherwise `Gateway::inbound`; when it returns a line, post it in the event's channel. A failed post is logged (without the token) and does not end the run.
2. Sync tick (not when paused): `Gateway::agents`; for each agent `ensure_category(team)`, `ensure_text_channel(category, name)` (ids cached for the process lifetime) and `bind_channel(channel, address)`; a bound channel whose agent is no longer listed is unbound (the Discord channel stays).
3. Pump tick (not when paused or broken): `Gateway::outbound` until `Idle`: `Send(out)`: post each piece in order; all succeed then `outbound_done(id, true)`, else `outbound_done(id, false)`. `NeedChannel { id, agent }`: ensure and bind the channel as in the sync step and continue; if that fails `outbound_done(id, false)`.
4. Member tick: `Discord::member_count`; above `max_members` sets `paused` (no outbound, no sync, inbound dropped, one line on standard error); at or below clears it; an `Err` keeps the last known state. The start check is strict (an `Err` fails the start). Residual risk, stated in the docs (3do): the count is approximate and can lag, and a new member can read past messages.
5. Any `Err` from a call that takes the daemon client (`agents`, `inbound`, `outbound`, `outbound_done`) marks the connection broken. A daemon `Response::Error` reaches the relay as an `Err` from `agents`, `outbound` and `outbound_done` (`gateway.rs:67`, `164`) and so also marks it broken; `inbound` turns every daemon response, errors included, into a fixed line (`gateway.rs:197-227`), so only its transport `Err` marks the connection broken. A broken connection is rebuilt with `connect_gateway` at the next tick, at most every `reconnect_ms`. Delivery is at-least-once: if a post succeeded and `outbound_done` then failed, the held message is offered again after the reconnect and may be posted twice; accepted.

### 2.6 Log filter

`main.rs` installs the `fmt` layer with the environment filter and, on the same layer, a second filter that drops every event whose target is `tungstenite`, `tokio_tungstenite`, `ureq`, `ureq_proto` or `rustls` or starts with one of them followed by `::`, for every command. A more specific `RUST_LOG` such as `tungstenite::protocol=trace` cannot override it. The relay itself logs no token, header, URL or response body; errors print the fixed `Display` text of `DiscordError`.

### 2.7 The fake Discord (`tests/common/fake_discord.rs`)

The websocket helpers and `HttpReply` move from `tests/discord.rs` (moved, not copied; its tests keep using them). New code: a stateful `FakeDiscord` serving REST by route: a channel store that assigns ids and answers the GET before the POST, a settable member count, captured POST bodies per channel, and a handle to push `MESSAGE_CREATE` frames with increasing snowflake message ids. Loopback only.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| the bot token, config and pairing files | read or swapped by another user, symlinked, loose mode, or `api_base` redirecting the token | constrain: regular file, `O_NOFOLLOW`, own uid, no group or other bits, for all three | one test per file: mode 0644, a symlink, and a foreign owner (when the test user can create one; else the mode and symlink cases only) make `run` fail naming the path and not printing the content |
| the bot token on the command line, in logs, in output | leaks | remove: stdin only; websocket, HTTP and TLS logging dropped by an unoverridable layer filter | `gateway token` takes no argument; the binary run with `RUST_LOG=trace,tungstenite::protocol=trace` against the fake Discord prints neither the token (nor its hex form) nor `Authorization` on standard output or error |
| pairing | a wrong account is bound | constrain: code posted, then a terminal confirmation of the account id | an answer of false writes no record; wrong text ignored; the record holds guild and user ids |
| an unpaired relay | acts for anyone | remove: refuses without a record | `run` without a pairing record fails |
| the pairing record names another guild | messages from the wrong server | constrain: mismatch is an error | a record with another `guild_id` fails the start |
| a member joins the private server | reads everything the agents send | constrain: member count above the limit pauses sending and sync | with the fake reporting 3 members a pending agent message is not posted and no channel is created; both resume when the count returns to 2 |
| the bot's own posts and other members' messages | a reply loop, or answers to anyone, while the daemon is down | constrain: the relay answers `UNREACHABLE_LINE` only to the paired author | with the daemon down, one paired-author event gets exactly one post; a bot-author and a stranger event get none |
| a daemon restart or crash | silent loss | constrain: reconnect, fixed line meanwhile | with the daemon stopped an event is answered `UNREACHABLE_LINE`; after it restarts in the same state directory a well-formed unknown 12-hex code in the control channel is answered `that code is not valid` |
| config file | unknown key hides a typo, a bad id reaches a URL path, a zero interval panics | constrain: unknown keys and bad values are errors | an unknown key, a non-numeric `guild_id`, and `idle_ms = 0` each fail the start |
| agents enabled with `send` | Discord messages reach them | constrain: the daemon refuses, the core answers the fixed line | a message in a send-mode agent's channel gets `that agent is not taking messages from you` and the session receives nothing |
| channels for agents with no lease | exposes agents not enabled | constrain: channels only for `GatewayAgents` entries | an agent with `remote` but no lease gets no channel |
| oversized files | memory growth | constrain: 64 KiB bound | a config file over 64 KiB is an error |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests are red first (they need the new command and module); the contract tests (the moved helpers still passing `tests/discord.rs`) pass on the old code.
2. `tests/relay.rs`: one test per row of section 3, plus the end to end: a real daemon, two confined sessions (`both` and `send`), `FakeDiscord` and a paired user (the test writes `pairing.json`): the lease code redeemed through the control channel; after it the category and the channel appear; a message from the paired author in the `both` agent's channel reaches the session as a human message; a message from the agent to `human:owner` is posted in its channel, split at 2000 characters when long, with empty `allowed_mentions`, and the daemon records it `submitted`; a message from another author is ignored. Pairing tests use `PairIo` closures; the binary tests use piped standard input and parse the code from standard error. Confined tests return early without bwrap like the others. No test sleeps over 2 s; the relay's intervals are a few milliseconds.
3. The log test is shown red first: before the filter is added, the binary under `RUST_LOG=trace` against the fake prints the token or `Authorization`; if it does not print anything even then, the developer reports it and the test searches the Identify payload hex form instead.
4. Mutation check, each restored by checksum, naming the failing test: the file-mode check removed; the symlink check removed; the pairing confirmation skipped; the guild mismatch check removed; the member check inverted; paused still posting; paused still creating channels; the log layer filter removed; `outbound_done(id, true)` after a failed post; the unlisted-agent unbind removed; the daemon reconnect removed; the author check removed from the `UNREACHABLE_LINE` reply.
5. `git diff --stat` shows only section 0 files; `scripts/prepush-fast.sh` (architect); Sonar after the push (the developer states hand counts for the loop and for each step function; the consultant's counts: loop 4 to 6, sync 9 to 11, pump 12 to 15 before splitting out `send_pieces`, member check about 5, pairing about 7).

## 5. Out of scope

`a2amx gateway up`, the sandbox and its network rules (3dn); the setup guide, firewall notes, the privacy warning and the residual member-count risk (3do); the `relay` row in `AGENTS.md` and `docs/architecture.md` (architect); more than one guild; Telegram; a systemd unit; reloading the config without a restart; acting on `max_members` with anything stronger than pausing; hiding history from a late joiner; exactly-once posting; the live Discord test (architect and owner, by hand after 3dn).
