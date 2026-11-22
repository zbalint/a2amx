# Spec 3gd: remote messaging leases as one module with a leaf-locked lease book (daemon S4)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing lease, gateway and remote-status tests (`tests/human_inbound/`, `tests/gateway.rs`, `tests/presence/remote_status.rs`, `tests/relay/`, `tests/broker/limits_and_lifecycle.rs`); `Leases` and the moved methods are private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/remote.rs`.

Does not touch: every other file under `src/`, every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `daemon` row of `AGENTS.md` and the backlog when committing.

## 1. Why

`src/daemon.rs` is 3,174 lines. Ten methods of `Runtime` (`:873-1099`, about 225 lines) implement remote messaging: the lease code and lease bookkeeping, the remote status of a session, the gateway's agent list and the eligibility verdict for a remote send. The bookkeeping state (`PresenceLeases`, `TeamLease`, `ActiveLease`, `PendingLeaseCode`, `:104-123`) is a plain field `leases: Mutex<PresenceLeases>` that any `Runtime` method can lock, so the lock's leaf property (spec 3gb's rule for the session map) holds only because today's callers behave: nothing is acquired under it, and the order is written nowhere. The wrong-code lockout, the code expiry and the lease expiry are three rules entangled in two functions that also take the lock (`take_matching_lease_code` mixes lookup, expiry sweep, lockout and clearing).

Target shape: a new module `src/daemon/remote.rs` holding the moved methods and one private type that owns the lease state behind its own lock, as `Sessions` does for the map:

```rust
/// The lease and code state of every team. The lock is a leaf: held only inside these methods.
pub(super) struct Leases { state: Mutex<LeaseState> }
impl Leases {
    pub(super) fn new() -> Self;
    /// Store a fresh pending code for `team`, replacing any earlier one, and reset the wrong-attempt count.
    pub(super) fn issue(&self, team: String, code: String, now: Instant, ttl: Duration);
    /// Consume the pending code that matches; None for a wrong, expired or already-used code or after too many wrong attempts.
    pub(super) fn redeem(&self, code: &str, now: Instant) -> Option<String>;
    /// Begin the lease of `team` for `members` (replacing an earlier one).
    pub(super) fn start(&self, team: &str, members: Vec<SessionId>, now: Instant, term: Duration);
    /// None when `id` is not a member of `team`'s lease, Active before expiry, Ended after.
    pub(super) fn state(&self, team: &str, id: &str, now: Instant) -> RemoteLease;
}
```

`LeaseState` (the old `PresenceLeases`, with `TeamLease`, `ActiveLease`, `PendingLeaseCode`) is private to `remote.rs`; the `Mutex` field is private to it, so the compiler, not a grep, stops any other module from locking it. The bodies of `issue`, `redeem`, `start` and `state` are the bodies of `issue_lease_code`'s locked tail, `take_matching_lease_code`, `start_lease`'s locked tail and `lease_state`, moved unchanged except that `Instant::now()` and `Duration::from_secs(self.limits.*)` arrive as arguments.

Rejected: a pure `LeaseBook` with `&mut self` and the mutex on `Runtime` (the lock then stays visible to every child module, and the leaf property is again by convention); a facts struct for `remote_ineligible` (one caller shape, no second implementation, no test seam below the daemon protocol, so it would be a single-use abstraction); splitting `remote_ineligible`'s direction table into its own function (used once); moving the gateway connection counter (it belongs with the gateway serve path, S6); renaming `PresenceLeases` is part of the move, not a separate change.

## 2. Mechanical changes

### 2.1 New `src/daemon/remote.rs`

Move, in this order, byte for byte except as 2.2 says:

- `Leases` with its private `LeaseState`, `TeamLease`, `ActiveLease`, `PendingLeaseCode` (from `:104-123`, with `PresenceLeases` renamed `LeaseState`) and the four methods above;
- inside one `impl Runtime` block: `remote_members` (`:873-893`), `issue_lease_code` (`:894-924`), `redeem_lease_code` (`:978-988`), `remote_status` (`:1012-1023`), `gateway_agents` (`:1024-1056`), `remote_ineligible` (`:1057-1099`).

Not moved, deleted: `take_matching_lease_code` (`:925-964`), `start_lease` (`:965-977`), `lease_state` (`:989-1007`) and `lease_valid` (`:1008-1011`) become `Leases` methods or call sites as below.

Visibility: `pub(super)` for what `daemon.rs` and `notices.rs` still call: `issue_lease_code`, `redeem_lease_code`, `remote_status`, `gateway_agents`, `remote_ineligible` (called at `:638, :741, :1114, :1174` and `notices.rs:230`, dispatch at `:2176-2180`, `:2923`); `remote_members` stays private (its callers move with it). The moved code calls two items of the parent that stay (`random_hex`, a `pub fn` at `:1972`, and the private `constant_time_eq` at `:141`); a child module sees them, so it needs `use super::{Runtime, constant_time_eq, random_hex};` and no visibility change. `remote.rs` imports what the moved code uses (`crate::messaging::{self, code}`, `crate::session::{Session, SessionId}`, `crate::wire::{GatewayAgent, RemoteLease, RemoteStatus, Response}`, `HashMap`, `Duration`, `Instant`, `Mutex`, `MutexGuard`, `PoisonError`, `Ordering`; the compiler names any miss). `daemon.rs` loses the `use` names `GatewayAgent`, `RemoteLease` and `RemoteStatus` (no other use; `:2923` names no type); `Ordering`, `Duration`, `Instant`, `HashMap`, `Mutex` and `PoisonError` stay in use there.

### 2.2 The call sites

- `Leases::issue` body: the code from `issue_lease_code`'s `let mut leases = ...` through `leases.wrong_codes = 0;` (`:912-917`), with `expires: now + ttl`.
- `issue_lease_code` keeps `members`, the `spawn_blocking(random_hex::<6>)`, the `agents` list and the `Response::LeaseCode`, and calls `self.leases.issue(team, code.clone(), Instant::now(), Duration::from_secs(self.limits.code_secs));` where the lock was.
- `Leases::redeem` body: `take_matching_lease_code`'s body (`:926-963`), taking `now` and `code` as arguments.
- `redeem_lease_code`: `let Some(team) = self.leases.redeem(&code, now) else { ...same error... };` then `let members = self.remote_members(&team).into_iter().map(|(id, _, _)| id).collect(); self.leases.start(&team, members, now, Duration::from_secs(self.limits.lease_secs));` (the body of `start_lease`, inlined: its only caller).
- Every `self.lease_state(team, id, now)` becomes `self.leases.state(team, id, now)` (`remote_status`) and every `self.lease_valid(team, id, now)` becomes `self.leases.state(team, id, now) == RemoteLease::Active` (`gateway_agents`, `remote_ineligible`).

The lock is taken by exactly one private function in `remote.rs`:

```rust
fn lock(&self) -> MutexGuard<'_, LeaseState> {
    self.state.lock().unwrap_or_else(PoisonError::into_inner)
}
```

used by all four methods; `remote.rs` has no other `lock()` call on `state`.

### 2.3 `src/daemon.rs`

Declare `mod remote;` (alphabetical with the others) and `use remote::Leases;`. Change the `leases` field type to `Leases` (`:92`) and its initializer (`:1764`) to `leases: Leases::new(),`. Delete the moved items. Remove from `daemon.rs`'s `use` lines only the names no remaining code uses (`-D warnings` fails on any other leftover; the compiler says which). Nothing else changes. `src/daemon.rs` shrinks by about 250 lines.

## 3. Behaviour

No observable change: the same lease codes, the same lockout, the same expiry, the same texts and codes of every refusal. One order detail: `issue_lease_code` read `Instant::now()` inside the lock; it now reads it just before calling `issue`, a few instructions earlier; the code's expiry is the same to within a microsecond and no test measures it. `redeem_lease_code` read `now` once and passed it to both steps already; unchanged. Lock order: unchanged and now enforced by privacy; `Leases` methods acquire nothing, and no `Leases` call is made with another daemon lock held (`remote_members` takes a registry snapshot before the call). Tracing targets of moved code (none log) do not change.

## 4. Out of scope

- Any behaviour change: the unpinned expiry-comparison edges, `gateway_connections`, the constant-time compare (stays in `daemon.rs` for `authenticate`).
- `human_inbound`, `fail_human_bound`, `next_eligible_human` and the admission code (S6).
- A unit test of `Leases` (a private type; the seam is the daemon protocol).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `a94c5a5` (architect, 2026-11-22): 66 suites, 1415 passed, 0 failed, 3 ignored; after: the same (no test added or removed). The developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'state\.lock\(\)' src/daemon/remote.rs` prints exactly one line (in `Leases::lock`; the four methods call `self.lock()`); `rg -n -i 'leases' src/daemon.rs` prints exactly three lines: the field, its initializer and the `use remote::Leases;` line; `rg -nU --pcre2 'fn (take_matching_lease_code|start_lease|lease_state|lease_valid|remote_members|issue_lease_code|redeem_lease_code|remote_status|gateway_agents|remote_ineligible)\b' src/daemon.rs` prints nothing; `rg -n 'PresenceLeases|TeamLease|ActiveLease|PendingLeaseCode' src` prints only lines in `remote.rs`.
- `git status --short` lists only `src/daemon.rs` and `src/daemon/remote.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Mutation check (tester, in the assigned checkout, one file at a time, restoring by checksum, tree equal to the developer diff afterwards), naming the repo test that fails on each mutant; survivors are reported, not fixed: `redeem` does not clear the matched pending code (`lease_codes_expire_are_single_use_and_wrong_attempts_reset_on_issue`); the drop check `leases.wrong_codes >= MAX_WRONG_CODES` after a wrong attempt (`:957`) becomes `>` (the sixth attempt with the real code would succeed: `five_wrong_codes_drop_pending_codes_until_issue_resets_the_counter`); that check at `MAX - 1` is expected to survive (the test only asserts the fifth wrong attempt is followed by a failed real code; record it); the entry check `wrong_codes >= MAX` at the top of `redeem` (`:927`) is an equivalent mutant (once the drop check fires no pending code exists until `issue` resets the counter), do not mutate it; `issue` does not reset `wrong_codes` (same test; the replacement redeem hits the entry check only under this mutant); the expiry test `now < pending.expires` becomes `true` (the first test above); `state` returns `Active` after expiry (`tests/presence/remote_status.rs` ended-lease test and `expiring-inbound-team`; tester names it); `state` skips the member check (`lease_membership_is_fixed_...`, `tests/human_inbound/inbound_admission.rs:317`); `remote_ineligible` loses the `ToAgent` mode check (`inbound_requires_explicit_both_mode_...`, `tests/human_inbound/remote_mode.rs:21`) or the lease check (`lease_membership_is_fixed_...` and the test ending at `inbound_admission.rs:224`); `remote_ineligible` or `gateway_agents` loses its `confined` check: expected survivors, since creation refuses a remote mode without confinement (`remote_mode.rs:35-43`) so an unconfined session reaches the next check, which carries the same code (record them); `gateway_agents` loses `exit_code().is_some()` (`tests/gateway.rs:105`, through the `exited-remote` session; a race with its `sleep 1`: rerun the mutant once if it survives).
- Known flaky tests: those that sleep past a one-second code or lease expiry (`lease_codes_expire...`, `expiring-inbound-team`) wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| The lease state is a private struct set reached only through `self.leases` | `rg -n 'PresenceLeases|\.leases' src tests`: `:92`, `:104`, `:912`, `:926`, `:971`, `:990`, `:1764` only (all in `daemon.rs`); no test names it | probed |
| `lease_valid` callers are `gateway_agents` and `remote_ineligible` only | `rg -n 'lease_valid|lease_state' src`: `:1009`, `:1016`, `:1036`, `:1094` | probed |
| `start_lease` has one caller (`redeem_lease_code`) | `rg -n 'start_lease' src` | probed |
| `remote_ineligible` has callers at `:638, :741, :1114, :1174` and `notices.rs:230` | `rg -n remote_ineligible src` | probed |
| `constant_time_eq` (`:141`) is a private free function of `daemon.rs` and `random_hex` a `pub fn` at `:1972`; the moved code needs both | `:141`, `:1972`, `issue_lease_code` | read (consultant m_8457) |
| The expiry, single-use and lockout rules are pinned | `tests/human_inbound/inbound_admission.rs:230` (`lease_codes_expire_are_single_use...`) and the next test (`five_wrong_codes...`) | read |
| Lease expiry (`Ended`) is pinned | `tests/presence/remote_status.rs` (`RemoteLease::Ended` at `:158`) | read |
| Nothing logs inside the moved lease code | the moved bodies | read |
