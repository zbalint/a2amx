# Spec 3au: A rule under the team line

## 0. Status

**LOCKED** (2026-10-27; pre-lock gate run against `develop` at `4254d83`; consultant review `m_4084`, findings adopted).
Architect-directed, owner request (backlog F29 follow-up), context `a2amx-team-rule-2026-10-27`. Implementation goes
to `a2amx-developer` only.

**Baseline (run 2026-10-27 on `4254d83`):** `env -u A2AMX_BIN -u NO_COLOR cargo test` passed (41 result lines, 627
passed, 0 failed, 3 ignored); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are the other two
acceptance commands. Known flaky test: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
(backlog I14); a failure is rerun once and recorded as pre-existing if it passes.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `status::render_team_rule` (pure, `tests/status.rs`); the attach CLI in a PTY against a real
daemon and against the fake daemon (`tests/attach_cli.rs`); `mouse::shift_row` (`tests/mouse.rs`, contract tests). No
test reaches a private function.

**Scope.** May edit exactly these paths and no others.

- `src/status.rs` (3.1), `src/main.rs` (3.2)
- `tests/status.rs`, `tests/attach_cli.rs`, `tests/mouse.rs`
- `docs/architecture.md` ("Status line" subsection, the team-line paragraph only) and `docs/cli.md` (the team-line
  sentences at lines 64-65: the "at least five rows" phrase and the sentence that names the team line and separator)

Does not touch: `src/daemon.rs`, `src/emulator.rs`, `src/mouse.rs`, `src/wire.rs`, `src/client.rs`, `src/picker.rs`,
`AGENTS.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `docs/backlog.md`, `docs/specs/`. No new dependency.
Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

Owner request, 2026-10-27: the status bar has a dim rule above it (spec 3f); the team line at the top (spec 3as) should
have the same kind of rule under it. Spec 3as decided "no separator of its own"; the owner reverses that. The daemon
already accepts any `top_rows` up to 8 and its emulator render shifts by that many rows (spec 3ar tests origin 1 and
8), so the change is client-side: reserve a second top row and draw the rule in it.

## 2. Design (owner decisions, 2026-10-27)

- Under the team line, terminal row 2, a dim `─` rule of `cols - 1` characters in the colour of the bottom rule
  (`SEPARATOR_OPEN`, grey 240).
- The rule is part of the team line: the number of top rows the client requests is 2 when the status line is visible,
  the session is in a team and the terminal has at least 6 rows; it stays 1 at exactly 5 rows (the line without a
  rule, as today) and 0 below 5 rows or without a team or with the status line hidden. At 6 rows the session keeps 2
  rows (6 - 2 top - 2 bottom), the floor it has at 5 rows today.
- Prefix then `s` hides the team line, the top rule, the bar and the bottom rule together, as today.
- The rule is not clickable. A press on it is swallowed with its release and neither reaches the session nor acts.

## 3. Changes

### 3.1 `src/status.rs`

- Extract the row-drawing bytes of the bottom rule (`status.rs:340-348`: move to `rows - 1`, reset, clear line, open
  the grey, `width` times `─`, reset) into one private function that takes the terminal row and `width`; `render`
  calls it for `rows - 1` (its `rows >= 4` guard stays in `render`) and the new function calls it for row 2. The bytes `render` emits do not change.
- New `pub fn render_team_rule(cols: u16, drawn: bool) -> Vec<u8>`: empty when `cols < 2`; otherwise wrapped in
  `ESC7 .. ESC8` (save and restore the cursor, as `render_team_line` does). With `drawn` it draws the rule on row 2
  with `width = cols - 1`; without it, it clears row 2 (`ESC[2;1H ESC[0m ESC[2K`).
- `render_team_line` and `TeamLine` do not change.

### 3.2 `src/main.rs`

- `team_rows` (`main.rs:1582`): returns 2 for `visible && in_team && rows >= 6`, 1 for `visible && in_team && rows ==
  5`, else 0. Its doc is the Design bullet above.
- `draw_status` (`main.rs:1669`): the branch `self.top_rows == 1` becomes `self.top_rows > 0`; when `top_rows == 2` it
  also appends `status::render_team_rule(self.cols, true)` after the team line bytes. The `else` branch is unchanged.
- `toggle_status_line` (`main.rs:1902`): the same `top_rows == 1` test becomes `top_rows > 0`; when `top_rows == 2`
  also send `status::render_team_rule(self.cols, false)` with the cleared team line.
- `handle_mouse_report` (`main.rs:1964`), claimed branch: `mouse::shift_row` with origin 2 claims rows 1 and 2.
  - A `button == 0` press on row 1 sets `MousePress::Team` as today. A `button == 0` press on row 2 sets a new
    `MousePress::Rule` (variant of `MousePress`, `main.rs:1591`), replacing any stored press. A claimed press with any
    other button (wheel 64/65, right click, motion) stores nothing and is swallowed, as today.
  - A release acts through `team_click` only when it is on row 1 and the stored press is `MousePress::Team`. A `Team`
    press released on row 2 does nothing, and a `Rule` press released anywhere does nothing.
  - Fix of an existing off-by-origin bug (visible only for a drag out of the grid): a `Grid` press released in a
    claimed row is forwarded to the session with `mouse::with_row(&claimed, 1)`, the grid's first row, not
    `top_rows + 1` (`main.rs:1973`): `with_row` writes a session row, and `shift_row` makes the first grid row session
    row 1. With two top rows the old value would be row 3.
- No other `top_rows` use changes: `pty_rows`, `member_by_number`, `next_member`, `select_member`, the attach
  negotiation (`AttachedTop` is already read from the daemon's reply) and the `top_rows == 0` fallback for an older
  daemon are generic. Keep the existing `// shortcut:` comments; a resize keeps the negotiated origin until the next
  attach, so a terminal shrunk below 6 rows after attach keeps both top rows and the rule may overdraw the bar rows until the next attach.

### 3.3 Tests

- `tests/status.rs`: `render_team_rule` drawn (exact bytes for `cols = 10`: `ESC7`, `ESC[2;1H`, `ESC[0m`, `ESC[2K`,
  `ESC[38;5;240m`, nine `─`, `ESC[0m`, `ESC8`), cleared (exact bytes), and `cols < 2` (empty). The existing bottom-rule
  tests keep passing unchanged (contract tests for the extraction).
- `tests/attach_cli.rs`:
  - Make the fake daemon (`fake_attach`, `attach_cli.rs:187`) reply `AttachedTop` with the `top_rows` the attach
    request carried instead of the literal 1 (`attach_cli.rs:196`); pass it from `Request::Attach { top_rows, .. }` at
    line 172.
  - Real-daemon PTY test on a 24-row terminal with a team: row 2 is `"─".repeat(79)`; the session's first output is on
    row 3; a press and release on row 2 at the team label's column opens no picker and sends nothing to the session;
    a press on row 1 released on row 2 does nothing; prefix `s` clears both rows and a second `s` restores them.
  - Terminal sizes: `stty rows 5` shows the team line on row 1 and row 2 is not a `─` run; `stty rows 6` shows both.
  - The existing team-line test (`attach_cli.rs:1664`): with two top rows, grid row N is terminal row N + 2. Its
    literals at `attach_cli.rs:1732-1737` change so each step keeps its meaning: `<0;18;1M` + `<0;10;2m` (Team press
    released "on the grid") becomes release `<0;10;3m`, and a new step covers a Team press released on row 2 (does
    nothing); `<0;10;2M` + `<0;10;1m` (Grid press, release in a claimed row) becomes `<0;10;3M` + `<0;10;1m` and now
    expects the session to receive the release at row 1, `^[[<0;10;1m`, per the fix above. The row-1 clicks (columns
    18 and 30, `attach_cli.rs:1715-1751`), the `stty rows 4` case and the non-team session are unchanged.
  - `status_toggle_clears_team_row_without_session_output` (`attach_cli.rs:1801`) also asserts row 2 is not a `─` run
    after the toggle.
- `tests/mouse.rs`: contract tests (they pass on the old code) for `shift_row` with origin 2: rows 1 and 2 are claimed,
  row 3 becomes grid row 1, in both SGR and X10.

## 4. Out of scope

`src/daemon.rs` and the wire (no change; `top_rows` 2 is already accepted). Resizing below 6 rows after attach
(existing shortcut). The `+N` overflow, member ordering and any other team-line behaviour. `CHANGELOG.md` and
`docs/backlog.md` (the architect updates both). Spec 3as's text (history).

## 5. Acceptance

All run from the main checkout; each is judged by its own output.

1. `env -u A2AMX_BIN -u NO_COLOR cargo test` passes with no failed tests (more than 627 passed; 3 ignored).
2. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are clean.
3. `git status --short` lists only paths named in section 0 Scope.
4. New tests that fail on the old code: the `render_team_rule` tests (the function is missing: a compile failure counts
   as red), the 24-row rule and row-3 assertions, the row-2 swallow assertions, the `stty rows 6` case. Contract tests
   that already pass on the old code: the `shift_row` origin-2 tests, the `stty rows 5` case and the bottom-rule tests.
5. Mutation check (a scratch copy of the checkout; the assigned tree must equal the developer's diff afterward): the
   developer breaks each behaviour below in `src/`, names the test that fails on it, and restores it: `team_rows`
   returning 1 at 6 rows; the rule not drawn in `draw_status`; the rule not cleared by `toggle_status_line`; a
   `button == 0` press on row 2 not setting `Rule` (a Rule press then released on row 1 would click); a `Team` press
   released on row 2 still calling `team_click`; the forwarded release row left at `top_rows + 1`.
6. The tester (`a2amx-tester`) attaches with this branch's `target/` binary to an isolated daemon in a temp home (never the
   working daemon) at 24, 6 and 5 rows and reports what each shows.
