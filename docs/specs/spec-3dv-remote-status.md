# Spec 3dv: the attach status line shows the remote messaging state

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-live-test-fixes-2026-11-10`.

Scope (files that may be edited): `src/wire.rs` (`StatusInfo` and two new types), `src/daemon.rs` (the runtime's gateway-connection count, one status builder, one lease reader), `src/status.rs` (one new cell and its colour), `tests/wire.rs`, `tests/status.rs` (new tests, plus its `StatusInfo` literal), `tests/attach_cli.rs` (only its `StatusInfo` literal), `tests/presence.rs` (the daemon tests), `docs/remote-messaging.md`, `docs/architecture.md`.

Does not touch: the lease rules and their limits, the gateway, `src/main.rs` (the attach client already passes `StatusInfo` to `status::render`), presence and the away notice, `CHANGELOG.md`.

Public test seams: a real daemon with attach status frames (`Request::Attach` with `status: true`, `ServerFrame::Status`), the lease helpers of `tests/common` (`create_confined_with_token`, `issue_and_redeem`, `gateway`), and the pure `status::render` through `tests/status.rs`.

## 1. Why

Owner request from the first live Discord test (2026-11-10): the attach status line should show the remote state. After `team up` the owner could not tell whether remote messaging was on, whether the lease had started, or whether the gateway was running. Four states matter for a remote-enabled session: no lease yet, lease active with the gateway connected, lease active but the gateway not connected, lease expired or ended.

Definitions fixed here:

- Remote-enabled: the session has `remote` set (`send` or `both`), `Session::remote()` (`src/session.rs:622`). Other sessions get no remote cell at all.
- Lease state of a session, from the daemon's lease table (`src/daemon.rs:111-123`, `lease_valid` at 1552): `none` when its team has no redeemed lease (including a team that has only an issued, unredeemed code) or the session is not among the lease's members (a session started or added after redemption, i.e. a new session id; it needs a new code, which is what `none` tells the owner; `a2amx reset` keeps the id, so a reset session stays a member); `active` when the lease exists, the session is a member and the lease has not expired; `ended` when the lease exists, the session is a member and it has expired. Nothing clears an expired lease; `ended` stays until the next redemption, and a redemption that does not include the session turns it into `none`.

One function decides all three: `lease_state(team, id, now) -> RemoteLease` in `src/daemon.rs`, with `lease_valid` becoming `lease_state(..) == Active` (one predicate, one `Instant::now()`, so the bar never disagrees with the send check at the expiry instant).
- Gateway connected: at least one authenticated gateway-role connection is open to the daemon. The relay holds one connection for its whole life and rebuilds it after an outage (`src/relay.rs`, `Relay.daemon`), and the connection closes when the process dies, so the count is immediate and needs no timer. It says "a gateway process is attached to the daemon", not "Discord is reachable" and not "relaying" (the daemon cannot know that): the relay opens its connection first (`src/relay.rs:31`), before the Discord member check and before a `--pair` wait finishes, and keeps it while paused for member count and after an outage until it reconnects. So `remote on` can show during pairing or a pause; the docs say so and the cell text stays `remote on`.

Alternatives rejected: a recency timer on gateway requests (would flicker with the owner's `sync_ms`/`idle_ms` settings and needs a threshold); showing the state for non-remote sessions (noise); extending `list`/`list_agents` (out of scope; `list_agents` already shows `owner`).

## 2. Mechanical changes

2.1. `src/wire.rs`: add
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteLease { None, Active, Ended }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteStatus { pub lease: RemoteLease, pub gateway: bool }
```
and, on `StatusInfo` (line 601), after `members`: `#[serde(default, skip_serializing_if = "Option::is_none")] pub remote: Option<RemoteStatus>`. Old clients ignore the field; old daemons omit it (the bar then shows no remote cell).

2.2. `src/daemon.rs`:
- `Runtime` gets `gateway_connections: AtomicUsize` (the struct starts at `src/daemon.rs:79`; built at `Arc::new(Runtime {` near line 2401). In `serve` (`src/daemon.rs:2715-2720`, the only caller of `authenticate_hello`), when it returns `Some(Role::Gateway)`, increment the count and hold a small guard that decrements on drop, so every exit path (EOF, `?` error, shutdown) decrements. Only the gateway role counts; the admin client and session tokens do not.
- `fn lease_state` as in section 1 (replaces the body of `lease_valid`'s predicate) and `fn remote_status(&self, session: &Session) -> Option<RemoteStatus>`: `session.remote()?`; `lease` from `lease_state` (a session without a team is `RemoteLease::None`); `gateway: self.gateway_connections.load(Ordering::Relaxed) > 0`. One lock on `leases`, taken in its own statement.
- `attachment_status` (line 3527; the `StatusInfo { .. }` literal at 3571) sets `remote: runtime.remote_status(session)`.
- The status frame is sent after the initial snapshot and then only when the whole `StatusInfo` changes (poll once a second, `src/daemon.rs:3716-3749`), so a change of the lease or gateway state reaches the bar within about a second without further code.

2.3. `src/status.rs`: a pure `fn remote_cell(remote: &RemoteStatus) -> (&'static str, &'static str)` returning the cell text and the dot's colour escape (the existing colour helpers return escape constants such as `"\x1b[38;5;78m"`, `activity_color` at `src/status.rs:521`; `Span::styled` takes a `&'static str` opener), and one new optional cell in `render`, built as spans (dot span plus text span) and rendered through `render_span`; `pending` is a bare string and is not the model.

| lease | gateway | text | dot colour (256-colour) |
| --- | --- | --- | --- |
| `none` | any | `remote: no lease` | 244 (grey) |
| `active` | true | `remote on` | 78 (green) |
| `active` | false | `remote: gateway off` | 209 (orange) |
| `ended` | any | `remote: lease ended` | 203 (red) |

The cell is `● ` (the existing dot glyph) plus the text, followed by one space, and sits right-aligned immediately left of `pending` (byte order on the bar: left part, padding, remote cell, pending, hold). Width rules, all three required: the cell's character length is added to the shrink loop's `total` (`src/status.rs:281`) AND subtracted in the final padding (`src/status.rs:339`, a `usize` subtraction that underflows if the cell is counted in only one); the cell is not added to `left_spans` (it already has seven parameters). Shrink order: team, harness, role, quota, activity, pending, hint, then the remote cell, then the confined marker (so the cell outlives everything but the confined marker, and it is dropped after the transient hold hint). The shrink loop gets one more flag and one more `else if`. `render` is already large: if the Sonar gate flags it, the fix is to extract the shrink step into a function over an ordered flag list, not a suppression; the architect runs `scripts/prepush-sonar.sh`; the developer keeps the new code in `remote_cell` and one small span-building helper.

2.4. Struct literals: adding a field to `StatusInfo` breaks every literal: `src/daemon.rs:3571`, `tests/status.rs:10`, `tests/attach_cli.rs:129`, `tests/wire.rs:1171` and `tests/wire.rs:1268`. Add `remote: None` to each (the exact-JSON comparisons in `tests/wire.rs` stay unchanged because `None` is skipped). The developer greps `StatusInfo {` across `src` and `tests` again before editing.

## 3. Documentation

3.1. `docs/remote-messaging.md`: a short section on the status line cell: the four texts, what each means and what to do (no lease: run `a2amx team up --detach` and post the code; gateway off: start `a2amx gateway up`; lease ended: run `team up --detach` again), and that "gateway" means the gateway process is attached to the daemon, not that Discord is reachable. Only remote-enabled sessions show the cell.
3.2. `docs/architecture.md`, the Status line section (around line 366): one sentence, the `remote` field of the status payload, additive.

## 4. Out of scope

Discord reachability (the gateway reporting its own connection health to the daemon); showing the state in `a2amx list`/`list_agents`; a remote cell for sessions that are not remote-enabled; changing lease durations or rules; the click or key actions on the cell; `CHANGELOG.md`.

## 5. Acceptance

Public interface change: additive (a new optional `remote` field in the attach status payload).

1. `tests/status.rs` (red first), through `status::render`: for each row of the 2.3 table the rendered bar contains the cell text and the exact escape `\x1b[38;5;NNm` of its dot (asserted per row, so swapped colours fail); `remote: None` renders exactly the bar it renders today (existing tests unchanged); the byte order is remote cell, then pending, then hold; at the exact width where everything just fits all of it shows and at that width minus one something is dropped (no panic and no misalignment at any width from 2 to 120); with a hold hint active, the hint is dropped before the cell; the cell is dropped before the confined marker; pending is dropped before the cell.
2. `tests/wire.rs`: a `StatusInfo` with `remote: Some(..)` round-trips and encodes `"remote":{"lease":"active","gateway":true}`; one without the field decodes as `None`; the exact-JSON tests are unchanged. Contract test for the second part.
3. `tests/presence.rs` (real daemon, confined remote-enabled session, attach with `status: true`, read `ServerFrame::Status` frames until the field shows the expected value, bounded by a timeout, no fixed sleeps): (a) before any lease: `lease none`, and again for a team that has an issued but unredeemed code; (b) after `issue_and_redeem` (a long lease): `active`, `gateway` per whether a gateway connection is open (the helper opens one; assert `true`, then drop it and assert `false` within the poll window while the admin client stays open, then reconnect and assert `true`); (c) a second daemon with a short `Limits.lease_secs` (as `limits_with(1, ..)` does): after it expires, `ended`, and a later redemption that excludes the session gives `none`; (d) a session with `remote` set, started after the redemption in the same team (not a member): `none`; (e) a session without `remote`: the `remote` field is absent. Command: `cargo test --test presence`.
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`; `git diff` only in the files of section 0.
5. Mutation check (developer, assigned checkout, restore by checksum, `timeout 120` each; name the failing test per mutant): `remote_status` returns `gateway: true` always (3b); the guard does not decrement (3b); `ended` reported as `none` or the reverse (3c, 3d); the gateway count incremented for every role (3b, admin client stays open); a team with only an issued code reported as `active` or `ended` (3a); the cell counted in only one of the loop total and the padding (1); colours swapped (1); cell placed after pending or hold (1); a non-member reported as `active` (3d); the cell shown for non-remote sessions (3e); the shrink order changed (1); the `remote` field missing from the `StatusInfo` literal in `attachment_status` (all of 3).
6. Trust boundary: none crossed (read-only daemon state, shown to the attached owner's own client; the gateway connection count reveals nothing new to an authenticated attach).

## 6. Claims table

| Claim | Where | Verified |
| --- | --- | --- |
| `StatusInfo` fields and its serde attributes | `src/wire.rs:601-624` | read |
| `StatusInfo` struct literals: `src/daemon.rs:3571`, `tests/status.rs:10`, `tests/attach_cli.rs:129`, `tests/wire.rs:1171`, `tests/wire.rs:1268` | grep `StatusInfo {` | grep |
| The status frame is sent after the snapshot and then when the whole payload changes, polling once a second | `src/daemon.rs:3711-3749` | read |
| The lease table: per team an `Option<ActiveLease { expires, members }>`; `lease_valid` is `now < expires && member` | `src/daemon.rs:109-123,1552-1562` | read |
| `Session::remote()` exists | `src/session.rs:622` | read |
| The gateway role is assigned by the gateway token in `authenticate`, and the hello is handled in `authenticate_hello` | `src/daemon.rs:368-371,2940-2961`; the only caller is `serve`, `src/daemon.rs:2715-2720` | read (consultant) |
| `render` shrinks cells in a loop with `show_*` flags; `left_spans` takes seven parameters; `pending` is a plain string placed right of the left part | `src/status.rs:214-353` (loop 261-304, padding 339) | read |
| The relay holds one daemon connection for its whole life and rebuilds it on an outage | `src/relay.rs` (`Relay.daemon`, `mark_broken`, reconnect) | read |
