# Spec 3f: a separator row above the attach status bar

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `02bc863`, the commit that accepted spec 3e; gate notes in section 5).
The owner said the status bar sits too close to the agent harness's own UI and asked for more
space above it, then said it need not be blank: a line would do, just to set the bar apart from
the rest of the screen. Decisions D1 to D3 are the architect's, marked **(architect)**.

**Baseline:** develop at `02bc863`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-status-separator`. Public test seams: `a2amx::status::render` (`tests/status.rs`) and the
attach PTY tests (`tests/attach_cli.rs`).

**Scope.** May edit exactly these files and no others.

- `src/main.rs` (`pty_rows`), `src/status.rs`
- `tests/status.rs`, `tests/attach_cli.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/daemon.rs`, `src/session.rs`, `src/wire.rs`, the picker, scroll mode, any
other test. No new dependency. Do not commit, stage or merge.

## 1. Why

The bar takes the terminal's last row and the session's PTY gets the rows above it, so a harness
that draws its own footer on its last row (Claude, OMP and Codex all do) ends directly against the
bar. A thin rule between them reads as two separate things.

## 2. Decisions

- **D1. (architect)** When the bar is visible and the terminal has at least 4 rows, the client
  reserves two rows: the last row for the bar and the row above it as a thin separator rule. The PTY gets
  `rows - 2`.
- **D2. (architect)** With exactly 3 rows only the bar is reserved (PTY `rows - 1`, as today); below
  3 rows or with the bar hidden (`s`) nothing is reserved. So `pty_rows(rows, visible)` is
  `rows - 2` when `visible && rows >= 4`, `rows - 1` when `visible && rows == 3`, else `rows`.
- **D3. (architect)** Every time the client draws the bar it also draws the separator: the row is
  erased first, so stale cells (left by the picker, scroll mode, or the bar being toggled back on)
  never show, then filled with `cols - 1` copies of `─` (U+2500) in dim grey (256-color 240) on the
  default background. Like the bar, it leaves the last column untouched.

## 3. Files

- `src/main.rs`: `pty_rows` per D2. Nothing else in `main.rs` changes: every resize and attach
  path already goes through `pty_rows`.
- `src/status.rs`: when `rows >= 4`, the output begins `ESC 7`, then `ESC [ {rows-1} ;1 H`,
  `ESC [0m`, `ESC [2K` (erase the row), `ESC [38;5;240m`, `cols - 1` copies of `─`, `ESC [0m`, and
  then continues exactly as before with `ESC [ {rows} ;1 H` and the bar bytes, ending `POST`. For `rows == 3` the bytes are as before
  (no spacer). `rows < 3 || cols < 2` still returns an empty vector.
- `README.md`: the status line sentence says it takes the last row and keeps a thin rule above it
  (hidden together by `s`). `docs/architecture.md` "Status line": the PTY is sized two rows shorter
  when the terminal has at least 4 rows (one row at exactly 3), and the bar's draw also
  draws the separator rule. `docs/backlog.md`: a Closed row `C10`, `Separator row above the attach status bar`, `spec 3f`.

## 4. Tests (failing first, expected values are literals)

- `tests/status.rs`: the byte-exact tests gain the separator prefix. For `rows = 24, cols = 40`,
  `PRE` becomes `ESC7 ESC[23;1H ESC[0m ESC[2K ESC[38;5;240m` + 39 `─` + `ESC[0m ESC[24;1H ESC[0m
  ESC[38;5;252;48;5;236m`. Add: `rows = 4` starts `ESC7 ESC[3;1H ESC[0m ESC[2K`, and its bar
  begins at `ESC[4;1H`; `rows = 3` has no `ESC[2K`, no `─`, and starts `ESC7 ESC[3;1H ESC[0m
  ESC[38;5;252;48;5;236m`; `rows = 2` is empty. The visible-text tests keep their section 4 spec
  3e expectations by applying `visible()` only to the part of the output from the bar's own
  `ESC[{rows};1H` onward (split the bytes at that sequence); the separator row is asserted
  separately, as `cols - 1` copies of `─`.
- `tests/attach_cli.rs`, PTY sizes shift by one in every case where the bar is visible (the
  harness terminal is 80x24 and the bar starts visible). The literals to change, with their line
  numbers at `02bc863`:
  - lines 440, 537 and 985: `wait_for_text("23 80", ...)` becomes `"22 80"`.
  - `picker_clips_long_cwd_to_narrow_terminal`, line 442: `resize(101, 10)` (line 441) stays; the wait
    `"9 101"` becomes `"8 101"`. The line-width bound `<= 101` and the literal
    `…-picker-path (current)` stay: the session's SIZE cell `101x8` is still 5 characters, so the
    CWD room is unchanged.
  - line 539: `"9 40"` becomes `"8 40"`.
  - `status_line_toggles_and_shows_the_session_address`: line 996 `"9 40"` becomes `"8 40"`; line 999
    `"10 40"` stays (bar hidden); line 1015 `"12 50"` stays (bar still hidden); line 1017 `"11 50"`
    becomes `"10 50"`.
  - line 793: the expected list row's SIZE `80x23` becomes `80x22`. Lines 509 and 670 (`80x24`) are
    detached sessions created at a given size and do not change.
  Any other test that fails after the change only because a visible row set or a size shifted by one
  is in scope to update the same way, with literal expected values; report each one.
- Add one PTY test: attach to a session in a 80x24 terminal, wait until the bar text is on the
  last screen row, and assert that the row above it holds only `─` characters (`cols - 1` of them) while the
  session's own output is on the rows above that. A `sh` fixture that prints a marker on every row of its
  `stty size` height is enough.

## 5. Out of scope

Colors, content or layout of the bar (spec 3e); a configurable separator height or glyph; the picker and scroll
mode screens; any daemon or wire change.

Post-implementation commands: `env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo
test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `git status --short`
(only the scoped files).

Gate notes, at `02bc863`: `pty_rows` is the only place the reserved row count is computed (`rg "pty_rows"
src/main.rs`: lines 826, 852, 1035, 1127, 1197 and 1292: the attach request, the `s` toggle, the
picker switches and the resize). The size literals above come from `rg` over `tests/` at that commit.
The status renderer's tests are the ones written for spec 3e in `tests/status.rs` (the `PRE` constant
and the `visible()` helper); this spec changes `PRE` and makes the visible-text tests skip the new
separator row. Baseline `cargo test` at `02bc863`: 26 suites, 356 passed, 0 failed, 3 ignored.
