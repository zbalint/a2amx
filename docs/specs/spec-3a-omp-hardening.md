# Spec 3a: OMP install and extension hardening (backlog I2)

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `af690a2`; gate notes in section 6).
Backlog item I2 (spec 2f review nits). Decisions D1 to D4 are the architect's, marked
**(architect)**, settled on the owner's instruction of 2026-10-04 to write a spec for every
planned item that needs no owner input.

**Baseline:** develop at `af690a2`, after specs 2y and 2z are accepted and committed (2y edits
`src/main.rs`, which this spec does not touch; the order is a precaution, not a dependency).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared
task `context_id`: `a2amx-omp-hardening`. Public test seam: `a2amx::omp::install` and
`render_extension` (`tests/omp_wiring.rs`).

**Scope.** May edit exactly these files and no others.

- `src/omp.rs`
- `extension/omp.ts`
- `tests/omp_wiring.rs`, and any other file under `tests/` that calls `render_extension()` (the
  baseline list is in section 6)
- `docs/backlog.md`

Does not touch: `src/main.rs` (it calls `omp::install` and keeps working unchanged),
`src/mcp.rs`, the bridge code, `tests/omp_smoke.rs` logic beyond a mechanical call-site fix if it
calls `render_extension`. No new dependency. Do not commit, stage or merge.

## 1. Why

The spec 2f review left three nits on the OMP install and extension: `render_extension` ends in an
`unreachable!` (a panic in a function the daemon calls at startup, against the repository's no
`unwrap`/`expect` rule in spirit), the install cleanup `?` on `remove_file` replaces the original
write error with the cleanup error, and the extension calls `child.kill()` and
`child.stdin.end()` without a guard, so a throw in the shutdown path or the bridge error path would
escape. None has bitten yet. Each is small and can be fixed without a design decision.

## 2. Decisions

- **D1. (architect)** `render_extension()` returns `anyhow::Result<String>`. The serialization
  error is returned, not turned into a panic. `install` uses `?`. Every other caller handles the
  result (tests may `unwrap`).
- **D2. (architect)** In `write_if_changed`, a failing cleanup must not replace the original
  error: remove the temp file best-effort, and return the original `write_all` or `rename` error.
  The ignored cleanup result carries a one-line comment saying why (the write error is the one
  the operator needs).
- **D3. (architect)** In `extension/omp.ts`, `state.child.kill()` in `recordBridgeError`
  (line 380 at the baseline) and `state.child.stdin.end()` plus the delayed `state.child.kill()`
  in `shutdown` (lines 519 to 522) are each wrapped so a throw cannot leave the extension in a
  half-torn-down state: `recordBridgeError` must still reach its end, and `shutdown` must still
  schedule the delayed kill if `stdin.end()` throws. A caught error is not dropped silently: it is
  kept as the extension's last diagnostic the same way a bridge process error is (see how
  `lastStderrLine` is set in `recordBridgeError`).
- **D4.** No behavior change on the success paths: the rendered extension for a healthy install is
  byte-identical except for the added guards, the installed file permissions stay `0600` and the
  directory `0700`.

## 3. `src/omp.rs`

`render_extension` (line 15) changes its return type per D1 and drops the `match` with
`unreachable!` for a plain `?`. `install` (line 29) uses `render_extension()?`. `write_if_changed`
(line 43): replace `fs::remove_file(&temp)?; return Err(error.into());` (lines 72 to 73) with a
best-effort remove and the original error, per D2.

## 4. `extension/omp.ts`

Implement D3 at the three call sites named above. Keep the guards minimal (a `try` around the one
call, the catch recording the message). Do not restructure `shutdown` or `recordBridgeError`.

## 5. Tests and acceptance

Tests (failing first where the behavior can be observed):

1. `tests/omp_wiring.rs`: the existing `render_extension` assertions use the `Result` and still
   compare the installed file against it; the existing install, repair and permission tests keep
   passing.
2. A test that `write_if_changed` reports the original error when the destination cannot be
   written and cleanup also fails is not required if it cannot be made through the public `install`
   seam; in that case say so in the report instead of testing a private function (AGENTS.md).
   The reviewer checks D2 by reading the diff.
3. `extension/omp.ts` has no automated test seam outside the ignored real-OMP smoke tests. D3 is
   reviewed by reading the diff, and the ignored smoke tests are run only if an `omp` binary is
   available and the owner asks.

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass. **Out of scope:** any other extension change, the other review nits not named here,
changing the bridge protocol, and installing or restarting the working daemon.

## 6. Gate notes

At lock time (baseline `af690a2`): `unreachable!` is at `src/omp.rs:18`; the cleanup `?` is at
`src/omp.rs:72`; `child.kill()` is at `extension/omp.ts:380` and `:522`, `stdin.end()` at `:519`.
`render_extension` is called in `src/omp.rs:38` and in `tests/omp_wiring.rs` (lines 118, 132,
144); the developer runs `rg render_extension src tests` first and fixes every hit it returns.
`src/main.rs:444` calls `omp::install` through `spawn_blocking` and needs no change. `git status`
was clean at lock time.
