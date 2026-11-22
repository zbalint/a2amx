# Spec 3gk: the daemon lifecycle leaves daemon.rs and start-up fails clean (daemon S7)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file, made after 3gj is accepted and pushed. The ranges below are for HEAD 1b8f3b4 (`src/daemon.rs` 684 lines, `serve.rs` already out); every item is also named: locate by name. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: `Daemon::start` and `Daemon::shutdown` through `tests/daemon/daemon_lifecycle.rs` and every test that starts a daemon (`tests/common::start_daemon`).

Scope (edit): `src/daemon.rs`, new `src/daemon/lifecycle.rs`, `tests/daemon/daemon_lifecycle.rs`.

Does not touch: every other file under `src/`, every other test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md`, the backlog and the changelog when committing.

This spec changes what a failed start leaves on disk (token and address files): see section 5 for the trust-boundary check.

## 1. Why

`Daemon::start` is one 243-line function that does seven jobs in a row: process prerequisites, the single-daemon lock, tokens and host name, opening the store, publishing the token and address files, binding listeners, and building the `Runtime` and spawning four background tasks (purge, OMP usage poll, presence poll, one accept loop per listener). `Daemon::shutdown` is a 74-line function that stops those tasks, aborts delivery tasks, kills sessions, fails open messages, closes the store and removes the address files in a fixed order that a comment explains. A reader cannot see the start-up order or the shutdown order without reading both in full, and the clean-up on a failed start is written four times by hand.

A real defect hides in the repeated clean-up. The start-up writes the token files before it binds, and removes them again only in four hand-written places (a failed link-token write, a failed admin-token write, a failed bind, a failed link-address write). Five gaps remain: `listener.local_addr()?` failing, the first `write_state_file(&path, addresses)` failing and the link-address failure (which removes the address files but leaves the tokens) all leave the tokens; the first gateway-token write and the admin-token write can leave a partial file of their own (the admin-token clean-up removes the other two but not itself); and the bind-failure clean-up leaves a stale `run/addr` from a crashed earlier daemon. After any of them the admin and gateway token files are left on disk for a daemon that is not running, which the code's own comment at the token write says must never happen ("an early startup error never leaves a token for a daemon that is not running"). Backlog F94 records it as B1. The test `partial_bind_failure_releases_addresses_and_lock_and_parents_are_private` also asserts the absence of `home.join("addr")`, a path that is never created (the address file is `run/addr`), so that assertion cannot fail.

Target shape, a new module `src/daemon/lifecycle.rs` holding `impl Daemon` and the helpers, with `Daemon`, `DaemonConfig` and `Runtime` staying in `daemon.rs`:

```rust
impl Daemon {
    pub async fn start(config: DaemonConfig) -> anyhow::Result<Self>;   // the orchestration only, about 60 lines
    pub fn addrs(&self) -> &[SocketAddr];
    pub fn host_name(&self) -> &str;
    pub async fn stop_requested(&self);
    pub async fn shutdown(mut self) -> anyhow::Result<()>;              // the order only, about 25 lines
}
// private helpers in lifecycle.rs, each one step:
async fn acquire_lock(layout: &Layout) -> anyhow::Result<File>;
async fn publish(layout: &Layout, token: &str, gateway_token: &str, listen: Vec<SocketAddr>)
    -> anyhow::Result<(Vec<TcpListener>, Vec<SocketAddr>)>;     // write tokens, bind, write addresses: plain `?`
async fn rollback(layout: &Layout, error: anyhow::Error) -> anyhow::Error;  // remove the five state files, append a cleanup failure to the error
fn spawn_purge(store: Store, stopping: watch::Receiver<bool>) -> JoinHandle<()>;
fn spawn_omp_usage(runtime: Arc<Runtime>, stopping: watch::Receiver<bool>) -> JoinHandle<()>;
fn spawn_listener(listener: TcpListener, runtime: Arc<Runtime>, stopping: watch::Receiver<bool>) -> JoinHandle<()>;
```

`rollback` removes the five state files that start-up can have written (`admin_token`, `gateway_token`, `link_gateway_token`, `addr`, `link_addr`; absent files are ignored as `remove_state_files` does today) and is called once, on any error out of `publish`; it replaces the four hand-written clean-ups and closes the three gaps. `shutdown` keeps its order and its two different failure semantics, split into three private methods: `stop_tasks` joins the listener, purge, OMP and presence tasks with `?` and returns at once on a `JoinError` (the later steps are skipped, as today; the lock is still released by the drop of the `File`); `stop_deliveries` and `stop_sessions` return `Option<anyhow::Error>` that `shutdown` feeds to `failure.get_or_insert` (first failure wins; the later steps still run); the final address-file removal stays `.await??` and its error overrides a remembered `failure`, as today.

Rejected: a `Drop` guard for the published files (the failure path is async and the clean-up uses `spawn_blocking`; an explicit `rollback` is simpler and testable); restructuring `Runtime` construction into a builder (one struct literal at one place); changing the shutdown order (it is pinned by comments and tests); moving `DaemonConfig`, `Daemon` or `Runtime` (public type, fields every sibling uses); the rename of the `Runtime.deliveries` handle vector and its pruning on kill (it touches `launch.rs` and changes behaviour: a separate spec if wanted).

## 2. Mechanical and deliberate changes

### 2.1 New `src/daemon/lifecycle.rs`

Move `impl Daemon` (`start` `:286-528`, `addrs`, `host_name`, `stop_requested`, `shutdown` `:544-617`), `impl Drop for Daemon` (`:620-633`), `remove_state_files` (`:646-663`) and `write_state_file` (`:665-675`) out of `daemon.rs`, then restructure as in section 1:

1. `start` becomes: `mark_inherited_fds_cloexec`, executable, `bwrap_path`, `env_path`, `layout.ensure()`, `let lock = acquire_lock(&layout).await?;` (the binding stays alive until `start` returns, across `publish` and `rollback`: not `let _ =`; a second daemon cannot exist for this root while the lock is held, and "another daemon is running" fails inside `acquire_lock`, before `publish`, so `rollback` never deletes a running daemon's files), tokens and host name and boot (the `spawn_blocking` block moves unchanged), `Store::open`, then `let published = publish(...).await; let (bound, addrs) = match published { Ok(published) => published, Err(error) => return Err(rollback(&layout, error).await) };`, then the `Runtime` literal (unchanged), the four spawns through the helpers, and `Ok(Self { ... })`.
2. `publish` writes the gateway token, the link gateway token and the admin token (the same order and modes as today), binds every listen address (the empty list still means `127.0.0.1:0`), takes `local_addr` of each, and writes the address file and its link; no clean-up inside; every error returns through `?`. The error context text of a failed write or bind is unchanged.
3. `rollback` is `tokio::task::spawn_blocking` over `remove_state_files(&[admin, gateway, link_gateway, addr, link_addr])`; on success it returns `error` unchanged, on a cleanup failure it returns `anyhow::anyhow!("{error}; failed to remove state files after a failed start: {cleanup}")`.
4. `spawn_purge`, `spawn_omp_usage` and `spawn_listener` are the three `tokio::spawn` bodies of `start` (`:434-516`) moved unchanged, each taking what it captured today. The presence task stays inline (one line).
5. `shutdown` keeps its exact sequence and failure handling as `stop_tasks`, `stop_deliveries` (returns the first delivery failure) and `stop_sessions` (returns the first kill, fail-open or close failure), called in that order with the semantics of section 1; the final address-file removal and lock release stay in `shutdown`.
6. The `Daemon` field `omp_usage` (the poller's handle) is renamed `omp_usage_poller` (it shares a name with the `Runtime.omp_usage` state); `daemon.rs` and `lifecycle.rs` are the only users (the only readers of the field are `start` and `shutdown`; every other `.omp_usage` in `admission.rs`, `attach.rs`, `views.rs` and `daemon.rs` is `Runtime.omp_usage`, which is not renamed; the compiler will find every use).

### 2.2 `src/daemon.rs`

Delete the moved items and declare `mod lifecycle;`. `random_hex`, `current_exe`, `constant_time_eq`, `parse_message_id`, `failed`, `lookup` stay (called from other modules). `Daemon` and `DaemonConfig` stay; `lifecycle.rs` builds `Self { ... }` (a child module sees the parent's private fields). Imports move with the code; `daemon.rs` loses the `use` lines nothing there uses (clippy says which).

### 2.3 `tests/daemon/daemon_lifecycle.rs`

- Fix the vacuous assertion in `partial_bind_failure_releases_addresses_and_lock_and_parents_are_private`: `home.join("addr")` becomes `home.join("run/addr")`.
- Add one test, written first and red on the old code: `failed_start_after_the_tokens_are_written_removes_every_state_file`. It creates `<root>/run/addr` as a directory (so the address-file write fails after the tokens were written and the listener bound), calls `Daemon::start` with `listen: vec!["127.0.0.1:0"]`, asserts it returns an error whose text contains the original write failure (`Is a directory`) followed by `failed to remove state files after a failed start` (the directory also makes `remove_file` fail with `EISDIR`, which is not `NotFound`, so this test drives `rollback`'s clean-up-failure arm; `remove_state_files` still removes the other four files because its loop continues; do not change `remove_state_files` to ignore it), then asserts `run/admin.token`, `run/gateway.token`, `run/link/gateway.token` and `run/link/addr` do not exist and that a second start after removing the directory succeeds (the lock was released; call `shutdown` on it). On the old code the tokens remain, so the test fails there.

## 3. Behaviour

One deliberate change: after any failure inside `publish` (after the first token file is written), the admin token, gateway token, link gateway token, address and link address files do not exist when `Daemon::start` returns. Everything else is unchanged: the order of start-up steps, the file modes, the listener loop, the task set, the shutdown order and its failure handling, the text of the existing error messages (a failed write or bind reports the same error; a failed clean-up now reports "failed to remove state files after a failed start" instead of the three older wordings, which no test pins: `rg -n 'failed to remove' tests src` hits only `daemon.rs`). Two small consequences of the single `rollback`: a bind failure now also removes a stale `run/addr` and `run/link/addr` left by a crashed earlier daemon, and a partial token file left by a failed write is removed too.

## 4. Out of scope

- The `Runtime.deliveries` rename and the pruning of finished handles; the OMP usage probe running `argv[0]` of an OMP session unconfined (consultant note from 3gc; separate security review against `docs/philosophy.md`); persistent sessions; changing the lock or token generation.

## 5. Trust-boundary check and acceptance

Attack paths: (a) a token file left behind by a failed start lets a later process that reads `run/admin.token` believe a daemon is running and send the admin token to whatever binds the address next: removed by `rollback`, which now covers every failure after the first token write; abuse-case test: the new test (fails on the old code); (b) a clean-up that itself fails leaves a token: constrained, the error reports it and names the files; the new test drives exactly this arm (the success arm is driven by `partial_bind_failure_releases_addresses_and_lock_and_parents_are_private`, whose bind failure now goes through `rollback`); (d) a stale `run/addr` from a crashed daemon beside a fresh `admin.token` between the token write and the address write: pre-existing, narrow (the stale port must be bound by someone else in that window), not changed here; the mitigation (remove the address files before writing the tokens) changes the start-up order and is a backlog follow-up; (c) files created with a wider mode: unchanged, `write_state_file` still sets 0600 (existing tests pin the modes).

- `cargo test --no-fail-fast`: all pass; the new test exists and was seen to fail on the old code (the developer writes it first and records the failing output); baseline at the commit before this spec: 66 suites, 1415 passed, 3 ignored (architect states it), after: 1416 passed (the baseline is the commit that accepts 3gj, which adds no test).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'fn start\b|fn shutdown\b|impl Drop for Daemon|fn remove_state_files|fn write_state_file' src/daemon.rs` prints nothing; `rg -n 'fn (acquire_lock|publish|rollback|spawn_purge|spawn_omp_usage|spawn_listener|stop_tasks|stop_deliveries|stop_sessions)\b' src/daemon/lifecycle.rs` prints 9 lines; `rg -n 'remove_state_files' src/daemon/lifecycle.rs` prints the definition, the use in `rollback` and the use in `shutdown` only (the old four hand clean-ups are gone).
- Function length: the developer reports `start` and `shutdown` line counts; target at most 80 and 40 (a report item, not a gate).
- `git status --short` lists only `src/daemon.rs`, `src/daemon/lifecycle.rs`, `tests/daemon/daemon_lifecycle.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Diff check (tester, mechanical): the three `tokio::spawn` bodies and the `Runtime` literal are byte-identical to `start`'s at the base commit apart from captured-variable plumbing; `shutdown`'s steps appear in the same order with the same failure handling; nothing else in `daemon.rs` changed.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant; survivors reported: `rollback` removes only the admin token (the new test); `rollback` is not called on an error from `publish` (the new test); `stop_sessions` skips `fail_open` (candidate: the shutdown-and-restart test in `tests/broker/limits_and_lifecycle.rs` near `:600-633`, which asserts the `daemon_stopped` detail; plausible, may survive if reopen also writes it); `stop_deliveries` skipped (no killer identified: expected survivor, the tester searches); `stop_tasks` turned into first-failure-wins (not observable without injecting a join error: expected survivor). Equivalent, not listed: writing the link address before the address file; releasing the lock before the address files are removed (both finish before `start`/`shutdown` returns).
- Known flaky tests: those that wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| `start` is 243 lines, `shutdown` 74; ranges listed | brace-matching script over `src/daemon.rs` at 1b8f3b4 | probed |
| the four hand clean-ups are the link-token write failure, the admin-token write failure, the bind failure and the link-address write failure; `local_addr()?` and the first address write have none | `src/daemon.rs` `Daemon::start` | read |
| the address file is `run/addr` and the link files `run/link/addr`, `run/link/gateway.token`; the existing assertion `home.join("addr")` checks a path never created | `src/layout.rs:69-92`, `tests/daemon/daemon_lifecycle.rs:165` | read |
| a directory at `run/addr` makes `write_state_file` fail (open for write on a directory) after the tokens exist and the listener is bound | `OpenOptions::open` on a directory returns `EISDIR` | to confirm by running the new test against the old code before the fix |
| `layout.ensure()` only checks and creates the eight directories it lists (`run_dir` among them) and never touches `run/addr`, so a pre-created `run/addr` directory survives into `publish` | `src/layout.rs:132-160` | read |
| no test or code outside `daemon.rs` pins the clean-up failure wordings | `rg -n 'failed to remove' src tests` | probed (hits only in `daemon.rs`) |
| `remove_state_files` and `write_state_file` have callers only in `Daemon::start`/`shutdown` | `rg -n` | probed (no hit outside `daemon.rs`) |
| `random_hex`, `current_exe` are called from outside the file (`relay.rs`, `relay_files.rs`, `commands/`) and stay | `rg -n` | probed |
| test killers per mutant | repo tests | consultant and tester to name |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer antigravity Gemini 4 Hydrogen (Medium) (the senior developer); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the spec described `shutdown`'s failure handling as first-failure-wins for every step; the four task joins use `?` and return early, only deliveries and sessions remember the first failure, and the final removal's `??` overrides | the two semantics stated per step; the tester's diff check named them |
| draft review | consultant | the new test drives `rollback`'s clean-up-failure arm (a directory also fails `remove_file` with `EISDIR`), which the spec did not say; the spec called that arm untestable | stated; the test asserts both the write failure and the clean-up text |
| draft review | consultant | the defect list was incomplete (five gaps: three leaked tokens, partial token files, a stale address file on bind failure); the lock binding had to stay alive across `rollback`; two mutants were equivalent or killerless | list, instruction and mutants corrected |
| draft review | consultant | the stale `run/addr` beside a fresh token between the token write and the address write | recorded as pre-existing, backlog follow-up |
| implementation | architect | `rollback` had two identical error arms (Sonar identical-branches smell) | merged into one with `map_err` and `and_then` before the gate |
| implementation | architect | the `Runtime` literal spelled `std::collections::HashMap::new()` in full (not byte-identical to the base) | `use std::collections::HashMap;` added |
| tester pass | tester | RED on the old code is the missing clean-up-phrase `expect`, not the token-absence assertions (those are never reached) | recorded; token retention on the old code is source-level evidence |
| tester pass | tester | `stop_deliveries` skipped is killed by `graceful_restart_marks_exited_and_continues_ids` (spec expected a survivor) | recorded |
| tester pass | tester | rollback on the acquire-lock error (deleting a running daemon's files) survived: nothing pinned that refusal leaves the running daemon's files | `state_is_owner_only_and_exclusively_locked` now reads the five files before the refused second start and asserts them unchanged after; the architect re-ran the mutant and it is killed |
| tester pass | tester | `stop_tasks` first-failure-wins survives (needs join-error injection); `start` is 86 lines (target 80, report only) | expected survivor; recorded |
