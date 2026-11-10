# Spec 3dq: viewer mode in the attach client (F83, second half)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/viewer.rs` (new), `src/lib.rs` (one `pub mod viewer;` line), `src/prefix.rs` (one command), `src/cli.rs` (one global option), `src/main.rs` (the attach client; `run_attachment` and its three callers), `tests/viewer.rs` (new), `tests/prefix.rs` and `tests/prop_prefix.rs` (only for the new command), `tests/attach_cli.rs`, `tests/common/pty.rs` (only if a helper is missing), `docs/cli.md`, `docs/remote-messaging.md`. Does not touch: the daemon, wire and MCP code (spec 3dp), `src/status.rs` (the marker is drawn by the client, as the scroll marker is), `src/picker.rs`, `CHANGELOG.md`, `AGENTS.md`, Cargo files.

Location: main checkout, branch `develop`; base is `80efb57` (spec 3dp accepted; `Client::attach_viewer`, `Request::Attach.viewer`, `ClientFrame::Mode` exist). Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: the pure `viewer` module API (with an injected `Instant`), and the real `a2amx` binary under `PtyHarness` against the fake daemon in `tests/attach_cli.rs`.

Second of two F83 specs. It depends on 3dp: `Request::Attach { viewer }`, `ClientFrame::Mode { viewer }` and the daemon's presence.

## 1. Why

Owner request (2026-11-09, `docs/backlog.md` F83): when the owner is at the computer the agents must not message the phone, and when the owner has walked away the agents should. The daemon decides from the attach mode (spec 3dp); this spec gives the owner the control. A viewer is an attach that keeps the screen live but cannot type, so the session looks detached to the agents while the owner can still watch. The terminal switches to viewer by itself after a period without the owner's input and does not come back on a stray key.

Owner decisions: viewer mode toggles with the prefix key then `v`; it also starts automatically after a configurable period without the owner's own input (agent output does not count; default 5 minutes; 0 turns it off); after an automatic switch the owner leaves it only by the toggle, never by "any key"; in viewer mode every key, mouse event and paste is ignored except the prefix commands detach, session picker, scroll and team switching; switching sessions keeps viewer mode; a viewer does not resize the session; the status line shows a bright VIEW marker, no blinking; a key, paste or non-scroll mouse report that is ignored shows a bordered box in the middle of the screen for about two seconds with the real prefix key in the text; mouse events and the prefix commands never show it.

Alternatives rejected: leaving viewer mode on any key (a stray key after an automatic switch would make the owner "present" and silence the agents, owner decision); a blinking marker (owner decision); counting agent output as activity (it would never time out while an agent works); a separate overlay layer or a second screen model in the client (the client already redraws the status line after every screen update and can ask the daemon for a full repaint, so the overlay uses those two mechanisms).

## 2. Changes

### 2.1 The pure module `src/viewer.rs`

Owns the viewer state and its two renderings. No terminal, no clock of its own; every method takes `now: Instant`.

- `pub struct Viewer` with: `viewer: bool`, `last_own_input: Instant`, `overlay_until: Option<Instant>`.
- `Viewer::new(now, viewer: bool) -> Self`.
- `is_viewer(&self) -> bool`.
- `own_input(&mut self, now)`: the owner typed, pasted or moved the mouse while interactive; resets the idle clock. Ignored while a viewer.
- `toggle(&mut self, now) -> bool`: switches the mode and returns the new `is_viewer`; entering interactive resets the idle clock; leaving it clears the overlay.
- `idle_expired(&mut self, now, after: Option<Duration>) -> bool`: when interactive, `after` is `Some`, and `now - last_own_input >= after`, switches to viewer and returns `true` once; `None` (option value 0) never fires.
- `blocked(&mut self, now)`: an ignored key or paste; sets `overlay_until = now + 2 s` (a repeat extends it).
- `overlay_active(&self, now) -> bool`; `take_overlay_end(&mut self, now) -> bool` returns `true` once when the overlay has just expired (the caller then repaints).
- `pub fn prefix_name(prefix: u8) -> String`: `Ctrl-Space` for 0, `Ctrl-A`..`Ctrl-Z` for 1..=26 (so `Ctrl-B` for 0x02), `Ctrl-[` `Ctrl-\` `Ctrl-]` `Ctrl-^` `Ctrl-_` for 0x1b..=0x1f, else `0x` and two lowercase hex digits. The status line's hold hint writes `^B`; the overlay and `docs/cli.md` write `Ctrl-B` on purpose, and no existing helper is reused (none exists).
- `pub fn overlay(cols: u16, pty_rows: u16, top_rows: u16, prefix: u8) -> Vec<u8>`: a bordered box drawn in the session area (rows `top_rows + 1` to `top_rows + pty_rows`). Two text lines: `VIEW MODE` and `typing is off, press <prefix_name> v to type`. Let `t2` be the second line's length; the inner width is `t2 + 2`, the box width `t2 + 4`, the height 4. Left column `(cols - width) / 2 + 1`, top row `top_rows + (pty_rows - 4) / 2 + 1` (integer division). Bytes: `ESC 7`, then for each of the four box rows `ESC [ row ; col H`, with the first row preceded by `ESC [ 0 m` and the rows `┌` + `─`×inner + `┐`, `│ ` + first line left-aligned and space-padded to `t2` + ` │`, `│ ` + second line + ` │`, `└` + `─`×inner + `┘`, then `ESC 8`. Empty when `cols < width` or `pty_rows < 4`. Worked example for `overlay(80, 22, 0, 0x02)` (width 41, column 20, top row 10): `"\x1b7\x1b[10;20H\x1b[0m┌───────────────────────────────────────┐\x1b[11;20H│ VIEW MODE                             │\x1b[12;20H│ typing is off, press Ctrl-B v to type │\x1b[13;20H└───────────────────────────────────────┘\x1b8"`. The longest name (`Ctrl-Space`) needs 45 columns.
- `pub fn marker(cols: u16, rows: u16) -> Vec<u8>`: ` VIEW ` (six characters) in bold black on yellow (`\x1b[1;30;43m`), no blink attribute, on the separator row `rows - 1` (the row `status::render` draws above the status row when `rows >= 4`), starting at column `cols - 6` so it ends at column `cols - 1` (the last column stays untouched, as `status::render` leaves it, `src/status.rs:224`). It does not use the status row, whose right end carries the pending and HELD alert (`src/status.rs:343-351`). Empty when `rows < 4` or `cols < 12`. Worked example `marker(80, 24)`: `"\x1b7\x1b[23;74H\x1b[1;30;43m VIEW \x1b[0m\x1b8"`.

### 2.2 Prefix command and option (`src/prefix.rs`, `src/cli.rs`)

`prefix::Command::ViewerMode`, produced by the byte `v` after the prefix (`src/prefix.rs:128-162`, today `v` falls into `_ => {}`). The existing prefix-batch handling in `handle_input` (`src/main.rs:2557-2560`) needs no change for it.

Global option `--viewer-after <SECS>` (env `A2AMX_VIEWER_AFTER`, default 300, `0` turns the automatic switch off), beside `--prefix` (`src/cli.rs:17-26`); the parsed value is handed to `run_attachment` (callers `src/main.rs:761` new, `:1797` team up attach, `:2320` attach, reached from the dispatch at `:210`, `:218`, `:234`) and `AttachmentConfig` like `prefix`, so every attach gets the timer.

### 2.3 The attach client (`src/main.rs`)

`AttachmentState` gets `viewer: Viewer` and `viewer_after: Option<Duration>`.

- **Entering and leaving.** `Command::ViewerMode` calls `toggle`. The frames below are sent only while an attachment exists (after a failed switch `attachment` is `None`, `src/main.rs:2910-2934`; the next `attach_session`, also used by `restore_current`, reads the flag). Interactive to viewer: send `ClientFrame::Mode { viewer: true }`. Viewer to interactive: send `ClientFrame::Mode { viewer: false }`, then `ClientFrame::Resize` with the client's current size (`pty_rows(self.rows, self.status_visible, self.top_rows)`), then `ClientFrame::Redraw`. The automatic switch (section 2.3, timer) sends the first.
- **Input in viewer mode** (`handle_input`, `handle_prefix_actions`). Mouse routing first (`src/main.rs:2520-2540`, today a session with mouse reporting on and `top_rows == 0` sends reports through the prefix machine as plain keys, and Grid presses go to `send_session_mouse` when `session_mouse` is set): in viewer mode every read of `session_mouse` in the mouse routing (`:2521`, `:2525`, `:2800`, `:2818`, `:2827`) uses the effective flag `session_mouse && !viewer`. Reports are then parsed in every state, Grid presses and wheel events go to `handle_mouse_button`, `send_session_mouse` is unreachable, and releases go through `handle_mouse_report` as today (a team-line click acts on the release, `:2795-2811`, so releases must not be dropped). An incomplete (split) report at the end of a chunk is treated as keys and may show the overlay once (`// shortcut:` comment; revisit if it is seen in use). Then, in the normal state (no picker, no scroll mode): `Action::Forward` bytes are dropped, no `ClientFrame::Input` is sent, and `blocked(now)` draws the overlay; except that a focus report (a chunk equal to `\x1b[I` or `\x1b[O`, which sessions that enable focus reporting cause the outer terminal to send) is dropped without an overlay. In interactive mode a focus report is forwarded as today but does not count as own input. Prefix commands `Detach`, `SessionPicker`, `ScrollMode`, `TeamMember`, `TeamNext`, `TeamPrev` and `ViewerMode` behave as today. `StatusLine` toggles visibility as today (hiding the line hides the VIEW marker with it; accepted) but sends no `Resize`; when the team rows change it reattaches through `switch_session` as a viewer. `Release` is ignored without an overlay. The picker and scroll mode keep their own key handling unchanged.
- **Own input.** Every read from the terminal while interactive (keys, mouse, paste, prefix commands, picker and scroll keys) calls `own_input(now)`. Frames from the daemon do not.
- **Automatic switch.** `run_attachment_loop` (`src/main.rs:3072`) gets a one-second tick arm that calls `idle_expired(now, viewer_after)`; on `true` it does the interactive-to-viewer steps and draws the marker. The same tick calls `take_overlay_end`.
- **Session switch keeps the mode.** `attach_session` (`src/main.rs:2885`) attaches with the viewer flag of the current state (the 3dp entry point `Client::attach_viewer`, else `attach_status`); a viewer attach is not followed by a `Resize`.
- **Marker and overlay drawing.** After `draw_status` in the `ServerFrame::Data` and `ServerFrame::Status` arms, after the toggle and on the idle tick, the marker is drawn when `is_viewer`, the status line is visible, and neither the picker nor scroll mode is open (the same three-part guard as `draw_status`, `src/main.rs:2485`; pattern as `scroll_status`, `src/main.rs:3134-3136,3773`); the overlay is drawn there too while `overlay_active` under the same guard. When the overlay ends the client sends `ClientFrame::Redraw` (the daemon answers with a full repaint, which covers the box) and redraws the status line.
- **Resize.** A terminal resize in viewer mode redraws the status line, marker and overlay for the new size and sends no `Resize` frame.
- `// shortcut:` comment: a viewer shows the session at the daemon's size, which can differ from the terminal's; revisit if owners report clipped or padded views.

### 2.4 Documentation (`docs/cli.md`, `docs/remote-messaging.md`)

`docs/cli.md`: the attach keys list gains `<prefix> v` (toggle viewer mode), what viewer mode does and does not allow, the automatic switch, `--viewer-after` / `A2AMX_VIEWER_AFTER`, and that `daemon start` has `--owner-away-secs` (spec 3dp). `docs/remote-messaging.md`: a section "When you are at the computer" that states, from the code: presence is an interactive attach; the 60-second grace and the flag; `owner_present` refuses an agent's send; `list_agents` shows `owner`; the one away line; the owner can always write from the phone; a viewer is not presence. Every behaviour sentence is read from the code before it is written; the tester does a per-sentence truth check as for spec 3do.

## 3. Attack paths (trust-boundaries)

The feature is a safety control for the owner's own terminal. The abuses that matter are the ones that defeat the control or leak input.

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| keys, paste in viewer mode | a stray key or a paste reaches the session and types into it, or leaves viewer mode | removed: no `Input` frame is sent in viewer mode; no key leaves viewer mode | under the PTY and a fake daemon: in viewer mode type `x`, a bracketed paste and a split escape sequence; the fake daemon records no `Input` frame and no `Mode` frame after the one that entered viewer mode; focus reports `\x1b[I` and `\x1b[O` in viewer mode show no overlay |
| the automatic switch | the owner is made present again by a key after the switch | removed: only the toggle leaves viewer mode | after the automatic switch (option `--viewer-after 1`), type a key: overlay shown, no further `Mode` frame; `prefix v` sends `Mode { viewer: false }`, then `Resize`, then `Redraw` |
| mouse reports in viewer mode | a click or drag reaches a session that reports the mouse | removed: reports are dropped; only wheel scroll and team-line clicks act, and neither reaches the session | the fake daemon enables session mouse reporting (`\x1b[?1000h`) and the test runs with and without team rows (`top_rows` 0 and 2); a click and a move in viewer mode produce no `Input` frame and no overlay; a wheel event enters scroll mode |
| session switch in viewer mode | the new attach arrives as interactive, so the owner is present for a moment and the new session is resized | removed: the attach request carries `viewer: true` and no `Resize` follows | after `prefix n` in viewer mode the fake daemon's second `Attach` has `viewer: true` and no `Resize` frame |
| the overlay and marker | session output or an agent name injects escape sequences into the overlay | removed: both are fixed text built from constants and the prefix byte | the overlay and marker bytes equal literal expected vectors for given sizes and prefix bytes (pure test) |
| `--viewer-after` | a huge value overflows the timer | constrained: parsed as `u64` seconds; compared as `now.saturating_duration_since(last_own_input) >= after`, which cannot overflow | `--viewer-after 18446744073709551615` starts and never fires (bounded negative wait of at least two ticks); `--viewer-after 0` the same; a non-number is a usage error |

## 4. Claims (the developer re-reads each; the architect has read the starred rows)

| claim | where | verified |
|---|---|---|
| the attach loop selects over stdin input, SIGWINCH and daemon frames; the status line is redrawn after each `Data` and `Status` frame | `src/main.rs:3072-3158` | read* |
| `handle_input` routes mouse reports, picker keys, scroll keys and the prefix machine; `Action::Forward` becomes `ClientFrame::Input` | `src/main.rs:2517-2615` | read* |
| a session switch is `Detach` plus a new `attach_status` request | `src/main.rs:2885-2960` | read* |
| the scroll marker is drawn after `draw_status` with an absolute cursor move | `src/main.rs:3134-3136,3773-3778` | read* |
| `ClientFrame::Redraw` makes the daemon send a full render | `src/daemon.rs:3606-3640` | read* |
| `v` after the prefix is not a command today | `src/prefix.rs:128-162` | read* |
| `--prefix` is a global option with an env var and a default | `src/cli.rs:17-26` | read* |
| the existing fake daemon decodes client frames but records almost nothing (it acts on `Detach` only, keeps counters and flags, and does not answer `Redraw`); this spec extends it in `tests/attach_cli.rs` (in scope): a frame log, the `Attach.viewer` value (its pattern at `:181` has `..`), and a `Redraw` reply that clears the screen (`\x1b[2J`) and repaints | `tests/attach_cli.rs:168-266` | read* |
| tests that enumerate `Command`: `tests/prop_prefix.rs:42` (spliced command selection) and the command table test at `:191-215` gain `v`; `tests/prefix.rs:91-95,113` use other bytes and do not break | `tests/prop_prefix.rs`, `tests/prefix.rs` | the consultant read; the developer re-greps |
| `src/lib.rs` needs one `pub mod` line; `src/main.rs` uses `a2amx::` paths | `src/lib.rs`, `src/main.rs:17-40` | read by the consultant |
| no test or script checks docs links; the link check is manual (the tester) | grep over `tests`, `scripts`, `.github` | consultant grep |
| docs list the attach keys in one place | `docs/cli.md` | the developer finds the section and lists the other docs that name the keys (`rg -n "prefix" docs/*.md`) |

## 5. Out of scope

Presence and the daemon side (spec 3dp), a CLI flag to start in viewer mode, a hint in the terminal title, resizing a viewer to the terminal, per-session viewer state (the mode belongs to the client), the CHANGELOG (the architect), a failed session switch that leaves the client in the picker with no attachment (the owner then reads as away after the grace while at the keyboard; accepted, noted for the owner), and the live Discord test.

## 6. Acceptance

Commands run from the repository root with `env -u A2AMX_BIN -u NO_COLOR`; every focused run uses `timeout 600`.

1. `cargo test --test viewer`: the pure module with injected times: `idle_expired` fires once after the period and never when `after` is `None`; `own_input` resets the clock; agent time passing does not; `toggle` round-trips; `blocked` sets and extends the two seconds; `take_overlay_end` fires once; `overlay` and `marker` equal literal vectors for fixed sizes and for prefix bytes 0x02, 0x00 and 0x1d, and are empty on a too-small terminal; the marker bytes contain the bright colour code and no blink attribute (`\x1b[5m`, `\x1b[6m`).
2. `cargo test --test prefix --test prop_prefix`: `prefix v` yields `Command::ViewerMode`, also across split reads and not inside a bracketed paste.
3. `cargo test --test attach_cli` (new tests, fake daemon records `Attach.viewer` and frames): every row of section 3 as written; plus the toggle round trip, the idle switch with `--viewer-after 1`, the picker still opens and a switch keeps viewer mode, the overlay text shows the real prefix, the overlay disappears after about two seconds (the screen text no longer contains `VIEW MODE`) and the fake daemon saw a `Redraw`, mouse and prefix commands show no overlay, `--viewer-after 0` never switches.
4. Docs truth check by the tester (spec 3do process): per-sentence table for the new `docs/cli.md` and `docs/remote-messaging.md` text; links resolve; `git status --porcelain` shows only scope files.
5. The full suite `env -u A2AMX_BIN -u NO_COLOR cargo test` passes; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny --locked check`, `scripts/prepush-fast.sh` pass.
6. Mutation check (each under `timeout 120`, restored by checksum, in the assigned checkout): `Forward` bytes sent as `Input` in viewer mode; any key leaves viewer mode; `own_input` counts daemon frames; the idle switch ignores `after = None`; the overlay shown for mouse reports; the overlay not repainted (`Redraw` not sent) when it ends; the viewer attach flag dropped on a session switch; a `Resize` sent in viewer mode; the marker drawn with the blink attribute; `Release` forwarded in viewer mode; the prefix shown as a fixed `Ctrl-B`; a focus report counted as own input or shown as an overlay; mouse reports with `top_rows == 0` not parsed in viewer mode; `StatusLine` sending `Resize` in viewer mode; the marker drawn on the status row.
7. `git status --porcelain` shows only scope files; `git diff --check` is clean.
