# Spec 3fs: move the attach screen and the CLI plumbing from the binary into the library

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seams: the new public
library items named in section 4 and the existing PTY suite in `tests/attach_cli/`.

Scope (edit):
- `src/main.rs` (remove the moved items, add imports and the three renamed call sites of section 3.4),
  `src/lib.rs` (four `pub mod` lines);
- new files `src/terminal.rs`, `src/table.rs`, `src/lookup.rs`, `src/attach.rs`, `src/attach/state.rs`,
  `src/attach/input.rs`, `src/attach/event_loop.rs`, `src/attach/picker.rs`;
- new tests `tests/table.rs` and `tests/lookup.rs`.

Does not touch: every other file under `src/` and `tests/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`,
`docs/` other than this spec, `CHANGELOG.md`. The architect updates the AGENTS.md module table in the commit
that accepts this spec.

## 1. Why

`src/main.rs` is 4,461 lines and holds three different things: the command handlers, the shared plumbing they
use (terminal writes and guards, the session list queries, the table formatting), and the whole attach screen
(a 680-line state type, its event loop, the session picker). Because it is the binary crate, none of it can be
reached by a test except by running the binary under a PTY. The owner wants the code readable and maintainable
and wants more of it testable, not just smaller files. The redesign of the attach screen (later specs) needs it
in the library first, and the attach code cannot move alone: it calls the plumbing, and a library cannot call
back into a binary. So this spec moves the attach screen and exactly the plumbing it shares with the commands
into four library modules and changes nothing else. It is a pure move: bodies are unchanged, the redesign and
the listed bug fixes come in later specs, so a reviewer can verify "nothing changed" mechanically, and the
behaviour change work is not hidden inside a large diff (consultant advice, adopted).

Rejected: moving only the attach files (does not compile: they call `write_stderr`, `session_in_team`,
`request_sessions`, `terminal_size_with_default` and the picker's table helpers); redesigning while moving
(see above); one big `commands` module for everything now (the command handlers are a separate spec, S5).

Prior lessons, stated here because specs cannot cite memory: a child module sees its parent's private items and
private struct fields, the parent does not see the child's, so a struct used by several children stays in the
parent with its impls in the children, and only an item called from a sibling or another module needs wider
visibility. Moving a tracing event changes its default target to the module path; that is accepted as not a
behaviour change (section 3.5).

## 2. Target shape (each module's one responsibility)

| Module | Responsibility | Owns |
|---|---|---|
| `terminal` | the human's terminal: raw writes, the output thread, restore-on-exit guards, size and tty tests | `write_stdout`, `write_stderr`, `Output`, `OutputHandle`, `EchoGuard`, `TerminalGuard`, `restore_terminal`, `run_cleanup`, `stdin_is_terminal`, `terminal_size_with_default`, `DEFAULT_COLS`, `DEFAULT_ROWS`, `RESTORE_TERMINAL` |
| `table` | turning sessions and messages into aligned text tables (pure) | the headers, the value-row builders, `column_widths`, `format_table`, `format_row`, `format_session_table`, `format_messages_table`, `session_confinement_mode` |
| `lookup` | asking the daemon for the session list and resolving a user's session reference | `request_sessions`, `resolve_reference`, `resolve_session`, `session_in_team` |
| `attach` | the interactive attach screen | root: `AttachOptions`, `run` (was `run_attachment`), `pty_rows`, `team_rows`, `AttachOutcome`, `MousePress`, `AttachmentConfig`, `AttachmentState`, `PickerState` (structs stay in the root so their private fields are visible to the children), `INPUT_CHUNK`, `MOUSE_CAPTURE` |
| `attach::state` | `impl AttachmentState`: drawing, input handling, member navigation, session switching | the 680-line impl, unchanged |
| `attach::input` | parsing terminal input: scroll keys, mouse reports, focus reports | `ScrollParser`, `ScrollKey`, `parse_scroll_sequence`, `scroll_status`, `parse_decimal`, `is_focus_report`, `find_mouse_report` |
| `attach::event_loop` | the select loop over stdin, resize, the viewer tick and daemon frames | `run_attachment_loop`, `recv_attachment`, `spawn_stdin_reader` |
| `attach::picker` | the session picker: `impl PickerState`, rendering, entries and row layout | `impl PickerState`, `picker_line`, `picker_entries`, `session_rows`, `shorten_left`, `PICKER_HEADERS`, `picker_value_rows` |

Placement check (spec-writing gate 14): each item sits in the module that owns its responsibility; `session_confinement_mode`
moves into `table` because `format_session_table` needs it and it is a display label; `PICKER_HEADERS` and
`picker_value_rows` are picker-specific cells and go with the picker, which calls `table::column_widths`,
`table::format_row` and `table::session_value_rows`. The `attach` children total about 1,350 lines before the
redesign; `state` (about 680 lines) is the largest and is redesigned in S4. No module reaches into another's
private state: everything a sibling or another module calls is listed in section 3.3.

## 3. The move

### 3.1 Partition (line ranges of `src/main.rs` at the base commit; item text moves unchanged)

| Destination | Ranges (inclusive) |
|---|---|
| `src/terminal.rs` | 47-48, 110, 3619-3642, 3889-4148 |
| `src/table.rs` | 1501-1506, 3644-3651, 3656, 3658-3688, 3724-3887 |
| `src/lookup.rs` | 4336-4342, 4370-4394 |
| `src/attach.rs` | 49, 109, 160-164, 2618-2750, 4203-4213 |
| `src/attach/state.rs` | 2752-3433 |
| `src/attach/input.rs` | 3435-3461, 4150-4201 |
| `src/attach/event_loop.rs` | 3463-3617 |
| `src/attach/picker.rs` | 3652-3655, 3690-3722, 4215-4334, 4344-4368, 4396-4461 |

Everything else stays in `src/main.rs`, including `run_attach_command` (2603-2616), which calls
`attach::run` and `lookup::resolve_session`. Ranges include each item's leading doc comments and attributes
and its trailing blank line is not counted. The architect's judge (section 6) uses exactly these ranges.

### 3.2 `src/lib.rs`

Add `pub mod attach;`, `pub mod lookup;`, `pub mod table;`, `pub mod terminal;` in alphabetical position among
the existing `pub mod` lines. Each new file starts with a `//!` comment of one or two lines naming its
responsibility (from the table in section 2). `attach.rs` declares its four children with `mod state;`,
`mod input;`, `mod event_loop;`, `mod picker;` (private).

### 3.3 Visibility (the narrowest that compiles; edits to item text are only `pub` qualifiers)

- Used by `src/main.rs` after the move: `pub`. These are exactly: `terminal::{write_stdout, write_stderr,
  stdin_is_terminal, terminal_size_with_default, EchoGuard}` (with `EchoGuard::enter`),
  `table::{format_session_table, format_messages_table, session_confinement_mode}`,
  `lookup::{request_sessions, resolve_reference, resolve_session}`, `attach::{AttachOptions, run}`;
  `AttachOptions`' two fields `prefix` and `viewer_after` are `pub` because `main.rs` builds it.
  `lookup::session_in_team` is used only inside the library and is `pub(crate)`.
- Used by another module of the library, not by `main.rs`: `pub(crate)`. Known today: `terminal::{Output,
  OutputHandle, TerminalGuard}` with the methods `attach` calls (`Output::{start, handle, send, finish}`,
  `OutputHandle::{send, flush}`, `TerminalGuard::{enter, restore}`), `table::{column_widths, format_row,
  session_value_rows}`, `lookup::session_in_team`.
- Used only by a sibling or the parent inside `attach`: `pub(super)`. Known today: every item of `input` that
  `state`, `event_loop` and the root use (`ScrollParser` with its methods, `ScrollKey`, `find_mouse_report`,
  `is_focus_report`, `parse_decimal`, `scroll_status`), the `state` methods the root (`AttachmentState::new`) and the loop call, the
  `picker` methods `state` and the loop call, `event_loop::run_attachment_loop`.
- Everything else stays private. A `pub`, `pub(crate)` or `pub(super)` that nothing needs is a finding at
  review. The architect checks with `rg` over the new files; compile errors name every missing one.
- A `use` of a moved name inside another new file is a plain `use`, never `use super::*` or a glob. All `use`
  lines sit at the top of each file, and no file carries an inner attribute (`#![...]`); the judge fails one.
- The crate already has a top-level `picker` module. Inside `attach` the child `picker` shadows it for relative
  paths, so every path to the crate's module is written `crate::picker::...`. The child keeps the name `picker`
  (it is the picker's attach glue); renaming it is left to S4.

### 3.4 `src/main.rs`

Remove the moved items. Add the imports for the moved names it calls (`a2amx::terminal`, `a2amx::table`,
`a2amx::lookup`, `a2amx::attach`), remove its now-unused imports (clippy reports them). Three call sites change
because of the rename `run_attachment` to `attach::run`: line 994 (in `run_new`), line 2083 (in
`run_team_up`) and line 2615 (in `run_attach_command`). No other line of the remaining items changes.

### 3.5 Behaviour preserved

No behaviour change. In moved code the default tracing target of three events (`terminal output cleanup
failed`, `terminal output thread panicked`, `terminal restoration failed`) changes from `a2amx` to
`a2amx::terminal`; this is adjudicated as not a behaviour change and needs no BLOCKED round. No test or doc
names these events (`rg` over `tests/`, `src/` and `docs/` found none). Inline `a2amx::` paths inside moved
code (two lines in the picker render) become `crate::`; the same applies to any other path the compiler
requires.

## 4. New tests (additive; the moved tables and references have no direct library tests today)

The tables and the reference resolution are pure and are now public, so they get literal-expectation tests.
Expected values are worked by hand from the rules below and written as literals before the first run, never
recomputed by calling the code under test.

Rules for the worked examples: cells are padded to the widest cell of their column (header included), columns
are separated by two spaces, the last column is not padded, every line ends with `\n`.

`tests/table.rs`:
1. `messages_table_aligns_columns_to_the_widest_cell`: one message `id "m_1"`, `from "a@h"`, `to "b@h"`,
   `state "delivered"`, `subject "hi"`, no detail, hold reason or evidence. Expected exactly
   `"ID   FROM  TO   STATE      DETAIL  SUBJECT\nm_1  a@h   b@h  delivered  -       hi\n"`.
2. `message_detail_prefers_detail_then_hold_reason_then_receipt_evidence`: the DETAIL cell is `detail` when set,
   else `hold_reason`, else the evidence when it is `submission_observed` or `native_receipt`, else `-`. Fixture,
   four messages, expected DETAIL cells in order: (`detail "d"` and `hold_reason "h"` both set) `d`;
   (`hold_reason "h"` and `evidence "native_receipt"`) `h`; (`evidence "submission_observed"` only)
   `submission_observed`; (nothing set) `-`. A second test: `evidence "other"` alone shows `-`. Assert on the
   DETAIL cell of each row of the exact table text.
3. `a_long_value_widens_its_column_and_shorter_rows_are_padded`: two messages, one with a subject-independent
   long `from`; check the exact text.
4. `session_table_has_a_team_column_only_when_a_session_has_a_team`: without teams the header is the 13 session
   headers; with one session in a team a `TEAM` column is inserted as the third column (after `ID` and `NAME`),
   a team-less row shows `-`, a private team shows `demo (private)`.
5. `session_table_details_add_cwd_and_command_columns`: one session `id "s1"`, `name "a"`, harness Claude, no
   exit code, no activity, not attached, not a viewer, pending 0, no hold reason, no quota, 80x24, no uptime, no
   activity seconds, no confinement, no team, `cwd "/w"`, `argv ["cat", "-n"]`, `details = true`. Expected exactly
   `"ID  NAME  HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE   CWD  COMMAND\ns1  a     claude   off      running  -       -         -         no        0        -     -      80x24  /w   cat -n\n"`.
   A second case with `cwd` unset shows `-` in the CWD cell (assert the text of that row).
6. `confinement_mode_label`: no confinement `off`, auto `auto`, manual `manual`.

`tests/lookup.rs`:
7. `resolve_reference_maps_a_name_to_its_id_and_leaves_other_references`: a name resolves to its session id; an id
   and an unknown text come back unchanged. The duplicate-name case is deliberately not pinned (the daemon
   keeps names distinct, and S4 may change the lookup).

Build `SessionSummary` and `MessageInfo` values with all fields spelled out, as `tests/wire/summaries.rs`
(`SessionSummary`) and `tests/wire/messaging.rs` (`MessageInfo`) do (fictional names, `host-a`). Related
binary-level tests already exist (`tests/refs_and_list.rs`, `tests/attach_cli/list_and_messages.rs`); the new
tests add fast, literal, mutation-tight library coverage and do not replace them.

## 5. Out of scope

Any change to the moved bodies; redesigning `AttachmentState`, the input routing or the picker; the listed
attach bug fixes (specs S3, S4); moving the command handlers (S5); the session list requests of the commands
becoming a `Client` method; renaming other items; deleting the `shortcut:` comments; `AGENTS.md`/docs updates
(architect); a `commands` module.

## 6. Acceptance

Run from the main checkout. Judge and command names are exact.

- `python3 -I target/scratch/3fs/verify.py <base-commit>` prints `RESULT PASS`. It checks, per destination
  file, that the multiset of items equals the base ranges of section 3.1 after normalizing: `pub`/`pub(crate)`/
  `pub(super)` qualifiers removed, `a2amx::` read as `crate::`, all whitespace removed, trailing commas before a
  closing bracket removed (rustfmt wrapping), `fn run(` read as `fn run_attachment(` in `attach.rs`; and that
  the remainder of `src/main.rs` equals the base remainder with `attach::run(` read as `run_attachment(`. The
  architect owns this file; the developer runs it, never edits it. A body edit, a lost or duplicated item, an item in
  the wrong file or an inner attribute fails it; `mod x;` lines and `use` lines are not compared (the compiler
  and clippy judge them).
- `cargo test --no-fail-fast`: all pass. Baseline at the base commit (run by the architect, 2026-11-20): 61
  suites ok, 1358 passed, 0 failed, 3 ignored. After: 63 suites (the two new test targets) and 1358 plus the
  new tests passed; the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`: pass, no warnings.
- `wc -l src/main.rs src/attach.rs src/attach/*.rs src/terminal.rs src/table.rs src/lookup.rs`: `main.rs`
  at most 2,700 lines, every other new file under 1,000.
- `git status --short` lists only the files of section 0; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Visibility audit by the architect: `rg -n 'pub(\(crate\)|\(super\))? ' src/attach.rs src/attach/ src/terminal.rs
  src/table.rs src/lookup.rs` shows nothing wider than section 3.3 requires.
- `tests/attach_cli/`, `tests/refs_and_list.rs` and `tests/team/` pass unchanged (they exercise the moved code
  through the binary).
  Known flaky: `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error` (I29),
  `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode` (I7),
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session` (I18): rerun a
  failure once and record it as pre-existing if it passes.
- Mutation check for the new tests (section 4): on the checkout, mutate `src/table.rs` and `src/lookup.rs` one
  line at a time, run the one named test, record the killing test, restore by checksum. Required mutants: the
  two-space column separator to one space (1); `max` to `min` in `column_widths` (3); swap the `detail` and
  `hold_reason` order (2); the team column insert index 2 to 1 in both `insert(2, ...)` calls, one mutant per call (4); drop the `CWD`/`COMMAND` push (5); `"off"` to
  `"on"` (6); the name comparison `==` to `!=` in `resolve_reference` (7). After the pass `git diff --stat --
  src/table.rs src/lookup.rs` shows only the developer's own move.
- The tester (assigned by the architect afterwards) reruns an independent mutation pass; the architect reviews
  the diff, runs the judge, and a consultant reads the design (a refactor).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| Partition covers each line once and no moved item calls an item left in `main.rs` | `target/scratch/3fs/partition.py` | run by the architect: no overlaps; only `table` needed `session_confinement_mode`, now moved |
| `main.rs` calls exactly: terminal 5 names, table 2 (+ `session_confinement_mode`), lookup 3, attach 2 | same script, identifier search over the remainder | run |
| Cross-group calls (attach to terminal/lookup/input/event_loop/picker to table/lookup/terminal) as in section 3.3 | same script | run |
| The judge passes on a correct rebuild and fails on an item in the wrong file | simulated rebuild and negative probe, scratch (deleted) | run |
| `run_attachment` is called at lines 994, 2083, 2615 only | `rg -n run_attachment src/main.rs` | run |
| No test or doc names the three moved tracing events | `rg` over `tests/ src/ docs/` | run, no match |
| Worked example 1 | `format_row`/`column_widths` code, 3750-3887 | read; hand computed |
| TEAM column index 2, team cell `-` / `name` / `name (private)` | `insert_team_column`, `session_team_cells`, 3772-3809 | read |
| `src/main.rs` unchanged since 392f063 | `git log -1 -- src/main.rs` | run |
| No `[lints]` entry requires module docs or `unreachable_pub` | `Cargo.toml` `[lints.clippy]` only denies unwrap, expect, todo, unimplemented, undocumented unsafe | read |
| semgrep rules judge moved code as before (only two rules are path-scoped, by exclude of other files) | `.semgrep/rules/`, `scripts/prepush-fast.sh` scans all of `src/` | read; the architect still runs the script before the push |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant a Claude Code session (model not reported to the
architect); developer Antigravity CLI, Gemini 4 Hydrogen (Medium); tester OMP, opencode-go/deepseek-v4.1-flash.
BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | the judge failed a correct build: `mod x;` lines matched its item regex and counted as extra items | `mod` lines skipped; inner attributes (`#![...]`) now fail the judge |
| Draft review | consultant | test 5's expected header text ("ends `SIZE  CWD  COMMAND`") was false for an 80x24 size cell | exact fixture and literal, hand computed and cross-checked by script |
| Draft review | consultant | test 2's fixture allowed the swap-detail/hold-reason mutant to survive | four messages spelled out |
| Draft review | consultant | duplicate-name case in test 7 would pin behaviour S4 may change; fixture source cited wrongly (`tests/attach_cli/main.rs`); visibility wording named the wrong callers; acceptance named `tests/daemon_cli` instead of the tests that cover the moved tables; `crate::picker` shadowing by the child not mentioned | all fixed in the locked spec |
| Tester pass | tester | `Some("native_receipt")` arm of the DETAIL cell was not killed by `tests/table.rs` | architect added test `message_detail_shows_native_receipt_evidence_alone`, mutant killed |
| Architect review | architect | the developer's full run showed one `tests/presence` failure under load (passes alone) | recorded as the known presence timing flake, not a regression |

Design read (consultant, accepted as a pure move): debts for later specs, not fixed here: the attach and
switch handshake is written twice (`attach::run` and `state.rs` switch path); data types sit in the attach root
while impls are in children; `table` exposes positional row primitives to the picker; `handle_input` is about 400
lines; two unrelated types named `Output`.
