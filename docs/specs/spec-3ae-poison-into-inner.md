# Spec 3ae: replace the `into_inner` closures with `PoisonError::into_inner` (Sonar S1612)

## 0. Status

**LOCKED** (2026-10-14). Architect-directed, context `a2amx-sonar-s1612-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `457a500` (pushed; Sonar gate `OK` on it). The developer records `cargo test` counts before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** none new. The change is a behavior-neutral expression rewrite; the existing suite is the check.

**Scope.** May edit exactly these files and no others.

- `src/daemon.rs`: 15 sites, no import change (`PoisonError` is already imported at line 11)
- `src/emulator.rs`: 3 sites, plus `PoisonError` in the `use std::sync::{Arc, Mutex}` import (line 8)
- `src/session.rs`: 3 sites, plus `PoisonError` in the `use std::sync::{Arc, Mutex, MutexGuard}` import (line 12)
- `src/codex.rs`: 2 sites, plus `PoisonError` in the `use std::sync::{Arc, Mutex}` import (line 5)
- `src/delivery.rs`: 2 sites, both inside `mod tests` (the module starts at line 464), plus `PoisonError` in that module's `use std::sync::Mutex;` (line 467; rustfmt then reorders it, see section 3)
- `src/store.rs`: 1 site, plus `PoisonError` in the `use std::sync::{Arc, Mutex, MutexGuard}` import (line 7)

Does not touch: every other file (`tests/`, `docs/`, `Cargo.toml`, `Cargo.lock`). No new dependency. Leave the diff uncommitted.

## 1. Why

Sonar `rust:S1612` ("replace this closure with a reference to the `into_inner` method") flags three of these sites today (`src/daemon.rs` lines 544 and 622, `src/session.rs` line 203); the code-smell gate condition is at 10 of 14. Twenty-six sites in `src/` share the same shape. The repo already uses the method-reference form in six places in `src/daemon.rs`. The owner chose to fix all 26 in one pass so the rule stops reappearing as nearby code changes.

## 2. Rule: behavior is unchanged

Every site is `<mutex>.lock().unwrap_or_else(|name| name.into_inner())`, recovering the guard from a poisoned `std::sync::Mutex`. It becomes `<mutex>.lock().unwrap_or_else(PoisonError::into_inner)`. Nothing else changes: no new function, no reordering, no comment edits, no `pub` change. The closure parameter name varies (`e`, `error`, `poisoned`); all are the same shape.

## 3. Changes

1. Add `PoisonError` to the existing `std::sync` import of `emulator.rs`, `session.rs`, `codex.rs`, `store.rs` and to the `mod tests` import in `delivery.rs` (line 467, which becomes `use std::sync::{Mutex, PoisonError};`). Keep rustfmt's ordering: in `delivery.rs` rustfmt (edition 2024) then moves that braced import below the `use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};` line, so lines 467 and 468 swap; that swap is expected and allowed. The other four files already have their braced `std::sync` import after any `std::sync::atomic` line, so nothing moves there. `daemon.rs` needs no import change; all its sites are before its `#[cfg(test)]` module at line 2496.
2. Rewrite the sites. Suggested order, because the tool is faster and exact for 25 of 26:

   ```sh
   ast-grep run --lang rust --pattern '$M.unwrap_or_else(|$E| $E.into_inner())' \
     --rewrite '$M.unwrap_or_else(PoisonError::into_inner)' --update-all src
   cargo fmt
   ```

   `ast-grep` matches 25 sites. It does NOT see the 26th: `src/delivery.rs` line 615, which sits inside an `assert_eq!(...)` macro call where `ast-grep` does not parse expressions. Edit that one by hand to the same form. The tool leaves some chains unformatted (it joins `.lock()` and the next call onto one line); `cargo fmt` fixes that, and may also pull a multi-line chain onto one line where it now fits. That reflow of the rewritten expression is expected and allowed. `cargo fmt` must not change any line unrelated to these sites, apart from the `delivery.rs` import swap above; the tree is rustfmt-clean at `457a500`.
3. Types: `PoisonError::into_inner` takes `PoisonError<T>` and returns `T`. All 26 sites are `Mutex::lock` results, so inference is the same as the six existing uses in `src/daemon.rs` (lines 235, 246, 272, 283, 520, 1966). No site uses a non-`lock` receiver.

## 4. Safety net

No test is added: the change is a type-checked expression rewrite with identical semantics, so compilation, clippy and the existing suite are the check. A wrong replacement fails to compile.

## 5. Out of scope

`docs/backlog.md` (the architect updates it separately), any other lint, any other file, the `PoisonError::into_inner` uses already present, any rename, and reformatting beyond what `cargo fmt` does to the rewritten expressions.

## 6. Acceptance

```sh
rg --pcre2 -n 'unwrap_or_else\(\|(\w+)\| \1\.into_inner\(\)\)' src tests   # no output, exit 1
rg -o 'unwrap_or_else\(PoisonError::into_inner\)' src | wc -l              # 32 (6 existing + 26)
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the six files of section 0
```

The `rg` count in the second command counts the single-line form only; a chain `cargo fmt` leaves split across lines may not match, so if it prints fewer than 32, count with `rg -c 'PoisonError::into_inner' src` and explain any difference in the report. The diff must contain only the 26 rewritten expressions (with their `cargo fmt` reflow) and the five import edits (including the `delivery.rs` swap); the report states `git diff --stat` and that nothing else changed.

Run the full suite twice and report both counts. Known flakes: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` and `attach_does_not_capture_session_already_in_drag_mode` (backlog I7): rerun once, record as pre-existing. Final proof is the Sonar rescan after the owner pushes: no open `rust:S1612` issue on `develop`.
