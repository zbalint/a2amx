# Spec 3ga: the OMP usage poller leaves `src/daemon.rs` as one `OmpUsage` type (daemon S1)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol (`List`, `ListAgents`, `SendMessage`) in `tests/quota/omp_usage_daemon.rs`; `OmpUsage` is private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/omp_usage.rs`, `tests/quota/omp_usage_daemon.rs` (one new test).

Does not touch: every other file under `src/` (`src/quota.rs` included), every other test, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `daemon` row of `AGENTS.md` when committing.

## 1. Why

`src/daemon.rs` is 3,918 lines, the only `.rs` file over 1,000. The consultant's design read (m_8377; plan in `docs/backlog.md` F94) found eleven responsibilities in it. The OMP usage poller is the most self-contained: two `Runtime` fields (`omp_usage`, `omp_usage_attempt`), five constants, three methods, two free functions, one spawned task, and one shutdown join. Nothing else reads the fields except `Runtime::omp_usage()` (four call sites). Today those pieces sit 2,000 lines apart, so a reader cannot see that the cache, the attempt stamp and the probe form one unit.

Target shape: a private type `OmpUsage` in `src/daemon/omp_usage.rs` that owns the cache, the attempt stamp, the constants and the probe, with a three-method interface that names nothing else in `daemon`: `new()`, `current()` (the fresh-enough provider map) and `poll(command, bwrap_path, stopping)` (one throttled probe). `Runtime` holds one field of that type. The session lookup that finds the command to probe (`omp_usage_command`) stays on `Runtime`: it reads the session map, which the registry spec (S2) will own. The 2 s tick loop stays in `Daemon::start` (about 15 lines, with `OMP_USAGE_CHECK`), so `OmpUsage` depends on no other daemon item; the lifecycle spec (S7) reorganizes `start`.

Rejected: a trait or a closure for the command source (one caller; no second implementation); leaving `omp_usage_command` inside the new type (it would reach into the session map and `Session` accessors, so the type would depend on the registry that S2 redesigns); unit-testing `OmpUsage` directly (AGENTS.md: test at agreed seams; the seven daemon-backed tests already cover it).

## 2. Mechanical changes

### 2.1 New `src/daemon/omp_usage.rs`

Move, byte for byte unless noted: the four constants `OMP_USAGE_REFRESH`, `OMP_USAGE_STALE`, `OMP_USAGE_TIMEOUT`, `OMP_USAGE_MAX_OUTPUT` (`src/daemon.rs:48-51`; `OMP_USAGE_CHECK` at `:47` stays), `Runtime::omp_usage` (`:299-306`) as `OmpUsage::current`, `Runtime::poll_omp_usage` (`:329-364`) as `OmpUsage::poll`, `run_omp_usage` (`:2197-2258`) and `kill_omp_process_group` (`:2260-2272`).

```rust
pub(super) struct OmpUsage {
    cache: Mutex<Option<(Instant, BTreeMap<String, QuotaInfo>)>>,
    attempt: Mutex<Option<Instant>>,
}
impl OmpUsage {
    pub(super) fn new() -> Self;                                            // both None
    pub(super) fn current(&self) -> Option<BTreeMap<String, QuotaInfo>>;   // was Runtime::omp_usage
    pub(super) async fn poll(&self, command: Option<(String, Option<PathBuf>)>,
        bwrap_path: &OsStr, stopping: &mut watch::Receiver<bool>) -> bool; // true = shutdown
}
```

- `poll` is `poll_omp_usage` with `self.omp_usage_command()` replaced by the `command` argument and `self.bwrap_path.as_os_str()` by `bwrap_path`; the early `return false` on `None` stays the first statement, before the attempt stamp (a daemon with no OMP session must not stamp).
- The task block at `:2475-2491` stays in `Daemon::start`; its tick body becomes `runtime.omp_usage.poll(runtime.omp_usage_command(), runtime.bwrap_path.as_os_str(), &mut stopping).await`. `Daemon.omp_usage` (the handle) and its `shutdown` join (`:2570-2572`) are unchanged.
- The two moved `tracing::debug!` events change their log target to `a2amx::daemon::omp_usage` (as in spec 3fp); nothing filters on it (`EnvFilter::from_default_env` only, `src/main.rs:31`). This is the one effect that is not byte-identical.
- Imports move with the code; `src/daemon.rs` drops the ones only this code used (the compiler and clippy say which; the rustix `Pid`, `Signal` and `kill_process_group` imports go with `kill_omp_process_group`). `src/daemon.rs` declares `mod omp_usage;` and `use omp_usage::OmpUsage;`.

### 2.2 `src/daemon.rs`

- Delete the moved items. `Runtime` loses `omp_usage` and `omp_usage_attempt` (`:94-95`) and gains `omp_usage: OmpUsage` (`OmpUsage::new()` at `:2443-2444`). The field and the `Daemon.omp_usage` handle share a name; no rename here (backlog).
- The four `runtime.omp_usage()` / `self.omp_usage()` calls (`:1436`, `:3003`, `:3089`, `:3623`) become `.omp_usage.current()`.
- `Runtime::omp_usage_command` stays where it is and stays private (a child module sees its ancestor's private items); add no `pub`.
- Nothing else changes: no rename, no reorder, no comment edit.

### 2.3 `tests/quota/omp_usage_daemon.rs`

One new test, `omp_usage_poll_runs_the_json_probe` (a contract test: it passes on the old code, so it has no red phase). It builds a fake `omp` with `fake_executable` whose usage body is `printf '%s\n' "$*" > usage-args` then prints `OMP_USAGE_FIXTURE` (the poll runs in the session's cwd, as `omp_usage_poll_keeps_absolute_session_executable` relies on), starts a direct OMP session, then waits (5 s limit, 20 ms steps) until the content of `usage-args` equals `usage --json --no-extensions --redact\n`, because the shell creates the file before `printf` writes it; a final `assert_eq!` on the last content read. In Rust source the shell newline is written `\\n` inside the `format!` string, as at `tests/quota/omp_usage_daemon.rs:339`. It pins the probe's argument vector, which no test pins today.

## 3. Out of scope

- Any behavior change; any rename; the registry type (S2); moving `omp_usage_command`, `session_quota` or `quota` code.
- Injecting a clock to test `OMP_USAGE_REFRESH`, `OMP_USAGE_STALE` and `OMP_USAGE_CHECK` (the 300 s, 900 s and 2 s values cannot be observed in a test run). A mutant changing `OMP_USAGE_STALE` or `OMP_USAGE_CHECK` survives; so does one removing `.env_remove("A2AMX_TOKEN")` or `.env_remove("A2AMX_ADDR")`: no test observes the probe's environment. Recorded as unpinned, in the backlog (F94) when this spec is accepted.
- `AGENTS.md`, `CHANGELOG.md` (no user-visible change).

## 4. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `5b61dbc` (architect, 2026-11-21): 66 suites, 1413 passed, 0 failed, 3 ignored; after: 1414 passed (the one new test); the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'OMP_USAGE_(REFRESH|STALE|TIMEOUT|MAX_OUTPUT)|run_omp_usage|kill_omp_process_group|poll_omp_usage|omp_usage\(\)' src/daemon.rs` prints nothing.
- `rg -n 'OmpUsage|omp_usage::' src/daemon.rs` prints exactly three lines: `use omp_usage::OmpUsage;`, the `Runtime` field `omp_usage: OmpUsage`, and `OmpUsage::new()` (`mod omp_usage;` has no `::`, so it is not matched; amendment 1); `rg -n '\.omp_usage\.current\(\)' src/daemon.rs` prints exactly four lines (the former callers `:1436`, `:3003`, `:3089`, `:3623`) and `rg -n '\.omp_usage\.poll\(' src/daemon.rs` exactly one (the tick loop).
- `git status --short` lists only the three files of section 0; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- `src/daemon.rs` is shorter by 125 lines net (3,918 to 3,793; the moved block is about 140 lines, partly offset by the new field, import and call lines; amendment 1).
- Mutation check (tester, in the assigned checkout, restoring each file by checksum afterward): each of these makes a named repo test fail. `current()` returns `None`: `list_and_list_agents_report_omp_usage_percentages`. `poll` skips the attempt stamp: `failed_omp_usage_poll_is_not_retried_every_check`. `poll` ignores `cwd`: `omp_usage_poll_keeps_absolute_session_executable` and `omp_usage_poll_runs_the_json_probe` (both wait for a file the fake writes into the session cwd; under this mutant the fake runs in the package root, so the tester deletes the stray `usage-ran`, `usage-args` and `counter` files from the checkout before the status check). The timeout branch skips `kill_omp_process_group`: `omp_usage_timeout_kills_process_group`. `poll` stamps before the `None` command check: the poller's first tick runs at daemon start, before any session exists, so the stamp blocks the next 300 s and `omp_usage_poll_keeps_absolute_session_executable` (5 s wait) fails; `omp_wrapper_named_bun_is_not_polled` cannot fail on it (it expects no probe). An argument dropped from the probe: `omp_usage_poll_runs_the_json_probe`. The survivors of section 3 are reported, not fixed.
- Known flaky tests: the OMP usage tests wait on timers; a failure is rerun once alone and recorded as pre-existing if it passes.

## 5. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| The five constants (four move), two fields, three methods, two functions, the task and the join are the only OMP usage code in `daemon.rs` | `rg -n 'omp_usage\|OMP_USAGE\|run_omp_usage\|kill_omp_process_group' src` | read: hits at `:47-51,74,94-95,299-308,329-355,1436,2197-2260,2443-2444,2475-2491,2541,2570-2571,3003,3016,3089,3116,3432,3461,3623 (308, 3016, 3116, 3432, 3461 and 2541 stay: a stay-behind method, locals, the `session_quota` parameter and the handle in the `Daemon` literal)` |
| Nothing outside `daemon.rs` names these symbols | same grep | read: only `src/quota.rs:208 parse_omp_usage` (not moved) |
| `omp_usage_command` reads only the session map and `Session` accessors | `src/daemon.rs:308-327` | read |
| `poll_omp_usage` returns `false` before stamping when there is no command | `src/daemon.rs:330-332` | read |
| `OmpUsage::current` callers: four | grep above (`:1436,3003,3089,3623`; `:3432,:3461` are the `session_quota` parameter, not a call) | read |
| The probe's arguments and env removal are unpinned | `rg -n 'no-extensions\|--json\|A2AMX_TOKEN' tests/quota/omp_usage_daemon.rs` | read: only the bridged session script mentions `A2AMX_TOKEN` (for the session, not the probe) |
| `omp_usage_command` is reachable from the child module; the moved code needs no other private daemon item | `Runtime` is private in `daemon` (`:80`), descendants see ancestor-private items; the moved code uses `quota::parse_omp_usage`, `confine::find_executable`, tokio and rustix only | read (consultant m_8382) |
| Seven tests cover the poller | `tests/quota/omp_usage_daemon.rs:145,212,302,336,366,393,425` | read |
| `Daemon.omp_usage` handle is awaited in `shutdown` | `src/daemon.rs:2570-2572` | read |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (fresh consultant for the pre-lock review; no post-implementation design read, because the consultant had reviewed the target shape and the diff is a move); developer OMP openai-codex/gpt-5.6-luna (advisor slot not exposed); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0 (one wording amendment, below).

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the second acceptance grep's line list was incomplete (`omp_usage` also matches the stay-behind `omp_usage_command`, locals, a parameter and the `Daemon` literal) | grep rewritten as exact-pattern checks; lesson: build an expected-lines list from a grep of the draft, not from the intended changes |
| draft review | consultant | the wrong test was named for the "stamp before the None check" mutant | the absolute-executable test named; the bun test cannot fail on it |
| draft review | consultant | the new test could read `usage-args` after the shell created it and before `printf` wrote it | wait on the content, not on existence |
| draft review | consultant | the `cwd` mutant writes stray files into the checkout; the env_remove survivor note was wrong (a survivor even with the variables set) | tester told to delete them; wording fixed |
| draft review | consultant | `spawn_poller(Arc<Runtime>)` left `OmpUsage` depending on `Runtime` | tick loop stays in `Daemon::start`; `OmpUsage` names no other daemon item |
| architect review | architect | the developer added an explicit `target:` to the two moved log events (redundant: a module path is the default), put `mod omp_usage;` in the middle of the import list and left no blank line before the new test | fixed in a follow-up (m_8395); the spec said only that the target changes |
| tester pass | tester | the audit `rg 'OmpUsage|omp_usage::'` prints three lines, not four, and the file shrinks by 125, not about 140 | amendment 1 (wording) |
| tester pass | tester | three mutants survive: the 1 MiB output check, the staleness guard in `current()`, a success path that returns `true` (stops the tick loop) | not observable in a test run (300 s and 900 s timers); recorded in the backlog (F94) |
