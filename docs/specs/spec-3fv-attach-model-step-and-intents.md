# Spec 3fv: the attach screen's pure model, `Model::step` and `Intent` (S4b)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file; code base `e79ed92`. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam:
the public `a2amx::attach::model` API of section 2 (`tests/attach_model.rs`, literal bytes) and the PTY suite
`tests/attach_cli/`.

Scope (edit): `src/attach.rs`, `src/attach/{model,state,event_loop,input,picker}.rs`, new `src/attach/model/{mouse,draw}.rs` (only if `model.rs` would pass
about 900 lines: `mouse.rs` holds the button and team-row handlers and `team_click`, `draw.rs` holds the status and viewer
rendering and the toggle's clears; `step`, `Intent` and the types stay in `model.rs`), `src/prefix.rs` (one added
method), `tests/attach_model.rs`, `tests/prefix.rs` (tests for the added method), `tests/prop_prefix.rs` only if its
property needs the new method. Does not touch: `src/main.rs`, `src/lib.rs`, every other file under `src/` and
`tests/`, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `AGENTS.md`, docs (the architect updates the module row).

Behaviour: none changes except what section 3.4 lists (frame boundaries of the forwarded stream, and how often the viewer
decorations repeat). The bytes sent to the daemon, their order, and the content and order of every other byte sent
to the terminal stay the same. The visible fixes found while reading (status toggle
reattaching on every show when rows are refused, the swallowed refresh error, bracketed-paste-aware mouse
scanning) are out of scope here and go into a small follow-up spec (section 5).

## 1. Why

Spec 3fu gave the attach screen its types. The decisions are still spread over async methods that change state and
perform I/O in the same breath: `handle_input` is about 60 lines of dispatch, `handle_prefix_actions`,
`handle_mouse_report`, `handle_picker_input`, `feed_scroll_input`, `toggle_status_line`, `switch_session`,
`team_click`, `select_member` each mix a decision ("which session is next", "is this a team-row click", "does the
status toggle need a reattach") with a send on the daemon connection or the terminal. Nothing can be tested without a
pseudo-terminal and a fake daemon; the six behaviours 3fu's tester pass found unpinned (below) cannot be pinned by the
PTY suite at all. The target: a synchronous, pure `Model` that owns every piece of state any event source changes and
turns each event into a list of `Intent`s, plus a thin driver that performs the intents. Then the decisions are
unit-testable from literal bytes and the driver has almost no logic.

Rejected: a router that owns only the input path (the loop would still share geometry, viewer, session_mouse, status
and mode with it); splitting the picker input into a separate spec (the driver would switch parsers in the middle of
a read: two owners of the byte stream); making the intents fine-grained per `ClientFrame` (`Intent` names a
decision, not a wire message, so the model never builds protocol frames).

Prior lessons, stated because a spec cannot cite memory: a pure step model must own the state of every event source
(input, output scan, tick, resize, status frame), or the "pure" part shares fields with the loop. A state type that
holds a network client cannot sit inside a pure mode; the client stays in the driver. When a flag plus reset becomes an
enum variant, entering the variant builds a fresh one unconditionally. A refactor that removes a field leaves callee
methods dead and can trip `large_enum_variant`: run clippy on the target shape before locking (done below).

## 2. Target shape

`src/attach/model.rs` (public module). New public items:

```rust
pub struct Model { /* fields below, all private */ }
pub struct Step { pub consumed: usize, pub intents: Vec<Intent> }          // consumed >= 1 while offset < read.len()
pub enum Intent {
    Forward(Vec<u8>),                    // ClientFrame::Input (keys, and session mouse already shifted/viewer-checked)
    Scroll(Scroll),                      // ClientFrame::Scroll
    Redraw,                              // ClientFrame::Redraw
    Release,                             // ClientFrame::Release
    SetViewer { viewer: bool },          // ClientFrame::Mode
    Resize { cols: u16, rows: u16 },     // ClientFrame::Resize (rows = geometry.pty_rows())
    Output(Vec<u8>),                     // bytes for the human's terminal, in order
    RefreshPicker,                       // driver: request_sessions on the picker control client, then Model::picker_sessions
    OpenPicker,                          // barrier
    Switch { to: String },               // barrier; also the same-session reattach of the status toggle
    Detach,                              // barrier; ends the screen, the rest of the read is dropped
}
pub struct Attached { pub in_team: bool, pub granted_top: u16 }
```

`Scroll` is `crate::emulator::Scroll`. At most one barrier intent per call, and it is the last. `Intent` and `Step` derive
`Debug, PartialEq`. `Mode`, `PickerScreen`, `ScrollParser`, `MousePress` and the other state structs stay
`pub(super)` or private: no public signature names them (rustc's `private_interfaces` warning, promoted by
`-D warnings`, would fire if one did). The driver reads the model through four `pub` getters: `session(&self) -> &str`,
`is_viewer(&self) -> bool`, `viewport(&self) -> Viewport`, `in_picker(&self) -> bool`; it keeps no copy of those fields.

`Model` fields (moved from `AttachmentState`): `session`, `geometry`, `mode`, `prefix_machine`, `prefix_byte`,
`mouse_press`, `session_mouse`, `viewer`, `viewer_after`, `status`, `team_regions`. `AttachmentState` keeps `attachment`,
`home`, `output`, `picker_control` and gains `model: Model`. `after_prefix` is deleted.

Methods (all synchronous, no I/O, no `Instant::now()` inside: the caller passes `now`):

| Method | Replaces | Returns |
|---|---|---|
| `Model::new(session, geometry, prefix, viewer, viewer_after, now)` | `AttachmentState::new` field setup | `Model` |
| `step(&mut self, read: &[u8], offset: usize, now: Instant) -> Step` | `handle_input` loop body, `prefix_batch_len`, `feed_scroll_input`, `handle_mouse_*`, `handle_prefix_actions`, `team_click`, `select_member`, `next_member`, `member_by_number`, `toggle_status_line`, `toggle_viewer_mode`, `forward_or_block`, `enter_scroll_mode`, the picker input half of `handle_picker_input` | `Step` |
| `on_output(&mut self, data: Vec<u8>, now) -> Vec<Intent>` | the `Data` arm of the event loop, `scan_session_mouse`, `draw_status`, `draw_viewer_decorations` | `Output` intents in today's send order; it scans the session mouse mode even in picker mode and then returns `[]` |
| `on_status(&mut self, info: StatusInfo, now) -> Vec<Intent>` | the `Status` arm | `Output` intents |
| `on_tick(&mut self, now) -> Vec<Intent>` | the tick arm | `SetViewer`, `Redraw`, `Output` intents |
| `resize(&mut self, cols, rows, now) -> Vec<Intent>` | the SIGWINCH arm | `Output` (picker re-render), then `Resize`, then in viewer mode `Output` (status), `Output` (decorations) |
| `picker_opened(&mut self, sessions: Vec<SessionSummary>) -> Vec<Intent>` | `open_picker` after the daemon calls | `Output(render)`; sets the mode, clears `team_regions` |
| `picker_sessions(&mut self, sessions) -> Vec<Intent>` | `refresh_picker` after the daemon call | `Output(render)` |
| `attached(&mut self, to: String, result: Attached) -> Vec<Intent>` | `adopt` plus the success block of `switch_session` | the title `Output`, always (the same-session switch of the status toggle sends it too, as today) |
| `switch_failed(&mut self, error: String, restored: Attached) -> Vec<Intent>` | the restored-after-failed-switch block | `RefreshPicker` when the mode is `Picker` |

`PrefixMachine::feed_until_command(&mut self, input: &[u8]) -> (usize, Vec<Action>)` is added to `src/prefix.rs`
beside `feed` (extend, not a second helper): it behaves like `feed` over `input` but stops right after the first
`Action::Command`, returning the number of bytes consumed (the whole input when no command occurs) and the actions
produced up to and including it. `feed` stays for its other callers.

The driver (`src/attach/event_loop.rs` and `state.rs`, no decisions beyond error handling):

```text
step loop:   offset = 0; while offset < read.len() { step = model.step(read, offset, now); offset += step.consumed;
             for intent in step.intents { perform(intent)?; } if a Detach was performed, return its outcome }
perform:     Forward -> attachment.send(Input) (skipped without an attachment), Scroll/Redraw/Release/SetViewer/Resize likewise,
             Output -> output.send, RefreshPicker -> request_sessions(picker_control) then model.picker_sessions, performing its intents,
             OpenPicker -> connect, request_sessions, model.picker_opened(...) (errors propagate and end the screen, as today),
             Switch{to} -> the 5 s detach drain, attach_session(to); Ok -> model.attached(to, ...);
                           Err -> attach_session(current): Ok -> model.switch_failed(err, ..) ; Err -> the combined error, as today,
             Detach -> attachment.send(Detach), return AttachOutcome{"[detached from <session>]"}.
```

A same-session `Switch` (status toggle) takes the same path. `Forward`, `Scroll`, `Redraw`, `Release`, `SetViewer`, `Resize` and `Detach` are skipped when there is no attachment (`Detach` still returns its outcome). Picker control lifetime: the driver sets `picker_control = Some(control)` after performing `OpenPicker` successfully, and after performing each batch of intents (a `step`, an `attached`, a `switch_failed`) drops it when `!model.in_picker()`, so a control connection never outlives its picker. `find_mouse_report` (`src/attach/input.rs`, in scope) now returns the parsed `MouseReport` with its span, so the old `Err("mouse report parser lost a report boundary")` branch disappears and `step` needs no `Result`.

## 3. Behaviour that must not change

### 3.1 Order and reads (all become model tests, 4.2)
- `viewer.own_input(now)` runs once per read, at `offset == 0`, unless the whole read is exactly a focus report.
- The viewer-blocked check runs per `Forward` batch (not per read); a batch that is exactly a focus report is not
  counted as blocked; a blocked batch emits `Output` (viewer decorations), never `Forward`.
- `escape_alone` is "the whole read is exactly one `0x1b`", given to the picker parser on every picker step.
- `pass_through` (live mode, `session_mouse && !viewer`, `granted_top == 0`) is recomputed on every step; when set,
  no report is looked for and the rest of the read goes to the prefix machine up to and including the next command.
- With `granted_top > 0` and session mouse on, split input stays unshifted until a complete report is found (the
  `// shortcut:` comment in `handle_input` moves with the code).
- A pending prefix survives a mouse report in the same read: the report check comes before the machine.
- Reports: `find_mouse_report(read, offset)`; a report at `offset` is consumed whole and handled by the team-row
  handler when `granted_top > 0` and the mode is live, else by the button handler; otherwise the segment before the
  next report is handled by the current mode.
- `Detach` drops the rest of the read. Picker and scroll modes take one byte per step.

### 3.2 Mouse (literal rules, kept)
Only button 0 records a press: row 1 `Team`, row 2 `Rule`, a grid row `Grid`; a `Team` press fires a team click only
if the release is on row 1; a `Grid` press released on a reserved row sends the report rewritten to row 1 only with
session mouse on; a release in the grid after a `Team` press is swallowed; a release with no press is forwarded with
session mouse on; `Rule` presses are absorbed; a grid report without session mouse goes to the button handler; in the
picker mouse reports are ignored; button 65 scrolls down only in scroll mode; button 64 enters scroll mode, sends
`Redraw` and three `Scroll(LineUp)`; rows are shifted with `granted_top`, never `desired_top`; X10 rows saturate at u8
(`mouse::shift_row` is unchanged).

### 3.3 Switches and results
`mouse_press` is cleared before the attach (the model does it when it emits `Switch`, nothing reads it in between);
the success block clears `team_regions`, sets the session, sends the title OSC `ESC ] 2 ; a2amx: <name> BEL` and
leaves the picker; the failure block restores the previous session with `status = None`, `session_mouse = false`,
fresh `in_team`/`granted_top`, does not clear `team_regions`, sends no title, and (with the picker open) sets the
error footer, refreshes (an error ends the screen, as today) and renders. A refused attach of top rows sends
`Resize` with top 0 (stays in `open_attachment`). `open_picker` clears `team_regions` after the first render.
Picker input renders on the next byte after the footer is cleared and calls `set_filter_mode` after every applied
action. `Cancel` and `Attach(current)` send `Redraw` and close the picker, then the byte is done.

### 3.4 What changes
`feed_until_command` coalesces forwarded bytes wherever `PrefixMachine::feed` does, so any batch the old code cut at a
prefix byte that produced no command (a doubled prefix `ab\x02\x02cd`, an unknown key after the prefix `ab\x02zcd`, a
prefix byte inside a bracketed paste) reaches the daemon as one `Input` frame instead of two or three; the bytes and
their order are identical. In viewer mode each forwarded batch runs the blocked check and redraws the viewer
decorations, so those redraws repeat once per batch instead of once per old cut: the same marker and overlay bytes,
fewer times. Frame boundaries of the forwarded stream and the repeat count of viewer decorations are not part of the
contract. `tests/attach_cli/chunked_input.rs:129-146` checks captured bytes only and must pass unchanged.

## 4. Tests

### 4.1 `tests/prefix.rs` (added method)
`feed_until_command` over literals (prefix `0x02`): `b"abc"` returns `(3, [Forward(abc)])`; `[0x02, b'd']` returns
`(2, [Command(Detach)])`; `b"ab\x02dxyz"` returns `(4, [Forward(ab), Command(Detach)])`; `[0x02]` returns `(1, [])` and
a following `b"w"` returns `(1, [Command(SessionPicker)])`; inside a bracketed paste (`ESC[200~` ... `ESC[201~`) a
`0x02 d` pair is forwarded and no command stops the batch.

### 4.2 `tests/attach_model.rs` (extend the file from spec 3fu, which has 3 tests; public seam; expected values are literals or worked examples from the code at
`e79ed92`, never computed by the model)
A `Model` over 80x24 visible, `in_team` false, `granted_top` 0, prefix `0x02`, session `"s1"`:

1. plain bytes `b"abc"` at offset 0: `consumed 3`, `[Forward(b"abc")]`.
2. `[0x02, b'd']`: `consumed 2`, `[Detach]`; `b"ab\x02d"`: `consumed 4`, `[Forward(b"ab"), Detach]`.
3. `[0x02, b'w']`: `consumed 2`, `[OpenPicker]`; after `picker_opened` with one session `s1`, a step at offset 2 of
   `b"\x02w/"` consumes 1 and returns exactly one `Output`.
4. `[0x02, b'[']` then `ESC[<65;10;5M` as one read: first step `consumed 2`, `[Redraw]` (scroll mode entered); second
   step at 2 consumes the 11-byte report and returns three `Scroll(LineDown)`; `b"q"` in scroll mode returns
   `[Scroll(Bottom), Redraw]` and a following `b"x"` is `[Forward(b"x")]` (live again).
5. wheel up with session mouse off, `ESC[<64;10;5M`, `granted_top` 0: `[Redraw, Scroll(LineUp) x3]`.
6. session mouse on (after `on_output(ESC[?1000h)`), `granted_top` 0, `ESC[<0;10;5M`: one `Forward` of the same
   bytes, no report parsing (pass-through); with `granted_top` 2 (viewport 24 rows, `in_team` true) the same press
   at row 5 is shifted: `Forward(ESC[<0;10;3M)`.
7. Survivor test 1 (shift uses the granted top): viewport 80x4 visible, `in_team` true, `granted_top` 2, session
   mouse on: press then release at row 1 (`ESC[<0;5;1M`, `ESC[<0;5;1m`) are claimed by the team row: both steps return
   no intents (no region is under column 5). With `desired_top` instead of `granted_top` the report would reach the
   session as `Forward`.
8. Survivor test 2 (`attached` resets), three cases on one 80x24 model:
   (a) session mouse on (`on_output(ESC[?1000h)`), `granted_top` 0; `attached("s2", Attached{in_team:false,
   granted_top:0})`; then `ESC[<0;10;5M` returns `[]` (no longer a pass-through; a left press in the grid with
   session mouse off does nothing). Before `attached` the same report returns `[Forward(ESC[<0;10;5M)]`.
   (b) `granted_top` 2, `in_team` true, an `on_status` frame listing members `s1`, `s2`; `attached("s1",
   Attached{in_team:true, granted_top:2})` clears the status, so `[0x02, b'n']` returns `[]`; before it,
   `[0x02, b'n']` returns `[Switch{to: "s2"}]`.
   (c) `granted_top` 2, then `attached("s1", Attached{in_team:false, granted_top:0})`, then session mouse on
   (`on_output(ESC[?1000h)`): `ESC[<0;10;1M` returns `[Forward(ESC[<0;10;1M)]` (granted 0 is a pass-through); with
   the old `granted_top` kept it would be claimed by the team row and return `[]`.
9. Survivor test 3 (team regions cleared on switch and on `picker_opened`): a model with a team status frame drawn
   (`on_status` with a team and members, `granted_top` 2, 80x24) holds a `TeamTarget::Picker` region at column 1
   (`status.rs:138`). Press and release at row 1 column 1 (`ESC[<0;1;1M`, `ESC[<0;1;1m`) return `[OpenPicker]`.
   After `attached("s2", Attached{in_team:true, granted_top:2})` the same press and release return `[]` (the stale
   region is gone; `status` is also `None`, so a member target could not tell the difference: click the picker
   region). For `picker_opened`: after `[0x02,b'w']` and `picker_opened(sessions)`, send `b"\x1b"` alone (Cancel, `[Redraw]`
   and live again), then press and release at row 1 column of a member region returns `[]` (`picker_opened` cleared
   the regions; the model draws none until the next `on_output`/`on_status`).
10. Survivor test 4 (scroll entry keeps the parser while scrolling): in scroll mode feed `b"\x1b"` (pending), then
    `ESC[<64;10;5M`, then `b"[A"` as three steps: the report is handled by the button handler (already scrolling, no
    rebuild) and `[A` completes `ESC [ A` as `Scroll(LineUp)`; a model that rebuilt the parser on the wheel returns no
    scroll for `[A`.
11. viewer: a model with `viewer == true`, 80x24: `b"a"` returns two `Output` intents (marker then overlay). `b"a" +
    ESC[<0;10;5M + b"b"` as one read, stepped at 0, 1, 12 (the report in the middle) returns marker+overlay for
    each of the two key batches. Focus reports: a model with `viewer == false`, `viewer_after == Some(1 s)`, created
    at `t0`; `step(ESC[I, 0, t0 + 900 ms)`; `on_tick(t0 + 1 s)` returns `[SetViewer{viewer:true}, ..]` (a focus report
    is not input); the same sequence with `b"a"` instead of the focus report returns `[]` at the tick.
12. `resize(100, 30, now)` returns `[Resize{cols:100, rows:28}]` in live mode (30 rows, status visible: 2 status rows, granted 0).
13. status toggle: `[0x02, b's']` with `in_team` true and `granted_top` 2 on a 80x24 viewport returns, in order,
    `Output(render_team_line("s1", "", &[], 80).bytes)`, `Output(render_team_rule(80, false))`,
    `Output(clear_status(80, 24))`, then `Switch{to: "s1"}` (desired top becomes 0, granted is 2). With `in_team`
    false and `granted_top` 0: `Output(clear_status(80, 24))` then `Resize{cols: 80, rows: 24}` (hidden status takes
    0 rows). The expected bytes are those functions' committed output; the test imports them from `a2amx::status`.

The developer proposes any further literal cases in the plan; the tester checks the list against 3.1 and 3.2.

### 4.3 Existing suite
`cargo test --test attach_cli` (63 tests, counted with `-- --list` at `e79ed92`, including 3fr's 11 and 3ft's 3) passes unchanged. `close_picker`/`picker_control` clearing has no model test (the model holds no client); it is covered by the driver rule in section 2 and is a known survivor unless a PTY test can show the control connection closing.

## 5. Out of scope
The three visible fixes named in section 0 (status-toggle reattach on refused rows with `requested_top`, the swallowed
`refresh` error after a failed switch, bracketed-paste-aware mouse scan); split mouse sequences across reads; SIGWINCH
renegotiating top rows; typed picker rows in `table`; `granted_top` as the only source of the top rows
(`attachment.top_rows()` stays); `PickerScreen.cols/rows` duplicating the viewport; renaming terminal `Output`; any
command-layer or `main.rs` change; moving `AttachmentConfig` or `open_attachment` out of `src/attach.rs`.

## 6. Acceptance
- `cargo test --no-fail-fast`: all pass; baseline at `e79ed92`: 64 suites, 1373 passed, 0 failed, 3 ignored; after:
  the new tests added, nothing removed. `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`: pass.
- Outside judge, `rg` (the architect runs; each prints nothing when done and prints matches today):
  `rg -n 'after_prefix|fn prefix_batch_len|fn handle_mouse_report|fn handle_prefix_actions|fn team_click|fn select_member|fn feed_scroll_input|fn forward_or_block|fn scan_session_mouse' src/attach.rs src/attach --glob '!src/attach/model.rs' --glob '!src/attach/model/*'`
  (13 matches today) and
  `rg -n '(self|state)\.(mouse_press|session_mouse|team_regions|viewer|status|prefix_machine|prefix_byte|geometry|mode|session)\b' src/attach/state.rs src/attach/event_loop.rs src/attach.rs`
  (the driver reaches the model only through `model.`; about 32 `self.` and 16 `state.` hits today). `model.rs` and
  `model/` are not scanned.
- Purity, read by the architect and the tester, plus `rg -n 'async|\.await|Instant::now|OutputHandle|\bClient\b|ClientFrame|Attachment|tokio|anyhow|crate::client' src/attach/model.rs src/attach/model/`
  printing nothing (comments excepted: say so).
- No file over 1,000 lines (`wc -l src/attach/*.rs`); `git status --short` lists only section 0's files;
  `git diff --stat -- Cargo.toml Cargo.lock src/main.rs src/lib.rs` is empty.
- Mutation check (developer; mutate in the checkout, restore by checksum; every mutant needs a named killing test,
  a survivor is reported): `step`: `own_input` called for a whole-read focus report (11; an `own_input` at every offset is an equivalent mutant: it is idempotent for the same `now`, do not list it); blocked check once per read instead of once per batch (11); `escape_alone` always
  false; `pass_through` without the `granted_top` term; `shift_row(.., desired_top())`; barrier intent not last;
  `attached` not resetting `session_mouse`, `status`, `granted_top` (8a, 8b, 8c); `attached` not clearing `team_regions`
  (9); `picker_opened` not clearing `team_regions` (9); a focus report counted as input (11); scroll entry skipping the parser rebuild; `Cancel` not sending
  `Redraw`; `on_output` skipping `MOUSE_CAPTURE` after `ESC[?1000l`; `resize` sending raw rows.
- Known flaky (rerun once, record as pre-existing if it passes): presence
  `viewers::viewer_attach_keeps_input_read_only_while_sizing_session`, discord
  `gateway_queue::gateway_queue_reads_heartbeats_while_consumer_is_slow`, agy_plugin
  `descendant_pipe_holder_fails_with_pipe_error`, attach_cli `attach_does_not_capture_session_already_in_drag_mode`
  and `status_toggle_reattaches_to_resize_the_session`.
- The tester runs an independent mutation pass and a diff walk; a consultant reads the design.

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| A picker byte yields at most one action (so `Attach` is never followed by more actions of the same byte) | `src/picker.rs:32-67`: each iteration pushes at most one action then `continue`s or falls through to one `escape_action` push | read |
| `escape_alone` returns exactly `[Cancel]` | `src/picker.rs:34-37` | read |
| `after_prefix` splits at the prefix byte even inside a paste | `state.rs prefix_batch_len` (read at `e79ed92`) | read |
| `enter_scroll_mode` is the only place that builds the scroll parser | `rg ScrollParser::default src` | run at 3fu |
| `switch_session` result in the toggle is ignored (`let _`) and nothing runs after it in `toggle_status_line` | `state.rs` toggle (lines 405-407) | read |
| the `Data` arm tests `has_mouse_mode && !session_mouse` after `scan_session_mouse` updated `session_mouse`, so `MOUSE_CAPTURE` is sent only after a mode `l` | `event_loop.rs` Data arm | read |
| `Mode`, `PickerScreen`, `ScrollParser` stay `pub(super)` and do not appear in a public signature; `private_interfaces` is a rustc default warning that `-D warnings` promotes | consult of 3fv | read |
| Worked examples 4.2 items 2, 4, 5, 6 | `state.rs handle_input`, `mouse::shift_row`, `ScrollParser::feed` | hand-walked; the consultant re-walks |
| Examples 9 and 13 need bytes the model renders from `status::*` (`render_team_line`, `clear_status`, `render_team_rule`) | `src/status.rs:61,200,404` | NOT verified: the developer writes the literals into its plan from the committed code; the tester checks them |
| No test names `after_prefix`, `prefix_batch_len`, `scan_session_mouse`, `handle_mouse_report`, `handle_prefix_actions`, `select_member`, `feed_scroll_input`, `forward_or_block` or `picker_control` (one test function name contains the word `team_click`: `viewer_team_click_acts_on_release_and_keeps_viewer_mode`, a test name, not a use) | `rg` over `tests/` | run |
| `Model`, `Step`, `Intent`, `Attached` public with `pub(super)` internals compile without warnings (no dead code after the move) | n/a | NOT verified: clippy at implementation |
| `find_mouse_report` already calls `parse_report` | `src/attach/input.rs:21-25` | read (consult) |
| `own_input(now)` is idempotent for the same `now`; a no-op in viewer mode | `src/viewer.rs:24-28` | read (consult) |
| `model.rs` will reach about 700-800 lines | consult hand count | estimate |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (fresh consultant session for the draft review and for the design
read); senior developer Antigravity CLI, single slot Gemini 4 Hydrogen (Medium); tester OMP
opencode-go/deepseek-v4.1-flash, single slot. BLOCKED/amend cycles: 1.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | `step` could not express the old `Err("mouse report parser lost a report boundary")` (no `Result`, no `unwrap`) | `find_mouse_report` returns the parsed report; the branch disappears |
| draft review | consultant | section 3.4 understated the change: any prefix-cut batch without a command coalesces, and viewer decorations repeat once per batch | 3.4 widened; section 0 promise reworded |
| draft review | consultant | missing read accessors, the `picker_control` lifetime rule, an unused `switched` parameter, the viewer-mode resize tail, `Detach` without an attachment | getters, driver rule, parameter dropped, rows and wording fixed |
| draft review | consultant | six test examples wrong or unable to kill their mutant (items 8, 9, 11, 12, 13), an equivalent mutant listed, acceptance greps matching the model, wrong count note | examples rewritten, mutant removed, greps scoped |
| implementation | developer (BLOCKED m_8221) | `Model::resize` had no `now` although the old viewer-mode path calls `draw_viewer_decorations(Instant::now())` | amendment 1 (93b9ed7): `resize(cols, rows, now)`; architect and consultant both missed it (no list of which old paths read the clock) |
| tester pass | tester | 8 of 33 mutants survived: exited-member skip, TeamPrev, team-click column bounds, `switch_failed` clearing regions, footer re-render (all expressible), `picker_control` drop and `Detach` dropping the rest of the read (no seam) | developer added five model tests (fix request, 4 further mutants killed); the two seam-less ones accepted as known survivors |
| tester pass | tester | spec text slips: item 11 offsets, dev-plan consumed count | corrected in the plan and test comments |
| architect review (consultant design read) | consultant | driver filtered intents by type at nested call sites and ignored `Detach` outcomes in four loop arms; lost `// shortcut:` comment; `Mode` visibility; implicit invariants | work queue in `perform_intents`, outcomes returned, comment restored, `Mode` private, doc lines added (fix request 2) |
