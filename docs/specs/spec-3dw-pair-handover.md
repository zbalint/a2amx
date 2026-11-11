# Spec 3dw: `gateway up --pair` hands over to the background gateway

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-live-test-fixes-2026-11-10`. Follows spec 3dt (background gateway) and spec 3ds (pairing flow).

Scope (files that may be edited): `src/cli.rs` (one hidden flag), `src/main.rs` (`run_gateway` and its dispatch at the `GatewayAction::Run` arm), `src/relay.rs` (a pair-only entry and a shared connect step), `src/gateway_launch.rs` (the pair step and the hand-over), `tests/relay.rs` (new tests only), `tests/gateway_background.rs` (new tests only), `docs/remote-messaging.md`, `docs/cli.md`, `docs/architecture.md`, `docs/backlog.md` (the architect closes F86 and the F84 note at commit; the developer does not edit it).

Does not touch: `PairIo` and `Io` (their struct literals in `tests/relay.rs` stay unchanged), the pairing rules, the pairing prompts, the lock and pid file code in `src/gateway_proc.rs`, the sandbox argument vector (`sandbox_argv` is called twice with different `Layout` values, not changed), `CHANGELOG.md`.

Public test seams: the real binary through `tests/gateway_background.rs` helpers with `FakeDiscord` and real bwrap (skipped without bwrap, required in CI), and `relay::pair_only` with the existing `FakeDiscord` in `tests/relay.rs`.

## 1. Why

Owner change list item 5: `up --pair` must stay in the foreground only until the pairing is confirmed, then hand over to the background gateway like a plain `up`. After spec 3dt it stays in the foreground for ever, so closing the terminal after pairing still stops the relay.

Decision: two sandboxed steps, one lock. Step 1 runs `a2amx --home <state> gateway run --pair-only` confined in the foreground (the 3dt foreground shape: `--die-with-parent`, inherited terminal), pairs, and exits 0. Step 2, only when step 1 exits 0, is exactly the 3dt background start. The launcher takes the lock before step 1 and keeps it until step 2 holds it (the lock file descriptor is close-on-exec, so step 1 does not inherit it; the launcher owns it). Rejected: a hand-over inside the sandbox (a process cannot leave its own bwrap and its parent-death rules); reusing `gateway run --pair` and exiting from the relay (the relay would have to know it is being handed over, which changes `Io`).

## 2. Mechanical changes

2.1. `src/cli.rs`: `GatewayAction::Run` gets a hidden `pair_only: bool` (`#[arg(long, hide = true, conflicts_with = "pair")]`), documented in a code comment as the launcher's internal step. `gateway up` is unchanged.

2.2. `src/relay.rs`:
- Extract the part of `run` from the Discord setup to the `watch` into a function `async fn connect_control(loaded: &Loaded, discord: &Discord) -> Result<(String, Watch)>` (member-count check, category, channel, `discord.watch()`); `run` calls it and behaves as before (same error texts, same order: the daemon connection stays first in `run`).
- `pub async fn pair_only(loaded: Loaded, discord_config: DiscordConfig, mut io: PairIo) -> Result<()>`: first `Client::connect_gateway(&loaded.link_dir)` and drops it at once (a stopped daemon fails before the owner pairs, as `run --pair` does today; the link directory is read-only bound into the sandbox), then builds `Discord`, calls `connect_control`, calls `pair`, announces the success line, returns. It does not sync and does not print `gateway ready`.
- `pair` keeps its four parameters and no longer announces the success line; it returns the `Pairing`. The callers announce: `run` announces `paired with Discord account {id}; starting the gateway (Ctrl-C stops it)` right after `pair` returns (byte-identical to today), `pair_only` announces `paired with Discord account {id}; starting the gateway in the background`.

2.3. `src/main.rs`: `run_gateway(home, pair, pair_only)` builds the same `PairIo` (stderr announce, stdin confirm) and, for `pair_only`, calls `relay::pair_only` and returns its result (so a refusal or timeout is a non-zero exit with the error text).

2.4. `src/gateway_launch.rs`: `up(home, pair, foreground)` keeps its signature. When `pair && !foreground`:
1. same checks and lock as today (the lock is taken before step 1);
2. build the step-1 argv with the foreground `Layout` (`die_with_parent: true`, caller's `RUST_LOG`) and inner command `... gateway run --pair-only`; run it as a child with inherited stdin, stdout and stderr, write the CHILD's pid (the step-1 bwrap, whose command line carries `--home <state> gateway run --pair-only`, so `gateway stop` accepts it and signals that pid only) to `gateway.pid` right after the spawn, and wait. Spawn and wait on the same thread (`spawn_blocking` already provides one). `run_pair_step` runs before any `clear_cloexec`: the lock stays owned by the launcher only, and its prologue is NOT shared with `exec_gateway`. If the child fails to start, or exits with a non-zero status or by a signal, return an error `pairing did not complete; the gateway was not started` (the child already printed the reason) and release the lock; step 2 overwrites the pid file;
3. on exit 0, build the background argv exactly as a plain `up` does (the 3dt `Layout`: `die_with_parent: false`, default `RUST_LOG=warn`, inner `... gateway run`) and call `start_background(lock, ..)`. Its output line is the existing one. If step 2 fails after a successful pairing, the error adds: `the pairing is saved; run a2amx gateway up (without --pair) to start the gateway`.
`pair && foreground` keeps the 3dt behaviour (`exec_gateway` with `gateway run --pair`). Keep each function at four parameters or fewer; the pair step is its own function (`run_pair_step(&argv) -> Result<()>`), and `up` stays flat.
A Ctrl-C during step 1 reaches the launcher and its child (same process group, foreground terminal): both end, the lock frees, nothing runs in the background.

## 3. Documentation

3.1. `docs/remote-messaging.md` (replace the sentence at line 172 that says `--pair` stays in the foreground until spec 3dw; add a troubleshooting row for `pairing did not complete; the gateway was not started`) and `docs/cli.md`: `gateway up --pair` pairs in the foreground (code, confirmation prompt) and then continues in the background; `--pair --foreground` keeps everything in the foreground until Ctrl-C; a declined or timed-out pairing leaves no gateway running. Replace the 3dt sentence that says `--pair` stays in the foreground.
3.2. `docs/architecture.md`: the launcher row or paragraph names the two steps and the hidden `--pair-only` run.

## 4. Out of scope

Pairing rules or timeouts; changing `PairIo`, `Io` or the pairing prompts; Windows or non-bwrap paths; any change to `gateway stop` or `status`; a log of the pairing step (it prints to the terminal only); `CHANGELOG.md`.

## 5. Acceptance

Public interface change: `gateway up --pair` now returns after a confirmed pairing and leaves the gateway running in the background (it used to run until Ctrl-C); the old shape is `up --pair --foreground`. A new hidden flag `gateway run --pair-only`. Say both in the developer's report so the changelog line can name them.

1. `tests/relay.rs` (red first), `relay::pair_only` with `FakeDiscord`: it pairs, writes the pairing file, announces `paired with Discord account {id}; starting the gateway in the background`, returns Ok, never syncs and never prints `gateway ready`, and with no daemon behind the link directory it fails before pairing (a mutant that skips the connect fails this); a declined confirmation returns an error and writes no pairing; an existing foreground `run --pair` test still sees the old success line unchanged.
2. `tests/gateway_background.rs`, real bwrap, `FakeDiscord`, piped stdin: (a) `up --pair`, post the code the child prints, answer `y`: `up` exits 0, prints the background line, `gateway status` says running, the pairing file exists, `gateway.log` has the ready line and no `post this code` line (step 2 did not get `--pair`), the gateway process is in its own session, then `gateway stop` stops it; (b) answer `n`: `up` exits non-zero with `pairing did not complete`, no pairing file is written, `gateway status` says not running, and the lock is free (a second `up --pair` starts at once); (c) `up --pair` while the lock is held by a running gateway is refused with the existing message; (e) while step 1 waits for the code: `gateway status` says running with the step-1 pid, a second `up --pair` is refused with the existing message, and `gateway stop` ends it (`up` then exits non-zero); (f) SIGKILL the launcher (by pid) while step 1 waits for the code: within 2 s no process whose `/proc/*/cmdline` contains the test home and `--pair-only` remains (kill the rest by pid if it fails); (d) `up --pair --foreground` still stays in the foreground (the test ends it with SIGTERM to its own process group created by `process_group(0)`).
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `git diff --check`; `git diff` only in the files of section 0.
4. Mutation check (developer, restore by checksum, `timeout 120`, name the failing test per mutant): hand-over skipped (step 2 not started) (2a); step 2 started even when step 1 fails (2b); the lock taken only after step 1 (2e); the pid file not written for step 1 (2e); step 2 failing silently after pairing (not tested; review); `--pair-only` connects to the daemon or prints the ready line (1); `pair` success line changed in the foreground path (1); step 1 started without `--die-with-parent` (2f).
5. Trust boundary: none crossed; the child is the launcher's own executable with a fixed argument vector (no shell, no caller text), stdin is the owner's terminal, as in 3ds.

## 6. Claims table

| Claim | Where | Verified |
| --- | --- | --- |
| `up` builds one layout and either execs the gateway (foreground, pair) or starts it in the background | `src/gateway_launch.rs:89-132` | read |
| `start_background(lock, state_dir, argv)` clears CLOEXEC on the lock, opens the log, spawns with `setsid`, waits for the ready line and prints the background line | `src/gateway_launch.rs:159-193` | read |
| `run` connects to the daemon first, then Discord, then pairs, then prints `gateway ready` after the first sync | `src/relay.rs:30-90` | read |
| `pair` takes `(loaded, control_channel, watch, io)` and prints the 3ds success line with `(Ctrl-C stops it)` at its end | `src/relay.rs:313-365` (line at 360-363) | read (consultant) |
| `run_gateway(home, pair)` builds `PairIo` (stderr announce, stdin confirm) and calls `relay::run` | `src/main.rs:377-417`; dispatch at `src/main.rs:205`; `up` already runs on `spawn_blocking` (`src/main.rs:335-338`) | read (consultant) |
| `Io` and `PairIo` literals in `tests/relay.rs` (8 and 23) would need edits if their fields changed | grep | counted earlier |
| The lock file is opened close-on-exec, so a child spawned without clearing it does not hold it | `src/gateway_proc.rs:57,67` and `clear_cloexec` in `src/gateway_launch.rs` | read |

## Review record

Models: architect and spec author Claude Sonnet 5.5; consultant (one pre-lock review) Claude, model not reported; developer openai-codex/gpt-5.6-luna (planning and working, advisor slot not exposed); tester OMP, glm-5.3-flash for planning, working and advisor.

BLOCKED/amend cycles: 0. Fix rounds after the first implementation: 1.

| found at | by | problem | resolution |
|---|---|---|---|
| draft review | consultant | step 1 ran with the lock held and no pid file, so `status` and `stop` would show a stale pid or ask for a manual stop | spec 2.4: write the step-1 child's pid |
| draft review | consultant | `pair_only` did not connect to the daemon first, a regression against `run --pair` | spec 2.2: connect first and drop |
| draft review | consultant | `pair` with a fifth parameter; no test could catch "lock released between the steps" or "step 1 without `--die-with-parent`" | callers announce the success line; tests 2e and 2f replaced the weak mutants |
| tester pass | tester | step-2 failure after pairing untested (error text), no own-session assertion for the pair-launched gateway, "second `up --pair` starts at once" asserted only up to the prompt | FIX REQUEST m_6640 (own-session, control channel, completed second pairing); the step-2 text stays review-only as the spec says |
| architect review | architect | `pair_only_refuses_before_pairing_when_daemon_is_stopped` failed 5 of 5 for the architect (15 s timeout) after passing for the developer and tester | test fixed (daemon really gone before the call), ten green runs, mutant still kills it |
| architect review | architect | `run_pair_step` swallowed the underlying error in its `Err(_)` arms | the cause is appended to the fixed text |
| tester pass | tester | `stop_signals_verified_nonleader_only_without_killing_runner_group` failed twice (reads /proc before the child's argv is verifiable) | test polls until the command line is verifiable |

Process notes: the developer's follow-up edits, a `cargo fmt --all` and a `Cargo.lock` restore ran while the architect's full suite was running; the suite was stopped and rerun on a settled tree. A new advisory (RUSTSEC-2026-0486, `rustix` 1.1.5, an xattr API this repository does not use) failed `cargo deny`; the architect updated `Cargo.lock` to `rustix` 1.1.6 in its own commit.

Surviving mutant, accepted: the saved-pairing sentence removed from the step-2 failure error (the spec marks step-2 failure review-only).

Architect checks on the final snapshot: `git diff --check`, fmt, clippy `-D warnings`, `cargo deny check`, `scripts/prepush-fast.sh`, full `cargo test --no-fail-fast` (51 suites passed; only the two confined-HOME failures as in 3dt), own mutant (step 2 started with `--pair-only`: killed by two gateway_background tests), Sonar gate.
