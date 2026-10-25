# Spec 3an: OMP quota follows the session's current provider

## 0. Status

**LOCKED** (2026-10-25). Architect-directed, context `a2amx-omp-quota-provider-2026-10-25`. Implementation goes to `a2amx-developer` only. Builds on spec 3j (OMP usage quota).

**Owner decisions (2026-10-25):** a session's quota is the quota of the provider its model uses right now, not the minimum across providers; a session changes model during its life (developer: Sol then Luna on `openai-codex`; tester: Sol on `openai-codex` then GLM on `opencode-go`), so the provider is sampled repeatedly, not read once.

**Baseline:** develop at 9c81806: `env -u A2AMX_BIN -u NO_COLOR cargo test` exits 0 with no failures, `cargo clippy --all-targets -- -D warnings` clean, `cargo fmt --check` clean.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `quota::parse_omp_usage` (pure), the bridge frame through `omp-bridge` as `tests/omp_extension/` and the bridge tests drive it, and the daemon through `Client` requests as `tests/quota.rs` does.

**Scope.** May edit exactly these files and no others; the developer finds the exact test files with `rg`.

- `extension/omp.ts` (embedded by `src/omp.rs` with `include_str!`; `src/omp.rs` itself needs no edit)
- `src/wire.rs` (`BridgeUp::State`; `src/bridge.rs` needs no edit because it parses and re-serializes typed frames), `src/session.rs`, `src/daemon.rs`, `src/quota.rs`, `src/channel.rs` (its two `BridgeUp::State` literals gain `provider: None`)
- `tests/quota.rs`, `tests/omp_extension/*`, `tests/bridge.rs` and `tests/broker.rs` (the only permitted change in the last two is adding `provider: None` to existing `BridgeUp::State` literals, plus the new daemon test's helpers if they live there)
- `docs/cli.md` (the QUOTA sentences in the `a2amx list` paragraph), `docs/architecture.md` (the **Quota** paragraph, about lines 490-506: parsing, the single cached value, `recipient_quota`), and `docs/harnesses.md` only if `rg -n -i 'usage --json|across providers' docs README.md` finds quota text there

Does not touch: `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md`, `docs/backlog.md`, locked specs, every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`quota::parse_omp_usage` (`src/quota.rs`) takes the smallest percentage across every provider report in `omp usage --json`, and the daemon caches that one value for all OMP sessions (`Runtime::omp_usage`). With two providers on the account this is wrong. Observed 2026-10-25: `openai-codex` 5h 100% / 7d 85%, `opencode-go` 7d 0% (exhausted); a2amx showed `wk 0%` for sessions running on `openai-codex`. Sessions also change provider mid-life, so a fixed per-session provider would be wrong too. The OMP extension context exposes a live `model` getter (verified in the OMP binary: the context object returned to extensions has `get model()`), and the extension already samples session state every 500 ms, so it can report the provider cheaply and keep it current.

## 2. Decisions

- **D1.** The extension reports the provider of its current model in the existing `state` frame as `provider` (string). It is read from `ctx.model` on every `reportState` call, which the existing 500 ms poll and turn events already trigger. A change of provider counts as a state change and is sent immediately, not held for the keepalive.
- **D2.** Missing data is never guessed. When `ctx.model` is absent, throws, or has no string provider, the frame omits `provider`. An older extension never sends it.
- **D3.** `BridgeUp::State` gains `#[serde(default, skip_serializing_if = "Option::is_none")] provider: Option<String>`, exactly like `queue` and `host_pending`: `default` makes an old extension's frame deserialize to `None`, and `skip_serializing_if` keeps the exact JSON strings pinned in `tests/bridge.rs` and `tests/broker.rs` unchanged when it is `None`. The daemon keeps the last provider per OMP session (`None` until the first frame carrying one, and reset to `None` when a later frame omits it).
- **D4.** `parse_omp_usage` returns `Option<BTreeMap<String, QuotaInfo>>`, usage per provider: each report's `provider` string maps to one `QuotaInfo` holding the smallest remaining percentage across that provider's limits for the `5h` and `7d` windows (several accounts of one provider merge by minimum, as today). A report without a string `provider` is skipped. Windows other than `5h` and `7d` are still ignored. A provider with no `5h` or `7d` window is left out of the map. A map with no entries, bad JSON or an empty `reports` returns `None`, which `run_omp_usage` (`src/daemon.rs:1334`) still treats as a `parse` failure that keeps the previous cache until it goes stale; an empty map never overwrites the cache.
- **D5.** `Runtime::omp_usage()` returns the cached map (it is no longer `Copy`, so the cache read clones it or the lookup happens under the lock), and `session_quota` takes the map and looks up the session's provider; all four callers change: `src/daemon.rs:956` (`recipient_quota` on `send_message`, so a recipient on a healthy provider is no longer reported as `usage quota exhausted` because another provider is exhausted), `1924` (`List`), `2004` (`ListAgents`) and `2313`. `session_quota` for an OMP session looks up the session's provider in the cached map. No provider, or a provider absent from the map, gives no windows (`-` in the cell). It never falls back to the minimum over other providers; that is the bug being fixed. The screen-detected `usage_limit_reached` still sets `limit_reached` on top, as today.
- **D6.** The `// shortcut:` comment at `src/daemon.rs:240-241` (account-level minima for every OMP session) describes what this spec fixes; remove it.
- **D7.** Refresh cadence is unchanged (about 5 minutes, stale after 15): provider changes are sampled continuously, but the quota numbers come from an `omp usage` command, which is expensive and rate-sensitive.
- **D8.** Compatibility: the `provider` field is additive on the bridge frame; an old extension with a new daemon, or the reverse, keeps working (it just shows `-`). Public interface per `AGENTS.md`: compatible, patch-level.

## 3. Existing tests this changes

`rg -n 'parse_omp_usage|omp_usage|OMP_USAGE_FIXTURE' tests` finds these in `tests/quota.rs`, all of which pin the old single-minimum behavior and are rewritten, not deleted without replacement:

- `parses_omp_usage_windows`, `omp_usage_may_report_one_window`, `rounds_and_clamps_omp_percentages`, `omp_usage_without_supported_windows_is_none`, `omp_usage_ignores_identity_fields`: fixtures have no `provider` (except the last), so under D4 they would all parse to nothing. Add a `provider` to each fixture report and assert on the per-provider result.
- `parses_the_most_pessimistic_omp_report`: replaced by worked example 1 (providers stay separate) and example 2 (same provider merges by minimum).
- `list_and_list_agents_report_omp_usage_percentages`: the session now needs a provider, which only a bridge frame can give. Seed it the way `omp_reset_target` in `tests/broker.rs` attaches a `BridgeLink`: the fake `omp` script (a new helper; `fake_executable` in `tests/quota.rs:292` falls back to `sleep 60` and other tests share it, so do not change it) prints the usage fixture when its first argument is `usage`, and otherwise writes `$A2AMX_TOKEN` and `$A2AMX_ADDR` to a file and runs `cat`; the test reads the token, connects a `BridgeLink`, and sends `BridgeUp::State { provider: Some("openai-codex"), .. }`. `failed_omp_usage_poll_is_not_retried_every_check`, `slow_omp_usage_poll_does_not_block_list` and `omp_usage_timeout_kills_process_group` do not read percentages and keep passing; the developer confirms that.

## 4. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| The extension context has a live `model` getter | OMP binary, the extension context object (`get model()`) | read (string search of the installed binary); the provider field of a model value is **not** verified, see section 7 |
| `reportState` runs every 500 ms and on turn events | `extension/omp.ts:113-116`, `575-579` | read |
| The state frame is deduped against `lastState` and a keepalive | `extension/omp.ts:233-244` | read |
| `BridgeUp::State` has no provider field; consumers are `daemon.rs:2172`, `channel.rs:37,63`, tests | `src/wire.rs:202-209` | `rg -n 'BridgeUp::State'` |
| One cached usage value serves all OMP sessions | `src/daemon.rs:78,231-238`, `2210-2230` | read |
| The usage command runs only for a session whose executable file name is `omp` | `src/daemon.rs:243-262` | read |
| The existing tests listed in section 3 are the only users of `parse_omp_usage` outside `src/` | `tests/quota.rs` | `rg -n 'parse_omp_usage' tests` |
| A fake `omp` session receives `A2AMX_TOKEN` and `A2AMX_ADDR` whatever the harness, and its `argv[0]` stays the fake path so the usage poll runs it with `usage --json ...` | `src/daemon.rs:388` (env), `395-410` (only `prepare_codex` rewrites argv); `tests/broker.rs:1525-1555` reads the token | read by the consultant; the Hello step under `Deliver::Hold` (used by `direct_omp_session`; `omp_reset_target` uses `Auto`) is not checked, the developer uses whichever works |
| `fake omp` with a bridge link is feasible in a test | `tests/broker.rs:1511-1580` (`omp_reset_target`) and `tests/quota.rs:298` (`direct_omp_session`) | read; the combined helper is not yet built |

## 5. Worked examples

Parser, input shaped like real `omp usage --json` (fields beyond those used are ignored):

| # | Input | Result |
|---|---|---|
| 1 | `openai-codex`: 5h fraction 1.0, 7d 0.85; `opencode-go`: 5h 1.0, 7d 0.0 | `openai-codex` -> 5h 100, wk 85; `opencode-go` -> 5h 100, wk 0 |
| 2 | two `openai-codex` reports: 7d 0.85 and 7d 0.40 | `openai-codex` -> wk 40 |
| 3 | a report with no `provider` | skipped |
| 4 | a limit whose window id is `monthly` | ignored |

Session lookup: session on `openai-codex` with example 1 cached shows `5h 100% wk 85%`; the same session after the extension reports `opencode-go` shows `5h 100% wk 0%` on the next state frame and the next list; a session that never reported a provider shows `-`.

## 6. Tests

Failing test first, one behavior at a time, literals for expected values. Also add: the extension drops back to omitting `provider` when the model disappears (D2), and the daemon forgets the provider when a later frame omits it (D3).

- `tests/quota.rs`: one test per parser example; session lookup per the second paragraph of section 5 through a daemon whose usage cache is seeded the way the existing OMP usage tests seed it.
- Extension behavior test under `tests/omp_extension/`: a fake context whose `model.provider` changes from `openai-codex` to `opencode-go` produces two `state` frames with those providers and no extra frame between; a context without `model` produces frames without `provider`. The shared default fake context in `tests/omp_extension/harness.mjs` keeps having no `model` (the existing whole-frame `deepEqual` checks in `startup_state.test.mjs:76,80` rely on that); the new test sets `model` through the harness overrides.
- Existing tests keep passing.

## 7. Acceptance

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The developer also checks the one assumption nothing here can prove from the repository: the string OMP puts in `model.provider` for a real session equals the `provider` string in `omp usage --json` (`openai-codex`, `opencode-go`). Print `ctx.model.provider` from a throwaway local extension or probe in a real OMP session on each provider (do not commit it) and compare; report both strings. If they differ, stop and reply `BLOCKED — SPEC ADJUDICATION REQUIRED`; do not add a mapping table without an amendment.

## 8. Out of scope

Per-model (not per-provider) quota, hosts whose OMP sessions use different accounts or configs of the same provider (the daemon runs `omp usage` once, from one session, so the map assumes one account set per host), a faster `omp usage` cadence, showing several providers in one cell, and Claude or Codex harness quota.

## Review record

**Models.** Architect: Claude Sonnet 5.5. Consultant: a Claude Code session, model not exposed to the architect. Developer and tester: OMP sessions reporting `openai-codex/gpt-5.6-luna`; their planning, working and advisor slot models are not exposed to the sessions. **BLOCKED/amend cycles:** 0.

| Found in | Found by | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Architect (gate) | Scope missed `src/channel.rs`, `tests/bridge.rs` and `tests/broker.rs` (literals of the changed frame), and the existing tests that pinned the single-minimum behavior were not listed | Added to scope; section 3 lists the tests to rewrite |
| Draft review | Consultant | `docs/architecture.md` quota paragraph not in scope; serde attributes for the new field unpinned (the JSON in `tests/bridge.rs` is exact); parser return type and the empty-map case unstated; only the definition of `session_quota` named, not its four callers; stale `shortcut:` comment | All fixed before lock (D3, D4, D5, D6, scope) |
| Draft review | Consultant | Several accounts per host share one `omp usage` run, so the map assumes one account set per host | Stated as out of scope |
| Draft review | Consultant | Wording "within one 500 ms sample" ignored the early return while a delivery is in flight | "On the next state frame" |
| Tester pass | Tester | The test for "provider cleared when a later frame omits it" survived a Some-only mutant, because it switched to a provider absent from the cache before omitting | Test changed to omit while the cached provider is still present; red against the mutant, green on the diff |
| Tester pass | Tester | Cache preservation after a failed poll (D4) is covered by code reading only, not a test | Accepted by the architect on a read of `poll_omp_usage`, which writes the cache only on success |
