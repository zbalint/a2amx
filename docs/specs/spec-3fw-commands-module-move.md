# Spec 3fw: move the command handlers from the binary into a library `commands` module

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: none new; the
existing binary suites (`tests/daemon_cli`, `tests/team`, `tests/teams`, `tests/auto_confine`, `tests/confine`,
`tests/refs_and_list.rs`, `tests/gateway_*`, `tests/migrate_cli.rs`, `tests/attach_cli/`, ...) run the moved
code through the `a2amx` binary.

Scope (edit):
- `src/main.rs` (remove the moved items, import `commands`, one renamed call), `src/lib.rs` (one `pub mod` line),
  `src/terminal.rs` (one added function, `confirm`);
- new files `src/commands.rs`, `src/commands/home.rs`, `src/commands/daemon.rs`, `src/commands/gateway.rs`,
  `src/commands/launch.rs`, `src/commands/grants.rs`, `src/commands/team.rs`, `src/commands/team/dry_run.rs`,
  `src/commands/sessions.rs`.

Does not touch: every other file under `src/` and `tests/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`,
`docs/` other than this spec, `CHANGELOG.md`. The architect updates the AGENTS.md module table (the `main`, `team` and new `commands` rows) and `docs/architecture.md:399` in the commit that accepts this spec.

## 1. Why

`src/main.rs` is 2,589 lines: the process entry (tracing setup, argument parsing, error reporting) and every
command handler (daemon, gateway, new/launch, auto-grants, team up/down/reset/status, list, kill, reset, explain,
screen, messages). It is the binary crate, so no handler can be reached by anything but the binary under a PTY or
a process, and the shared helpers the handlers use (`confirm`, `resolved_confine_request`, `wired_command`,
`parse_activity_watch`, `auto_grants_for`) cut across the command groups. The owner wants the code readable,
maintainable and more testable, not just smaller files. The next specs (typed daemon requests, pure report
renderers, a launch type, the team up/down/reset cleanup) work on library code and need the handlers there first.
This spec is a pure move: bodies are unchanged, so a reviewer verifies "nothing changed" mechanically with the
judge of section 6, and the behaviour-changing work is not hidden in a large diff (the 3fs approach, consultant
advice adopted).

Rejected, with the reason: moving only the query commands first (the shared helpers stay stranded in `main.rs`
where the library cannot reach them); typed daemon replies first (edits bodies that are about to move); a
"query/session" module (kill and reset mutate; launch helpers are shared by new, team up and dry run, so the
cohesive cut is by responsibility: root preparation, daemon, gateway, launch, grants, team, sessions); one `pub`
function per command (public items are never reported as dead code, which turns off the compiler's free
unused-item check that the later specs rely on; so `commands` exposes exactly one public item, `run`).

Prior lessons, stated here because specs cannot cite memory: a child module sees its parent's private items and
private struct fields, the parent does not see the child's; an item called from a sibling needs `pub(super)`
(visible in the parent and everything under it), a struct field built or read in a sibling needs `pub(super)` too.
Moving a tracing event changes its default target to the module path (accepted, see 3.5).

## 2. Target shape (each module's one responsibility)

| File | One responsibility |
|---|---|
| `src/main.rs` | The process entry: tracing setup and filter, argument parsing and its stdout/stderr split, the root-user refusal, the final error line and exit code |
| `src/commands.rs` | `run`: the command dispatch over `Cli` (the only public item) |
| `src/commands/home.rs` | Which a2amx root a command uses and preparing it: `--home` resolution, old-state migration, the old-layout warning |
| `src/commands/daemon.rs` | The `daemon` commands: run (foreground and background), status, stop |
| `src/commands/gateway.rs` | The `gateway` commands: run, up, status, stop, token |
| `src/commands/launch.rs` | Turning `new` options into a launch request and starting one session (also used by team up and dry run) |
| `src/commands/grants.rs` | Auto-grant detection from a directory, the accepted-grants file and the consent prompt |
| `src/commands/team.rs` | The `team` commands: status, up, down, reset, resolving the team file and the auto-grants of a team up |
| `src/commands/team/dry_run.rs` | The team up dry-run report and command-line redaction |
| `src/commands/sessions.rs` | Commands on existing sessions and messages: list, confine show, kill, reset, explain, screen, messages, cancel, attach |
| `src/terminal.rs` | (existing) gains `confirm`, the yes/no prompt on the human's terminal (its `//!` line gains "the yes/no prompt") |

## 3. The move

### 3.1 Partition (top-level items of `src/main.rs` at the base commit; item text moves unchanged)

The base commit is `8c9246a`. An item is a top-level `fn`, `struct`, `const` with its leading doc comments and
attributes. `use` lines are not items and are rewritten by the developer (3.3). The architect's judge
(section 6) reads `src/main.rs` at the base commit and uses this table.

| Destination | Items |
|---|---|
| `src/main.rs` | `log_target_allowed`, `main` |
| `src/commands.rs` | `dispatch` (renamed `run`) |
| `src/commands/home.rs` | `resolve_home`, `migrate_old_state`, `shell_quote`, `warn_explicit_old_layout` |
| `src/commands/daemon.rs` | `DAEMON_STOP_TIMEOUT`, `run_daemon`, `run_daemon_background`, `read_addrs`, `run_daemon_status`, `run_stop` |
| `src/commands/gateway.rs` | `MAX_GATEWAY_TOKEN_BYTES`, `run_gateway_token`, `read_gateway_token`, `read_gateway_token_line`, `run_gateway_up`, `run_gateway_status`, `run_gateway_stop`, `run_gateway` |
| `src/commands/launch.rs` | `parse_activity_watch`, `run_new`, `NewOptions`, `wired_command` (with its `#[allow(clippy::too_many_arguments)]` and doc), `resolve_confine_paths`, `resolved_confine_request`, `create_session` |
| `src/commands/grants.rs` | `MAX_AUTO_GRANTS_BYTES`, `read_accepted_grants`, `load_accepted_grants`, `save_accepted_grants`, `project_key`, `confirm_new_grants`, `MAX_GIT_CONFIG_BYTES`, `auto_grants_for` |
| `src/commands/team.rs` | `resolve_auto_grants`, `TeamSource`, `TeamOrigin`, `resolve_team_file`, `read_team`, `conflict_text`, `write_conflicts`, `team_label`, `run_team_status`, `note_lost_messages`, `validate_team_session`, `start_planned_session`, `write_auto_summaries`, `issue_lease_code`, `wait_for_enter`, `run_team_up`, `run_team_down`, `run_team_reset` |
| `src/commands/team/dry_run.rs` | `team_detail`, `is_redactable_flag`, `redact_argv`, `dry_run_report`, `dry_run_start`, `session_details`, `activity_summary` |
| `src/commands/sessions.rs` | `run_list`, `run_confine_show`, `run_kill`, `kill_session`, `reset_one`, `run_reset`, `run_explain`, `run_screen`, `run_messages`, `run_cancel`, `run_attach_command` |
| `src/terminal.rs` | `confirm` (appended after the last item of the file; its text moves unchanged) |

2,437 lines move (`main.rs` keeps 46 of the 2,483 item lines). Item lines per destination at the base: `commands.rs` 165, `home.rs` 67, `daemon.rs` 220, `gateway.rs` 145, `launch.rs` 427, `grants.rs` 129, `team.rs` 730, `dry_run.rs` 187, `sessions.rs` 352, `main.rs` 46, `terminal.rs` +15. A file adds a blank line between items and about 20 to 30 lines of `//!`, `use` and `mod` lines; `team.rs` lands near 780.

### 3.2 `src/lib.rs`, new files

`src/lib.rs` gets `pub mod commands;` in alphabetical position (after `codex`, before `config`). Each new file
starts with a `//!` line naming its responsibility (the table of section 2). `src/commands.rs` declares its
children with private `mod home; mod daemon; mod gateway; mod launch; mod grants; mod team; mod sessions;`
and `src/commands/team.rs` declares `mod dry_run;` (private). `dispatch` is renamed `run` and is
`pub async fn run(cli: Cli) -> anyhow::Result<std::process::ExitCode>`.

### 3.3 Visibility and imports

- The only `pub` item under `src/commands*` is `run`. The `pub(super)` set (visible inside `crate::commands`) is
  exactly the following and nothing else; every other item and field stays private. Counts are lines containing
  `pub(super)`:
  - `daemon.rs` (4): `run_daemon`, `run_daemon_background`, `run_daemon_status`, `run_stop`;
  - `gateway.rs` (5): `run_gateway`, `run_gateway_status`, `run_gateway_stop`, `run_gateway_token`, `run_gateway_up`;
  - `home.rs` (3): `resolve_home`, `migrate_old_state`, `warn_explicit_old_layout`;
  - `grants.rs` (4): `auto_grants_for`, `confirm_new_grants`, `load_accepted_grants`, `project_key`;
  - `launch.rs` (24): `run_new`, `create_session`, `parse_activity_watch`, `resolved_confine_request`,
    `wired_command`, the struct `NewOptions` and its 18 fields (`team.rs` builds it with a struct literal);
  - `sessions.rs` (10): `run_attach_command`, `run_cancel`, `run_confine_show`, `run_explain`, `run_kill`,
    `run_list`, `run_messages`, `run_reset`, `run_screen`, `reset_one`;
  - `team.rs` (4): `run_team_down`, `run_team_reset`, `run_team_status`, `run_team_up`;
  - `dry_run.rs` (1): `dry_run_report`;
  - `commands.rs`: none (`run` is `pub`).
  `TeamSource`, `TeamOrigin` and their fields stay private (only `team.rs` and its child `dry_run.rs` use them;
  `dry_run.rs` sees `team.rs`'s private items without a qualifier). rustc does not warn that a used `pub(super)` is
  wider than needed, so the counts above are checked by `rg -c` (section 6).
- `confirm` in `src/terminal.rs` is `pub(crate)`; the handlers call `crate::terminal::confirm`. Its callers are
  `run_stop` (daemon), `confirm_new_grants` (grants), `run_kill` (sessions) and `run_team_reset` (team); `run_gateway` only has a struct field named `confirm` and does not use the function.
- Cross-file calls, a superset found by an identifier scan (the compiler is the final judge: an unused `use` fails
  clippy): `run` calls the 4 daemon, 5 gateway, 1 launch (`run_new`), 9 sessions, 4 team items,
  `grants::auto_grants_for` and `home::resolve_home`; `daemon` and `gateway` call `home` (`migrate_old_state`,
  `warn_explicit_old_layout`); `team` calls `grants` (`auto_grants_for`, `confirm_new_grants`,
  `load_accepted_grants`, `project_key`), `launch` (`NewOptions`, `create_session`, `parse_activity_watch`, `resolved_confine_request`),
  `sessions::reset_one` and `dry_run::dry_run_report`; `dry_run` calls `launch` (`resolved_confine_request`,
  `wired_command`).
- Names: `commands.rs` declares `mod daemon; mod gateway; mod team;`, which shadow `crate::daemon`, `crate::gateway`
  and `crate::team` inside `commands.rs`; `run` uses none of those crate modules (it uses `mcp`, `hook`,
  `statusline`, `bridge`, `grant`, `suggest`, `config`, `layout`), so `commands.rs` imports none of the three. The
  child files import `crate::team`, `crate::daemon`, `crate::gateway` freely.
- Paths: inline `a2amx::` paths in moved code become `crate::` (the judge reads them as equal). A plain `use` for
  every name a file needs, at the top of the file; no glob in `src/commands*` (`src/main.rs` keeps
  `use tracing_subscriber::prelude::*;` unchanged); no inner attribute (`#![...]`) in any file; no `#[allow]` or
  `#[expect]` is added or removed (the one `#[allow(clippy::too_many_arguments)]` on `wired_command` travels with
  it). Add no comment between items and use no qualifier other than `pub(super)` (the judge cannot read
  `pub(in ...)` or a column-0 `//` comment between items, and a pure move adds neither).

### 3.4 `src/main.rs`

Keeps `log_target_allowed` and `main`. `main` calls `commands::run(cli).await` instead of `dispatch(cli).await`
(import `a2amx::commands`); no other line of `main` changes. Remove every import the remaining code does not use
(clippy reports them). `main.rs` imports only what `main` needs: `clap::Parser`, `tracing_subscriber`,
`a2amx::cli::Cli`, `a2amx::commands`, `a2amx::terminal::{write_stderr, write_stdout}`, and keeps
`use tracing_subscriber::prelude::*;` as it is.

### 3.5 Behaviour preserved

No behaviour change. The default tracing target of moved events changes from `a2amx` to the module path
(`tracing::error!(%error, "cannot install SIGTERM handler")` in `run_gateway`, and any other `tracing` macro in
moved code); the default filter only drops `tungstenite`, `ureq` and `rustls` targets by prefix (the filter in
`main` is unchanged), and no test or doc names the moved events' target (the `rg` in section 7). `std::env::current_exe()`,
`daemon::current_exe()`, the `pre_exec` closure and the signal handlers behave the same in the library: they use
the running process, not the crate.

## 4. New tests

None. Every moved handler is exercised through the binary by the existing suites, which pass unchanged; the
judge shows nothing else changed. Characterization tests for commands whose output no test pins today are a
separate spec (the consultant's list: `list --viewer`, the `daemon stop` refusal/decline text, the `kill` and
`reset` decline paths, `gateway stop` success line, `gateway token is empty`, `does not support --confine`, `no
free generated session name`), which comes before any spec that changes a handler body.

## 5. Out of scope

- Any change to a moved body: typed daemon replies, pure renderers, a launch type, the `home` preamble dedupe,
  the `team down`/`team reset` common code, `confirm`'s `io::stdin().is_terminal()` versus
  `terminal::stdin_is_terminal`, the client.toml load before every command, double daemon connections in
  reset/explain/screen/messages/attach, the silent swallow in `note_lost_messages` (a `shortcut:` comment would be a
  body edit; later spec), the synchronous pairing-confirm closure in `run_gateway` (check `relay` later).
- `src/daemon.rs` and every other large file.
- `AGENTS.md`, `docs/architecture.md` (the line "the binary (`src/main.rs`) hands the result to `DaemonConfig`"
  becomes stale: the architect updates it in the accepting commit), `CHANGELOG.md` (internal change).

## 6. Acceptance

Run from the main checkout. Judge and command names are exact.

- `python3 -I target/scratch/3fw/verify.py 8c9246a` prints `RESULT PASS`. It checks, per destination file, that
  the multiset of top-level items equals the base items of section 3.1 after normalizing `pub`, `pub(crate)`,
  `pub(super)` qualifiers removed, `a2amx::` read as `crate::`, all whitespace outside string literals removed,
  trailing commas before a closing bracket removed (rustfmt wrapping), `fn dispatch(` read as `fn run(` and
  `dispatch(cli)` as `commands::run(cli)` in `main`; every moved item is found in exactly the file named
  (extra or missing items fail); an inner attribute fails. `use` lines and `mod` lines are not compared (the
  compiler and clippy judge them). The architect owns this file; the developer runs it, never edits it.
- `cargo test --no-fail-fast`: all pass. Baseline at the base commit (architect, 2026-11-21 session): 64 suites
  ok, 1393 passed, 0 failed, 3 ignored. After: the same suites and counts; the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`: pass, no warnings.
- `wc -l src/main.rs src/commands.rs src/commands/*.rs src/commands/team/*.rs`: `main.rs` at most 120 lines,
  every other new file under 800 lines.
- `rg -n '^\s*pub ' src/commands.rs src/commands/` prints only `pub async fn run(`; `rg -n 'pub\(crate\)'
  src/commands.rs src/commands/` prints nothing; `rg -n 'use .*\*;' src/commands.rs src/commands/` prints nothing.
- `rg -c 'pub\(super\)' src/commands.rs src/commands/` prints exactly `daemon.rs:4`, `gateway.rs:5`, `home.rs:3`,
  `grants.rs:4`, `launch.rs:24`, `sessions.rs:10`, `team.rs:4`, `team/dry_run.rs:1` and no line for `commands.rs`.
- `git status --short` lists only the files of section 0; `git diff --stat -- Cargo.toml Cargo.lock` is empty;
  `git diff -- src/terminal.rs` shows only the `confirm` addition, the imports it needs and the `//!` line.
- The binary suites pass unchanged. Known flaky: `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error`
  (I29), `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode` (I7),
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session` (I18),
  `tests/presence` `viewers::viewer_attach_keeps_input_read_only_while_sizing_session`,
  `tests/attach_cli/viewer.rs::viewer_team_click_acts_on_release_and_keeps_viewer_mode`,
  `tests/attach_cli/attach_session.rs::client_file_prefix_reaches_attached_session`,
  `tests/discord/gateway_queue.rs::gateway_queue_reads_heartbeats_while_consumer_is_slow`: rerun a failure once
  and record it as pre-existing if it passes.
- No mutation check (no new tests). The tester (assigned by the architect afterwards) reruns the judge and the
  suite and checks the visibility and diff conditions above; a consultant reads the design (a refactor).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| `src/main.rs` has 2,589 lines and the items of 3.1 cover every top-level non-`use` item exactly once | `target/scratch/3fw/partition.py` | run by the architect: no unassigned, no unknown names |
| The cross-file references of 3.3 are a superset (a field named `confirm` in `run_gateway` matches the scan) and the `pub(super)` set follows the code: `TeamOrigin.dir` is the only team field read outside `team.rs` and moves out with `resolve_auto_grants`; `NewOptions` is built in `start_planned_session` | consultant read of main.rs 492, 761, 786, 1177, 1192, 1824 | read; the compiler judges |
| The judge passes on a correct rebuild and fails on a one-string change | simulated rebuild and probe in scratch (deleted) | run |
| Nothing in `main.rs` is binary-only except `#[tokio::main]` and the tracing setup | read of `src/main.rs` and consultant read | read |
| The one `unsafe` block carries its `// SAFETY:` comment inside `run_daemon_background` (line 642) and moves with it | `rg -n SAFETY src/main.rs` | run |
| `#[allow(clippy::too_many_arguments)]` occurs once, on `wired_command` (line 998) | `rg -n '#\[allow' src/main.rs` | run |
| `dispatch` is called only from `main` (line 128) | `rg -n '\bdispatch\b' src tests` | run (other hits: a `unreachable!` message text and docs prose) |
| `confirm` has no callers outside `src/main.rs` (`relay.rs` has its own `confirm` closure) | `rg -n 'confirm\(' src` | run |
| No string literal in `src/main.rs` contains `a2amx::` | `rg '"[^"]*a2amx::[^"]*"' src/main.rs` | run, no match |
| Only one `tracing` macro is used in `src/main.rs` (line 515) and no test names it | `rg 'tracing::' src/main.rs`; `rg -n 'cannot install SIGTERM handler' tests` | run, no test match |
| `Cargo.toml` has no `[features]`; lints (`unwrap`, `expect`, undocumented unsafe, ...) are package-wide, so a move does not change them | consultant read | read; clippy judges |
| `docs/architecture.md:399` and the `team` row of `AGENTS.md` ("the commands live in `main`") name `src/main.rs` handing a result to `DaemonConfig` | `rg 'main.rs' docs/architecture.md` | run |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (fresh consultant sessions for the design consult, the draft
review and the design read); junior developer OMP openai-codex/gpt-5.6-luna (automatic advisor); tester OMP
opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the cross-file list named `confirm` as called by `gateway`, but `run_gateway` only has a struct field of that name (a word-boundary scan matches field names), so a developer would add an unused import | list corrected, scan results described as a superset |
| draft review | consultant | the struct-field visibility rule was imprecise: `TeamSource` fields were said to be used in `grants.rs`; only `NewOptions` (18 fields) and `TeamOrigin.dir` cross files | the `pub(super)` set written as an exact counted list and audited by `rg -c` |
| draft review | consultant | `resolve_auto_grants` made `grants` depend on `team`, and `resolve_home` sat in `commands.rs` away from the root preparation | `resolve_auto_grants` moved to `team.rs` (`TeamOrigin` private), `resolve_home` to `home.rs` |
| draft review | consultant | prelude glob in `main.rs` contradicted the no-glob rule; the `mod daemon/gateway/team` names shadow crate modules in `commands.rs`; no row for the tracing-target claim; line count 2,480 was 2,437; `AGENTS.md:183` and the `terminal.rs` `//!` line missing from the doc/scope lists | each stated in the spec before the lock |
| tester pass | tester (correction m_8272) | the new files had no blank line between items although section 3.1 promised one, and no acceptance check covered it | developer fix request (whitespace only); `rg -U` boundary audit run, prints nothing |
