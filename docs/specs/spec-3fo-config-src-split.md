# Spec 3fo: split `src/config.rs` into the daemon root and `gateway_file` and `client_file` child modules (refactor)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at this spec's own commit (it descends from `3e5c1e3`, spec 3fn accepted).
Shared task `context_id`: `a2amx-codescene-refactor-batch7-2026-11-18`.

**Scope.** May edit exactly these paths and no others.
- `src/config.rs`: shrinks to the root (section 2).
- `src/config/gateway_file.rs`, `src/config/client_file.rs`: new.

Does not touch: every other file under `src/`, `tests/`, `Cargo.toml`, `Cargo.lock`, `docs/`, `AGENTS.md`, `CHANGELOG.md`. Do not commit, stage or merge. The architect updates the backlog and the review record in the accepting commit.

Public test seam: the `config` module's public functions as used by `src/main.rs`, `src/relay.rs`, `src/relay_files.rs`, `src/gateway_launch.rs` and the integration tests (`cargo test`, in particular the suites that call `a2amx config show`, the gateway and client config). `src/config.rs` has no inline unit tests. No test is added, removed or changed.

## 1. Why

`src/config.rs` is 1152 lines, over the owner's goal of no `.rs` file over 1,000 lines; the owner told the architect to continue the refactor with the source files (spec 3fm, `src/discord.rs`, and spec 3fn, `src/team.rs`, are done). The module table row for `config` names three policy files, `daemon.toml`, `gateway.toml` and `client.toml`, and the code is grouped by file, with the halves interleaved (for example the client display functions at 644 sit between `ensure_daemon_file` and `append_template_keys`, and `GatewayKey` at 803 sits between `show_daemon` and `source_label`), which is why the cut below has twelve ranges: the daemon half (`Source`, `Overrides`, `DaemonSettings`, `LoadedDaemon`, `FileConfig`, `KEYS`, `load_daemon`, `show_daemon`, `daemon_template`, the shared readers and validators), the gateway half (`GatewayFile`, `GatewaySettings`, `LoadedGateway`, `GatewayKey`, `GATEWAY_KEYS`, `load_gateway`, `gateway_template`, `ensure_gateway_file`, `show_gateway`) and the client half (`ClientFile`, `ClientSettings`, `ClientOverrides`, `LoadedClient`, `ClientKey`, `CLIENT_KEYS`, `load_client`, `client_template`, `ensure_client_file`, `show_client`). Measured by script on HEAD (all names are items defined in `src/config.rs`): the gateway ranges and the client ranges below use only root items (`DurationRule`, `Source`, `append_template_keys`, `ensure_file`, `file_error`, `format_duration`, `parse_limited_duration`, `resolve`, `source_label`, plus `parse_counter` for the gateway and `read_config` for the client); neither uses an item of the other; and the root uses no item defined in either range. That makes both a clean leaf of the root: the shared readers, validators and the `Source`/`Key` machinery are the root, the two files that build on them are children. The module table row stays true.

The child names `gateway_file` and `client_file` avoid the namesakes `crate::gateway` (the chat gateway core) and `crate::client` (the attach client), and match `ensure_gateway_file` and `ensure_client_file`.

Alternatives rejected: the shorter names `gateway` and `client` (they compile, but make grep, file pickers and CodeScene output ambiguous with `src/client.rs` and `src/gateway.rs`); a `src/config/mod.rs` (the repository uses flat root files; specs 3fm and 3fn chose the 2018 layout); moving the shared helpers into a `common` child (a third module for helpers both children import from the root adds a hop for no gain); extracting the 152-line `load_daemon` or the 69-line `KEYS` table (a later step, not a move).

## 2. Target shape

`src/config.rs` stays the root and declares the children. Every public path stays: `config::{load_daemon, load_client, load_gateway, show_daemon, show_client, show_gateway, ensure_*_file, *_template, parse_duration, format_duration, file_error, Overrides, ClientOverrides, GatewaySettings, ...}`.

| File | Old lines of `src/config.rs` (HEAD) | Contents |
| --- | --- | --- |
| `src/config.rs` | 1-66, 130-643, 724-732, 774-802, 811-818, 1061-1152, and the declarations below | the imports, `Source`, `Overrides`, `DaemonSettings`, `LoadedDaemon`, `FileConfig`, `FileLimits`, `Key`, `DefaultValue`, the `display_*` functions, `KEYS`, `resolve`, `file_error`, `read_config`, `parse_file`, `parse_counter`, `DurationRule`, `validate_duration`, `parse_limited_duration`, `default_listen`, `parse_listen`, `validate_host`, `KeySources`, `load_daemon`, `ensure_file`, `ensure_daemon_file`, `append_template_keys`, `show_daemon`, `source_label`, `default_text`, `daemon_template`, `parse_duration`, `format_duration` (about 720 lines) |
| `src/config/gateway_file.rs` | 67-101, 803-810, 819-1060 | `GatewayFile`, `GatewaySettings`, `LoadedGateway`, `GatewayKey`, the `gateway_display_*` functions, `GATEWAY_KEYS`, `load_gateway`, `gateway_template`, `ensure_gateway_file`, `show_gateway` (about 290 lines with imports) |
| `src/config/client_file.rs` | 102-129, 644-723, 733-773 | `ClientSettings`, `ClientOverrides`, `LoadedClient`, `ClientFile`, `DEFAULT_CLIENT_PREFIX`, `DEFAULT_CLIENT_VIEWER_AFTER_SECS`, `client_display_prefix`, `client_display_viewer_after`, `ClientKey`, `CLIENT_KEYS`, `load_client`, `client_template`, `ensure_client_file`, `show_client` (about 160 lines with imports) |

The ranges partition lines 1-1152 exactly (the architect checked the sum, 1152, and that every range starts on an item's first attribute or comment line). `append_template_keys` (724-732) stays in the root because both `client_template` and `gateway_template` call it.

The edits, and the only edits besides the moves:

1. **Declarations in `src/config.rs`.** After the `use` lines and before the first item, in this order: `mod client_file;`, `mod gateway_file;`, a blank line, `pub use client_file::{ClientOverrides, ClientSettings, LoadedClient, client_template, ensure_client_file, load_client, show_client};` and `pub use gateway_file::{GatewaySettings, LoadedGateway, ensure_gateway_file, gateway_template, load_gateway, show_gateway};` (`cargo fmt` chooses the line breaks). These are exactly the `pub` items of the two ranges; the private items stay private.
2. **Visibility.** None changes. Moved `pub` items stay `pub` and are re-exported; moved private items stay private and are used only inside their own file. The root helpers the children call are private items of the root, which a child module can name (`use super::{...}`), so they need no change.
3. **Imports.** Each file keeps exactly the `use` lines it needs; the children name the root helpers they use through `use super::{...}`. The architect's grep expects: `gateway_file.rs` uses from the root `DurationRule`, `Source`, `append_template_keys`, `ensure_file`, `file_error`, `format_duration`, `parse_counter`, `parse_limited_duration`, `resolve`, `source_label`; `client_file.rs` uses `DurationRule`, `Source`, `append_template_keys`, `ensure_file`, `file_error`, `format_duration`, `parse_limited_duration`, `read_config`, `resolve`, `source_label` (a name unused after all is dropped by the compiler's and clippy's `unused_imports`). The root probably drops none of its imports (the architect's grep finds each still used by root code); the children need `std::fs`, `std::time::Duration`, `anyhow::{Context, anyhow, bail}`, `serde::Deserialize`, `crate::layout::Layout` (gateway) and `Duration`, `anyhow::anyhow`, `Deserialize`, `Layout` (client); `toml::`, `crate::prefix::` and `crate::relay_files::` are written as full paths there. The compiler and clippy decide.
4. **Everything else** is moved byte-for-byte, comments included, with the order inside each file unchanged; the moved items keep their relative order. `cargo fmt` runs once after assembly.
5. **Blank lines.** The root joins its ranges 66→130 and 802→811 with no blank line between items, as HEAD does at 66/67 and 802/803; adding one blank line there is allowed (acceptance 2 ignores blank lines).

## 3. What must not change

- Every public path and signature, every key name, default, error text and the exact text of `config show` and the templates. No renames, no new `#[allow]`, `#[cfg]` or suppression, no changed `// shortcut:` comment.
- No test file changes.

## 4. Out of scope

Shortening `load_daemon`, `load_gateway` (about 120 lines), `KEYS` or the three near-identical `show_*`/`*_template` functions (a later step if `cs check` still flags them); other `src/` files; the backlog, review record and changelog (the architect).

## 5. Acceptance

From the main checkout, as an ordinary user, with the environment form in `AGENTS.md`. Create `target/scratch/3fo` first and keep it until the architect accepts.

1. **Baseline.** Before editing: save `cargo test -- --list 2>/dev/null | rg ': test$' | sort > target/scratch/3fo/before.txt` (check the file is not empty; `2>/dev/null` hides a build failure) and the pass/ignored totals of a full `cargo test`; save `git show HEAD:src/config.rs` as `target/scratch/3fo/config.rs`. After the split the sorted list is identical (`diff` exit 0) and the totals equal (61 suites, 1347 passed, 3 ignored expected).
2. **Same code.** `old` = `git show HEAD:src/config.rs`, `new` = `cat src/config.rs src/config/gateway_file.rs src/config/client_file.rs`. After `sed '/^\(pub \)\?use .*{$/,/^};$/d; s/^[[:space:]]*//; /^$/d; /^\(use \|mod \|pub use \)/d'` and `sort`, `diff old new` prints nothing (exit 0): the file has no visibility change and no inline test module, and the first expression deletes every multi-line `use` or `pub use` block (the root's, the children's and the two re-export declarations) from both sides, so rustfmt's line breaking of them does not show.
3. **Order inside each file.** The architect rebuilds the three files from the section 2 ranges and compares in order with the developer's; the tester repeats this independently.
4. `wc -l src/config.rs src/config/*.rs`: every file under 1,000. `cs check` on each new file and on the old copy (`target/scratch/3fo/config.rs`); report the scores.
5. `cargo build --all-targets`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; the whole `cargo test` passes. Save `rg -n 'config::' src tests | sort > target/scratch/3fo/uses-before.txt` before editing; the same command after editing diffs empty (no use site changed).
6. `git status --short` shows only ` M src/config.rs` and `?? src/config/`.

Known flaky tests: `tests/team/reset_and_validation.rs::team_up_forwards_reset_configuration_and_consent` ("screen not ready", backlog I28, unrelated). A failure of it is rerun once and recorded as pre-existing if it passes.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review, model slot not reported); developer Antigravity (agy), Gemini 4 Hydrogen (Medium), one model for all slots; tester OMP opencode-go/deepseek-v4.1-flash (advisor model not exposed). No separate consultant design read of the diff: the consultant attacked the target shape before locking and the diff is the mechanical move it reviewed.
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | the Why said the code "is already laid out per file", but the halves are interleaved (the reason for twelve ranges) | wording corrected |
| Draft review | consultant | the root probably drops none of its imports, which the spec said it drops; the children's expected imports were not listed | import expectations corrected and listed |
| Draft review | consultant | the module names `gateway` and `client` would be namesakes of `crate::gateway` and `crate::client` | renamed `gateway_file` and `client_file` |
| Draft review | consultant | the public test seam listed `src/daemon.rs`, which does not use `config`, and missed `relay_files.rs` and `gateway_launch.rs`; acceptance 5 had no pass or fail | seam list corrected; acceptance 5 became a before and after diff of the use sites |
| Implementation | developer | no problems found; the root had no blank line after its `pub use` block | the architect added the blank line (whitespace only), tester told |
| Tester pass | tester | no problems found; seven mutants killed, none survived | none |
| Architect review | architect | no problems found: normalized diff empty, fmt, clippy; the full `cargo test` failed once on an unrelated flaky test (`tests/agy_plugin.rs`, backlog I29) and passed on the rerun (61 suites, exit 0); `cs check` root 8.10, `gateway_file` 9.34, `client_file` 10.00 against 7.58 for the old file | none |
