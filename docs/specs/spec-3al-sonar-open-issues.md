# Spec 3al: clear the open SonarQube Cloud findings on `develop`

## 0. Status

**LOCKED** (2026-10-16). Architect-directed, context `sonar-open-issues-2026-10-16`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `463867a`; Sonar (project `zbalint_a2amx`, branch `develop`) lists 18 open issues, 3 of them already marked false positive by the owner's instruction on 2026-10-16. The developer records the counts of `A2AMX_REQUIRE_NODE=1 env -u A2AMX_BIN -u NO_COLOR cargo test` before the first edit (architect baseline: section 7).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the daemon's TCP control protocol (`tests/daemon.rs`, run by the full suite) for `serve`; the OMP extension's behavior tests (`tests/omp_extension/*.test.mjs`, run through `cargo test --test omp_extension`) for `extension/omp.ts` and the harness. No new tests: every change is behavior-preserving and an existing test covers it (section 6).

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: `serve` (starts at line 1737) and two new private functions placed directly after it
- `src/harness.rs`: `Harness::infer` (line 54) and the `use` line it needs
- `src/messaging.rs`: line 373
- `src/wire.rs`: line 497
- `extension/omp.ts`: sections 4 and 5 only
- `tests/omp_extension/harness.mjs`: `waitForFrame` and `waitForNewFrame` (lines 107-120)

Does not touch: every other file, including `CHANGELOG.md` and `docs/backlog.md` (the architect edits those). No new dependency. Leave the diff uncommitted.

## 1. Why

Sonar lists 12 findings that are real code-quality issues with no behavior change attached: one `rust:S3776` (cognitive complexity 31 over 30; `serve` was split by spec 3ad and grew back), two `rust:S1612` closures and one `rust:S2737` conversion, one `javascript:S9382` that is a real duplicate, and seven style findings in `extension/omp.ts`. The gate is `OK` today; this clears them. Three more are marked false positive and three are proposed false positives, see section 8; they are not code changes.

## 2. Rule: behavior is unchanged

Same responses, same messages byte for byte, same order of checks, same control flow in `serve`. Same extension behavior and same rendered output apart from the moved and renamed functions. `extension/omp.ts` lines 1-6 stay byte-identical: `src/omp.rs:17` replaces the `[/* a2amx:tools */]` text on line 5 and the JS coverage report is mapped back by line number.

## 3. Rust changes

1. `src/harness.rs:54`: `.and_then(|name| name.to_str())` becomes `.and_then(OsStr::to_str)`; add `use std::ffi::OsStr;` next to `use std::path::Path;` (line 4).
2. `src/messaging.rs:373`: `.any(|character| character.is_control())` becomes `.any(char::is_control)`.
3. `src/wire.rs:497`: `String::from_utf8(body.to_vec()).map_err(anyhow::Error::new)?` becomes `String::from_utf8(body.to_vec())?`. The function returns `anyhow::Result`, so `?` converts `FromUtf8Error` itself.
4. `src/daemon.rs` `serve`: move the loop's first guard (the `if let Role::Session(id) = &role && !lookup(..).is_some_and(..)` condition at lines 1744-1746; the whole `if` is 1744-1756) into a private `fn session_has_exited(runtime: &Runtime, role: &Role) -> bool` placed after `serve`; `serve` keeps writing the `"session has exited"` error, shutting the stream down and returning. Only the condition moves.
5. `src/daemon.rs` `serve`: also move the permission test (`matches!(&role, Role::Session(_)) && !session_token_permitted(&request)`, near line 1770) into a private `fn session_request_denied(role: &Role, request: &Request) -> bool` placed after `serve`. Two extractions give an estimated count of 29 against the limit of 30 (the consultant's hand count of Sonar's rules; one extraction alone leaves zero margin). If Sonar still reports above 30 after the push, the developer does not guess a third split: report it to the architect (nothing local measures S3776; the proof is a Sonar scan after the push).

## 4. Extension changes (`extension/omp.ts`)

1. Line 7: `export default function (pi) {` becomes `export default function a2amxExtension(pi) {` (S7726).
2. Move `firstContentText` (line 102), `receiptText` (line 133) and `parseStderr` (line 358) out of the default function to module scope, directly above `export default` and below the `TOOLS` line. Each uses only its own parameters (checked: `content`; `message`; `state` and `chunk`). Their bodies do not change except item 3. (S7721 x3)
3. `receiptText`: `if (!message || message.role !== "user" || message.attribution !== "agent")` becomes `if (message?.role !== "user" || message.attribution !== "agent")` (S6582).
4. Lines 179-183: merge the two identical `deferredReceipts.push(text)` branches into one condition: `if ((fifo.length > 0 && text === fifo[0].envelope) || (fifo.length === 0 && delivering.envelope === text))`. Keep the `return;` after the outer `if (delivering)` block (S1871).
5. Line 494: `{ ...(params || {}) }` becomes `{ ...params }` (S7744).

## 5. Harness change (`tests/omp_extension/harness.mjs`)

`waitForNewFrame` (lines 112-120) polls every 5 ms; `waitForFrame` (lines 107-111) already waits on a waiter list that line 83 feeds. Give `waitForFrame` a third parameter `start = 0` whose first check is `connection.frames.slice(start).find(predicate)`, and reduce `waitForNewFrame` to `return waitForFrame(connection, predicate, start)` with its default `start = connection.frames.length` kept. The 17 `waitForNewFrame` callers (`delivery_receipts.test.mjs` 11, `startup_state.test.mjs` 6) do not change, and `lifecycle_tools.test.mjs` calls `waitForFrame` with two arguments, so `start = 0` keeps its behavior; that file is covered by section 6 but not edited. The waiter predicate at line 83 only tests the newest frame, which is new by definition, so no frame can be missed between the first check and the waiter's registration (JS runs both synchronously). Do not touch the `timeout` helper or any other function in the file.

## 6. Existing tests that must stay green (contract tests, already passing on old code)

- `tests/daemon.rs:878` asserts the `"session has exited"` response through a real daemon (covers section 3.4).
- `tests/omp_extension/delivery_receipts.test.mjs` (covers 4.4 and the deferred receipt branches), `startup_state.test.mjs` and `lifecycle_tools.test.mjs` (cover 5).
- `tests/harness_infer.rs` (covers `infer`), `tests/prop_messaging.rs:111` (the subject control-character rejection; `tests/daemon_cli.rs:877` covers a different check, the reset step) and `tests/wire.rs:268,307` (the `Detached` frame round trip). The invalid-UTF-8 error branch of `Detached` has no test; the change there only drops a redundant conversion and keeps the error type's message path, so no test is added.

There is no red phase: these are refactors guarded by tests that already pass.

## 7. Claims verified for this spec

| claim | where | verified how |
| --- | --- | --- |
| `TOOLS` is a placeholder filled at render time, so S4158 is a false positive | `extension/omp.ts:5`, `src/omp.rs:17` | read |
| `stderrTail` is capped at 2048 bytes, so the S8786 regex at line 403 has bounded input | `extension/omp.ts:359` | read |
| The three functions to hoist use only their own parameters | `extension/omp.ts:102-107, 133-146, 358-360` | read |
| Nothing depends on the text `export default function (pi)` | `rg 'export default'` over `src tests extension scripts docs` | probed (one hit, the definition) |
| `serve` exited-session check is covered | `tests/daemon.rs:878` | read |
| `infer`, the subject control-character check and the `Detached` round trip are covered | `tests/harness_infer.rs`, `tests/prop_messaging.rs:111`, `tests/wire.rs:268,307` | grep (files and lines found; test bodies not read) |
| Baseline suite | `A2AMX_REQUIRE_NODE=1 env -u A2AMX_BIN -u NO_COLOR cargo test` | architect run, result recorded below |

Architect baseline result: develop at `463867a`, `A2AMX_REQUIRE_NODE=1 env -u A2AMX_BIN -u NO_COLOR cargo test` exit 0: 572 passed, 0 failed, 3 ignored across 36 test binaries (summed from the `test result` lines); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.

## 8. Out of scope (Sonar-side, not code)

- `javascript:S7747` line 82 and `javascript:S9382` lines 103 and 168 in `tests/omp_extension/harness.mjs`: marked false positive in Sonar by the architect on the owner's instruction (the loop splices while iterating; two bounded retry loops). Do not edit those lines.
- `typescript:S4158` (`extension/omp.ts:472`, `TOOLS` is always empty) and `typescript:S8786` (`extension/omp.ts:403`, bounded 2048-byte input): proposed false positives, pending the owner. Do not edit those lines either way.
- `rust:S2629` (`src/store.rs:167`, "use `with_context`"): `error` is an `anyhow::Error` already inside an `Err` arm; `with_context` exists only on `Result`/`Option`, and the format only runs on the failure path. Proposed false positive, pending the owner. Do not edit `src/store.rs`.
- No other refactor, rename or "while I'm here" cleanup.

## 9. Acceptance

Run from `/home/zbalint/workspace/a2amx` after the edits:

```sh
A2AMX_REQUIRE_NODE=1 env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short            # only the six files of section 0 (plus the pre-existing untracked .cargo/)
git diff --stat               # confirms nothing else changed
```

All three commands pass with no warnings and the test counts equal the developer's baseline counts. `rg -n 'any\(\|character\| character\.is_control\(\)\)|\|name\| name\.to_str\(\)|map_err\(anyhow::Error::new\)' src/harness.rs src/messaging.rs src/wire.rs` finds nothing (other `is_control` uses in `src/messaging.rs` are out of scope and stay). The Sonar scan of `develop` after the push is the final proof for the 12 findings and the S3776 count; the architect reads it, not the developer.

Mutation check: this spec has no new tests, so the rule for new tests does not apply. For section 5 the developer temporarily makes `waitForNewFrame` scan from 0 (ignore `start`), runs `cargo test --test omp_extension`, and reports which tests fail, or that none does, then restores. Either result is reported, not a blocker: a pass only means the existing tests do not pin that behavior. Mutate in the assigned checkout; the tree must match the diff afterward.
