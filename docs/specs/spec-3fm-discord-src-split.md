# Spec 3fm: split `src/discord.rs` into the REST root and a `watch` child module (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `1aba86f`).
Shared task `context_id`: `a2amx-codescene-refactor-batch7-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `src/discord.rs`: shrinks to the root (section 2).
- `src/discord/watch.rs`: new.

Does not touch: every other file under `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge. The architect updates the `discord` row of the module table in `AGENTS.md` and the backlog in the accepting commit.

Public test seam: `cargo test --test discord` (the Discord client's integration tests, which reach the module only through `a2amx::discord`). No test is added, removed or changed; nothing in the public path set changes (section 3), and the equivalence checks in section 5 replace a mutation check.

## 1. Why

`src/discord.rs` is 1024 lines, over the owner's goal of no `.rs` file over 1,000 lines, and the owner has told the architect to continue the refactor with the source files after the test files (specs 3ev to 3fl). The file holds two responsibilities that share only configuration and error types: the REST client (`DiscordConfig`, `DiscordError`, `ChannelInfo`, `Discord` and its HTTP retry helpers, lines 1-462) and the gateway connection that feeds events to the relay (`Watch` and everything it runs, lines 463-1024: the watch task, the websocket connection loop, op and dispatch decoding, close-code classification, reconnect backoff, resume-URL validation, the rustls connector). Nothing in lines 463-1024 is used by lines 1-462 except through `Discord::watch` (lines 182-199), and nothing in 1-462 is used by 463-1024 except the public `DiscordConfig`, `DiscordError` and `Fatal` types. The `discord` row of the module table in `AGENTS.md` already describes the two halves ("REST calls ... the gateway websocket connection and its reconnect rules"), so the split follows the documented responsibility and gives no module a new job.

Alternatives rejected: a `src/discord/mod.rs` with `rest.rs` and `watch.rs` (more files and a longer diff for the same result; the root already is the REST client); a top-level `src/discord_watch.rs` (would add a public module to `lib.rs` and the module table); making the watch types `pub(crate)` or widening fields (the one real seam is the constructor in section 2).

## 2. Target shape

The root stays `src/discord.rs` (Rust 2018 style, no `mod.rs`) and declares `mod watch;` and `pub use watch::Watch;`. Every public path stays: `a2amx::discord::{ChannelInfo, Discord, DiscordConfig, DiscordError, Fatal, Watch}`.

| File | Old lines (of `src/discord.rs` at HEAD) | Contents |
| --- | --- | --- |
| `src/discord.rs` | 1-462 with the edits below | `RESPONSE_LIMIT`, `MAX_RETRIES`, `DiscordConfig` and its `Debug`, `DiscordError`, `ChannelInfo`, `Fatal`, `Discord` and its `impl`, `HttpResponse`, `map_ureq_error`, `retry_wait`, `valid_channel_name`, `response_id`, `find_channel_id`, `parse_channel`, `valid_id` (about 440 lines) |
| `src/discord/watch.rs` | 463-1024, plus the edits below | `Watch` and its `Drop`, `WatchState`, `Outcome`, `Session`, `ConnectionState`, `run_watch` and every function below it to the end of the file (about 590 lines) |

The edits, and the only edits besides the moves:

1. **Constants.** `FRAME_LIMIT` (line 15) and `INTENTS` (line 17) move to the top of `watch.rs`; `RESPONSE_LIMIT` and `MAX_RETRIES` stay in the root. (Use sites: `FRAME_LIMIT` 620-621 and `INTENTS` 695 are in the moved range; `RESPONSE_LIMIT` 355 and `MAX_RETRIES` 296 are in the root.)
2. **Imports.** Each file keeps exactly the `use` lines it needs (the compiler and clippy's `unused_imports` are the judge). `watch.rs` adds `use super::{DiscordConfig, DiscordError, Fatal};` and keeps `use crate::gateway::Event;`. `Event` is also used by the root only through `Watch`, so it moves out of the root. The root adds `mod watch;` and `pub use watch::Watch;`.
3. **The constructor, the one real seam.** `Discord::watch` (lines 182-199) builds `Watch` and `WatchState` from private fields, which the root cannot do once they live in a child module. Its body from `let capacity` (line 183) to the closing `}` of the struct literal (line 198) moves into a new associated function, inserted as the first item of the existing `impl Watch` block (old line 470, before `recv`), followed by one blank line:

   ```rust
   pub(super) fn start(config: Arc<DiscordConfig>) -> Self { /* the moved body */ }
   ```

   In the moved body the line `let config = Arc::clone(&self.config);` is deleted (the argument is the `Arc` the task receives) and `self.config.queue.max(1)` becomes `config.queue.max(1)`; the `tokio::spawn` call, the `WatchState { fatal: Mutex::new(None) }` literal, the `Watch { ... }` literal and its field order are otherwise unchanged. `Discord::watch` stays in the root as `pub fn watch(&self) -> Watch { Watch::start(Arc::clone(&self.config)) }` (`cargo fmt` writes it on three lines). `start` is the only new item and the only new visibility (`pub(super)`).
4. **Placement of the new lines.** In the root: `mod watch;` and `pub use watch::Watch;` go directly after the `use` lines, before `RESPONSE_LIMIT`. In `watch.rs`: the `use` lines first (including `use super::{DiscordConfig, DiscordError, Fatal};`), a blank line, `FRAME_LIMIT` and `INTENTS` (in that order, as in the old file), a blank line, then old lines 463-1024.
5. **Everything else** is moved byte-for-byte, including comments, with its relative order inside each file unchanged. `cargo fmt` runs once after assembly.

`watch.rs` uses only `DiscordConfig`, `DiscordError` and `Fatal` from the root (all public); `valid_id` and `Discord`'s private fields are not used by it. The child's private items are not visible to the root; the only root-to-child references are the `Watch` type and `Watch::start`.

## 3. What must not change

- The public path set above, every public signature and every behavior of the REST client and the gateway. No renames, no new `#[allow]`, `#[cfg]` or suppression, no change to a `// shortcut:` comment.
- Test files: none changes. The module table row for `discord` stays true.

## 4. Out of scope

Shortening `run_watch`, `connection_loop` (about 130 lines) or the 252-line `impl Discord` (a later step if `cs check` still flags them); moving `valid_id` or the retry helpers; any other `src/` file; the `AGENTS.md` row, backlog and changelog (the architect).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`. Create `target/scratch/3fm` first and delete it before reporting.

1. **Builds and tests.** `cargo test --test discord` before editing (save `cargo test --test discord -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fm/before.txt`, 31 lines expected, and record the pass counts) and after: the list is identical (`diff` exit 0) and the counts equal.
2. **Same code.** `old` = `git show HEAD:src/discord.rs`, `new` = `cat src/discord.rs src/discord/watch.rs`. After `sed '/^$/d; /^\(use \|mod \|pub use \|const \)/d'` and `sort`, `diff old new` shows exactly these lines and nothing else (after `cargo fmt`): removed `        let capacity = self.config.queue.max(1);` and `        let config = Arc::clone(&self.config);`; added `        let capacity = config.queue.max(1);`, `        Watch::start(Arc::clone(&self.config))`, `    pub(super) fn start(config: Arc<DiscordConfig>) -> Self {` and `    }`.
3. **Order inside each file.** The architect rebuilds both files from the section 2 ranges and the edits and compares them in order with the developer's files; the tester repeats this independently.
4. `wc -l src/discord.rs src/discord/watch.rs`: both under 1,000. `cs check` on each new file and, from a copy of `git show HEAD:src/discord.rs` saved as `target/scratch/3fm/discord.rs`, on the old file; report the scores. A score below the old one is reported with its warnings (a move cannot change function health); the architect decides. Note that `cs check` scores by path.
5. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass; `cargo test` (the whole suite, 61 suites) passes. The semgrep rules in `.semgrep/` are run by the architect's pre-push gate.
6. `git status --short` shows only ` M src/discord.rs` and `?? src/discord/`.
7. `rg -n 'discord::' src tests --glob '!src/discord*'` and the imports `use a2amx::discord::...` still resolve: `cargo build --all-targets` succeeds with no edit outside scope.

Known flaky tests: `tests/team/reset_and_validation.rs::team_up_forwards_reset_configuration_and_consent` ("screen not ready", backlog I28, an unrelated test). A failure of it is rerun once and recorded as pre-existing if it passes.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed). No separate consultant design read of the diff: the consultant attacked the target shape before locking and the diff is the mechanical move plus the one constructor change it reviewed.
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | the spec did not say where `Watch::start` goes relative to the existing `impl Watch` block, so the in-order rebuild check was not reproducible | placement fixed (first item of the existing `impl Watch`), placement of `mod`/`pub use`/constants/`use` lines added |
| Draft review | consultant | acceptance 2 ("shows exactly the constructor lines") was left to judgment | the expected 2 removed and 4 added lines are listed verbatim |
| Draft review | consultant | wording: "is not needed" for the deleted `let config` line, and a visibility sentence that suggested a crossing that does not exist | both reworded |
| Implementation | developer | no problems found at the first report | none |
| Tester pass | tester | no problems found; one mutant survived: `config.queue.max(1)` to `config.queue` (no test sets `queue` to 0), a coverage gap in the moved guard that exists on the old code too | recorded, not fixed (out of scope for a move) |
| Architect review | architect | no problems found: normalized diff exactly the six listed lines, fmt, clippy, full `cargo test` | none |
