# Permissions: the human as a messaging participant

Status: draft, not locked. This is the first part of the permissions design (F21), written
for remote human messaging (F81, roadmap 0.3.0). It decides only what that feature needs
and leaves a place for the rest of F21. Nothing here is built.

The first transport is Discord (owner decision, 2026-11-08); Telegram may follow. Everything
except the sections on the bot token, pairing and the server layout is independent of the
transport.

Terms: the **gateway** is the process that talks to Discord for the daemon. It is not the
`omp-bridge` relay, which already uses the word "bridge".

## Why this page exists

Today an authenticated caller is the admin or one session, and the daemon checks what a
session may ask for in a single list (`session_token_permitted` in `src/daemon.rs`).
Messages come from an agent or from the daemon (`Origin::Agent`, `Origin::System`). A human
who is away from the computer is neither. This page says who that human is, who may talk to
them, and what their messages may do, before any code exists.

## Decisions

### D1. A human is a third kind of sender, not a fake session

A message from the paired human has its own origin, `Origin::Human`, and its own envelope
sentence. The daemon renders the sentence; a body cannot supply it, and the existing check
that refuses the text `<a2amx-message` applies to human messages too. Only that sentence marks
a message as coming from the human. An agent's body that says "the operator told me on their
phone" is a peer's claim and counts for nothing more than a peer message.

The human's address is `human:owner`. A session name matches `[a-z0-9][a-z0-9-]{0,62}`, so no
session can be named `human:owner`, and an agent addresses the human with the existing
`send_message`. One human per daemon in 0.3.0. The form `human:<name>` leaves room for more
people, or a host suffix for cross-host use (0.4.0), without a change.

### D2. The gateway has its own role, checked fail-closed

The gateway gets a third daemon role beside admin and session, with its own token file in the
state directory (0600), never the admin token. Its requests are a short list: hand the daemon
one inbound human message, fetch pending outbound human messages, acknowledge one.

Today the check is a deny-list on sessions only: `session_request_denied` refuses a request
only for `Role::Session`, so admin and any role added beside it pass every request. Adding the
gateway role to that check would let it read and cancel messages and start sessions. The spec
must make the check exhaustive per role, with no default allow, and a test per role that every
request outside its list is refused.

No request carries an a2amx sender or origin; the daemon assigns both from the role and the
pairing record. The gateway does supply the Discord identity of a message (user id and channel),
and the daemon can trust that only as far as it trusts the gateway (see "What a compromised
gateway or bot token can do").

### D3. One eligibility check, run twice

One function decides whether a session may exchange remote messages. It runs when the daemon
accepts a message and again when it delivers one. A session is eligible only when all of these
hold:

1. remote messaging is enabled for that agent in its team entry or by its command-line option
   (off by default);
2. the team's presence lease is valid (D4);
3. the session is running;
4. it runs confined (owner decision, 2026-11-08: a message that arrives over the network could
   be forged, and a confined agent that acts on one can reach less. Confinement is a safety
   net, not a sandbox, so this limits the damage and does not rule it out);
5. its harness is not Codex while confining Codex is unbuilt (F31).

A team file can come from a repository, so the file alone grants nothing: the lease does (D4).

### D4. Presence lease

- **Starting.** `team up` needs the admin token. It lists the agents the team file enables
  ("remote messaging for: architect, developer"), then prints a short single-use, expiring code.
  The lease starts only when the paired human posts that code in the server's control channel
  and the gateway hands it to the daemon. The same mechanism pairs the human the first time,
  with a terminal confirmation of which Discord account is being bound.
- **Who can start one.** Anyone who holds the admin token can issue a code, and only the paired
  Discord account can return it. A process that can read the admin token is the same user as
  the operator, which the [philosophy](philosophy.md#what-to-expect-and-what-not) does not
  defend against.
- **Lifetime.** At most 24 hours. It belongs to the team, is kept in memory only, and ends when
  the team has no running agent left (a whole `team down`) or when the daemon stops. Stopping
  one agent does not end it. A restart always needs a new lease.
- **What persists.** The pairing record (the Discord user id and the server id) is stored, in a
  0600 file or `messages.db` under the state directory, and only the daemon writes it. The lease
  does not persist.
- **Expiry.** A message that arrives while the lease is invalid is rejected with a fixed error
  line from the bot; nothing reaches an agent, and no agent message reaches the human.
- **The code** is random, single use, compared in constant time, with a capped number of wrong
  attempts. It passes through the gateway and Discord each time, so a compromised gateway sees
  it.

### D5. What a remote message may authorize

A reply from the phone counts as the operator's direction for ordinary work: answering a design
question, adding context, choosing an option. It does not count for the actions the project
already keeps with the owner: changing the default branch, production access, handling secrets
and deleting data. For those the agent asks the operator to come back to the computer.

A2AMX cannot enforce this inside the agent. A delivered message is pasted into the composer, so
the harness treats it as ordinary user input (see
[architecture](architecture.md#identity-and-authority)); only the envelope sentence and the
operator line in the session prompt separate a phone message from typed input. "Less authority"
is a request to the model, nothing more. What actually limits damage:

- a harness permission prompt, which a phone user cannot answer (good), but which a session in a
  permission-bypass mode does not have;
- confinement, which every eligible session has (D3);
- the lease, which bounds a hijacked channel to at most 24 hours and to the agents the operator
  enabled.

### D6. Rate limits, enforced by the daemon

The gateway may drop excess traffic early, but only the daemon's limits count. Message limits
reuse the shape of the existing per-sender limit (in memory, sliding 60 s window, code
`rate_limited`):

- inbound human messages per minute, with a burst allowance because a long reply may be sent as
  several messages;
- outbound messages to the human, per agent and in total;
- queue length and body size for human messages (a Discord bot message is at most 2000
  characters, so a longer agent message is split or refused; the spec decides).

Failed lease attempts need a counter of their own: the existing limit is keyed by sender session
and only counts accepted messages. That is new code. The numbers are decided in the spec and
tested, not set here.

### D7. Delivery reports only what is known

For an agent-to-human message the states are: accepted by the daemon, sent to Discord, failed.
For a human-to-agent message: accepted, delivered to the session's channel, and the receipt the
harness gives. Nothing is reported as processed by the model. The human sees "delivered to
architect", or the reason it was rejected.

### D8. Deduplication after a reconnect

The gateway keeps the id of the last Discord message it handled in each channel and ignores
anything at or before it. Discord message ids increase with time, so one value per channel is
enough, not a table of every message. If the values live in `messages.db`, the schema changes
and the changelog says so, because the schema is public interface.

### D9. The gateway is a separate, confined process

Decided by the owner on 2026-11-08. The daemon holds the admin token and the sessions, so the
code that talks to the internet runs in its own process, which keeps the network stack and the
bot token out of the daemon. The alternative, a transport inside the daemon, is simpler but puts
network code next to the admin token. The gateway costs a role and a token file (D2), something
to launch and supervise, and its own confinement.

The gateway always runs confined, and the daemon refuses to start it if confinement cannot be
set up; there is no unconfined option. What the spec has to settle, because bubblewrap alone
does not cover it:

- The gateway needs the network to reach Discord and the daemon's local address. Bubblewrap
  either shares the host network or removes it; it cannot allow one host and refuse others. The
  gateway therefore keeps full outbound access unless the operator adds a firewall or a systemd
  unit with address limits, and the docs say so (philosophy rule 1: no claim of a sandbox).
- The existing session confinement profile is built for agents and does not restrict the
  network; the gateway needs its own, much smaller profile: no home directory, no repository, no
  harness configuration or credentials, no sockets other than its own, a state directory it can
  write, and read-only access to its own two tokens.
- Whether to run as a separate unprivileged user where that is practical, and which Linux
  capabilities to drop, are for the spec.

### D10. The Discord server

Owner decision, 2026-11-08, with the layout to be confirmed by a throwaway experiment: the
operator makes a private server that holds only the operator and the bot, with no invite link.
Each team is a category and each enabled agent is a channel in it, so the channel names the
agent and nothing needs typing. A control channel carries lease codes and the bot's own notices.

- The gateway reads the connection to Discord over an outbound websocket, which needs no public
  address. It needs the Message Content setting turned on for the bot, or message text arrives
  empty.
- A message counts only if its author is the paired user id, in the paired server. Everything
  else is dropped: other members, other servers and direct messages.
- The bot is created by the operator in Discord's developer portal. Its permissions in the
  server are limited to what the layout needs: to read and send messages and, if the bot makes
  the channels, to manage channels. Whether the operator should make the channels instead is for
  the spec.
- A server with a second member would show that member everything the agents send. The setup
  tells the operator to keep it private, and the spec evaluates whether the gateway can check
  the member count and stop sending when it is not one.

## The bot token

The Discord bot token is the strongest credential in this design and the daemon's limits do not
cover it. Whoever holds it can act as the bot in the server within the bot's permissions: send
messages that look like any agent or the daemon (a fake "reply YES to push", a fake renewal code
request), read every channel the bot can read, and if it can manage channels, delete or create
them. It can also open its own connection and read the lease code as it arrives.

- It lives in a file readable only by the gateway, outside every session's view, and never in
  argv, logs or envelopes.
- The operator resets it in the developer portal, which makes the old token useless, and `a2amx`
  has a command to store a new one.
- The human cannot tell a forged agent message from a real one by its text. This is a limit, not a
  gap to close.

## What a compromised gateway or bot token can do

The daemon cannot verify Discord itself, so it trusts the gateway's word that a message came from
the paired account. A gateway that is taken over can send messages as the human to eligible
agents and read what is sent to the human, including the lease code as it passes. The gateway
role and its confinement limit what else it can reach: no admin power, no agent sender, no
other messages. They do not stop it from speaking as the human while a lease is valid. The
lease bounds that to at most 24 hours and to the agents the operator enabled. A stolen bot token
reaches the human, not the agents, but it bypasses every daemon limit on that direction.

## Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| inbound human message | caller supplies an a2amx sender or origin | remove: the request has no such field; the daemon assigns both | a gateway request that names a sender is rejected or ignored; the stored sender is `human:owner` |
| gateway role | uses admin or session requests | constrain: own exhaustive, fail-closed allowlist (D2) | every request outside the gateway list, sent with the gateway token, is refused, including list and cancel messages |
| Discord message | from another member, another server or a direct message | constrain: the gateway drops it; the daemon check on the paired id catches gateway bugs only, not a compromised gateway | a message from another user id is not delivered and does not start a lease |
| Discord message | replay or redelivery after a reconnect | constrain: last handled id per channel (D8) | the same message twice is delivered once, also after a restart |
| inbound flood | fills a queue or the quota | constrain: daemon limits | past the limit: `rate_limited`, no delivery |
| outbound flood | an agent buzzes the phone | constrain: per-agent and global limits | the same, outbound |
| lease code | guessing, reuse, use after expiry | constrain: random, single use, expiring, capped attempts (D4) | second use, expired code and the attempt cap each fail |
| stolen or taken-over account | attacker messages agents | constrain: lease expiry, rejection after expiry, lower authority (D5); a dedicated account with two-factor authentication in the setup guide | a message after expiry is rejected |
| message body | text containing an envelope tag, to forge a peer or system message | constrain: existing body validation, applied to human messages | a body with `<a2amx-message` is rejected |
| agent body | claims "the operator said so" | constrain: only the daemon-rendered sentence marks a human message (D1) | a peer body quoting the human sentence is delivered with the agent sentence |
| team file from a repository | enables remote messaging | constrain: config grants nothing; `team up` lists the enabled agents and the lease returns from the paired account (D4) | a session enabled in the file with no lease cannot exchange messages |
| session name | an agent takes the human's address | remove: the name grammar forbids `:` | creating a session named `human:owner` fails |
| restart or exit | pending messages reach a different session | constrain: the check runs again at delivery | a message for a session that exited is not delivered |
| channel to agent mapping | a message in one agent's channel reaches another agent | constrain: the daemon takes the recipient from the channel the gateway names and rechecks eligibility for that agent | a message in agent A's channel is never delivered to agent B |
| pairing record | an unconfined process repoints it to its own account | constrain: written only by the daemon, 0600 in the state directory, hidden from confined sessions; an unconfined same-user process is out of scope (philosophy) | a confined session cannot read or write it |
| gateway token and bot token files | read by an agent | constrain: 0600, gateway-only, outside session views | a confined session cannot read either |
| bot token | holder speaks to the human as any agent, reads the channels | cannot be removed: resettable in the developer portal; permissions kept small; documented | none; a documentation check |
| second member in the server | reads everything the agents send | constrain: private server, no invite link, member check if the spec can do it | the gateway stops sending when the member check fails (if built) |
| Discord outage | messages lost without a sign | constrain: bounded retries, status reported | a failed send ends in `failed`, never a silent drop |
| content sent to Discord | secrets or code leave the machine | cannot be removed: Discord text is not end-to-end encrypted. Documented, agent guidance says what not to send, body size bounded | none; a documentation check |

## Out of scope

Anything else in F21: who may reset, move, manage or launch sessions, and the cross-host rules.
Those keep today's rules until their milestone. Direct messages with agents, attachments, other
transports, terminal output and any remote control are not part of this feature.

## Open questions for the owner

Decided by the owner on 2026-11-08:

- Remote messaging is for confined sessions only (D3). This is a new refusal beside the short
  list in the [philosophy](philosophy.md); the spec adds it there.
- After the lease expires no agent message passes in either direction. The bot answers a message
  from the phone with a fixed error line (D4).
- The lease stays valid for as long as the team has any running agent; a partial `team down`
  does not end it (D4).
- The gateway is a separate, always confined process (D9).
- The first transport is Discord, with the layout in D10 as the working design.

Still open:

1. **Whether the layout in D10 works as hoped.** A category per team and a channel per agent
   fits the owner's four-team, ten-agent scenario. Not yet checked against Discord: the bot
   permissions that creating channels needs, the Message Content setting for a small private
   bot, sending rate limits, and how it looks on a phone. A throwaway experiment against a test
   server answers these before the spec.

## Next steps

The throwaway experiment, then specs in this order: the core (origin, fail-closed roles,
eligibility, lease, limits, with a test client in place of the gateway), the Discord gateway and
its confinement, and the setup flow and documentation.
