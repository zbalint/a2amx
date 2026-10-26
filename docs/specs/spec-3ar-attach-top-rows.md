# Spec 3ar: Row origin for client-reserved top rows, and the mouse-report row shift

## 0. Status

**LOCKED** (2026-10-26; pre-lock gate run against develop at `ff40b3f` plus the backlog commit `ab3cf59` on
`feature/team-line`; consultant review `m_3989`, findings adopted).
Architect-directed, backlog F29 part 1 of 2, context `a2amx-team-line-2026-10-26`. Implementation goes to
`a2amx-developer` only. Part 2 (the team line itself) is a separate spec and builds on this one.

**Baseline (run 2026-10-26):** `env -u A2AMX_BIN -u NO_COLOR cargo test` exits 0 (37 test binaries, 606 passed, 0
failed); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass with no output.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `feature/team-line`.
**Public test seams:** `Emulator::render_full` / `render_update` output bytes (`tests/emulator_render.rs`), the raw
attach exchange against a real daemon (`tests/daemon.rs`), and the pure mouse functions of section 3.4 (new
`tests/mouse.rs`). No test reaches a private function.

**Scope.** May edit exactly these paths and no others.

- `src/wire.rs` (section 3.1)
- `src/emulator.rs` (section 3.2)
- `src/daemon.rs` (section 3.3: the `Request::Attach` arm, `attach`, the response sent)
- `src/client.rs` (section 3.1: one struct literal gains `top_rows: 0`)
- new `src/mouse.rs` and its `pub mod` line in `src/lib.rs` (section 3.4)
- `tests/emulator_render.rs`, `tests/daemon.rs`, `tests/wire.rs`, new `tests/mouse.rs`
- `tests/attach_cli.rs` and `tests/broker.rs`: only `top_rows: 0` added to the existing `Request::Attach` literals
  (`tests/attach_cli.rs:1267`, `tests/broker.rs:262`)
- `docs/architecture.md`, the "Status line" subsection only (section 3.5)

Does not touch: `src/status.rs`, `src/main.rs` (the attach client does not send the field yet, so no visible
behavior changes), `src/session.rs`, `src/messaging.rs` (`sgr_mouse_report_len` stays where it is), `AGENTS.md`
(the new module row is the architect's, section 3.5), `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`. No new
dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

The owner wants a team line on row 1 of the attach screen (backlog F29). The bottom status bar needed no daemon
change because the session's PTY is smaller and the bar sits below the session's last row. A row **above** the
session does not work that way: the daemon renders the session from the emulator with absolute cursor positions
(`append_cursor_position` writes `row + 1`) and a full render begins with `ESC[2J ESC[H`, so a client that
reserves row 1 would have the session draw over it. Mouse reports the client passes to a session that uses the
mouse carry rows counted from the real top of the terminal, which would be one row too low for the session's
grid.

This spec adds the groundwork and nothing visible: an additive attach field that makes the daemon render the
session with a row origin, an acknowledgement so a new client never reserves a row against an old daemon, and
pure functions that shift or claim mouse reports. The attach client does not send the field until spec 3as.

## 2. Design

- **Attach field.** `Request::Attach` gets `top_rows: u16` (default 0, omitted when 0). The value is the number of
  terminal rows the client keeps above the session. The request's `rows` stays the PTY height (client rows minus
  every reserved row), as it is today for the bar.
- **Acknowledgement.** A daemon that honors a nonzero `top_rows` answers with the new `Response::AttachedTop {
  top_rows }` instead of `Response::Attached`. An old daemon ignores the unknown request field and answers
  `Attached`; the client then knows the rows were not honored and must not draw into them. A request with
  `top_rows == 0` is answered with `Attached` exactly as today, so old clients and the existing tests are
  unaffected.
- **Row origin.** The emulator renders with an origin: every absolute cursor row gets `+ origin`, and the full
  render replaces `ESC[2J ESC[H` with `ESC[<origin+1>;1H ESC[J`. The origin is set at attach, under the session
  lock, before `render_full`, and is `0` for every attachment that did not ask for rows. `snapshot()` always
  renders with origin `0`, because its contract is to reproduce `screen()` in a fresh emulator.
- **Mouse functions** (pure, `src/mouse.rs`): find a mouse report in SGR or legacy X10 encoding, and given a row
  origin, classify it as on one of the reserved rows (claimed, with its column and row) or in the session grid
  (rewritten with the row reduced by the origin).

Rejected: a client-side clip of the daemon's output (the client would need its own terminal model); a separate
frame type for the team line (does not fix the session drawing at row 1); sending the row count in the status
frame (too late: the first render has already been drawn).

## 3. Changes

### 3.1 `src/wire.rs`, `src/client.rs`

- `Request::Attach`: add `#[serde(default, skip_serializing_if = "is_default")] top_rows: u16,` after `status`
  (`src/wire.rs:182`). `is_default` is at `src/wire.rs:391`.
- `Response`: add `AttachedTop { top_rows: u16 }` next to `Attached` (`src/wire.rs:277`).
- Every `Request::Attach` struct literal gains `top_rows: 0`: `src/client.rs:146`, the destructuring arm
  `src/daemon.rs:1926` (which instead binds `top_rows`), `tests/daemon.rs:693,703,713,1592,1634`,
  `tests/wire.rs:140,802,818`, `tests/attach_cli.rs:1267`, `tests/broker.rs:262`. A client method that sends a
  nonzero value is spec 3as, where its caller and test exist; this spec adds none.

### 3.2 `src/emulator.rs`

- Field `row_origin: u16` on `Emulator`, default 0; `pub fn set_row_origin(&mut self, rows: u16)`.
- `render_screen(&self, include_alt_screen, origin: usize)`: the `ESC[2J ESC[H` at `src/emulator.rs:466` becomes
  `ESC[2J ESC[H` when `origin == 0` (byte-identical to today) and otherwise `ESC[<origin+1>;1H ESC[J`.
  `snapshot` passes `0`; `render_full` passes `usize::from(self.row_origin)`.
- `append_cursor_position` (`src/emulator.rs:737`) and `append_cursor` (`src/emulator.rs:728`) take the origin and
  write `row + origin + 1`. Their callers are `render_line` (lines 476, 505), `render_screen` (470) and
  `render_update` (334); thread the origin through `render_line` the same way.

### 3.3 `src/daemon.rs`

- The `Request::Attach` arm (`src/daemon.rs:1926`) passes `top_rows` to `attach`.
- `attach` answers an error for `top_rows > 8` (a bound on a caller-controlled count; the client needs 1 or 2).
  Inside the locked block that calls
  `resize_locked` and `render_full` (`src/daemon.rs:2517-2521`), call `set_row_origin(top_rows)` first.
- The response after a successful attach (`src/daemon.rs:2545`) is `AttachedTop { top_rows }` when
  `top_rows > 0`, else `Attached`. The bridge path (`src/daemon.rs:2200`) is unchanged.
- The frame loop's `Resize` handling keeps the origin (it is per attachment, not per resize).

### 3.4 `src/mouse.rs` (new)

Pure functions, no I/O:

- `parse_report(bytes: &[u8], start: usize) -> Option<MouseReport>` recognizes `ESC[<b;x;yM|m` (SGR) and `ESC[M`
  plus three bytes (X10) at `start`. It checks the `ESC [ <` prefix itself: `sgr_mouse_report_len`
  (`src/messaging.rs:528`) does not (it assumes `start` is at `ESC`, skips three bytes, and returns the absolute
  end index, not a length), and `find_mouse_report` (`src/main.rs:1946`) checks the prefix before calling it.
  `MouseReport` carries the encoding, the button value (with the motion bit 32 and wheel values 64/65), whether it
  is a release (SGR final `m`; X10 button value 3), the 1-based column and row, and the end index. Names are the
  developer's. In X10 every byte after `ESC[M` is at least 32, so coordinates reach 223; a byte below 32 means
  `None`.
- `pub fn shift_row(report: &MouseReport, origin: u16) -> Shifted`, where `Shifted` is `Claimed(MouseReport)`
  when `origin >= 1` and `row <= origin` (a click on a reserved row, including a malformed row 0), else
  `Grid(Vec<u8>)`: the report re-encoded with the row reduced by `origin`, the button and column bytes verbatim.
  With `origin == 0` the result is always `Grid` with the original bytes.
- Worked examples (also test literals), origin 1: SGR `ESC[<0;5;1M` is `Claimed` with column 5, row 1;
  `ESC[<0;5;2M` becomes `ESC[<0;5;1M`; `ESC[<64;10;24m` becomes `ESC[<64;10;23m`; `ESC[<0;5;0M` is `Claimed`;
  X10 `ESC[M` `0x20 0x25 0x22` (button 0, column 5, row 2) becomes `ESC[M` `0x20 0x25 0x21`; X10 row 1
  (`0x21`) is `Claimed`. Origin 0: `ESC[<0;5;0M` is `Grid` and unchanged.

### 3.5 `docs/architecture.md`

In "Status line" add a short paragraph: the attach request's `top_rows`, the `AttachedTop` acknowledgement, the
origin rule, and that the session grid's rows are shifted while the mouse functions shift reports. No other doc
edits. The architect adds the `mouse` row to the `AGENTS.md` module table when committing.

## 4. Out of scope

The team line, `StatusInfo.members`, click handling, prefix keys, `pty_rows` changes, a client method that sends a
nonzero `top_rows` (and the Resize it must send after a plain `Attached` reply), and any change to what the attach
client sends: spec 3as. Moving `sgr_mouse_report_len`. Urxvt (1015) and X10-extended (1005) mouse
encodings. Rewriting reports for the bottom bar row (pre-existing). Any `unwrap`/`expect` outside tests.

## 5. Acceptance

Run from the main checkout. `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`
and `cargo fmt --check` all pass, and `scripts/prepush-fast.sh` exits 0 (judge: the commands' exit codes, read by
the architect at final review).

1. **Default origin is byte-identical.** The existing `tests/emulator_render.rs` and
   `tests/emulator_roundtrip.rs` pass unmodified, and a new origin-0 test pins the old bytes literally: an
   emulator of `Size { cols: 3, rows: 2 }` fed `AB` renders (`render_full`, up to and including the cursor-show)
   `ESC[2J ESC[H ESC[1;1H ESC[0m AB ESC[0m ESC[K ESC[2;1H ESC[0m ESC[0m ESC[K ESC[1;3H ESC[?25h` (contract test,
   passes on the old code; the mode-reset tail that follows is not asserted).
2. **Origin shifts every position.** A new test sets origin 1 on the same emulator, feeds `AB`, and asserts that
   `render_full` starts with `ESC[2;1H ESC[J ESC[2;1H ESC[0m AB ESC[0m ESC[K ESC[3;1H ESC[0m ESC[0m ESC[K
   ESC[2;3H ESC[?25h` (fails on the old code). A `render_update` test with origin 1 on `Size { cols: 4, rows: 2 }`
   fed `AB` then, after a first render, `C`, asserts the update bytes put the line at `ESC[2;1H` and the cursor
   at `ESC[2;4H` (a width of 4 keeps `C` off the pending-wrap edge). The developer reads the exact update bytes
   from the real output once and states them as literals; they must show the origin `+1` and nothing else
   changed relative to the origin-0 render of the same input.
3. **Wire.** `tests/wire.rs`: `top_rows` absent decodes to 0; 0 is omitted when encoding; `AttachedTop` round
   trips; an old-shape `Attach` JSON decodes. (`Attached` still decodes: contract test.)
4. **Daemon.** `tests/daemon.rs`: against a real daemon, an attach with `top_rows: 1` is answered `AttachedTop {
   top_rows: 1 }` and the first data frame begins `ESC[2J`-free with `ESC[2;1H ESC[J`; an attach with `top_rows:
   0` or without the field is answered `Attached` and begins with `ESC[2J ESC[H` (contract test); `top_rows: 9` is
   an error response; a second attach without rows after one with rows renders with origin 0 (the origin does not
   stick).
5. **Mouse.** `tests/mouse.rs` pins the worked examples of section 3.4 literally, including a report split
   across the slice end (returns `None`, never panics), an X10 byte below `32` (`None`), `origin == 0` (always
   `Grid`, unchanged, including row 0), a row field of `0` with origin 1 (`Claimed`), a release (`m`) and a
   motion/wheel button surviving the shift verbatim, and a `start` that is not at `ESC` (`None`).
6. **Mutation check** (tests are part of the deliverable): on a scratch copy of the final snapshot
   (`git archive` then `git init` in the copy), the developer breaks each of: the `+ origin` in
   `append_cursor_position`; the clear sequence for origin > 0; the `origin == 0` branch; `top_rows` omission in
   the encode; the `AttachedTop` choice in the daemon; the `row <= origin` comparison; the X10 `- 32` offset,
   names the failing test for each, and restores. Its checkout must match its own diff afterward.
7. **Scope.** `git status --short` lists only the paths of section 0 (judge: the architect).

## 5a. Claims about existing code (pre-lock gate, step 9)

| Claim | Where | Verified how |
| --- | --- | --- |
| A full render starts `ESC[2J ESC[H` and positions every line and the cursor through `append_cursor_position` | `src/emulator.rs:466`, `476`, `505`, `728-742` | read |
| `render_line` and `append_cursor` are called only from `render_screen` (468, 470) and `render_update` (331, 334) | `src/emulator.rs` | `rg "render_line\(|append_cursor\("` |
| `snapshot()` has no caller outside the emulator and tests | `tests/emulator_roundtrip.rs:16,91`, `tests/emulator_render.rs:301` | `rg "snapshot\(\)"` |
| The daemon renders `render_full` at attach inside the session lock, then sends frames from `render_update`; `Resize`, `Scroll`, `Redraw` re-render with `render_full` | `src/daemon.rs:2517-2521`, `2597`, `2632` | read |
| The unit `Response::Attached` is built at `src/daemon.rs:2200` (bridge) and `2545`, matched at `src/client.rs:155,166`, `tests/daemon.rs:1605`, `tests/attach_cli.rs:1274` | listed | `rg "Attached"` |
| The attach client skips mouse parsing when `session_mouse` is true and then feeds bytes to the prefix machine | `src/main.rs:1626-1631`, `1651-1657` | read |
| `sgr_mouse_report_len(bytes, start)` expects `start` at `ESC` and returns the report end | `src/messaging.rs:528-557` | read |
| The wire enums are `serde(tag = "type")` and `is_default` exists | `src/wire.rs:139,255,263,391` | read |
| Old daemons ignore an unknown request field (no `deny_unknown_fields`) | `src/wire.rs` | `rg deny_unknown_fields` found none (not probed against a running old daemon) |
| Every `Request::Attach` struct literal: `src/client.rs:146`, `src/daemon.rs:1926`, `tests/daemon.rs:693,703,713,1592,1634`, `tests/wire.rs:140,802,818`, `tests/attach_cli.rs:1267`, `tests/broker.rs:262` | listed | consultant `rg` (review pass 1) |
| `render_update` falls back to `render_full` when nothing was rendered, and `append_cursor` calls `append_cursor_position` | `src/emulator.rs:309`, `729` | read (consultant) |
| `sgr_mouse_report_len` does not check the `ESC [ <` prefix and returns the end index | `src/messaging.rs:528-557`, `src/main.rs:1946-1958` | read |
| `ESC[J` from the origin row erases the separator and bar rows exactly as `ESC[2J` did, and the client redraws them after every Data frame | `src/main.rs:2031,2033` | read (consultant) |
| No other sequence in the emulator's output is position-dependent (no scroll region, no relative motion) | `src/emulator.rs:464-808` | `rg` of the escapes (consultant) |
| A new `Response` variant breaks no decoder in this repo: every decoder is this binary and has a wildcard arm | `src/client.rs:157,168` | read (consultant) |

Not probed: serde's behavior when an old binary receives an extra key (irrelevant here: only a client that sent
`top_rows` can receive `AttachedTop`), and alacritty's default modes in the mode-reset tail (the literals in
acceptance 1 and 2 stop at the cursor-show for this reason).

## 6. Review record

Not yet reviewed.
