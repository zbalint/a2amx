# Spec 3dp: attach mode and owner presence in the daemon (F83, first half)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/wire.rs`, `src/messaging.rs`, `src/daemon.rs`, `src/client.rs`, `src/mcp.rs`, `src/cli.rs`, `src/main.rs` (only the `daemon start` flag and its pass-through into `Limits`), `tests/wire.rs`, `tests/prop_wire.rs`, `tests/presence.rs` (new), and, for mechanical compile fixes only (`..` added to patterns and struct literals, `viewer: false` added to `Request::Attach` literals, the new field or `..` added to `Limits` literals), `tests/broker.rs`, `tests/daemon.rs`, `tests/human_inbound.rs`, `tests/attach_cli.rs`, `tests/teams.rs`, `tests/quota.rs`, `tests/mcp.rs`; `tests/mcp.rs` also changes the two description strings it pins (lines 225 and 234). Does not touch: `src/session.rs` (the mode lives on `AttachmentGuard`), the attach client (`src/main.rs` outside the flag, `src/status.rs`, `src/prefix.rs`; that is spec 3dq), `src/gateway.rs`, `src/relay.rs`, `src/discord.rs`, docs, `CHANGELOG.md`, `AGENTS.md`, Cargo files.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: the daemon's request/response API through `Client` against a real daemon (`tests/common`), the frame encoder and decoder in `tests/wire.rs`, the MCP tool output.

First of two F83 specs. 3dq (the attach client: viewer mode, the toggle, the idle timer, the overlay, the status marker) follows and depends on this wire change.

## 1. Why

Remote messaging should not reach the phone while the owner sits at the computer: it burns an agent's tokens on crafting a message nobody needs, and the owner can read the terminal. The daemon needs to know whether the owner is present, and the agents need a clear signal. Owner decisions (2026-11-09, recorded in `docs/backlog.md` F83):

- Presence is per owner, not per agent. The owner is present exactly while an interactive attach exists. A detached session, a viewer attach, and being away from the terminal all mean away.
- A viewer attach is a client state: the screen stays live, but it counts as neither attached-for-input nor presence, and it does not resize the session.
- While present, an agent's send to `human:owner` is refused with a clear error so the agent writes in the terminal instead. There is no hold-and-release queue; a message queued before the owner came back is still sent.
- Presence turns to away 60 seconds (configurable) after the last interactive attach ends or turns viewer. Then the daemon sends one short fixed line to each agent that has remote messaging on. Nothing is sent when the owner returns.
- The owner may always write to the agents from the phone, whatever the mode.

Alternatives rejected: a hold-and-release queue (owner decision: no queue); presence by terminal activity or a heartbeat from the client (an attach already exists and the viewer state is explicit); per-agent presence (owner decision: per owner); pushing a notice when the owner returns (owner decision: nothing is pushed, the agent finds out from `list_agents` or from the refused send).

## 2. Changes

### 2.1 Wire (`src/wire.rs`)

- `Request::Attach` gets `viewer: bool` with `#[serde(default, skip_serializing_if = "is_default")]`. An old client omits it and attaches interactive; an old daemon ignores an unknown field, so a new client attaching as viewer to an old daemon attaches as interactive (accepted: client and daemon ship together).
- `ClientFrame` gets `Mode { viewer: bool }`: tag `0x07`, a one-byte body that is `0x00` (interactive) or `0x01` (viewer); any other body length or value is a decode error like the existing frames. `encode` and `decode` are extended; the frame order of the existing tags is unchanged.
- `Response::Agents { agents }` gets `owner: Option<String>` (`"present"` or `"away"`) with `#[serde(default, skip_serializing_if = "Option::is_none")]`.

### 2.1a Client API (`src/client.rs`)

`Client::attach` and `Client::attach_status` keep their signatures and send `viewer: false`. A new public `Client::attach_viewer(self, session, force, cols, rows, top_rows)` is the status-bearing attach with `viewer: true` (the private `attach_with_status` gets a `viewer: bool` parameter). A viewer attach sends no `Resize`; `cols`/`rows` only shape the render. 3dq uses `attach_viewer`.

### 2.2 Presence (`src/messaging.rs`, `src/daemon.rs`)

A pure type `Presence` in `src/messaging.rs` (it holds no daemon internals and takes every time as an explicit `Instant`, so its tests use literal instants and no sleeps), owned by `Runtime` behind a std mutex (`AttachmentGuard::drop` is synchronous):

- state: the number of interactive attachments, the `Instant` the count last fell to zero or the last interactive attachment turned viewer (`last_interactive`), and the last announced value for the away notice.
- `attach_started(now, viewer: bool)`, `mode_changed(now, viewer: bool)`, `attach_ended(now, was_viewer: bool)` update the count and `last_interactive`.
- `present(now) -> bool`: the count is above zero, or `now < last_interactive + away_secs`. With no interactive attach ever, it is away. `away_secs == 0` is accepted: away the moment the count is zero (a session switch in the client detaches and re-attaches, so with 0 a poll tick inside that gap sends a notice; documented, not prevented).
- `poll(now) -> bool` returns `true` once on a present-to-away transition (it compares `present(now)` with the last announced value). `list_agents` and the refused send call `present(Instant::now())` directly and never read the last announced value, so they cannot disagree with each other.
- `Limits` gets `away_secs: u64`, default 60. `a2amx daemon start` gets `--owner-away-secs <SECS>` (env `A2AMX_OWNER_AWAY_SECS`, default 60) that fills it; today `Limits::default()` is used at `src/main.rs:386`. `Limits` literals without `..` in `tests/broker.rs` (three) get the new field or `..`.

**Where the count is driven (daemon, `attach`).** `AttachmentGuard` (`src/daemon.rs:3362`) gets two fields, `viewer: bool` and `counted: bool` (the guard is a local of `attach()` and the frame loop runs on the same task, so no atomic is needed; `AttachmentSlot` in `src/session.rs` is not changed). `attach_started` is called right after the slot-installing block succeeds (the `match snapshot` at `src/daemon.rs:3524`, where `exited` is known), not at the reservation (`:3492`), and only when `exited` is `None`; it sets `counted = true`. A pending attach that is taken over or fails never counts. An attach to an already exited session still succeeds as today (final render, then `Exit`; `docs/specs/spec-1-terminal-core.md:323-324`, `tests/attach_cli.rs:623-630`) and does not count, because `counted` stays false. The guard is dropped as today. `AttachmentGuard::drop` calls `attach_ended(.., guard.viewer)` only if `counted`. The `Mode` arm updates `guard.viewer` and calls `mode_changed`. A take-over by `--force` decrements the old attachment once (its guard drops) and counts the new one once.

`attach` takes the presence handle (the call site at `src/daemon.rs:2681` has `runtime`).

A viewer attach does not resize: in `attach`, `session.resize_locked(&mut state, size)` is skipped when `viewer` is true (the row origin from `top_rows` is still set, because it only shapes the render for that client). In the frame loop the daemon ignores `ClientFrame::Input`, `ClientFrame::Resize` and `ClientFrame::Release` while the attachment is a viewer (defense in depth: the client already ignores them); `Scroll`, `Redraw` and `Detach` still work. `ClientFrame::Mode { viewer }` has its own explicit arm before the catch-all `frame =>` arm (`src/daemon.rs:3617-3626`, whose inner `_ => unreachable!()` would panic on an unlisted frame) and switches the mode. `session.attached()` (so `AgentSummary.attached` and `a2amx list`) stays true for a viewer, because the slot is occupied and a plain attach is refused; the MCP description says so. Switching back to interactive does not resize by itself (the client sends a `Resize` after the `Mode` frame; 3dq does that).

### 2.3 The refused send (`src/daemon.rs`, `src/messaging.rs`)

`messaging::code::OWNER_PRESENT = "owner_present"`. In `send_message`, in the branch `to == messaging::HUMAN_SESSION` (`src/daemon.rs:1210`), after `remote_ineligible(..., ToHuman)` returns `None` and before the rate-limited insert: if `presence.present(Instant::now())`, return `failed(code::OWNER_PRESENT, "the owner is at the computer; write to them in the terminal instead")`. That branch is the only place an agent's human-bound message is accepted, so the check lives once. `Direction::ToHuman` is also read by `next_eligible_human` (`src/daemon.rs:1612`, behind `human_outbound_next`), which re-checks eligibility at delivery; the presence check must not go into `remote_ineligible`, or every queued message would fail with "sender is no longer eligible" when the owner returns. `human_outbound_next` and `human_outbound_done` are unchanged: a message accepted before the owner returned is still delivered. The gateway's `HumanInbound` (the owner writing from the phone) is unchanged.

### 2.4 The away notice (`src/daemon.rs`, `src/messaging.rs`)

A background task started with the daemon (like the heartbeat tasks) ticks once a second, calls `poll(now)`, and on a transition from present to away sends one message to every session for which `remote_ineligible(session, now, ToHuman)` is `None` (running, confined, remote set, team lease valid): sender `a2amx-daemon` (`messaging::SYSTEM_SENDER`), subject `messaging::PRESENCE_SUBJECT = "owner away"`, body the fixed text `messaging::OWNER_AWAY_BODY = "The owner is away from the computer. You may message them with send_message(to=\"human:owner\")."`. The task sends nothing on the way back to present and nothing at daemon start. A full queue is logged and skipped, as for the activity message (`src/daemon.rs:1002-1030`), and a session that already has an unread "owner away" system message gets no second one (as `send_activity` does with `open_system_message`, `src/daemon.rs:984-990`). An agent whose lease starts after the transition gets no notice; `list_agents` tells it the state.

### 2.5 `list_agents` and the MCP text (`src/daemon.rs`, `src/mcp.rs`)

`list_agents` (`src/daemon.rs:2900`) sets `owner` in `Response::Agents` only for a `Role::Session` caller whose session has `remote()` set; every other caller gets `None` (presence is the owner's private state). The notice (2.4) goes to the narrower set that is also confined with a valid lease; `owner` is read-only state and needs only `remote()`, so the two predicates differ on purpose. The MCP `list_agents` tool output includes it as `owner` when present, and the descriptions of `list_agents` and `send_message` (`src/mcp.rs:467,476`; `tests/mcp.rs:225,234` pin both byte for byte) say: `send_message` to `human:owner` is refused with `owner_present` while the owner is at the computer, and `owner` is `present` or `away`.

## 3. Attack paths (trust-boundaries)

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `Request::Attach` and `ClientFrame::Mode` | a session fakes owner presence to block the others' messages, or fakes absence | constrained: `Attach` is admin-only (`request_permitted`, `src/daemon.rs:2742`); a confined session's token cannot attach. An unconfined agent runs as the owner's user and could use the admin token; that is owner-trusted by design (confinement is the control) | a session-token connection sends `Attach`: refused (contract test; the existing `request_permitted` table already denies it, the test pins it for this request) |
| a raw client in viewer mode | sends `Input`, `Resize` or `Release` frames to type into or resize the session | constrained: the daemon ignores them while the attachment is a viewer | a viewer attachment sends `Input(b"x")`: the session receives nothing; sends `Resize`: the size is unchanged |
| `send_message` to `human:owner` | an agent messages the phone while the owner is present | constrained: `owner_present` from the one human-bound branch | send while an interactive attachment exists: refused; the same send after the owner is away is accepted |
| `list_agents` `owner` field | an agent that is not remote-enabled learns when the owner is away | removed: `owner` is set only for a remote-enabled session caller | a non-remote session and the admin get no `owner`; a remote session gets it |
| away notice | the body carries variable data an attacker could shape (prompt injection) | removed: the body is a fixed constant; the sender is the daemon | the notice body equals the constant byte for byte |
| notice volume | presence flapping floods agents with notices | constrained: one notice per present-to-away transition, never on return; a transition needs the owner (admin) to attach and detach | five attach/detach cycles inside the away window (exact count with literal instants in the pure test; `away_secs` 3 to 5 in the daemon test) produce exactly one notice |

## 4. Claims (the developer re-reads each; the architect has read the starred rows)

| claim | where | verified |
|---|---|---|
| `Request::Attach` has no mode field; `skip_serializing_if = "is_default"` is the convention | `src/wire.rs:190-200` | read* |
| `ClientFrame` tags 0x01 to 0x06, bodies validated by length | `src/wire.rs:629-636,705-800` | read* |
| the frame loop matches `Detach`, `Input`, `Release`, and a catch-all for `Resize`, `Scroll`, `Redraw` | `src/daemon.rs:3606-3640` | read* |
| `attach` is called from one place with `runtime` in scope; the guard drop clears the slot | `src/daemon.rs:2681,3362-3383,3444` | read* |
| `remote_ineligible` is shared by both directions; the human-bound send branch is the only place an agent's human-bound message is accepted; `next_eligible_human` also reads `Direction::ToHuman` at delivery | `src/daemon.rs:1210,1495-1536,1612` | read* |
| `AttachmentGuard` is built at reservation and the slot installed later; the guard is a local of `attach()` | `src/daemon.rs:3362-3383,3462-3523` | read* |
| the frame loop's catch-all has an inner `unreachable!()` | `src/daemon.rs:3617-3626` | read* |
| literals and patterns that break on the new fields: `Limits` without `..` at `tests/broker.rs:669,983,1049`; `Request::Attach` at `tests/human_inbound.rs:1184`, `tests/broker.rs:312`, `tests/daemon.rs:418`, `tests/attach_cli.rs:1520`, `tests/wire.rs`; `Response::Agents` patterns in `tests/teams.rs`, `tests/quota.rs`, `tests/broker.rs`, `tests/daemon.rs`, constructions in `tests/mcp.rs:689`, `tests/wire.rs` | consultant grep | the developer re-greps and lists every hit in the plan |
| the activity message shows how the daemon sends a system notice to a session | `src/daemon.rs:1002-1030` | read* |
| `Limits::default()` is the only `Limits` the binary uses | `src/main.rs:386` | read* |
| only `Role::Admin` may send `Attach` | `src/daemon.rs:2742-2777` | read* |
| `list_agents` builds `AgentSummary` per session; `Response::Agents { agents }` has one field | `src/daemon.rs:2900-2960`, `src/wire.rs` | the developer confirms the response shape and every construction site |
| tests that pin frame bytes and the Agents response shape | `tests/wire.rs:686`, `tests/prop_wire.rs:42` and others | the developer greps `ClientFrame`, `Response::Agents` and `Request::Attach` across `src/` and `tests/` and lists every hit in the plan |

## 5. Out of scope

The attach client (viewer state, the toggle, the idle timer, the overlay, the status marker, `Mode` sending): spec 3dq. Documentation and the CHANGELOG (written after 3dq). Presence for several owners, per-agent presence, a hold-and-release queue, the gateway's behaviour (it needs no change). Persisting presence across a daemon restart (it starts away).

## 6. Acceptance

Commands run from the repository root with `env -u A2AMX_BIN -u NO_COLOR`; every focused run uses `timeout 600`.

1. `cargo test --test wire --test prop_wire`: the new `Mode` frame round-trips (both values), a body of length 0 or 2 and a value other than 0 or 1 are rejected; `Request::Attach` without `viewer` decodes as interactive and an interactive request serialises without the key; `Response::Agents` without `owner` still decodes. Contract tests for the old shapes pass on the old code.
2. Pure `Presence` tests (in `tests/presence.rs`, literal `Instant`s, no sleeps): grace, `away_secs` 0, viewer, flapping, `poll` fires once per transition. Then `cargo test --test presence` (new, real confined daemon from `start_confined_daemon_with_limits`; the remote cases skip silently without bwrap, so the developer's report states whether they ran; the daemon tests use `away_secs` of 3 to 5 where cycles are counted, and a bounded negative wait of at least three seconds for "no notice"): present while an interactive attach exists; a viewer attach does not make the owner present; after the last interactive attach ends the owner stays present for `away_secs` and then is away; `Mode` interactive to viewer starts the same grace; take-over by `--force` does not double count; the refused send and the later accepted send (section 3 rows); a message accepted before the owner returned is still delivered by `human_outbound_next`; the notice arrives once per transition to each eligible agent and not to a non-remote agent, not on return, not at start; `list_agents` `owner` for a remote session, absent for a non-remote session and for the admin.
3. Viewer abuse: a viewer attachment's `Input`, `Resize` and `Release` frames change nothing with a positive control: a `cat` session; the viewer's `Input(b"x")` leaves the screen without it, then `Mode` interactive and `Input(b"y")` shows only `y`; size unchanged (observed through the size the daemon reports or `stty size` in the session); `Scroll` and `Redraw` still answer.
4. The full suite `env -u A2AMX_BIN -u NO_COLOR cargo test` passes; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny --locked check`, `scripts/prepush-fast.sh` pass.
5. Mutation check (the developer breaks each behaviour, names the repo test that fails, restores by checksum; each run under `timeout 120`, in the assigned checkout): the presence check removed from `send_message`; `present` ignoring the grace; a viewer counted as interactive; the viewer-input guard removed; the viewer resize skip removed; the notice sent on return; the notice body changed; `owner` set for every caller; `attach_ended` not called from the guard; the `Mode` frame ignored.
6. `git status --porcelain` shows only scope files; `git diff --check` is clean.

## 7. Amendments

1. (Developer BLOCKED, verified by the architect against `src/daemon.rs:3505-3548`.) The locked text said an attach to an already exited session is refused. It succeeds today (final render, then `Exit`) and `tests/attach_cli.rs:623-630` pins it. Section 2.2 now keeps that behaviour and only withholds the count. Add a test: an attach to an exited session does not make the owner present.
