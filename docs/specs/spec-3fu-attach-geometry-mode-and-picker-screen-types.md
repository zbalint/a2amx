# Spec 3fu: the attach screen's geometry, mode and picker-screen types (S4a)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the new public
`a2amx::attach::model::{Viewport, Geometry}` (tests/attach_model.rs) and the existing PTY suite in
`tests/attach_cli/`.

Scope (edit): `src/attach.rs`, `src/attach/state.rs`, `src/attach/event_loop.rs`, `src/attach/picker.rs`,
new `src/attach/model.rs`, new `tests/attach_model.rs`.
Does not touch: `src/main.rs`, `src/lib.rs`, every other file under `src/` and `tests/`, `Cargo.toml`, `Cargo.lock`,
`CHANGELOG.md`, `AGENTS.md`, docs (the architect updates the AGENTS.md module row and the changelog when
committing).

## 1. Why

After specs 3fs and 3ft the attach screen lives in the library, but `AttachmentState` is still a bag of loose
fields: `cols`, `rows`, `status_visible`, `in_team` and `top_rows` are five facts about one thing (the screen
geometry) read and written in 88 places; `picker: Option<PickerState>`, `scroll_mode: bool` and `scroll_parser`
encode one three-way mode (live, scroll, picker) as three independent fields, so every decision re-derives the mode
(`picker.is_some() || scroll_mode`); the picker's pure data (sessions, view, parser, footer) is mixed with its
network client (`control: Client`), so the picker cannot be reasoned about, or tested, without a daemon; and the
result of opening a session is copied into the state by hand in three places that differ in small ways. The
redesign target (spec S4b) is a pure model with a `step(bytes, now)` function and an async driver; that needs these
types in their final form first, so the step spec's diff is only the routing change. This spec changes data
structure and nothing observable: the input decision tree keeps its current order and every byte sent to the
terminal or the daemon stays the same.

Rejected: doing the model and the step together (one ~1,500-line behaviour-relevant diff); a `Mode::Live{press}`
that carries the mouse press now (leaving live view would then drop a pending press, a change; S4b decides);
keeping `PickerState` whole and putting a client in `Mode` (a network client in a "pure" type).

Prior lessons, stated because specs cannot cite memory: a state type that holds a network client cannot sit inside
a pure model; split the data from the client first. A pure step model must own every piece of state that any
event source (input, output scan, tick, resize) changes, not only what input changes; this spec moves the state
there and S4b moves the logic.

## 2. Target shape

New file `src/attach/model.rs` (`pub mod model;` declared in `src/attach.rs`; the module is public, only the two
geometry types are `pub`, everything else in it is `pub(super)`):

```rust
/// What the human's terminal currently looks like, and whether the status line is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Viewport { pub cols: u16, pub rows: u16, pub status_visible: bool }
impl Viewport {
    /// Rows the session's PTY gets: the terminal rows minus the status rows and the granted top rows, at least 1.
    pub fn pty_rows(&self, top: u16) -> u16;
    /// Top rows the team line wants for this viewport: 2 at six rows or more, 1 at exactly five, else 0; 0 when
    /// the status line is hidden or the session is not in a team.
    pub fn desired_top(&self, in_team: bool) -> u16;
}

/// The viewport plus what the daemon negotiated for the current session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Geometry { pub viewport: Viewport, pub in_team: bool, pub granted_top: u16 }
impl Geometry {
    pub fn pty_rows(&self) -> u16 { self.viewport.pty_rows(self.granted_top) }
    pub fn desired_top(&self) -> u16 { self.viewport.desired_top(self.in_team) }
}

/// Exactly one input mode at a time.
pub(super) enum Mode { Live, Scroll(ScrollParser), Picker(PickerScreen) }
impl Mode { pub(super) fn is_live(&self) -> bool { matches!(self, Mode::Live) } }
```

Every "is it live" test in the code uses `mode.is_live()` (not `matches!`), so S4b can give `Live` a payload
without touching those sites. Entering scroll mode builds `Mode::Scroll(ScrollParser::default())` only when the
mode is not already `Scroll` (the wheel path in `handle_mouse_button` guards with `if !scroll`, as `if
!self.scroll_mode` does today, so a half-parsed sequence is not lost); leaving it (`ScrollKey::Quit`) sets
`Mode::Live` after the keys of that byte were handled.

`Viewport::pty_rows` and `Viewport::desired_top` contain the bodies of today's free functions `pty_rows` and
`team_rows` (`src/attach.rs:149-173`) unchanged; the free functions are deleted and every caller uses the methods
(`viewport.pty_rows(top)` where the code passed a top, `geometry.pty_rows()` for the current granted top).
`open_attachment`'s private `Geometry {cols, rows, status_visible}` in `src/attach.rs` becomes `Viewport`.

`AttachmentState` changes (the struct stays in `src/attach.rs`):

| Today | After |
|---|---|
| `cols`, `rows`, `status_visible`, `in_team`, `top_rows` | `geometry: Geometry` |
| `picker: Option<PickerState>`, `scroll_mode: bool`, `scroll_parser: ScrollParser` | `mode: Mode` |
| the picker's `control: Client` | `picker_control: Option<Client>` on the state (the driver half), `Some` exactly while `mode` is `Picker` |
| `mouse_press`, `session_mouse`, `status`, `team_regions`, `viewer`, `viewer_after`, `prefix_machine`, `prefix_byte`, `after_prefix`, `attachment`, `home`, `session`, `output` | unchanged (S4b moves them) |

`PickerState` is split in `src/attach/picker.rs` into `PickerScreen` (a `pub(super) struct`; its fields `cols`, `rows`, `offset`, `footer`, `parser`, `view`, `sessions`, `current` are `pub(super)` because the state and the event loop read and write them) holding
`sessions`, `current`, `view`, `cols`, `rows`, `offset`, `parser`, `footer`, and the driver's `Client`:

- `PickerScreen::new(sessions: Vec<SessionSummary>, current: &str, cols: u16, rows: u16) -> Self` (pure; builds the
  view from `picker_entries`, parser default, offset recomputed).
- `PickerScreen::render(&self) -> Vec<u8>`: the bytes `PickerState::render` sends today; the driver sends them with
  `output.send(...)`. No `OutputHandle` parameter.
- `PickerScreen::replace_sessions(&mut self, sessions: Vec<SessionSummary>)`: the second half of today's `refresh`
  (view replace, offset recompute); the driver's `refresh` becomes `let sessions = request_sessions(&mut control).await?;
  screen.replace_sessions(sessions);`.
- `feed`, `apply`, `recompute_offset`, `picker_entries`, `session_rows`, the headers and helpers keep their bodies.
- `PickerState::open(home, current, cols, rows, output)` becomes a driver function on `AttachmentState`
  (`open_picker`, already there): connect `Client`, `request_sessions`, `PickerScreen::new`, send `render()`, set
  `self.team_regions.clear()` (as today, before the mode is set), `mode = Mode::Picker(screen)` and
  `picker_control = Some(client)`; the failure behaviour (error propagates) is unchanged. One private method
  `close_picker(&mut self)` sets `mode = Mode::Live` and `picker_control = None`; it is used at all three exits
  (successful switch, `Cancel`, `Attach(current session)`), so the pair cannot drift. A refresh while `mode` is
  `Picker` but `picker_control` is `None` returns an error (`anyhow!`, no `expect`).

`apply_opened` (`attach_session` now returns the `Opened` it builds, `Ok(opened)`, instead of the tuple `(Attachment, u16, bool)`; its callers pass the value on): `src/attach/state.rs` has two hand-copied result blocks that differ (section 3.2). One private
method `fn adopt(&mut self, opened: Opened)` sets what both share (attachment `Some`, `geometry.in_team`,
`geometry.granted_top`, `session_mouse = false`, `status = None`); the successful switch then sets `session`,
clears `team_regions`, sends the window title, and sets `mode = Mode::Live` (dropping `picker_control`); the
restored-after-failed-switch path sets nothing more. Differences are not unified.

`restore_current` is removed (section 3.3); its two callers send `ClientFrame::Redraw` and call `close_picker`.
`switch_session` cannot run in scroll mode (its callers, prefix commands and the team click, run only in live
view), so the successful switch's `mode = Mode::Live` is equivalent to today's `picker = None`.

`src/attach/input.rs`: `ScrollParser` is already `pub(super)`; the file is not edited.

## 3. Behaviour that must not change

### 3.1 Input decision tree

`handle_input` keeps its order: find a report (or not) from `pass_through`, route a found report, else
`Mode::Picker` byte-by-byte (`handle_picker_input`), `Mode::Scroll` byte-by-byte (`feed_scroll_input`), `Mode::Live`
through `prefix_batch_len` and the prefix machine. `pass_through` becomes
`self.mode.is_live() && session_mouse && self.geometry.granted_top == 0`; the `live` local becomes
`self.mode.is_live()`.

### 3.2 The two result blocks (read at 563623d)

- successful switch (`state.rs:595-607`): attachment, session, in_team, top rows, `session_mouse = false`, `status
  = None`, `team_regions.clear()`, title OSC, `picker = None`. Also, before the attach, `mouse_press = None`.
- restored after a failed switch (`state.rs:610-621`): attachment, in_team, top rows, `session_mouse = false`,
  `status = None`; the picker, when open, gets the error footer, a refresh (with `?`) and a render. `team_regions`
  is not cleared and the title is not sent.

### 3.3 Dead branch

`restore_current`'s `if self.attachment.is_none()` branch (`state.rs:549-567`) cannot run: `self.attachment` is set
to `None` only at `state.rs:591` inside `switch_session`, and every path out of that function either sets it again
(`Some` on success, `Some` on restore) or returns `Err`, which ends the loop. After removal the picker's
`Cancel` and `Attach(current session)` outcomes do exactly what `restore_current` did in its live path: send
`ClientFrame::Redraw` to the attachment and close the picker (`mode = Live`, `picker_control = None`), then
`handle_picker_input` then `return Ok(None)` for that byte (so its remaining picker actions are dropped, as today),
and `handle_input` goes on with the next byte in live mode. The unreachable `continue` after a failed same-session
restore (`state.rs:673`) goes with the branch.

## 4. New tests

`tests/attach_model.rs` (public seam, literal worked examples, no recomputation):

1. `viewport_pty_rows_subtracts_status_and_top_rows`: `(cols 80, rows 24, visible true).pty_rows(0) == 22`,
   `.pty_rows(2) == 20`; `visible false` with `rows 24`, `pty_rows(2) == 22`; `rows 3` visible `pty_rows(0) == 2`;
   `rows 2` visible `pty_rows(0) == 2`; `rows 1` visible `pty_rows(0) == 1`; `rows 4` visible `pty_rows(2) == 1`
   (4 - 2 - 2 = 0, raised to 1).
2. `viewport_desired_top_follows_rows_visibility_and_team`: in a team and visible: rows 24 gives 2, rows 6 gives
   2, rows 5 gives 1, rows 4 gives 0; hidden status line gives 0 at rows 24; not in a team gives 0 at rows 24.
3. `geometry_uses_the_granted_top_not_the_desired_one`: `Geometry {viewport: 80x24 visible, in_team: true,
   granted_top: 0}`: `pty_rows() == 22` while `desired_top() == 2`; with `granted_top: 2`, `pty_rows() == 20`.

No new PTY test: the existing suite is the evidence that behaviour is preserved (section 6).

## 5. Out of scope

Everything that is S4b or later: `Model::step`, `Intent`, the prefix machine inside the model, `Mode::Live{press}`,
moving `viewer`, `session_mouse`, `status`, `team_regions` out of `AttachmentState`, `requested_top`, the
status-toggle reattach on refused rows, `granted_top` becoming the only source of the top rows (it duplicates `attachment.top_rows()` for now), the
`force`/`viewer` bool pair, typed picker rows, renaming `Output`, the `switch_session` restore branch's
`refresh().await?` versus `restore_current`'s swallowed refresh (both stay as they are: the restore branch keeps its
`?`, the removed branch's `let _` goes away with the dead code), `scan_session_mouse`, `event_loop.rs` beyond field
renames. No behaviour fix of any kind.

## 6. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `563623d` (architect, 2026-11-20): 63 suites, 1370 passed,
  0 failed, 3 ignored. After: 64 suites (`tests/attach_model.rs`) and 1370 plus the 3 new tests.
- `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`: pass.
- Structural, outside judge (`rg`, architect runs; each prints nothing when the work is done, and each prints
  matches today): `rg -n '\bscroll_mode\b|picker: Option|\bPickerState\b|restore_current' src/attach.rs src/attach/state.rs src/attach/event_loop.rs src/attach/picker.rs`;
  `rg -n '^(pub )?(async )?fn (pty_rows|team_rows)' src/attach.rs`;
  `rg -n '(self|state)\.(cols|rows|top_rows|status_visible|in_team)\b' src/attach.rs src/attach/state.rs src/attach/event_loop.rs`.
  `src/attach/model.rs` and the `PickerScreen` bodies in `picker.rs` legitimately use `self.rows` and are not
  scanned by the last command. `rg -c 'Mode::|is_live' src/attach/state.rs src/attach/event_loop.rs` shows the
  uses (the architect reads each).
- The PTY suite is the behaviour evidence: `tests/attach_cli/` (all 63 tests, including spec 3fr's 11 and spec
  3ft's 3) passes unchanged: `cargo test --test attach_cli`. Known flaky: `tests/presence`
  `viewers::viewer_attach_keeps_input_read_only_while_sizing_session`,
  `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error`,
  `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode`,
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session`: rerun once, record
  as pre-existing if it passes.
- `wc -l`: no file over 1,000 lines. `git status --short` lists only the files of section 0;
  `git diff --stat -- Cargo.toml Cargo.lock src/main.rs src/lib.rs` is empty.
- Mutation check (developer; mutate in the checkout, restore by checksum; each mutant needs a named killing
  test): `Viewport::pty_rows` status rows 2 to 1 (killed by test 1 and PTY resize tests); `desired_top` rows >= 6
  to rows >= 7 (test 2); `pass_through` without the `granted_top == 0` term (PTY `team_row_press...` tests);
  `adopt` not resetting `session_mouse` (a PTY test that switches away from a session with mouse on: name it, or
  say none exists); the successful switch not clearing `team_regions` (name the test or say none); picker
  `render` bytes dropping the footer (PTY picker tests). A survivor is reported, not hidden.
- The tester (assigned afterwards) runs an independent mutation pass, with a diff read for any behaviour the
  rename could change; a consultant reads the design (refactor spec).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| `AttachmentState` has these fields and no others used by the rows of section 2's table | `src/attach.rs:197-222` | read |
| 88 uses of the moved fields in `state.rs`; the loop uses them at event_loop.rs 28-102 | `rg -c` (above), event_loop read | run |
| picker and scroll modes are exclusive: `open_picker` and prefix commands run only in the live branch, `handle_mouse_button` returns early with a picker, `team_click` only from the live-or-top-rows path | `state.rs:136-163, 407-411, 292-304` | read (the live branch is the only caller of `handle_prefix_actions`; `handle_mouse_report` runs only with no picker and no scroll mode) |
| `self.attachment = None` only at `state.rs:591`; the two exits either set `Some` or return `Err` | `rg -n 'attachment = None' src/attach` | run (3fs/3ft read): sets at 591 only |
| `restore_current` has two callers, both in `handle_picker_input` | `rg -n restore_current` | run |
| The free `pty_rows` and `team_rows` have these bodies | `src/attach.rs:149-173` | read |
| `Viewport` worked examples in section 4 | `pty_rows` body: status rows 2 if visible and rows >= 4, 1 if visible and rows == 3, else 0; result `rows - status - top`, at least 1 | hand computed from the body |
| `pub mod model` with `pub(super)` members compiles without `private_interfaces` warnings | n/a | NOT verified: the developer checks clippy; a `pub` fn must not name a `pub(super)` type |
| A pending mouse press survives leaving live mode today (so `Mode::Live{press}` would change behaviour) | `state.rs:44,556,592`: `mouse_press` is cleared only on a switch, a toggle that reattaches, and `select_member` | read |

## Review record

(Appended by the architect at acceptance.)
