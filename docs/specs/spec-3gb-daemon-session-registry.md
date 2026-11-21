# Spec 3gb: the session map becomes a leaf-locked registry (daemon S2)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol (every daemon-backed suite); `Sessions` is private to `daemon`, so it has no test of its own.

Scope (edit): `src/daemon.rs`, new `src/daemon/registry.rs`.

Does not touch: every other file under `src/`, every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `daemon` row of `AGENTS.md` and the backlog when committing.

## 1. Why

`Runtime.sessions` is a `Mutex<BTreeMap<u64, Arc<Session>>>` locked at 19 sites in `src/daemon.rs` (the creation gate and the id counter are two more), each repeating `.lock().unwrap_or_else(PoisonError::into_inner)`. Most sites keep the lock while they call into each `Session` (its state lock, its channel, its quota reader) and some into other `Runtime` locks: `list_sessions` (`:2880`) and `list_agents` (`:2937`) call `session_activity`, `note_activity`, `hold_reason` and `quota::read` for every session under the map lock; `gateway_agents` (`:1579`) calls `lease_valid`, which takes the lease lock; `poll_transitions` (`:789`) holds the map and the presence lock together, and so does `list_agents` (`:2937`, through `owner_cause`, which locks presence at `:1256`; the order recorded as "the `list_agents` order" in spec 3ef amendment 2). No reverse nesting exists today, but nothing states the order, so the next change can create a deadlock without any test failing. The consultant's design read (m_8377, backlog F94) ranks this the highest-value redesign of the file.

Target shape: a type `Sessions` in `src/daemon/registry.rs` that owns the map, the id counter and the creation gate, and whose methods never call out while holding the map lock. Callers take a `snapshot()` (a `Vec<Arc<Session>>` in id order) and filter it with no lock held. The map lock becomes a leaf lock: nothing is acquired while it is held, so no lock order involving it can exist. The lock-poisoning idiom is written once.

```rust
pub(super) struct Sessions {
    map: Mutex<BTreeMap<u64, Arc<Session>>>,
    next: AtomicU64,                      // starts at 1
    creating: tokio::sync::Mutex<()>,
}
impl Sessions {
    pub(super) fn new() -> Self;
    pub(super) async fn begin_creation(&self) -> tokio::sync::MutexGuard<'_, ()>;  // was new_session_gate.lock()
    pub(super) fn next_number(&self) -> Option<u64>;     // the fetch_update; None when ids are exhausted
    pub(super) fn insert(&self, number: u64, session: Arc<Session>);
    pub(super) fn remove(&self, number: u64);
    pub(super) fn clear(&self);
    pub(super) fn get(&self, number: u64) -> Option<Arc<Session>>;
    pub(super) fn snapshot(&self) -> Vec<Arc<Session>>;  // every session, ascending number
}
```

Rejected: methods taking a predicate (`any`, `find`, `filter`) that run it under the lock (that keeps the nesting); returning a `MutexGuard` (same); a read-write lock (the writers are rare but the reads call out, which is the hazard; the fix is not to call out, not a finer lock); folding `by_name`, `live` and `team_members` helpers into this spec (the same few filters appear with small differences; a follow-up can add them once the call sites are uniform).

## 2. Mechanical changes

### 2.1 New `src/daemon/registry.rs`

The type above. `snapshot` is `self.map.lock().unwrap_or_else(PoisonError::into_inner).values().cloned().collect()`; `next_number` is the existing `fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1)).ok()`; the other methods are one lock and one map call, except that a value the map releases is dropped after the lock is released (amendment 1): `insert` binds the replaced value (`let replaced = self.map.lock()...insert(number, session);`, dropped at the end of the method), `remove` binds the removed value the same way, and `clear` moves the whole map out under the lock (`std::mem::take`) and drops it after the guard is gone. The reason: the last `Arc<Session>` runs `Session::drop` (`src/session.rs:602-611`: a process-group HUP signal and a `tracing::warn!`), which is a call out and, for the PTY, must not run under a lock. A doc comment states: the map lock is held only inside these methods, which call nothing else, so it is a leaf lock; hold no other lock's guard across `begin_creation().await`.

The type has eight methods (`new`, `begin_creation`, `next_number`, `insert`, `remove`, `clear`, `get`, `snapshot`). Its value is that `map` is private to `registry.rs`: the compiler, not a grep, stops a future site from calling out under the lock. A `Runtime::snapshot` helper that kept the field would leave `.lock()` open to the next change (rejected).

### 2.2 `src/daemon.rs`

Declare `mod registry;` and `use registry::Sessions;`; drop `AtomicU64` from the `std::sync::atomic` import at `:10` (nothing else in `daemon.rs` uses it once `next_id` is gone; `AtomicBool`, `AtomicUsize` and `Ordering` stay). `Runtime` loses `sessions`, `next_id` and `new_session_gate` (`:75-77`, `:94`) and gains `sessions: Sessions` (initializer `Sessions::new()` replaces `:2301-2302`, `:2321`). Each site becomes a snapshot taken first and used after, in this table (line numbers at `d5d0537`):

| Site | Today | After |
|---|---|---|
| `omp_usage_command` `:294` | `values().find_map` under the lock | `self.sessions.snapshot().iter().find_map(..)` |
| `authenticate` `:317` | loop under the lock | loop over a snapshot |
| `create_session` `:498`, `:503` | `new_session_gate.lock().await`, `next_id.fetch_update(..).map_err(..)` | `let _gate = self.sessions.begin_creation().await;`, `self.sessions.next_number().ok_or_else(\|\| anyhow::anyhow!("session id space exhausted"))?` |
| `create_session` `:569` | `insert(number, session.clone())` | `self.sessions.insert(number, session.clone())` |
| `check_team_and_name` `:653`, `:677` | `any(..)` under the lock | `any(..)` over a snapshot |
| `poll_transitions` `:789` | holds map then presence | lock presence first, then take the snapshot (a leaf call); see 2.3 |
| `observe_exit` `:897` | `filter(..).cloned().collect()` under the lock | filter over a snapshot |
| `live_watched_peers` `:1107` | holds the map across the filter | one snapshot, filter after |
| `recipient` `:1269` | `find(..).cloned()` | `find` over a snapshot (the `Arc` is already owned) |
| `remote_members` `:1425` | `filter_map` under the lock | over a snapshot |
| `gateway_agents` `:1579` | `filter_map` calling `lease_valid` under the lock | over a snapshot; `lease_valid` runs with no map lock |
| `reset_target` `:1818` | `find(..).cloned()` | over a snapshot |
| `Daemon::shutdown` `:2473`, `:2493` | `values().cloned().collect()`, `.clear()` | `snapshot()`, `clear()` |
| `list_sessions` `:2880` | `map` calling `session_activity`, `note_activity`, `hold_reason`, `quota::read` under the lock | the same `map` over a snapshot |
| `list_agents` `:2937` | holds the map guard across the whole function (`owner_cause` locks presence under it) | one `let sessions = runtime.sessions.snapshot();`; the viewer is `sessions.iter().find(..).cloned()`; the agents loop reads `sessions.iter()` instead of `sessions.values()`; no second snapshot (membership stays one moment) |
| `kill_session` `:3009` | `remove(&number)` | `self.sessions.remove(number)` |
| `lookup` `:3401` | `get(&number).filter(..).map(..)` | `get(number).filter(..).map(..)` |
| `attachment_status` `:3454` | `filter(..).cloned().collect()` | over a snapshot |

The filters and closures keep their text; only the receiver changes (a snapshot is a `Vec`, so a `.values()` on a held guard becomes `.iter()` on the snapshot, at `poll_transitions` and `live_watched_peers` too; one snapshot per function wherever a guard was held across several uses). No rename, reorder or comment edit outside these lines.

### 2.3 `poll_transitions`: the one ordering change

Today `poll_transitions` takes the map lock, then the presence lock, and prunes presence entries of sessions that are not live. A snapshot taken before the presence lock could miss a session created and attached in the gap; `retain` would then drop its presence entry (an interactive hold state), and a hold must not be lost. So the order becomes: lock presence, then `self.sessions.snapshot()` while holding it, then prune and poll as before. This is safe because the map lock is now a leaf (nothing is acquired under it), so presence-then-map cannot deadlock against any other path, and no path holds the map lock while taking presence anymore. A comment on the snapshot line says so in one sentence.

## 3. Behaviour

No observable change. Membership seen by each site is the same as before (a snapshot is atomic with respect to insert and remove, like one lock hold). What changes is how long the map lock is held: it is no longer held while a session's state lock, the lease lock or the channel code runs.

## 4. Out of scope

- Helper methods (`by_name`, `live`, `team_members`), a `reserve_name`, merging the duplicated name lookups (`:677`, `:1107`, `:1269`, `:1818`): a follow-up once this lands.
- Moving any `Runtime` method out of `daemon.rs` (S3 to S6); changing `Session`; the `presence`, `leases` and `rate` locks.
- A test of `Sessions` itself (private; AGENTS.md); a new concurrency test (no deterministic way to provoke the old nesting).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `d5d0537` (architect, 2026-11-21): 66 suites, 1414 passed, 0 failed, 3 ignored; after: the same count (no test added); the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- No direct map access remains: `map` is private to `registry.rs`, so the compiler enforces it. As a readable check, `rg -nU --pcre2 '\.sessions\s*\.(?!(snapshot|get|insert|remove|clear|begin_creation|next_number)\()' src/daemon.rs` prints nothing (rustfmt may put `.sessions` at the end of a line, so a line-anchored grep would be wrong; the multi-line form above handles it; it prints 36 lines on `d5d0537`).
- `rg -n 'new_session_gate|next_id|sessions: Mutex' src/daemon.rs` prints nothing.
- `rg -n 'map\.lock\(\)' src/daemon/registry.rs` prints only inside `snapshot`, `insert`, `remove`, `clear`, `get` (`begin_creation` locks `creating`); no method of `Sessions` calls a `Session` method or another lock (the architect and the tester read the file).
- `git status --short` lists only `src/daemon.rs` and `src/daemon/registry.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Mutation check (tester, in the assigned checkout, restoring by checksum): `snapshot` returns an empty vector: `ids_are_numerically_ordered_and_not_reused_after_kill` (`tests/daemon/session_creation.rs`) and the session-token suites fail; `insert` does nothing: the same test; `remove` does nothing: `kill_by_name_removes_the_named_session` and `kill_exited_removes_only_exited_sessions_and_frees_names` (`tests/refs_and_list.rs`); `get` returns the first session whatever the number: the tester names the test that fails (expected `kill_exited_removes_only_exited_sessions_and_frees_names`); `next_number` returns a constant: the creation test above; `poll_transitions` using a snapshot taken before locking presence: report whether any test fails (expected: none, the race is not provokable; recorded as unpinned). The tester names the repo test that fails on each mutant that dies.
- Known flaky tests: the daemon-backed suites time out under load now and then; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| 19 map-lock sites, the gate and the counter, and the field and initializer lines | `rg -n '\.sessions\b\|next_id\|new_session_gate' src/daemon.rs` at `d5d0537`: lines 76, 77, 94, 294, 317, 498, 503, 569, 653, 677, 789, 897, 1107, 1269, 1425, 1579, 1818, 2301, 2302, 2321, 2473, 2493, 2880, 2937, 3009, 3401, 3454 | read: contexts of each site |
| No code outside `daemon.rs` touches the map | `rg -n 'runtime\.sessions\|\.sessions\.' src --glob '!src/daemon.rs'` | read: only `src/attach/picker.rs:111`, an unrelated `Vec` field |
| `list_sessions` and `list_agents` call into each session under the map lock | `src/daemon.rs:2880-2905`, `:2937-` | read |
| `gateway_agents` calls `lease_valid` under the map lock | `src/daemon.rs:1579-1595` | read |
| `poll_transitions` holds map then presence | `src/daemon.rs:789-791` | read |
| The map lock is held while taking presence in `poll_transitions` and `list_agents` (through `owner_cause`); no path holds presence while taking the map except as 2.3 introduces | `src/daemon.rs:789-791`, `:2937-2956`, `:1256`; consultant m_8404 read every presence site (`:790, :1256, :3434, :3608, :3701`) | read |
| `next_id` increments with `fetch_update(.., checked_add(1))` and the error text is `session id space exhausted` | `src/daemon.rs:503-505` | read |
| The only `daemon.rs` unit test is `heartbeat_body_matches_digest_example` and names none of these items | `src/daemon.rs` test module | read |
| The creation gate is held across `plan_confinement`, `codex::start` and the spawn | `src/daemon.rs:498-580` | read: unchanged by this spec (`begin_creation` returns the same guard type) |

## Amendment 1 (developer BLOCKED, m_8408)

Verified against `src/session.rs:602-611`: `Drop for Session` calls `kill_process_group` and may log, so dropping the last `Arc<Session>` inside `BTreeMap::clear`, `remove` or `insert` (which drops a replaced value) would call out under the map lock, contradicting section 1. Resolution: section 2.1 now requires `insert`, `remove` and `clear` to release the lock before the dropped values are destroyed. `snapshot` and `get` only clone `Arc`s, and a clone's drop is not the last one while the map holds the original, so they need nothing. Acceptance addition: the tester's mutation list gains "`clear` drops under the lock" as a read-only review item (not a test; no test can observe it), and the architect checks the three bodies at review.
