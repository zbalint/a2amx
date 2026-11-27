# Spec 3ho: function-level code health of `src/daemon.rs` and `src/daemon/lifecycle.rs`

## 0. Status

LOCKED after consultant review m_9650 (existing reset pins named, gateway arm unreachable so mutant (b) dropped, mutants (f)/(i) expected non-kills, audit-line exception stated, hand counts). Context `a2amx-3ho-daemon-lifecycle-reset-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `adcec92`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`tests/common`, `Client`), the daemon start and shutdown API used by `tests/daemon/daemon_lifecycle.rs`.

Scope (edit): `src/daemon.rs`, `src/daemon/lifecycle.rs` (production code; neither has a `#[cfg(test)]` module) and, only if Step A finds an unpinned behaviour, new tests appended to the existing files named in Step A (`tests/broker/reset_basics.rs`, `tests/teams/team_visibility.rs`, `tests/daemon/daemon_lifecycle.rs`), never a new file. Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same responses and texts, same check order, same startup and shutdown order, same state-file handling, except the one audit-log target named in section 3.

## 1. Why

`cs check` (`adcec92`) reports: `daemon.rs:192` `reset_audit` (5 arguments, counting `self`, which it does not use); `daemon.rs:202` `reset_session` (cc 11); file level "String Heavy Function Arguments" on `daemon.rs`; `lifecycle.rs:30` `Daemon::start` (Large Method, 82 lines); `lifecycle.rs:363` `spawn_listener` (Bumpy Road, 2 bumps).

Rules that bind (earlier refactors 3hk-3hn): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function; split along contiguous runs; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

The developer first lists, per behaviour below, the test that already pins it (file and name). A behaviour with no pin gets a new test, appended to the existing test file that owns the behaviour (`tests/broker/reset_basics.rs` for reset permission and the exited refusal, `tests/teams/team_visibility.rs` for reset visibility, `tests/daemon/daemon_lifecycle.rs` for start and shutdown), with literal expectations, green on the unchanged code.

1. Reset refusals, each with its code and message: unknown target (`unknown_session`, "unknown session"); a session sender that cannot see the target (same code and text, before `control_from` is consulted; existing `reset_visibility_fails_before_control_from`); a session sender not in the target's `control_from` and a session sender resetting itself (`not_permitted`, "sender is not permitted to reset this session"; existing `reset_authorizes_controller_and_rejects_unlisted_or_self` in `tests/broker/reset_basics.rs`, the developer confirms its assertions name the code and text); an admin reset that succeeds (`Reset { steps }`, existing `tests/daemon/confine.rs`); a failed step carrying the sequence error code and message (candidates `tests/broker/main.rs:293-372`, `reset_in_flight.rs`). Two pins are new unless the developer finds them: an exited target reset by a live, visible session that is not in the target's `control_from` expects `not_permitted` (permission before exited), and an exited target reset by admin expects code `exited` and text "session has exited" asserted literally (the CLI test `tests/daemon_cli/reset.rs:169-179` checks only a stderr substring). A gateway token never reaches `reset_session`: `trust.rs` classifies `Reset` as admin-or-session and `serve` refuses the gateway first (`tests/human_inbound/gateway_capability.rs`); the `Gateway` arms of the three reset functions are kept for exhaustiveness and are not pinned.
2. `start`: a second daemon on the same root is refused with "another a2amx daemon is running for this a2amx directory"; an invalid host name fails with "invalid host name"; a failed publish removes every state file (existing `failed_start_after_the_tokens_are_written_removes_every_state_file`, `partial_bind_failure_releases_addresses_and_lock_and_parents_are_private`).
3. `shutdown`: listeners stop, sessions are killed gracefully unless `now`, state files removed (existing `kill_removes_running_session_and_shutdown_disconnects_attachments`).

The reset audit line (`tracing::info!`, "session reset attempt") has no test and is not pinned; its fields are unchanged except the one named in section 3.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon.rs`

- `fn reset_audit(sender: &str, target: &str, outcome: &str, step: Option<(usize, usize)>)`: an associated function without `self` (it never used it), four parameters; callers use `Self::reset_audit`.
- `fn reset_sender_label(&self, role: &Role) -> String`: the sender text of today's first `match` (`"admin"`, `"gateway"`, or the session's address).
- `fn reset_visible(&self, role: &Role, target: &Session) -> bool`: true for `Admin` and `Gateway`; for a session sender, true when the sender session exists and `messaging::visible(sender.party(), target.party())`.
- `fn reset_permitted(&self, role: &Role, target: &Session) -> bool`: today's `permitted` match (`Admin` true, `Gateway` false, a session only when it is not the target and its name is in the target's `control_from`).
- `fn reset_refusal(&self, role: &Role, target: &Session) -> Option<(&'static str, &'static str)>`: `(code, message)` of the first failing rule in today's order: not visible (`UNKNOWN_SESSION`, "unknown session"), not permitted (`NOT_PERMITTED`, "sender is not permitted to reset this session"), exited (`EXITED`, "session has exited"); `None` when all pass.
- `reset_session` keeps: sender label, target lookup (audit with the reference and `UNKNOWN_SESSION` on failure), `reset_refusal` (audit with the target address and the code, then `failed`), `reset_sequence` with its two outcomes.

One deliberate audit change: when the sender session of a `Role::Session` token has vanished, today's audit line names the caller's `reference` as target; after the split it names the target address like the visibility refusal. The response is unchanged. Log-only, reachable only through a race (the serve loop closes exited sessions before dispatch).

### `src/daemon/lifecycle.rs`

- `fn generate_identity(host_name: Option<String>) -> anyhow::Result<(String, String, String, String)>`: the blocking closure of `start` (admin token, gateway token, validated or default host name, boot id), run through `spawn_blocking` by `start`.
- `struct Tasks { listeners: Vec<JoinHandle<()>>, purge: JoinHandle<()>, omp_usage_poller: JoinHandle<()>, presence: JoinHandle<()> }` with `fn spawn(runtime: &Arc<Runtime>, shutdown: &watch::Sender<bool>, bound: Vec<TcpListener>) -> Self` (today's four spawn blocks, same order) and `async fn stop(self) -> anyhow::Result<()>` (today's `stop_tasks`: listeners, purge, omp poller, presence, each awaited with `?`). `Tasks` is `pub(super)` and `Daemon` (struct in `daemon.rs`, imported with `use lifecycle::Tasks;`) holds `tasks: Option<Tasks>` in place of the four fields `listeners`, `purge`, `omp_usage_poller`, `presence`; `shutdown` takes it (`self.tasks.take()`), `Drop` is unchanged.
- `async fn serve_until_stopped(socket: TcpStream, runtime: Arc<Runtime>, mut stopping: watch::Receiver<bool>)`: the body of the per-connection task of `spawn_listener` (select between the stop signal and `serve`, the `tracing::debug!` on error).
- `start` keeps: exe, paths, layout, lock, identity, store, publish with rollback, runtime literal, `Tasks::spawn`, the `Daemon` literal.

If a finding remains after this shape the developer reports the per-function cc and stops; no extra split without the architect. The file-level "String Heavy Function Arguments" on `daemon.rs` may clear because the new functions take `&Role` and `&Session`, which dilutes the share of string-argument functions; if it stays, or a new one appears on `lifecycle.rs` (`generate_identity` takes an `Option<String>`), the developer reports it and the architect decides. Hand count: `start` about 57 lines, `reset_session` cc 4.

## 4. Out of scope

Other `daemon/` files; `Runtime` fields; the `Request` and wire formats; changing the shutdown order or what `stop_sessions` and `stop_deliveries` do; any change to `serve.rs`.

## 5. Trust boundary

No boundary is created or moved. The reset permission verdict (admin yes, gateway no, a session only when named in the target's `control_from` and visible) is the same rule in the same order; abuse-case coverage is the existing `team_visibility` and `gateway_capability` pins plus the Step A pins above.

## 6. Acceptance

Baseline (`adcec92`): `cargo test` passes (1526 passed, 3 ignored); whole-tree cs count 862.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon.rs
cs check src/daemon/lifecycle.rs
git status --short
```

- All pass; `cs check` lists no finding on `daemon.rs` and `lifecycle.rs` (or the developer reports each remaining one with its cc); the whole-tree count is below 862 (judge: the architect's `cs` run).
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources).
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3ho/` (deleted before its REPORT); per mutant run `cargo test --lib` and the targets `daemon`, `broker`, `team`, `teams`, `daemon_cli`, `client_requests`, `human_inbound`, `bridge`, `presence`, `gateway_up`. Mutants, one change each: (a) `reset_visible` true for every session sender; (b) dropped (the `Gateway` arm is unreachable, see Step A); (c) `reset_permitted` drops the self-reset exclusion; (d) `reset_refusal` checks exited before permission; (e) `reset_refusal` uses the wrong code for exited (killed only by the new literal pin); (f) `Tasks::stop` skips the purge task (expected non-kill: the task ends on the same stop signal and nothing observes it); (g) `Tasks::spawn` omits the presence task; (h) `generate_identity` skips host validation; (i) `serve_until_stopped` ignores the stop signal (expected non-kill: `spawn_listener` aborts and drains every connection task after the loop); (j) `reset_sender_label` returns the session id instead of the address (expected non-kill if only the log shows it). Any survivor in moved code is a fix request; an expected non-kill is reported with its reason.

## 7. Claims (architect, `adcec92`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `daemon.rs:192,202`, `lifecycle.rs:30,363` | `cs check` run, output read |
| `reset_audit` does not use `self` | `daemon.rs:192-200` | read |
| the audit line is untested | "session reset attempt" | grep of `src tests docs` |
| `stop_tasks` has one caller (`shutdown`) | `lifecycle.rs:131-133` | read |
| the `Daemon` fields `purge`, `omp_usage_poller`, `presence`, `listeners` are used only in `lifecycle.rs` | `daemon.rs` struct, `lifecycle.rs` | grep (developer to re-confirm before editing) |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-senior-developer` on the agy harness, planning and working model Gemini 4 Hydrogen (Medium), no advisor (single-slot harness); tester `a2amx-tester` on the omp harness, planning and working model `opencode-go/deepseek-v4.1-flash` (advisor model not exposed).

BLOCKED/amend cycles: none. Three fix rounds after the implementation.

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | Step A omitted the existing reset pins (`tests/broker/reset_basics.rs`) and two mutant targets (`team`, `daemon_cli`); section 0 contradicted Step A on test files | Spec amended before the lock |
| Draft review | consultant | The gateway reset claim was false at this layer: `trust.rs` refuses a gateway token before `reset_session`, so mutant (b) was equivalent | (b) dropped, the `Gateway` arms stated as kept for exhaustiveness |
| Draft review | consultant | Mutant (d) would survive without a pin for an exited target reset by a visible, unpermitted session; mutants (f) and (i) are equivalent through the public seams; the no-behaviour-change sentence conflicted with the audit-target change; the String Heavy mechanism was misstated | New pins named, (f)/(i) expected non-kills, the exception stated, the mechanism reworded |
| Implementation | architect | `spawn_listener` kept its Bumpy Road (bumps = 2) after the specified shape, although the spec expected it to clear | Fix round 1: `spawn_connection` extracted; cs 10.00 on `lifecycle.rs` |
| Implementation | architect | The new exited/unknown test was an 89-line Large Method, a new cs warning in `reset_basics.rs` | Fix round 2: split into three small tests with one helper |
| Tester pass | tester | Mutant (c), `reset_permitted` without the self-reset exclusion, survived: the only self-reset test used an empty `control_from` | Fix round 3: `reset_rejects_self_even_when_listed_in_control_from`, red on the mutant |
| Tester pass | tester | The spec called the existing `reset_authorizes_controller_and_rejects_unlisted_or_self` a code-and-text pin; it asserts the code only | The new tests pin code and text literally; the claim is corrected here |
| Tester pass | tester | "Shutdown returned only after the task group finished" is unpinned (mutant l survives); also expected survivors (k) stop order, (m) accept-failure log, (n) identity swap | Left: the awaits are structural and the watch stop races any observer; diff-inspected |
| Design read | consultant | Nits: `spawn_connection` is better named `handle_accept`; a four-field `Identity` struct would remove the same-type tuple swap risk | Left: the tuple is HEAD's shape and the spec names it |
| Design read | consultant | The file-level "String Heavy Function Arguments" on `daemon.rs` has no file-local fix: the arguments are tokens, codes and session references; a `SessionId` newtype or typed error codes would cross `trust.rs`, `serve.rs` and `messaging` | Recorded as known and not fixed; a crate-wide typing spec would need its own justification |

Tester mutants (scratch with its own `CARGO_TARGET_DIR`, deleted afterwards): (a), (d), (e), (g), (h) killed; (f), (i), (j) expected non-kills; (c) survived and is fixed above. Architect spot-check (own scratch, deleted): `reset_refusal` skipping the permission check, killed by three reset tests including the new self pin; `Tasks::spawn` without the presence poller, killed by three `away_notice` tests. Fresh checks by the architect: `cargo test` 1531 passed, 3 ignored, `clippy -D warnings` and `fmt --check` clean, `cs` 10.00 on `lifecycle.rs`, `daemon.rs` 9.68 (the one file-level finding above), `reset_basics.rs` and `daemon_lifecycle.rs` unchanged in findings (pre-existing, for the tests batch), whole-tree cs warnings 862 to 858.
