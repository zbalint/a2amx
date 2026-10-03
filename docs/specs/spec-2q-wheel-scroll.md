# Spec 2q: mouse-wheel scrolling in the attach client

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 follow from the owner's report on 2026-10-03 and
the owner's agreement to the approach, including the Shift-to-select trade-off (D7). Builds on
`152aaff`. **Worktree and branch:** `/home/zbalint/workspace/a2amx-2q`, branch
`feature/2q-wheel-scroll` (created from `152aaff`); the main checkout belongs to spec 2p's
developer and must not be touched. Shared task `context_id`: `a2amx-wheel-scroll`. Public test
seam: the `a2amx attach` process in a pty against a real daemon, as `tests/attach_cli.rs` does
(`tests/common`).

**Scope.** May edit exactly these files and no others.

- `src/main.rs`, `src/messaging.rs` (only to make `sgr_mouse_report_len` `pub`)
- `tests/attach_cli.rs`
- `README.md`, `docs/architecture.md`

Does not touch every other file under `src/` and `tests/`, `scripts/`, `Cargo.toml`,
`Cargo.lock`, `AGENTS.md`, the locked spec documents. No new dependency. Do not commit and do
not merge: leave the diff uncommitted in the worktree.

Spec 2p is being implemented in parallel in the main checkout and also edits `src/main.rs`
(`quota_cell`), `README.md` and `docs/architecture.md`. Keep every hunk away from those
regions; the architect merges the two branches.

## 1. Why

Natively, OMP draws on the terminal's normal screen, so the terminal's own scrollback and mouse
wheel work and the session stays in scrollback after OMP quits. Codex and Claude Code draw on the
alternate screen themselves and handle the wheel or arrow keys, so they behave the same under
a2amx. The attach client always switches the terminal to its own alternate screen
(`src/main.rs:626`, `\x1b[?1049h`), which has no terminal scrollback, and OMP never enables mouse
reporting (checked on a live session: its snapshot contains only mouse-off resets), so with OMP
under a2amx the wheel does nothing. The daemon already keeps 10,000 lines of scrollback per
session and the client already has a scroll mode (Ctrl-B `[`, PageUp and the other keys of
`parse_scroll_sequence`); the wheel has no route to it.

Out of scope and a known limit: scrollback in the real terminal after detaching, which OMP gives
natively. The attached screen is wiped on exit, as it is for Codex and Claude.

## 2. Decisions

- **D1.** While the attached session has not turned on mouse reporting, the client owns the
  mouse: it asks the outer terminal for SGR mouse reports (`\x1b[?1000h` and `\x1b[?1006h`).
- **D2.** In that state a wheel-up report (SGR button 64) enters scroll mode if it is not active
  and sends `ClientFrame::Scroll(Scroll::LineUp)` three times; a wheel-down report (button 65)
  sends `Scroll::LineDown` three times, and does nothing when scroll mode is not active. Three
  lines is the wheel step.
- **D3.** In that state every other mouse report (clicks, drags, motion, other buttons) is
  dropped: never forwarded to the session, never fed to the prefix machine, scroll parser or
  picker.
- **D4.** When the session has turned mouse reporting on (Data frames set mode 1000, 1002 or
  1003), the client does not interfere: no capture of its own, and input, including wheel
  reports, is forwarded to the session exactly as today.
- **D5.** The client learns the session's mouse state from the Data frames it already receives
  (private-mode set or reset of 1000, 1002, 1003 in the daemon's rendered output; the daemon
  emits each sequence whole inside one frame). The state starts as "not on"; a full snapshot
  begins with explicit resets, so a reattach resynchronises it.
- **D6.** Scroll mode entered by the wheel is the existing scroll mode: `q` leaves it, the
  `[scroll: q to exit]` indicator shows, and PageUp, arrows and Home/End keep working in it.
  The wheel never leaves scroll mode on its own (the client does not know the scroll offset).
- **D7.** Trade-off accepted by the owner: while the client owns the mouse, plain click-drag
  selection in the outer terminal does not work for that session; Shift-drag does.
- **D8.** A mouse report split across input chunks is not reassembled; the pieces are handled
  as ordinary input. This is a corner cut on purpose (see section 3).
- **D9.** Terminal restore is unchanged: `RESTORE_TERMINAL` already resets 1000, 1002, 1003 and
  1006 on exit and detach.

## 3. `src/main.rs`, `src/messaging.rs`

`src/messaging.rs`: make `sgr_mouse_report_len` `pub` (no other change) so the client reuses it
rather than a second parser. It takes the byte slice and the index of an `ESC`, whose `[<` the
caller has checked, and returns the end index of a complete `ESC [ < b ; x ; y (M|m)` report.

`src/main.rs`:

1. `AttachmentState` gains a field for the session's mouse state (a bool for "session owns the
   mouse"), updated from each `ServerFrame::Data` before it is written to the outer terminal.
   Detection scans the frame bytes for `ESC [ ? <digits> (h|l)` and handles the mode numbers
   1000, 1002 and 1003; the last one seen wins (`h` sets, `l` clears); a frame without any such
   sequence leaves the state alone. Combined parameters (`ESC [ ? 1000 ; 1006 h`) are not
   produced by the daemon renderer and need no handling.
2. When the state is "not on", the client keeps `\x1b[?1000h\x1b[?1006h` in force on the outer
   terminal: send it once right after `\x1b[?1049h` at attach, and again after each Data frame
   whose scan changed the state to "not on" or that contained a mouse reset (a snapshot's
   explicit resets would otherwise turn it off). When the state becomes "on", send nothing (the
   session's own sequences already went out in the frame). Do not emit the capture while the
   picker is open.
3. In `handle_input`, before the picker, scroll and prefix branches run, take out every complete
   SGR mouse report with `sgr_mouse_report_len`:
   - state "on" (D4): leave the bytes in place and continue exactly as today;
   - state "not on": remove the report from the bytes. Wheel up and down act as in D2 (the
     existing `ScrollMode` command path sets `scroll_mode`, clears `scroll_parser`, and sends
     `ClientFrame::Redraw`; share it rather than copying), every other report is dropped (D3).
   The status line must keep drawing as it does now when scroll mode changes.
4. `// shortcut:` comments: on the split-report corner (D8, "a report split across input
   chunks reaches the session as typed input; upgrade if it proves noisy", like
   `input_is_typing`) and on the fixed wheel step of three lines ("upgrade to a setting if
   asked").

Edit scope inside `src/main.rs`: `AttachmentState`, `handle_input`, the Data-frame arm of
`run_attachment_loop` and the attach setup near the `\x1b[?1049h` send. Not `quota_cell`,
`SESSION_HEADERS`, the team code, the picker, or the list code.

## 4. Tests

Failing test first, one behavior at a time, in `tests/attach_cli.rs`, using its pty helpers.
Expected screen text comes from literals.

1. A session whose child prints `line-001` to `line-100` one per line and then sleeps (so the
   history is above the 24-row screen), mouse reporting off. Attach in a pty, write one wheel-up
   report `\x1b[<64;10;5M`; the attach output eventually contains an earlier line (for example
   `line-070`) and the text `[scroll: q to exit]`. A wheel-down report after it moves toward
   newer lines.
2. Same session: a click report `\x1b[<0;10;5M` followed by `\x1b[<0;10;5m` produces no scroll
   mode and nothing reaches the child (the child here echoes its input with `cat -v` into a
   marker the test can read through `a2amx` output; follow the existing forwarding tests).
3. A child that turns mouse reporting on (`printf '\033[?1000h\033[?1006h'`) and then runs
   `cat -v`: after the attach has seen that, a wheel-up report is forwarded and `cat -v` shows
   `^[[<64;10;5M`; no scroll mode appears.
4. A child that is on and then turns mouse reporting off again returns to capture: a wheel-up
   report then enters scroll mode.
5. The outer terminal gets `\x1b[?1000h` and `\x1b[?1006h` at attach for a mouse-off session,
   and neither for a session already in mouse mode.

If a case cannot be observed through the pty, say so in the report rather than testing a
private function.

## 5. `README.md`, `docs/architecture.md`

`README.md`: after the sentence about `[` scroll mode, add: `While a session has not turned on
mouse reporting, the mouse wheel scrolls it (three lines a step) and enters scroll mode; hold
Shift to select text with the mouse. Scrollback in the real terminal after you detach is not
kept.`

`docs/architecture.md`: in the client section next to scroll mode (`docs/architecture.md:270` and
`:285`), one short paragraph in the document's style: the client owns the mouse while the
session's mouse reporting is off, turns wheel reports into scroll-mode steps, drops other
reports, learns the session's mouse state from the mode sequences in its Data frames, and leaves
the mouse to the session when it turns reporting on.

## 6. Out of scope

Writing a session's scrolled-off lines into the real terminal's scrollback, leaving scroll mode
on reaching the bottom, a configurable wheel step, reassembling split reports, mouse selection
or copy inside the client, changes to the daemon, wire or emulator, `src/quota.rs`, and anything
spec 2p covers.

## 7. Acceptance

Run all three in the worktree, no warnings, with `A2AMX_BIN` unset (`env -u A2AMX_BIN`):

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`git diff --stat` shows only the files in section 0; `git diff --check` is clean. The wheel
itself needs a real terminal: say in the report that the end-to-end wheel check is left for the
owner, with the exact steps (attach to an OMP session, scroll the wheel, press `q`, Shift-drag
to select).

## Amendment 1 (architect, on the developer's pre-implementation check)

Section 3 point 2 says to send the capture sequences once right after `\x1b[?1049h` at attach,
and section 4 test 5 says a session that is already in mouse mode gets neither of them. At
attach the client has not seen a Data frame yet, so it cannot tell the two cases apart; the two
requirements contradict. Resolution:

- **Section 3 point 2** drops the send at attach. The capture is sent only after a scanned Data
  frame leaves the state "not on" and that frame contained a private-mode set or reset of 1000,
  1002 or 1003 (the first Data frame, a full snapshot, always contains the explicit resets, so a
  mouse-off session is captured right after its first snapshot). Nothing else in that point
  changes.
- **Section 4 test 5** is replaced by: a mouse-off session's attach output contains
  `\x1b[?1000h` and `\x1b[?1006h` after its snapshot; a session whose child sends only
  `\x1b[?1002h` (no 1006) has no `\x1b[?1000h` and no `\x1b[?1006h` anywhere in the attach
  output, so the client added nothing of its own.

No other decision changes. The architect committed this amendment on the branch; it does not
touch the working tree.
