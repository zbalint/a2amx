# Spec 3dl: A Discord client that gives the gateway core its events and its channels

## 0. Status

LOCKED (consultant review applied; base 1352447; amendment 1: `reconnect_max`, handshake timeout, Identify-only gap, 4007/4009 grow the backoff, stable reconnect waits the base, extra tests).

Scope (files that may be edited): `src/discord.rs` (new), `src/lib.rs` (one `pub mod discord;` line), `Cargo.toml` and `Cargo.lock` (the dependencies in section 2.4), `deny.toml` (licenses the new dependency tree needs, section 2.4), `tests/discord.rs` (new). Does not touch: `src/gateway.rs`, `src/daemon.rs`, `src/wire.rs`, `src/main.rs`, `src/cli.rs`, `AGENTS.md`, the docs, `CHANGELOG.md`.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: `a2amx::discord` against a fake Discord server that the tests start on a loopback port; no test reaches the internet.

Second of four gateway specs after 3dk (the core). Next: 3dm `a2amx gateway up`, its config, the bot token command and pairing; 3dn confinement and launch; 3do the setup guide.

## 1. Why

3dk left the chat side to the caller. This spec writes the Discord side as a library, so the next spec only has to wire the two together: open a gateway (websocket) connection and turn each message in the one private server into a `gateway::Event`, create the category and a text channel per agent when they are missing, and post text to a channel without letting agent text ping anyone.

Owner decisions (2026-11-08/09): the bot creates the channels; the client is written here, not taken from a Discord library; the HTTPS and websocket libraries are `ureq` and `tokio-tungstenite`, both with rustls.

Alternatives rejected:

- `serenity` or `twilight`: large dependency trees to trust for a gateway that makes a handful of calls (owner decision).
- Hand-written HTTP over rustls: we would own HTTP/1.1 parsing, chunked replies and header handling; `ureq` already does it (owner decision).
- Tracking Discord's per-route rate-limit buckets: the gateway sends a few messages a minute; a bounded retry on `429` is enough.
- One long `next_event` future on the connection: cancelling it in a `select!` could drop a half-read frame. A background task owns the socket and the caller reads from a channel, which is cancel-safe.
- Sharding, compression (`zlib-stream`), slash commands, voice, attachments, edits and deletes: not needed for one bot in one server.

## 2. Changes

Claims (the consultant re-checks):

| claim | where | verified |
|---|---|---|
| `gateway::Event { channel, message_id: u64, author, text }` is what the core consumes; ids are decimal strings in Discord JSON and parse as `u64` | `src/gateway.rs:14-19` | read |
| Gateway v10 URL `wss://gateway.discord.gg/?v=10&encoding=json`; resume uses `resume_gateway_url` with the same query | https://docs.discord.com/developers/events/gateway | read (fetched 2026-11-09) |
| op codes: 0 dispatch, 1 heartbeat, 2 identify, 6 resume, 7 reconnect, 9 invalid session (`d` bool), 10 hello (`d.heartbeat_interval` ms), 11 heartbeat ACK | same | read |
| Identify needs `token`, `intents`, `properties` (`os`, `browser`, `device`); intents GUILDS `1`, GUILD_MESSAGES `512`, MESSAGE_CONTENT `32768` (privileged); without MESSAGE_CONTENT `content` is empty | same | read |
| first heartbeat after `heartbeat_interval * jitter` (jitter 0 to 1), then each interval, `d` = last sequence `s` or `null`; no ACK before the next heartbeat means a zombie: close with a code other than 1000/1001 and reconnect with resume | same | read |
| Ready carries `session_id` and `resume_gateway_url`; Resume `d` is `{token, session_id, seq}` | same | read |
| close codes that must not reconnect: 4004 authentication failed, 4010 invalid shard, 4011 sharding required, 4012 invalid API version, 4013 invalid intents, 4014 disallowed intents; others (4000-4003, 4005, 4007-4009) may | https://docs.discord.com/developers/topics/opcodes-and-status-codes | read |
| `MESSAGE_CREATE` is a message object (`id`, `channel_id`, `author`, `content`, `type`) plus `guild_id` (absent for DMs and ephemeral); the user object field `bot` is not defined on the pages read | https://docs.discord.com/developers/events/gateway-events, .../resources/message | read; `author.bot` unverified, so not used |
| Create Message: `POST /channels/{id}/messages`, `content` up to 2000 characters, `allowed_mentions: {"parse": []}` disables all pings | https://docs.discord.com/developers/resources/message | read |
| Create Guild Channel: `POST /guilds/{id}/channels` with `name` (1-100), `type`, `parent_id`; needs MANAGE_CHANNELS. Types: `0` guild text, `4` category. Get Guild Channels: `GET /guilds/{id}/channels` returns channel objects with `id`, `type`, `name`, `parent_id` | .../resources/guild, .../resources/channel | read; the response field list is from the channel object page |
| 429 body: `retry_after` (seconds, float), `global`; header `Retry-After`; global limit 50 requests/s; 401 and 403 count as invalid requests | https://docs.discord.com/developers/topics/rate-limits | read |
| the channel name normalization Discord applies to text channel names is not documented | .../resources/channel | read: not stated; the client therefore creates names already in `[a-z0-9-]` and matches them exactly |
| no HTTP, TLS or websocket crate is a dependency today | `Cargo.toml` `[dependencies]` | read |
| `deny.toml` allows exactly `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `MIT`, `Unicode-3.0`, `Unlicense`, `Zlib`, denies yanked, unmaintained crates ("all") and unknown registries; `scripts/prepush-fast.sh` and CI run it | `deny.toml`, `AGENTS.md` | read |
| the repository has no Discord code | `rg -i discord src tests` | the developer re-runs it |

### 2.1 Types

```rust
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: String,
    pub api_base: String,         // default "https://discord.com/api/v10"
    pub gateway_url: String,      // default "wss://gateway.discord.gg"
    pub reconnect_base: Duration, // default 1 s
    pub reconnect_max: Duration,  // default 30 s, cap on one reconnect wait
    pub queue: usize,             // default 1024
    pub request_timeout: Duration,  // default 30 s, whole REST request
    pub max_retry_wait: Duration,   // default 30 s, cap on one 429 wait
    pub identify_gap: Duration,     // default 5 s, least time between two Identifies
    pub stable_after: Duration,     // default 60 s, a connection this old resets the backoff
}
impl DiscordConfig { pub fn new(token: String, guild_id: String) -> Self; }
// Debug is implemented by hand and prints the token as "<redacted>".

pub struct Discord { /* config, a ureq agent */ }
impl Discord {
    pub fn new(config: DiscordConfig) -> Self;
    pub async fn ensure_category(&self, name: &str) -> Result<String, DiscordError>;
    pub async fn ensure_text_channel(&self, parent_id: &str, name: &str) -> Result<String, DiscordError>;
    pub async fn send(&self, channel_id: &str, text: &str) -> Result<(), DiscordError>;
    pub async fn list_channels(&self) -> Result<Vec<ChannelInfo>, DiscordError>;
    pub async fn member_count(&self) -> Result<u64, DiscordError>;
    pub fn watch(&self) -> Watch;
}

pub struct ChannelInfo { pub id: String, pub kind: u8, pub name: String, pub parent_id: Option<String> }

/// What a caller branches on: 404 (a channel was deleted by hand), 401/403 (bad token, bot not in
/// the guild), a refused input, a transport failure. Display text is fixed; it never contains the
/// token, a URL, a header or a response body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordError { Status(u16), Transport(&'static str), Invalid(&'static str) }
// implements Display and std::error::Error

pub struct Watch { /* receiver, task handle */ }
impl Watch { pub async fn recv(&mut self) -> Result<crate::gateway::Event, Fatal>; }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fatal(pub String); // implements Display and std::error::Error
```

The four durations and `api_base` / `gateway_url` exist so the tests can use tiny values and a loopback fake; 3dm decides what an operator may set. The Debug hazard below applies to the token in every form.

### 2.2 REST (`ureq`, run with `spawn_blocking`)

The `ureq` agent is built with default features off and `rustls` on, `http_status_as_error(false)` (so a `429` can be read for its `retry_after` and `Retry-After`), `max_redirects(0)` and `timeout_global = request_timeout` (ureq's own timeouts default to none, so a server that never answers would block a thread forever). Every request carries `Authorization: Bot <token>`, a `User-Agent` of the form `DiscordBot (https://github.com/zbalint/a2amx, <crate version>)` and, for bodies, `Content-Type: application/json`. A response body is read with a 1 MiB limit; a larger one is an error. An error is a `DiscordError`: `Status(code)` for a non-2xx answer, `Transport(kind)` with a fixed name per `ureq` error variant (the `ureq` error's own Display is never used, some variants carry the URI), `Invalid(what)` for a locally refused input; none of them contains the token, a header, a URL or a body.

- `ensure_category(name)`: `GET /guilds/{guild}/channels`; the first channel with `type == 4` and `name == name` wins and its `id` is returned; otherwise `POST` with `{"name": name, "type": 4}` and return the new `id`.
- `ensure_text_channel(parent_id, name)`: the same with `type == 0`, `name`, and `parent_id == parent_id`; create with `{"name": name, "type": 0, "parent_id": parent_id}`. `name` must match `[a-z0-9][a-z0-9-]{0,99}` (an agent name always does) or the call fails before any request.
- `list_channels()`: `GET /guilds/{guild}/channels`, every entry as `ChannelInfo` (`kind` is the `type` number; entries with a missing or odd field are skipped). `member_count()`: `GET /guilds/{guild}?with_counts=true`, the integer field `approximate_member_count`.
- `send(channel_id, text)`: refuse locally (no request) when `text` is empty or longer than `crate::gateway::MAX_CHAT_CHARS` characters; otherwise `POST /channels/{id}/messages` with `{"content": text, "allowed_mentions": {"parse": []}}`.
- `channel_id` and `guild_id` must be 1 to 20 ASCII digits or the call fails before any request (they are placed in a URL path).
- `429`: wait `min(retry_after, max_retry_wait)` (from the JSON body, else `Retry-After`, else 1 s) and retry, at most 3 retries in all, then fail with `Status(429)`. Any other non-2xx status fails at once with `Status(code)`. `5xx` and transport errors are not retried (the caller decides).

### 2.3 The gateway connection (`tokio-tungstenite`)

`watch()` spawns one task that owns the socket and sends results into a bounded channel of `queue` entries. `Watch::recv` is cancel-safe and returns `Err(Fatal)` once the task stopped on a fatal close code (or, if the task ended in any other way, `Fatal("gateway task ended")`), and the same `Err` on every later call. Dropping `Watch` aborts the task.

The task, in a loop:

1. The TCP connect, the websocket handshake and the Hello must all complete within `request_timeout`; otherwise the attempt ends as a transient failure (a server that accepts and stays silent must not stall the task before heartbeating starts). Connect to `gateway_url` or, after a resumable drop, to the stored `resume_gateway_url`; the query `v=10&encoding=json` is added (`/?v=10&encoding=json` when the URL has no path). A `resume_gateway_url` is used only when its scheme equals that of `gateway_url` and its host equals the host of `gateway_url` or ends with `.discord.gg`; otherwise `gateway_url` is used (the token is sent to that host). The accepted websocket message and frame size is 1 MiB; a larger frame ends the connection (treated as a transient drop).
2. Receive Hello; start the heartbeat timer (first beat after `heartbeat_interval * jitter` with `jitter` in `[0, 1)` from the clock's sub-second part, then every interval; the interval is taken from Hello and may be a few milliseconds in tests). A beat carries the last sequence number or `null`. If a beat is due and the previous one was not acknowledged, close the socket with code 4000 and go to step 1 with resume. Whenever the client ends a connection it intends to resume (zombie, `op 7`, an oversized frame, a full queue below) it closes with 4000 or drops the TCP stream; it never sends 1000 or 1001, which would invalidate the session.
3. Send Identify (`token`, `intents = 33281`, `properties` `{os: "linux", browser: "a2amx", device: "a2amx"}`) when there is no session, or Resume (`{token, session_id, seq}`) when there is one.
4. Handle frames. `op 0`: record `s` as the sequence; `READY` stores `session_id` and `resume_gateway_url`; `MESSAGE_CREATE` is turned into an `Event` (below). `op 1`: send a heartbeat now. `op 7`: end the connection and resume. `op 9`: if `d` is `true` resume, else clear the session and Identify again, never sooner than `identify_gap` after the previous Identify (the gap applies to Identify only, never to Resume, and only an Identify sets the time). `op 11`: mark acknowledged. Any other op, unknown event name, malformed JSON or non-text frame is ignored without error.
5. A close code of 4004, 4010, 4011, 4012, 4013 or 4014 stops the task with a `Fatal` whose text names the code and its meaning (never the token). Any other close, a transport error or a stream end is transient: wait `reconnect_base * 2^n` (n from 0, capped at `reconnect_max`) and go to step 1. `n` grows by one after every connection that ended and is reset to 0 only when the connection had been up for `stable_after` (not on `READY`: Discord resets the token after 1000 Identifies in 24 h, so an invalidating server must not be answered with fast Identifies). A close with 4007 or 4009 also clears the session and grows `n` like any other ending (it never resets it). The wait after a connection that was up for `stable_after` is the base wait (`n` is 0), not the wait of the previous failure. `RESUMED` is the dispatch that confirms a resume.

`MESSAGE_CREATE` becomes `Event { channel: d.channel_id, message_id: d.id parsed as u64, author: d.author.id, text: d.content }` only when `d.guild_id == config.guild_id`; a message without `guild_id` (a direct message), from another guild, or with a field that is missing or not a string or a `u64` is dropped. No event is filtered on `author.bot` (the core checks the author). Reading never pauses, because unread frames include the heartbeat ACKs (a paused reader would zombie itself). Events that the channel cannot take yet wait in a local queue of at most `queue` entries, each with its sequence number `s`. Two sequence numbers are kept: the last seen `s` (used in heartbeats) and the resume sequence, which is the `s` of the last dispatch that is not waiting (an event moves it when it is sent into the channel; a dispatch that is not a message moves it only while nothing waits). When the local queue is full and one more event arrives, the connection is ended with code 4000, the local queue is cleared, and the connection resumes from the resume sequence, so Discord replays what was not yet handed over (nothing is lost and nothing is handed over twice).

Structure (for the complexity limit): one function per connection that runs the `select!` loop (heartbeat tick, socket read, send of the first waiting event) and returns `Outcome { Fatal(String), Resume, Reidentify, Transient }`; frame handling split into an op dispatch and an event-name dispatch; the close-code decision is a pure function `classify_close(code) -> Fatal | Transient | TransientFresh`; the outer loop only matches `Outcome` and applies the backoff.

### 2.4 Dependencies

`Cargo.toml`: `ureq` with `default-features = false, features = ["rustls"]` (no gzip, no OpenSSL), `tokio-tungstenite` with `rustls-tls-webpki-roots`, `futures-util` for the stream and sink traits, and `rustls` directly with `default-features = false` and the `ring`, `std` and `tls12` features. The websocket connection is made with an explicit `Connector::Rustls` whose `ClientConfig` is built with the `ring` provider (`builder_with_provider`), because `tokio-tungstenite` otherwise builds its config from rustls's compiled-in provider and panics at the first `wss` connect if a dependency ever enables a second provider; a test without network builds that config. Use the newest release of each that is stable (no `-dev`, `-alpha`, `-rc`); `Cargo.lock` pins the rest. The `ring` crypto provider is preferred over `aws-lc-rs` (no CMake). The developer reports the exact versions and the number of new crates in the lock file.

`deny.toml`: add to `allow`, one line each with a comment naming the crate that needs it, only the licenses `cargo deny check` reports as missing, and only if they are permissive (expected: `ISC`, `CDLA-Permissive-2.0`, `BSD-3-Clause`; `OpenSSL` is not expected). A copyleft or non-commercial license, a yanked or unmaintained crate or an advisory is `BLOCKED`, not a new `ignore` entry. Nothing else in `deny.toml` changes.

Amendment 1 (tester pass and architect review): the tests also pin: a client-initiated heartbeat close carries code 4000 (the fake records the close code); the reconnect cap through `reconnect_max` (tiny values in the test); a `MESSAGE_CREATE` whose `id` is not numeric yields no event while the next valid one does; `ensure_text_channel` refuses an uppercase name without a request; a send of exactly 2000 characters succeeds; after a close with 4009 and after one with 4007 the next connection Identifies (does not Resume); the slow-consumer test asserts the Resume `seq` equals the last handed-over `s`; repeated 4009 closes show growing gaps; an invalid `guild_id` in the config makes every guild call fail without a request. The `webpki-roots` crate is a direct dependency (needed to build the explicit rustls connector), a gap in section 2.4 found by the developer.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| the bot token in memory, `Debug`, logs and errors | token leaks to a log, error text or a chat | constrain: redacted `Debug`, fixed error text, no logging of config | `format!("{:?}", config)` lacks the token; a `401` yields `Status(401)` whose Display lacks the token and the body |
| a `resume_gateway_url` that names another host | the token is sent to an attacker's host | constrain: scheme and host check, fall back to `gateway_url` | a READY with a `resume_gateway_url` for another host is not connected to; the next connection goes to the configured URL |
| a REST server that never answers | a blocked thread, a send that never returns | constrain: `request_timeout` | a fake that accepts and stays silent makes `send` return `Transport("timeout")` within the configured timeout (set to about 100 ms in the test) |
| a server that invalidates sessions in a loop | a fast Identify loop burns the 1000 per day and the token is reset | constrain: `identify_gap`, backoff that resets only after `stable_after` | two `op 9` with `d:false` in a row: the gaps between the three Identifies are at least `identify_gap` and the second is longer than the first |
| a message from another server or a direct message | reaches the core | constrain: only `guild_id == config.guild_id` | a `MESSAGE_CREATE` without `guild_id`, and one with another `guild_id`, yield no event; a later one in the right guild does |
| agent text with `@everyone` or role mentions | pings people | constrain: `allowed_mentions: {"parse": []}` on every send | the fake server sees `allowed_mentions` exactly `{"parse":[]}` on a send containing `@everyone` |
| a text longer than the chat limit | refused by Discord, or truncated | constrain: refused locally | a 2001-character send and an empty send make no request and fail |
| a hostile or broken server answer: huge body, huge frame, malformed JSON, unknown ops | memory growth, a panic, a stuck task | constrain: 1 MiB limits, unknown input ignored | a 2 MiB REST body that is a valid channel list (with the wanted channel and padding) fails; a 2 MiB frame and a malformed frame do not stop later events from arriving |
| ids placed in a URL path | path injection | constrain: digits only; names `[a-z0-9-]` | `send("1/../2", ..)`, `ensure_text_channel("x y", ..)` and a bad name make no request |
| `429` loops | hammering, a hang | constrain: at most 3 retries, wait capped at `max_retry_wait` | a fake answering `429` four times ends in `Status(429)` after 3 retries; a fake answering `retry_after` 10 with `max_retry_wait` 50 ms waits about 50 ms, not 10 s; the wait value is read from the body (a body of 0.05 is waited, not the 1 s fallback) |
| fatal close code | an endless reconnect loop with a bad token | constrain: stop and report | closing with 4004 yields `Err(Fatal)` and the fake counts one connection only; the second `recv` is `Err` too (also, a `Watch` whose task ended otherwise returns `Fatal("gateway task ended")`) |
| a server that accepts the connection but never sends Hello | the task stalls with no heartbeat and no reconnect | constrain: handshake and Hello inside `request_timeout` | a fake that accepts and stays silent is dropped after about `request_timeout` (set to about 100 ms) and a second connection arrives |
| a connection that goes silent | a stuck gateway that never delivers | constrain: no ACK means reconnect with resume | a fake that never acknowledges a heartbeat is reconnected, and the Resume carries the stored `session_id` and `seq` |
| a slow consumer | the connection becomes a zombie, or events are lost or repeated | constrain: reading never pauses; a full local queue ends the connection with 4000 and resumes from the resume sequence | with `queue = 2`, the consumer idle for several heartbeat intervals and five events (`s` 1 to 5) sent: every heartbeat is acknowledged and no close is caused by a missing ACK; after the consumer starts reading, all five events arrive exactly once, in order (the fake replays dispatches with `s` above the Resume `seq`) |
| invalid session | wrong resume loops | constrain: `op 9` false clears the session | after `op 9` with `d: false` the next connection sends Identify, not Resume |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass, and `cargo deny check` passes (the architect also runs `scripts/prepush-fast.sh`). New tests red first (they need the new module).
2. `tests/discord.rs`: one test per attack row above, plus tests for: `ensure_category` and `ensure_text_channel` find an existing channel without a create request and create a missing one with the exact JSON; `send` posts exactly `content` and `allowed_mentions` and the `Authorization: Bot` and `User-Agent` headers; a successful Identify has `intents` 33281 and the properties; the first heartbeat has `d` null and a heartbeat after a dispatch with `s` N has `d` N; `list_channels` and `member_count` parse the fake's answers; the Rustls `ClientConfig` builds without network; a `MESSAGE_CREATE` in the right guild becomes the right `Event`; `READY` followed by a drop is followed by a Resume with the stored session and the last sequence; an `op 7` reconnects with Resume; the defaults of `DiscordConfig::new` are the real Discord URLs, a 1 s `reconnect_base` and 1024 (no connection is made). The fake server (HTTP and websocket on `127.0.0.1:0`, written in the test file) records requests and frames; timings in the tests use heartbeat intervals and `retry_after` values of a few tens of milliseconds, and no test sleeps longer than 2 s.
3. Mutation check, each restored by checksum, naming the failing test: the guild filter removed; `allowed_mentions` removed; the 2000-character check removed; the fatal-code list shortened by 4004; Resume replaced by Identify on a transient drop; `op 9` false resuming; the heartbeat sequence always `null`; the token left in the `Debug` output; the response-size limit removed; the 429 retry count unbounded; the 429 wait uncapped; `http_status_as_error(false)` removed (the body-read wait test fails); the request timeout removed; the `resume_gateway_url` check removed; backoff reset on `READY`; the socket read paused while the local queue is full (the ACK/zombie assertion fails).
4. `git diff --stat` shows only section 0 files; the report lists the new dependency versions, the number of new lock entries, the licenses added to `deny.toml` and `cargo tree --duplicates` is not required. Sonar after the push (the developer states a hand count for the connection loop function and keeps it split into connect, handshake, frame handling and backoff functions).

## 5. Out of scope

`a2amx gateway up`, reading config or the token from files, who may set `api_base` and `gateway_url`, pairing, the control channel, acting on the member count (this spec only reads it), a "connected" signal for readiness (3dm decides if it needs one), keeping the token out of logs (tungstenite logs frames, Identify included, at trace level and ureq's trace level is not redacted: the gateway process of 3dm must filter `tungstenite`, `tokio_tungstenite` and `ureq` out of its log output, and the module itself never initializes or configures logging), the confinement allowlist (it needs `discord.com` and `*.discord.gg`), channel permission overwrites (the private server's `@everyone` rights are set up by the operator, spec 3do), deleting or renaming channels, rebuilding the channel-to-agent bindings after a restart (3dm), sharding, compression, intents beyond the three, interactions, attachments, message edits, Discord's per-route rate-limit buckets, proxies, and a live test against the real Discord (the architect and owner do that by hand after 3dm).

## Review record

Models: architect and spec author Claude Sonnet 5.5; consultant (pre-lock review) model not reported; developer openai-codex/gpt-5.6-luna (planning and working), advisor slot model not exposed; tester reported GLM (zai) under OMP.

BLOCKED/amend cycles: 1 (amendment 1, written from the tester and architect findings, not from a developer BLOCKED). Fix rounds after amendment: 3.

| found at | by | problem | resolution |
|---|---|---|---|
| draft review | consultant | 4 high, 7 medium and 5 low findings on the first draft (reconnect and resume rules, rate limit handling, limits, test gaps) | spec amended before lock |
| draft review | architect | `webpki-roots` is a direct dependency the spec did not list | noted in amendment 1 |
| tester pass | tester | a transient reconnect reset the backoff, so repeated 4007/4009 would not grow the wait; stale exponent after a stable connection | amendment 1: 4007/4009 clear the session and grow the backoff; a stable connection waits the base time |
| architect review | architect | no timeout around TCP connect, TLS handshake and Hello | amendment 1: all within `request_timeout`; test with a silent server |
| architect review | architect | `identify_gap` also delayed a Resume | amendment 1: the gap applies to Identify only |
| implementation | developer | the invalid `guild_id` case had no test | test added |
| implementation | developer's advisor | suggested resetting the backoff on READY | overruled: Discord bans repeated Identifies, READY is not proof of a stable link; mutant now killed by a test |
| tester pass (rebaseline) | tester | two surviving mutants: Resume subject to `identify_gap`, and the Resume sequence after a queue hand-over | two tests added, mutants killed |
| architect review | architect | a surviving mutant: the stored Resume sequence lost after a filtered or non-message dispatch | two tests added (lines 777 and 808 mutants killed) |
| architect review | architect | the first `cargo test` run overlapped a developer edit of the test file | rerun on the final snapshot |
