# Spec 3fp: split `src/session.rs` into the session root and `spawn`, `native`, `reset` and `exit` child modules (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `c4d48bc`, spec 3fo accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch7-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `src/session.rs`: shrinks to the root (section 2).
- `src/session/spawn.rs`, `src/session/native.rs`, `src/session/reset.rs`, `src/session/exit.rs`: new.

The spec's own commit already carries the two boundary edits the move needs, so the developer does not touch them: `.semgrep/rules/tty-outside-session.yml` (its `exclude` list now names `src/session/spawn.rs` instead of `src/session.rs`; the rule, its message and fixture are otherwise unchanged) and the `tty` sentence in `AGENTS.md` (Rust conventions). Does not touch: every other file under `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `docs/`, `CHANGELOG.md`, `.semgrep/`. Do not commit, stage or merge. The architect updates the backlog and the review record in the accepting commit.

Public test seam: the whole suite, in particular the daemon, broker, reset and Codex tests that drive `Session` (`cargo test`); `src/session.rs` has no inline unit tests. No test is added, removed or changed.

## 1. Why

`src/session.rs` is 1477 lines, over the owner's goal of no `.rs` file over 1,000 lines; the owner told the architect to continue the refactor with the source files (specs 3fm discord, 3fn team and 3fo config are done). The file's `impl Session` alone is 1005 lines (lines 391-1395). It holds four responsibilities that are already separate groups of methods and free functions with few crossings: starting a session (`spawn`, `spawn_confined`, the confined-spawn worker thread, the PTY reader, writer and monitor loops, `mark_inherited_fds_cloexec`), the native (OMP bridge) channel (`native_attach` .. `native_timing`, `NativeGuard`), the reset gate and sequence (`reset_native_reason` .. `reset_sequence`, `ResetError`, `ResetGuard`, the stale-draft checks and the reset constants), and the exit path (`kill`, `log_graceful`, `graceful_exit`, `wait_exit`, the graceful constants). The accessors, input, resize, delivery and the types stay in the root. The module table row for `session` ("One PTY, its child, its emulator") stays true.

Measured by script on HEAD (every `fn`, `const`, `static`, `struct`, `enum` and `type` name of the file, checked for definition and use across the groups): the only private item used across a group boundary is `Shared` (a private root struct that the `spawn` child builds, visible to a child), plus three private methods that become `pub(super)` (section 2); every other cross-use is of an item that is already `pub` or `pub(crate)`. The names other modules reach through the `session` path (`rg` on `src` and `tests`) are `Session`, `SessionId`, `SessionSpec`, `AttachmentSlot`, `NativeGuard`, `mark_inherited_fds_cloexec` and `spawn_on_worker` (`daemon.rs`, `codex.rs`), `Delivery`, `DeliveryOutcome` and `Hold` (`delivery.rs`) and `observe_activity` (`tests/daemon/status_reporting.rs`); `NativeGuard`, `mark_inherited_fds_cloexec` and `spawn_on_worker` move and are re-exported, the rest stay in the root.

The tty boundary: `use alacritty_terminal::tty::{self, ChildEvent, EventedPty}` and the `tty::` uses (`tty::Pty` in `MonitorArgs`, `tty::Options`, `tty::Shell`, `tty::new`) are all in the spawn group, so they move to `spawn.rs`. The semgrep rule `tty-outside-session` and the `AGENTS.md` sentence say the PTY module is named only in `src/session.rs`; the spec's own commit moves both to `src/session/spawn.rs`, so the boundary stays one file wide and the root can no longer name the PTY module without failing the rule.

The `exit` child depends on `reset` (`begin_reset`, `reset_gate_reason`) because a graceful exit goes through the reset gate; that is the existing call graph, moved unchanged.

Alternatives rejected: a `src/session/mod.rs` (the repository uses the 2018 root-file layout, as in specs 3fm to 3fo); keeping the exit path in the root (it would leave the root at about 730 lines; the exit path is one responsibility with its own constants and is the cheapest to name); splitting `impl Session` by visibility instead of by responsibility; extracting the 70-line `reset_sequence` or the 120-line `spawn` (a later step, not a move).

## 2. Target shape

The root `src/session.rs` stays the root and declares the children. Methods of `impl Session` that move go into an `impl Session { ... }` block in the child (Rust allows several impl blocks, and a child module can use the root's private struct fields).

| File | Old lines of `src/session.rs` (HEAD) | Contents |
| --- | --- | --- |
| `src/session.rs` | everything not listed below (611 lines) plus the declarations below | header, imports, `SessionId`, `SessionSpec`, `Session`, `Shared`, `State`, `NativeSlot`, `Hold`, `Delivery`, `DeliveryOutcome`, `AttachmentSlot`, `observe_activity`, `impl Session` with the accessors, quota, activity, human input, draft restore, `begin_delivery`, `write_input`, `resize`, `exit_code`, `lock`, `enqueue`, `resize_locked`; `impl Delivery`, `impl Drop for Session` |
| `src/session/spawn.rs` | 36-86, 244-390, then `impl Session {` + 392-516 + `}` | `SpawnTask`, `SpawnJob`, `SPAWN_JOBS`, `spawn_worker`, `confined_spawn_sender`, `spawn_on_worker`, `writer_loop`, `reader_loop`, `MonitorArgs`, `monitor_loop`, `exit_code_from_status`, `mark_inherited_fds_cloexec`, `Session::spawn_confined`, `Session::spawn` (about 330 lines with imports) |
| `src/session/native.rs` | 184-188, then `impl Session {` + 787-927 + `}`, then 1397-1410 | `NativeGuard` and its `Drop`, `Session::native_attach` .. `native_reason`, `native_timing` (about 170 lines with imports) |
| `src/session/reset.rs` | 30-33, 207-222, then `impl Session {` + 928-1171 + `}` | `RESET_SETTLE`, `STALE_DRAFT_MARGIN`, `RESET_POLL`, `RESET_TIMEOUT`, `ResetError`, `ResetGuard` and its `Drop`, `reset_native_reason`, `stale_*_draft`, `draft_is_stale`, `clear_stale_draft`, `reset_gate_preview`, `reset_gate_reason`, `reset_gate_decision`, `reset_error`, `begin_reset`, `reset_sequence` (about 275 lines with imports) |
| `src/session/exit.rs` | 34-35, then `impl Session {` + 1278-1394 + `}` | `GRACEFUL_TIMEOUT`, `EXIT_KEY_GAP`, `Session::kill`, `log_graceful`, `graceful_exit`, `wait_exit` (about 130 lines with imports) |

The ranges and the root's complement partition lines 1-1477 exactly (checked by script, together with the expected diff in section 5); line 391 (`impl Session {`) and line 1395 (its closing `}`, followed by the blank line 1396) stay in the root, whose `impl Session` keeps the methods in 517-786 and 1172-1277. A method range starts on the first attribute or comment line of its method.

The edits, and the only edits besides the moves:

1. **Declarations in `src/session.rs`.** After the `use` lines: `mod exit;`, `mod native;`, `mod reset;`, `mod spawn;`, a blank line, then `pub(crate) use native::NativeGuard;` and `pub(crate) use spawn::{mark_inherited_fds_cloexec, spawn_on_worker};`.
2. **Visibility.** Exactly three private methods become `pub(super)`, because a sibling child or the root calls them: `native_timing` (called by `reset`), `draft_is_stale` (called by the root) and `reset_gate_reason` (called by `exit`). The signature text is otherwise unchanged. No other visibility changes; any further private item the compiler reports across a boundary is a BLOCKED, not a free change.
3. **Imports.** Each file keeps exactly the `use` lines it needs (the compiler and clippy's `unused_imports` are the judge); the children name the root items they use through `use super::{...}` (for example `Session`, `SessionId`, `SessionSpec`, `Shared`, `State`, `NativeSlot`, `Hold`, `DeliveryOutcome`). The root drops the imports only moved code used (the architect expects the whole `alacritty_terminal::tty` line to go to `spawn.rs`).
4. **Everything else** is moved byte-for-byte, comments and `// shortcut:` comments included, with the relative order inside each file unchanged. `cargo fmt` runs once after assembly.

## 3. What must not change

- Every public and `pub(crate)` path and signature, and every behavior. No renames, no new `#[allow]`, `#[cfg]` or suppression. The `unsafe` blocks and their `// SAFETY:` comments move unchanged.
- No test file changes.

- **Amendment 1 (tester BLOCKED, tracing targets).** `tracing` events default their target to the module path, so the events that move into `spawn` and `exit` now carry the targets `a2amx::session::spawn` and `a2amx::session::exit` instead of `a2amx::session`. This is accepted and is not a behavior change under this section: the repository's public interface (`AGENTS.md`, Versioning) is the CLI, the config and team file formats, the wire protocol, the message database and the MCP tools, and log target text is not part of it; a `RUST_LOG` directive for `a2amx::session` still matches every child by prefix; the only filter in the code (`log_target_allowed` in `src/main.rs`) names dependency crates only; no test asserts on a target. No explicit `target:` is added, and no event line changes. The same applies to specs 3fm to 3fo, which moved events into child modules too.

## 4. Out of scope

Shortening `spawn`, `reset_sequence`, `reset_gate_decision`, `monitor_loop` or `graceful_exit` (a later step if `cs check` still flags them); other `src/` files; the backlog, review record and changelog (the architect).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`. Create `target/scratch/3fp` first and keep it until the architect accepts.

1. **Baseline.** Before editing: save `cargo test -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fp/before.txt` (check it is not empty), the totals of a full `cargo test` (61 suites, 1347 passed, 3 ignored expected), `rg -n 'session::' src tests | sort > target/scratch/3fp/uses-before.txt`, and `git show HEAD:src/session.rs` as `target/scratch/3fp/session.rs`. After the split: the sorted list is identical, the totals are equal, and the same `rg` command diffs empty.
2. **Same code.** `old` = `git show HEAD:src/session.rs`, `new` = `cat src/session.rs src/session/spawn.rs src/session/native.rs src/session/reset.rs src/session/exit.rs`. After `sed '/^\(pub \|pub(crate) \|pub(super) \)\?use .*{$/,/^};$/d; s/^[[:space:]]*//; /^$/d; /^\(use \|mod \|pub use \|pub(crate) use \|pub(super) use \)/d'` and `sort`, `diff old new` shows exactly: the three signature lines removed and their `pub(super)` forms added (`fn native_timing(native: &NativeSlot) -> String {`, `fn draft_is_stale(&self, state: &State) -> bool {`, `fn reset_gate_reason(&self) -> Option<(&'static str, String)> {`), four added `impl Session {` lines and four added `}` lines. Nothing else. The architect ran this on a scripted rebuild of the section 2 ranges before locking and it produced exactly these lines.
3. **Order inside each file.** The architect rebuilds the five files from the section 2 ranges and compares in order with the developer's; the tester repeats this independently.
4. `wc -l src/session.rs src/session/*.rs`: every file under 1,000. `cs check` on each new file and on the old copy; report the scores.
5. `cargo build --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; the whole `cargo test` passes. The semgrep rules (`scripts/prepush-fast.sh`, run by the architect) pass with `src/session/spawn.rs` as the only file naming `alacritty_terminal::tty` (`rg -l 'alacritty_terminal::tty' src` prints only that file).
   Acceptance 2 compares the multiset of lines and cannot see a line in the wrong file or position, so also: `rg -c '^impl Session \{' src/session.rs src/session/*.rs` prints 1 for each of the five files (and `exit.rs`, `native.rs`, `reset.rs` and `spawn.rs` each contain exactly one such block), and acceptance 3 is the structural check.
6. `git status --short` shows only ` M src/session.rs` and `?? src/session/`.

Known flaky tests: `tests/team/reset_and_validation.rs::team_up_forwards_reset_configuration_and_consent` (backlog I28) and `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error` (backlog I29), both unrelated. A failure of either is rerun once and recorded as pre-existing if it passes.
