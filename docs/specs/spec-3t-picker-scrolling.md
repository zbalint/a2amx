# Spec 3t: the session picker scrolls and takes page keys

## 0. Status

**LOCKED** (2026-10-09, pre-lock gate run against develop at `5603369`; consultant advice `m_905` and
pre-lock review `m_908`, context `a2amx-picker-teams`; gate notes in section 9). Owner request: the picker (prefix + `w`) is too dense with several teams and many
agents; the rows themselves stay as they are. The owner's wish is split into two specs: this one (3t)
fixes the part that is a bug and adds the testable seam; spec 3u adds team sections, folding and a
filter on top of it. Architect decisions are in section 2.

**Baseline:** develop at `5603369` (spec 3s implemented and committed).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `a2amx-picker-teams`. Public test seams: `a2amx::picker` (pure, `tests/picker.rs`) and the
`a2amx` binary under a PTY (`tests/attach_cli.rs`).

**Scope.** May edit exactly these files and no others.

- `src/picker.rs` (new), `src/lib.rs`, `src/main.rs`
- `tests/picker.rs` (new), `tests/attach_cli.rs`
- `AGENTS.md` (one row in the module table), `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/wire.rs`, `src/daemon.rs`, row formatting (`session_rows`, `picker_value_rows`,
`column_widths`, `format_row`, `insert_team_column`, which `list` shares) beyond what section 4 says, every
other test file, every spec document. No new dependency. Do not commit, stage or merge: leave the diff
uncommitted.

## 1. Why

`PickerState::render` (`src/main.rs`, the loop `if row_number >= usize::from(self.rows) { break; }`) stops
drawing rows that do not fit, but `move_selection` still moves past them, so with more sessions than screen
rows the highlighted row is off-screen and there is no way to tell. The picker also has no page, home or
end keys. `PickerState` and `PickerParser` live in the binary, so tests cannot reach them; this spec moves the
key parser into a library module and adds the window arithmetic there, as pure functions.

## 2. Decisions

- **D1. Pure module.** New `pub mod picker` in `src/picker.rs`, placed after `omp` and before `prefix` in
  `src/lib.rs`. It owns `PickerAction`, `PickerParser` (moved unchanged except for D3) and the function in D2.
  Row formatting stays in `src/main.rs`.
- **D2. Window.** `pub fn scroll_into_view(offset: usize, selection: usize, height: usize, total: usize) -> usize`:
  returns the new first visible index. Rules, in order: `height == 0` or `total <= height` gives `0`; otherwise
  `selection < offset` gives `selection`; `selection >= offset + height` gives `selection + 1 - height`;
  otherwise `offset`; the result is finally clamped to `total - height`. The offset is stored in `PickerState`
  and recomputed after every selection change, every refresh and every resize.
- **D3. New keys.** `PickerAction` gains `Page(isize)` (`-1`/`1`), `Home` and `End`. `PickerParser` maps
  `ESC [ 5 ~` to `Page(-1)`, `ESC [ 6 ~` to `Page(1)`, `ESC [ H`, `ESC O H` and `ESC [ 1 ~` to `Home`, `ESC [ F`,
  `ESC O F` and `ESC [ 4 ~` to `End`. The existing keys and the pending-buffer limit (clear when more than
  3 bytes are pending and no sequence matched; the four-byte `ESC [ 5 ~` and `ESC [ 6 ~` and `ESC [ 1 ~`/`4 ~`
  are matched before that check) do not change.
- **D4. Page size.** A page move is `max(height - 1, 1)` rows, clamped at the ends (no wrap). `Home` selects
  index 0, `End` the last index. All three do nothing on an empty list. The arithmetic is the pure function
  `pub fn target(selection: usize, action: PickerAction, height: usize, total: usize) -> usize` in
  `src/picker.rs`: `Move(d)` adds `d` clamped to `0..=total-1`, `Page(d)` adds `d * max(height-1, 1)` clamped
  the same way, `Home` gives `0`, `End` gives `total - 1`, `Cancel` and `Select` return `selection`, and
  `total == 0` gives `0`. `PickerState::move_selection` and the new key handling call it.
- **D5. Height.** The list height is `rows - 3` rows (title, column header and footer take three), `0` when
  `rows <= 3`; this is exactly the number of rows the current loop draws.
- **D6. Position indicator.** When `total > height`, the footer shows `{selection + 1}/{total}` (counting
  sessions, one-based) unless a footer error message is set; the error takes priority. When everything fits the
  footer stays empty as today.
- **D6b. Footer error.** A footer error (a failed switch) stays until the picker closes, as today, so the
  indicator does not return in that picker session. Accepted; not part of this spec.
- **D7. No team change.** Layout, columns, the `(current)` suffix, the `>` marker and the TEAM column behave
  exactly as today. Sections come in spec 3u.

## 3. `src/picker.rs` (new)

Move `PickerAction` and `PickerParser` (and its `impl`) from `src/main.rs` into this module with `pub`
visibility; add the three variants and parser arms of D3; derive `Debug, Clone, Copy, PartialEq, Eq` on `PickerAction` (tests compare it) and make `PickerParser`
(keep its `Default`) and `feed` `pub`; add `scroll_into_view` (D2) and `target` (D4) with a doc comment that
says why the offset is stored and recomputed (the window must not jump on every move). No I/O, no async.
Keep the existing `// shortcut:` comments with the moved code.

## 4. `src/main.rs`

- Remove the moved items; add `use a2amx::picker::{PickerAction, PickerParser, scroll_into_view, target};`
  (match the file's existing `use a2amx::...` style).
- `PickerState`: add `offset: usize`. `open` sets it with `scroll_into_view(0, selection, height, total)`.
  After `move_selection`, `refresh` (the clamp already there) and a resize (the existing
  `picker.cols = new_cols; picker.rows = new_rows;` site) recompute it.
- Handle `Page(dir)`, `Home`, `End` in `handle_picker_input` beside `Move`: set the selection with `target` (D4),
  recompute the offset, render. `move_selection` also uses `target`.
- `render`: draw only `rows[offset..min(offset + height, total)]`, at screen rows `3..`; the footer per D6.
  Row numbers, the `\x1b[2J` clear and the title line are unchanged.
- Do not change `session_rows` or its callers' signatures.

## 5. Tests (failing first, expected values are literals)

`tests/picker.rs` (new), pure:

1. `scroll_into_view` table: `(0,0,5,12)=0`; `(0,5,5,12)=1`; `(1,0,5,12)=0`; `(3,5,5,12)=3`; `(0,11,5,12)=7`;
   `(9,3,5,12)=3`; `(0,0,0,12)=0`; `(4,2,5,5)=0` (fits); `(10,11,5,12)=7` (clamped to `total - height`).
2. `PickerParser::feed`, one byte at a time as the binary does, with `escape_alone=false`: the eight new
   sequences give `Home`, `End`, `Page(-1)`, `Page(1)` as listed in D3; `ESC [ A` still gives `Move(-1)`,
   `j` `Move(1)`, `\r` `Select`; `ESC [ 5 ~` split across two `feed` calls (`ESC [ 5`, then `~`) still gives
   `Page(-1)`; an unknown `ESC [ 9 ~` gives no action and leaves the parser usable (a following `j` works).
3. `feed(&[0x1b], true)` gives `Cancel`.
4. `target` table: `(0,Move(1),5,12)=1`; `(0,Move(-1),5,12)=0`; `(11,Move(1),5,12)=11`; `(0,Page(1),5,12)=4`;
   `(10,Page(1),5,12)=11`; `(6,Page(-1),5,12)=2`; `(2,Page(-1),5,12)=0`; `(3,Page(1),0,12)=4` (height 0 pages by 1);
   `(3,Home,5,12)=0`; `(3,End,5,12)=11`; `(0,End,5,0)=0`; `(7,Select,5,12)=7`.

`tests/attach_cli.rs`, PTY (`PtyHarness`, `attached.resize(cols, rows)`, `attached.send`):

5. Start 8 detached sessions named `pk-a` .. `pk-h` (`new --detach --name pk-a -- sh -c "sleep 30"` and so
   on), attach to `pk-a`, `resize(100, 8)` (height 5), open the picker (`[2, b'w']`). Wait for the text `1/8` first (the resize
   reaches the picker through SIGWINCH, so asserting before that races), then the screen contains `pk-a` and
   does not contain `pk-h`. Send `End` (`ESC [ F`): wait for `8/8`, then the screen contains
   `pk-h` and not `pk-a`. Send `Home` (`ESC [ H`): wait for `1/8`, `pk-a` is shown again. Send `ESC [ 6 ~`: wait for `5/8`.
6. Existing picker tests (`attach_cli.rs` near lines 273, 304-404, 411-470) and the team column test (about line
   1275) must pass unchanged.

If any existing test fails because of this change, stop and report `BLOCKED — SPEC ADJUDICATION REQUIRED` with the
test name; do not edit it.

## 6. Documentation

- `AGENTS.md`: module table row `picker | The session picker's key parser and scroll window (pure)`, after `omp`.
- `README.md`: in the attach keys paragraph (about line 152) add that the picker takes PageUp/PageDown and
  Home/End and scrolls when sessions do not fit.
- `docs/architecture.md`: where the picker is described (about lines 297 and 340), one sentence on the scrolling
  window and the position indicator.
- `docs/backlog.md`: add a row after the last one: `| C24 | Session picker scrolls and takes page/home/end keys | spec 3t |`.

## 7. Out of scope

Team sections, folding and the filter (spec 3u); live refresh; mouse; the cell-width fix; changes to the
columns or to `list`; any change to how the picker is opened or to `attach`.

## 8. Acceptance

Post-implementation commands (new code required), with `A2AMX_BIN` and `NO_COLOR` unset:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short   # only files listed in section 0 scope (the committed spec is not listed)
```

All pass with no warnings. Known flake: `attach_cli::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
failed once in a loaded full run and passes alone; rerun alone and report both results if it fails.

## 9. Gate notes (pre-lock)

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` at `5603369` (3s accepted run): 446 passed, 0 failed,
  3 ignored.
- Consultant review `m_908` applied: derives on `PickerAction` and `pub` parser items (§3), eight-sequence
  count (test 2), `use` wording (§4), PTY waits before asserts (test 5), pure `target` function with table
  test (D4, test 4), footer-error note (D6b). Anchors, the `scroll_into_view` table and the PTY test arithmetic
  were verified by hand by the consultant.
- Scope reconciliation: every file named in §§3-6 is in §0 scope. `PickerAction`/`PickerParser` have no users
  outside `src/main.rs` (consultant grep).
