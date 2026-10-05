# Spec 3j: OMP quota percentages from `omp usage --json`

## 0. Status

**LOCKED** (2026-10-05, after the consultant's design review `m_433`; gate notes in section 9). The owner
said the `omp usage` command (and `omp usage --json`; `omp usage --help` also offers `--redact`, `--no-extensions` and `--provider`) returns the quota that Claude and Codex
already report, and asked for OMP parity. Decisions D1 to D10 are in section 2; the architect
chose them (the owner delegated), marked **(architect)** where they are a choice.

**Baseline:** develop at the commit that accepted spec 3i (this spec is committed on top of it). **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-omp-usage`.
Public test seams: `a2amx::quota::parse_omp_usage` (pure) and `Request::List` through `Client`
against a real daemon with a fake `omp` script (`tests/quota.rs`).

**Scope.** May edit exactly these files and no others.

- `src/quota.rs`, `src/daemon.rs`, `src/wire.rs` (the doc comment of `QuotaInfo::limit_reached` only)
- `tests/quota.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`, `AGENTS.md` (the `quota` row of the
  Modules table only)

Does not touch: the fields of `QuotaInfo` (it already has `five_hour`, `weekly` and `limit_reached`),
`src/main.rs`, `src/status.rs`, every other test file, every other spec document. No new
dependency (`serde_json` and `tokio::process` are already used). Do not commit, stage or merge:
leave the diff uncommitted in the working tree.

## 1. Why

Claude and Codex sessions show `5h N% wk N%` because their status line says so (spec 2l). OMP's
footer does not, so spec 2p could only detect an exhausted session from the error row and report
`limit`. `omp usage --json` returns the real windows: for each provider a list of limits with a
window id (`5h`, `7d`) and `amount.remainingFraction`. The values belong to the account, shared by
every OMP session on it, so the daemon polls once and applies the result to every running OMP
session. The command's output also contains the account email, account and organisation ids and
reset-credit details (checked on a real run, values not recorded here): those must never be logged,
stored or put in a test.

## 2. Decisions

- **D1.** For a running `Harness::Omp` session, `quota` is `QuotaInfo { five_hour, weekly,
  limit_reached }` where `five_hour` and `weekly` come from the last fresh poll of `omp usage --json`
  and `limit_reached` from the existing screen detection (spec 2p). If neither source has anything,
  `quota` is `None`, as today. Claude, Codex and Generic are unchanged. This supersedes spec 2p D6 ("`five_hour` and `weekly` stay
  `None` for OMP"); the rest of spec 2p stands.
- **D2.** `quota::parse_omp_usage(json: &str) -> Option<QuotaInfo>` (new, pure, pub) reads only
  `reports[].limits[].window.id` and `reports[].limits[].amount.remainingFraction` (serde structs
  with those fields only; unknown fields ignored, including every identity field). A window id of
  `5h` fills `five_hour`, `7d` fills `weekly`. The percent is `round(remainingFraction * 100)`
  clamped to `0..=100`, rounded with `f64::round` (half away from zero: `0.005` gives `1`). **(architect)** With several limits for one window (several providers or
  accounts), the smallest percent wins: the most pessimistic value answers "can the agent act".
  `None` for invalid JSON, no `reports`, or no `5h`/`7d` limit.
- **D3.** `QuotaInfo.limit_reached` is not set by the poll. `exhausted()` already reports `5h`,
  `weekly` or both when a window is at 0; the screen sets `limit_reached` as today.
- **D4.** The daemon polls: a task started in `Daemon::start` beside the `purge` task, stopped by
  the same shutdown signal and awaited the same way. Every `OMP_USAGE_CHECK` (2 seconds) it looks
  for the lowest-id running session whose harness is `Omp` **and** whose `argv[0]` has the file name
  `omp` (the rule `Harness::infer` uses; a wrapper such as `bun x omp` or `nix run` is not polled, so
  the daemon never runs an arbitrary program). If there is one, and the time since the last
  *attempt* (success or failure; a new `Instant` in `Runtime` next to the cache) is at least
  `OMP_USAGE_REFRESH` (300 seconds) or there has been no attempt yet, it runs
  `<argv[0]> usage --json --no-extensions --redact` (`--no-extensions` so the user's extensions are
  not loaded by a background poll, `--redact` so the account email and ids are not even emitted) and,
  on success, stores `(Instant, QuotaInfo)` in `Runtime`. **(architect)** Using the session's own
  `argv[0]` means the daemon runs the same `omp` the user launched. It runs in the session's working
  directory with the daemon's own environment minus `A2AMX_TOKEN` and `A2AMX_ADDR`; a bare `omp` is
  resolved on the daemon's `PATH` (documented: the daemon's `PATH` may differ from the client's).
- **D5.** The command runs as an argument vector (`tokio::process::Command`, no shell), stdin and
  stderr null, in its own process group (`process_group(0)`), a 20-second timeout after which the
  child and its process group are killed (a `bun` or `node` grandchild must not survive), and the
  stdout capped at 1 MiB; it does not block the async runtime. Any failure (spawn error, timeout,
  non-zero exit, parse failure) keeps the previous cache and writes one `tracing::debug!` line
  naming the failure kind, never the output; the attempt time of D4 is set either way, so a failure
  is retried after `OMP_USAGE_REFRESH`, not every 2 seconds.
- **D6.** A cached value older than `OMP_USAGE_STALE` (900 seconds) is ignored, so a daemon that can
  no longer reach `omp` stops showing old percentages and falls back to the screen detection.
- **D7.** `session_quota` takes the cache snapshot as a parameter (`Option<QuotaInfo>`, already
  checked for staleness) so the free function stays free of `Runtime`; its four call sites pass
  `runtime.omp_usage()` (a small `Runtime` method returning the fresh value). In the `List` handler
  the snapshot is read once, before the sessions lock is taken, so lock order stays one-way. No behavior change for
  other harnesses.
- **D8.** The cell shows what `quota::cell` already renders (`5h 97% wk 5%`); when a window is 0 the
  existing `exhausted` logic applies. `quota::cell` is extended so a screen-detected limit is never
  hidden by healthy percentages: with `limit_reached` and at least one window the cell is the
  windows followed by ` limit` (for example `5h 97% wk 5% limit`); with `limit_reached` and no window it
  is `limit`, as today. No wire change.
- **D9.** Report-only, as in 2l and 2p: delivery, holds and message states do not change.
- **D10.** **(architect)** `omp usage` reports every authenticated account of every provider and
  does not say which one a session uses, so the minimum across reports (D2) is the accepted corner.
  Its effect reaches peers, not only the cell: an unused provider at 0% makes every polled OMP
  session `exhausted()`, and `send_message` then adds `recipient_quota` to its result (src/daemon.rs
  about lines 690 to 692); that is a notice, never a refusal (D9). Leave a `// shortcut:` comment:
  per-session provider matching, ideally by the OMP extension reporting its own usage through the
  bridge (it runs per session, knows its provider, spawns nothing and cannot race the session's
  credentials), is the upgrade. Also state in the README that the account-level values come from
  `omp usage`, which may refresh provider credentials like the interactive command does; the
  refresh interval is 5 minutes for that reason, and an OMP session running nothing costs nothing
  because no OMP session means no poll.

## 3. `src/quota.rs`

Add `parse_omp_usage` per D2 (private `#[derive(Deserialize)]` structs for the three nesting levels,
`#[serde(default)]` on every list and optional field so a missing key means "nothing", not an
error). Update the module doc comment (it now also parses a command's output). `read` and `cell` do
not change; `read_omp_limit` stays as the screen detector.

## 4. `src/daemon.rs`

`Runtime` gains `omp_usage: Mutex<Option<(Instant, QuotaInfo)>>` and a method
`fn omp_usage(&self) -> Option<QuotaInfo>` returning the value when `OMP_USAGE_STALE` has not
passed. Add the consts of D4 and D6 beside `ACTIVITY_QUIET`. Add the poller task (D4/D5) and its
handle on `Daemon` next to `purge`, shut down the same way (`stopping.changed()`). `session_quota`
becomes `session_quota(session: &Session, omp_usage: Option<QuotaInfo>) -> Option<QuotaInfo>`:
for `Harness::Omp` it merges per D1; the other harnesses behave as before. Update its call sites
(about lines 690, 1427, 1549 and 1804) to pass `runtime.omp_usage()`.

## 5. Tests (failing first, expected values are literals)

`tests/quota.rs`, pure (`parse_omp_usage`, fictional fixtures only; never a real account's
output):

1. A report with a `5h` limit `remainingFraction: 0.97` and a `7d` limit `remainingFraction: 0.05`
   gives `QuotaInfo { five_hour: Some(97), weekly: Some(5), limit_reached: false }`.
2. Two reports (two providers): `5h` fractions `0.97` and `0.40`, `7d` fractions `0.95` and `0.50`
   give `five_hour: Some(40)`, `weekly: Some(50)`.
3. Rounding and clamping: `remainingFraction: 0.004` gives `0`; `0.005` gives `1`; `1.5` gives `100`;
   `-0.2` gives `0`.
4. Only a `5h` limit gives `five_hour: Some(...)` and `weekly: None`.
5. `None` for each of: `""`, `"not json"`, `"{}"`, `{"reports":[]}`, a report whose only limit has
   window id `30d`.
6. Unknown and identity-like fields are ignored: the fixture carries extra keys (`email`,
   `accountId`, `metadata`) with obviously fictional values and the result is the same as test 1.

`tests/quota.rs`, real daemon with a fake `omp` (a `sh` script **whose file name is `omp`**, in a temp
dir, that prints a fixture JSON when `$1` is `usage` and otherwise sleeps, made executable; the
fake in test 11 is named `bun`):

7. Create a session with `harness: Omp` and the script as `argv`; within 15 seconds `Request::List`
   reports `quota == Some(QuotaInfo { five_hour: Some(97), weekly: Some(5), .. })`, and the
   `list_agents` quota for it is the same. (The existing OMP limit tests keep passing: a session whose screen
   shows the limit error and a poll with no data still reports `limit_reached`.)
8. A fake `omp` whose `usage` branch appends a line to a counter file and exits non-zero never
   produces a quota from the poll: during about 5 seconds the session reports `None`, the counter
   file has exactly one line (a failure is retried only after `OMP_USAGE_REFRESH`), and the daemon
   keeps running.
9. A fake `omp` that sleeps for 60 seconds in its `usage` branch does not block the daemon: `Request::List`
   still answers within 1 second and the session reports `None`. (The 20-second timeout itself is
   not waited for.)
10. `quota::cell` literals: `five_hour 97, weekly 5, limit_reached true` gives `5h 97% wk 5% limit`;
   `limit_reached` alone still gives `limit`; the existing cell tests are unchanged.
11. A fake script named `bun` (a wrapper) given `harness: Omp` is not polled: its counter file stays
   empty for about 5 seconds.

## 6. Documentation

- `README.md` (the QUOTA paragraph near line 156): for OMP the QUOTA column now shows the
  5-hour and weekly percentages left when `omp usage --json` works (polled by the daemon about
  once every five minutes from the running OMP session's own `omp`, only when that executable is
  named `omp`), and `limit` is appended when the screen shows the usage-limit error; values older
  than 15 minutes are dropped; the smallest value across providers and accounts is shown; the poll
  may refresh provider credentials as the interactive `omp usage` does, and the daemon's `PATH` is
  used for a bare `omp`.
- `docs/architecture.md` (the Quota paragraph near line 448): the poller, the refresh and staleness rule, the process-group kill, and that
  the command output is parsed for the two windows only and never logged.
- `docs/backlog.md`: Closed row `C14`, `OMP quota percentages from omp usage --json`, `spec 3j`.
- `AGENTS.md`: the `quota` row says it extracts quota from screen and conversation state and the
  `omp usage` command output.

## 7. Out of scope

- Per-session or per-provider matching, an `omp` path configuration, a configurable interval.
- Holding or refusing delivery on an exhausted quota (report-only, D9).
- Reset times (`resetsAt`), plan type, reset credits, or any other field of the command's output.
- Any change to Claude or Codex quota reading.

## 8. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported. Also
report `git status --short` showing only scoped files, the bare `cargo test` result with its
disposition, and a manual transcript on an isolated daemon (temp state dir) with a fake `omp`:
`a2amx new --detach --name omp-demo --harness omp -- <fake>`, `a2amx list` showing
`5h 97% wk 5%` within about 10 seconds. Do not run the real `omp usage` in tests or the transcript,
and do not paste its output anywhere.

## 9. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test --test quota` passes (22 tests) on `4d20479`, the commit that
  accepted spec 3i.
- `session_quota` callers and the `Runtime` literal (`src/daemon.rs`, about lines 690, 1127, 1427,
  1549, 1804): re-grepped by the developer's plan; every one is in scope.
- Real `omp usage --json` shape observed 2026-10-05: top-level `reports[]` with `provider`,
  `limits[]` (each `id`, `label`, `scope`, `window{id,label,durationMs,resetsAt}`,
  `amount{used,limit,remaining,usedFraction,remainingFraction,unit}`, `status`), `metadata` with
  identity fields, a top-level `capacity` summary. `omp usage --help` (same day) lists
  `--json`, `--redact`, `--no-extensions`, `--provider` and says reports are cached by `omp`
  (`omp usage invalidate`). Whether the command refreshes credentials was **not** verified; the
  5-minute interval and D10 record that.
- Rounding: `f64::round` of `0.005 * 100.0` is `1.0` (half away from zero), of `0.004 * 100.0` is `0.0`.
- Contradicts nothing locked except spec 2p D6, which D1 supersedes explicitly.
- Acceptance commands need the new code and run after implementation.

## Amendment 1 (2026-10-05): `cell` is extended; section 4 names the last-attempt field

The developer reported BLOCKED (`m_458`): section 3 says "`read` and `cell` do not change", while D8
and test 10 require `quota::cell` to append ` limit` when windows and `limit_reached` coexist.
Verified real: when D8 was added, the old sentence in section 3 was left behind. **Ruling:** D8 and
test 10 govern; section 3's sentence reads "`read` does not change; `cell` is extended as D8 says;
`read_omp_limit` stays as the screen detector." While re-reading the spec for the gate I also found
that section 4 describes only the cache and not the last-attempt `Instant` that D4 and D5 require:
section 4's first sentence reads "`Runtime` gains `omp_usage: Mutex<Option<(Instant, QuotaInfo)>>`,
`omp_usage_attempt: Mutex<Option<Instant>>` (the last poll attempt, success or failure) and a method
`fn omp_usage(&self) -> Option<QuotaInfo>` ...". The `session_quota` call sites are now at about lines
693, 1433, 1562 and 1817 of `src/daemon.rs` (`rg -n session_quota src/daemon.rs`). Nothing else
changes.

Gate re-run for the amendment: `rg -n "do not change|does not change|unchanged" docs/specs/spec-3j-omp-usage-quota.md`
leaves only D9 (delivery), section 7 (Claude and Codex reading) and test 10's "existing cell tests are
unchanged" (the existing cases keep their results; only the new combined case differs), none of which
conflicts with D8. A grep of "last attempt" and "attempt" finds D4, D5, test 8 and this amendment, all
consistent.
