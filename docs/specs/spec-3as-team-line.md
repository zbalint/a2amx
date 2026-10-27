# Spec 3as: The team line at the top of the attach screen

## 0. Status

**LOCKED** (2026-10-27; pre-lock gate run against `feature/team-line` at `5afb49f`, which holds spec 3ar; consultant
review `m_4036`, findings adopted).
Architect-directed, backlog F29 part 2 of 2, context `a2amx-team-line-2026-10-26`. Implementation goes to
`a2amx-developer` only.

**Baseline (run 2026-10-27 on `5afb49f`):** `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --check`; the suite passed at `5afb49f` (38 binaries, 616 passed, 0 failed, 3 ignored).
Known flaky test: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (backlog I14); a failure
is rerun once and recorded as pre-existing if it passes.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `feature/team-line`.
**Public test seams:** `status::render_team_line` (pure, `tests/status.rs`); the wire types (`tests/wire.rs`); a
real daemon's status frame (`tests/daemon.rs`); `Command` from the prefix machine (`tests/prefix.rs`); the
attach CLI in a PTY (`tests/attach_cli.rs`). No test reaches a private function.

**Scope.** May edit exactly these paths and no others.

- `src/wire.rs` (3.1), `src/daemon.rs` (3.2), `src/status.rs` (3.3), `src/client.rs` (3.4),
  `src/prefix.rs` (3.5), `src/main.rs` (3.6), `src/mouse.rs` (3.6a)
- `tests/wire.rs`, `tests/daemon.rs`, `tests/status.rs`, `tests/prefix.rs`, `tests/prop_prefix.rs`,
  `tests/attach_cli.rs`, `tests/mouse.rs`
- `tests/daemon.rs` and `tests/status.rs` also hold the existing `attach_status(..)` and `StatusInfo { .. }`
  literals that gain the new argument or field (`StatusInfo` has no `Default`; every literal gets
  `members: vec![]`)
- `docs/architecture.md` ("Status line" subsection only) and `docs/cli.md` (the prefix-key prose near line 54 and the sentence at line 63 that says `s` hides the status line and separator)

Does not touch: `src/emulator.rs`, `src/picker.rs`, `src/session.rs`,
`src/messaging.rs`, `AGENTS.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `docs/backlog.md`. No new dependency.
Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

Owner request (backlog F29): with teams the user checks on different agents from time to time, so a row at the top of
the attach screen lists the team's members, each with an activity dot, and a click or a key switches to that
member. Spec 3ar added the groundwork (a row origin in the daemon's render, the `AttachedTop` acknowledgement and
the pure mouse functions); nothing sends `top_rows` yet. This spec builds the visible line on it.

## 2. Design (owner decisions, 2026-10-26)

- The line is the top terminal row, drawn in the status bar's style (same background, bold name, dot colors).
  It is shown only when the attached session has a team, the status bar is visible, and the terminal has at least 5
  rows. It has no separator of its own.
- Content: ` team <name> │ ● alpha  ● beta  ...`. Each member is a dot (idle green 78, working yellow 221, busy
  orange 209; no activity or exited grey 244) and the member's name; the attached session's name is bold.
- A left click (press and release) on a member switches to it as the session picker does (detach, attach);
  clicking the attached session does nothing; clicking the team label opens the picker. Prefix then `1` to `9`
  selects the Nth member, `n` and `p` the next and previous.
  Exited members are shown grey and are never selected.
- Prefix then `s` hides the line together with the bar.

Rejected: a client-side clip of the daemon's output and a second bottom row (owner declined; spec 3ar).

## 3. Changes

### 3.1 `src/wire.rs`

- New `pub struct TeamMember { pub id: String, pub name: String, pub activity: Option<Activity>, pub exited: bool }`
  (`Debug, Clone, PartialEq, Eq, Serialize, Deserialize`).
- `StatusInfo` (`src/wire.rs:441`): add `#[serde(default, skip_serializing_if = "Vec::is_empty")] pub members:
  Vec<TeamMember>`. A frame without the key decodes to an empty list; an empty list is omitted when encoding.

### 3.2 `src/daemon.rs`

- `attachment_status` (`src/daemon.rs:2446`) fills `members`: when the attached session has a team, every session in
  the runtime whose team name equals the attached session's team name (the rule the picker groups by; the private
  flag is not compared), sorted by `name` then `id`, each as `TeamMember { id, name (the session name, or the id when
  it has none), activity: session_activity(..), exited: exit_code.is_some() }`. A session without a team gets an
  empty list. Clone the matching `Arc<Session>` list inside the `runtime.sessions` lock block (a std mutex, no `.await` held) and compute activity, name and exit state after the lock is released.
- The change-only comparison of the 1 Hz status frame already compares the whole `StatusInfo`, so a member's activity
  change sends a frame; nothing else changes in the loop.

### 3.3 `src/status.rs`

New pure function and types:

```rust
pub struct TeamRegion { pub first_col: u16, pub last_col: u16, pub target: TeamTarget }
pub enum TeamTarget { Session(String), Picker }
pub struct TeamLine { pub bytes: Vec<u8>, pub regions: Vec<TeamRegion> }
pub fn render_team_line(current_id: &str, team: &str, members: &[TeamMember], cols: u16) -> TeamLine
```

- Returns an empty `TeamLine` when `cols < 2`. Otherwise the bytes draw terminal row 1: `ESC7`, `ESC[1;1H ESC[0m
  ESC[2K`, the bar background, the content padded with spaces to `cols - 1` characters, `ESC[0m ESC8` (save and
  restore the cursor, as the bar does). Spans reuse `Span`, `render_spans`, `sanitize` and the color helpers of
  this file; add the grey 244 for a member with no activity or an exited member.
- Content: `" team <name> "` (sanitized), then the detail prefix `│ `, then for each member `● <name>` followed by two
  spaces. A member name or the team name longer than 16 characters is cut to 15 plus `…`. If the content plus
  ` +N ` would exceed `cols - 1`, drop members from the end, never the one whose `id` is `current_id`, until content
  and ` +N ` (N dropped, counted in the width) fit; with nothing dropped there is no ` +N `. If the label and the
  current member alone do not fit, cut the content at `cols - 1` characters the way `render_spans(.., limit)` does,
  cut the regions to that width and omit a region that starts beyond it.
- An empty `members` list (the status frame has not arrived, or the session has no team) gives a cleared row: the
  same `ESC7 .. ESC8` frame with the bar background, `cols - 1` spaces and no regions.
- `regions` use 1-based inclusive columns: the label region (the leading space through the `│ `) is `Picker`; each
  member's region covers its dot, name and the two trailing spaces and is `Session(id)`; the ` +N ` region is
  `Picker`; the padding has none.
- Worked example (pins the layout; `cols = 40`, team `dev`, members `a` `alpha` idle and `b` `beta` working,
  current `a`): the content before the padding is 29 characters (` team dev ` 10, `│ ` 2, `● alpha  ` 9, `● beta  ` 8)
  and the padding is 10 spaces. Expected bytes as one literal (no spaces between escapes):
  `b"\x1b7\x1b[1;1H\x1b[0m\x1b[2K\x1b[38;5;252;48;5;236m team dev \x1b[38;5;240m\xe2\x94\x82\x1b[38;5;252m \x1b[38;5;78m\xe2\x97\x8f\x1b[38;5;252m \x1b[1malpha\x1b[22m  \x1b[38;5;221m\xe2\x97\x8f\x1b[38;5;252m beta  "`
  followed by ten spaces and `b"\x1b[0m\x1b8"`. Regions: `(1, 12, Picker)`,
  `(13, 21, Session("a"))`, `(22, 29, Session("b"))`.

### 3.4 `src/client.rs`

- `Client::attach_status` gains a `top_rows: u16` parameter after `rows` and sends it in the request
  (`attach` keeps sending 0). A reply of `AttachedTop { top_rows }` stores that value on the `Attachment`; a plain
  `Attached` stores 0. `Attachment` gains `pub fn top_rows(&self) -> u16`. The four callers in `src/main.rs`
  (`1523`, `1822`) and the two in `tests/daemon.rs` (`551`, `2103`) pass the new argument.

### 3.5 `src/prefix.rs`

`Command` gains `TeamMember(u8)` (prefix then `1` to `9`, carrying 1 to 9), `TeamNext` (`n`) and `TeamPrev` (`p`).
Every other byte after the prefix keeps its current effect (dropped).

### 3.6 `src/main.rs`

- **Top rows.** `pty_rows(rows, visible)` (`src/main.rs:1549`) gains a `top: u16` argument and returns the old value
  minus `top`, never below 1. A new `fn team_rows(rows: u16, visible: bool, in_team: bool) -> u16` is 1 when all of
  `visible`, `in_team` and `rows >= 5` hold, else 0. Whether a session is in a team comes from the session list
  (`request_sessions`, the same request `resolve_session` makes): a new helper returns the target's `team.is_some()`
  and propagates a list failure as the attach error. It runs at the initial attach (`run_attachment`) and in
  `attach_session`.
- **Signatures.** `attach_session(&self, session: &str) -> anyhow::Result<(Attachment, u16)>` looks up the team,
  attaches with `team_rows(..)` and returns the `top_rows` the daemon acknowledged; `switch_session(&mut self,
  selected: String)` stores it in `self.top_rows`. The toggle and the restore path use the same function.
- **Old daemon.** After every successful attach (the first one in `run_attachment`, and `attach_session` for a
  switch, toggle or restore), when 1 was requested and the acknowledged value is 0, send `ClientFrame::Resize {
  cols, rows: pty_rows(rows, visible, 0) }` at once and never draw the line.
- **State.** `AttachmentState` gains `top_rows: u16`, `team_regions: Vec<TeamRegion>` and the press-tracking of the
  next bullet. `draw_status` also draws the team line when `top_rows == 1`, no picker and no scroll mode: from
  `self.status.members` and `self.status.team` (with `status` still `None`, it draws the cleared row of 3.3), keeps the returned regions and sends the bytes together with the bar's. A resize keeps the
  attached `top_rows`; the PTY rows use `pty_rows(.., top_rows)`.
  // shortcut: top_rows is decided at attach and on a switch or toggle; a resize below 5 rows keeps it until then.
- **Mouse.** `find_mouse_report` (`src/main.rs:1946`) is extended to X10 by trying `mouse::parse_report` at each
  `ESC` (no second scanner). When `top_rows > 0`, no picker is open and scroll mode is off, reports are found with
  it whatever `session_mouse` is, and each is classified with `mouse::shift_row(report, top_rows)`. With the
  picker open or in scroll mode the current handling stays: the picker drops reports (`handle_mouse_button`
  returns early) and scroll mode runs the wheel logic on `Grid` reports; `Claimed` never acts then. `Claimed` is never passed to the session: a left press (button 0, no motion bit) on the line is
  remembered; its release (SGR `m`, or X10 button value 3) looks up `team_regions` by column and acts (`Session(id)`
  switches unless it is the attached session or the member is exited; `Picker` opens the picker); drag and wheel
  reports on the line are dropped. `Grid(bytes)` is handled as today (with `session_mouse` the bytes go to the
  session, otherwise the wheel logic runs) using the shifted bytes. A release that lands on the line is, when
  `session_mouse` is set and the matching press was a grid press, passed to the session with its row set to
  terminal row `top_rows + 1` (session row 1) through `mouse::with_row` (3.6a); without it the session would never
  see the button go up. The remembered press is cleared on any release and on a switch. With `top_rows == 0` the
  existing path is unchanged.
  // shortcut: a report split across input chunks is typed input (the existing shortcut); with `top_rows > 0` and
  `session_mouse` it reaches the session unshifted.
- **Keys.** `Command::TeamMember(n)`, `TeamNext` and `TeamPrev` select from `self.status.members` (list order); `1` to `9` index
  `members` in list order including exited ones (an exited one does nothing); `n` and `p` cycle over the
  non-exited members starting from the attached one, wrapping. With `top_rows == 0` or no
  such member they do nothing. A selection of the attached session does nothing.
- **Switching.** Selection calls the existing `switch_session(id)`. `switch_session` sends `Detach` and then waits for the
  connection to close: it reads frames until `recv` returns `None` or 5 seconds pass (today it waits for one frame
  of any kind, but the daemon never answers a `Detach`, it only drops the attachment and the connection, so a
  re-attach of the same session with `force=false` can race into "attached elsewhere"). When the attach of the
  target fails, re-attach the current session through `attach_session` (it, too, only after the connection has
  closed) and, if that fails too, end with the error. A failed switch leaves `attachment` empty until then.
  // shortcut: the failed switch is not shown; add a bar notice if asked.
- **Structure.** Keep `handle_input` and `handle_prefix_actions` thin: new methods `handle_mouse_report`,
  `team_click`, `select_member` and `toggle_status_line` carry the logic, so neither function grows a nested
  branch (the Sonar S3776 limit is 30; spec 3y cut `handle_input` below it).
- **Hide.** `Command::StatusLine` toggles `status_visible`. When `team_rows(..)` for the new state differs from
  `top_rows`, call `switch_session` on the attached session (the row origin is an attach property) with the new
  top rows; otherwise send the `Resize` as today.

### 3.6a `src/mouse.rs`

Add `pub fn with_row(report: &MouseReport, row: u16) -> Vec<u8>`: the report re-encoded with its row set to `row`
(1-based; X10 as `row + 32`). `shift_row`'s rewrite calls it, so there is one encoder.

### 3.7 Docs

`docs/architecture.md`, "Status line": one paragraph for the line, `members` on the status frame, the click and key
behavior. `docs/cli.md`: the prefix-key prose gains `1`-`9`, `n`, `p`, and the sentence about `s` names the team line.

## 4. Out of scope

A notice for a failed switch; a configurable member order; a mouse-wheel on the line; keyboard focus on the line;
ambiguous-width and wide glyph handling (the existing status-bar shortcut applies to the regions too); showing the line for sessions without a
team; changing the picker; the `a2amx update` work (F28); a CHANGELOG line (the architect adds it).

## 5. Acceptance

`env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` and
`scripts/prepush-fast.sh` all pass (judge: exit codes read by the architect).

1. **Wire.** `tests/wire.rs`: `members` absent decodes to empty; empty is omitted when encoding; a `StatusInfo` with
   two members round-trips (new tests; the absent-key one is a contract test).
2. **Daemon.** `tests/daemon.rs`: a real daemon with two sessions of one team and one without: an attach with the
   status opt-in on a team session gets a `Status` frame whose `members` lists both team members sorted by name,
   with the activity and `exited: false`; a member whose child exited by itself shows `exited: true` (a killed session is removed from the runtime, so it is absent); a session without a team
   gets an empty list.
3. **Line.** `tests/status.rs`: the worked example of 3.3 byte for byte and its regions; a 16+ character name is
   cut to 15 plus `…`; an overflow drops members from the end but keeps the current one and shows ` +N `; `cols < 2`
   gives an empty line.
4. **Keys.** `tests/prefix.rs`: prefix then `1` and `9` give `TeamMember(1)` and `TeamMember(9)`, `n` and `p`
   `TeamNext` and `TeamPrev`; `0` and other bytes still drop. `tests/prop_prefix.rs` (alphabet at lines 19-31, exact
   command table at 185-200) gains `1`, `9`, `n`, `p` in the table.
5. **Attach.** `tests/attach_cli.rs`, in a PTY: with a two-member team the first screen shows the line on row 1
   and the session output below it; a left click (press then release, SGR) on the other member's columns switches
   to it (the screen text shows its address in the bar); a press and release on the attached member does nothing;
   prefix then `2` switches; prefix then `s` removes the line (row 1 is empty of team text) and prefix then `s`
   again restores it; a session without a team shows session output on row 1 and no team text.
6. **Mutation check** on a scratch copy of the final snapshot (`git archive` then `git init`): break the
   dropped-member rule (drop the current one), the `team_rows` threshold (`>= 5` to `>= 4`), the region column
   arithmetic, the exited-member guard and the clamped release; name the failing test for each, restore. Its
   checkout must match its own diff afterward.
7. **Mouse.** `tests/mouse.rs`: `with_row` on `ESC[<0;5;1m` with row 2 is `ESC[<0;5;2m`; on X10 `ESC[M` `0x23 0x25 0x21`
   with row 2 is `ESC[M` `0x23 0x25 0x22`; the existing shift tests still pass (contract tests).
8. **Scope.** `git status --short` lists only the paths of section 0 (judge: the architect).

## 5a. Claims about existing code (pre-lock gate, step 9)

| Claim | Where | Verified how |
| --- | --- | --- |
| `StatusInfo` is compared whole for the change-only frame | `src/daemon.rs:2606-2608` | read |
| The daemon sends a `Status` frame at attach when the client opted in | `src/daemon.rs:2573-2576` | read |
| `switch_session` sends `Detach`, waits for one frame of any kind (5 s cap), attaches the target, sets `status = None` on success only; the daemon never answers `Detach` (`ClientFrame::Detach => return Ok(true)`) | `src/main.rs:1853-1875`, `src/daemon.rs:2631` | read (consultant m_4036; daemon line re-read) |
| A killed session is removed from `runtime.sessions` | `src/daemon.rs:2150-2154` | read (consultant) |
| `MouseReport.bytes` and `row_start` are private; `mouse.rs` has `parse_report` and `shift_row` only | `src/mouse.rs:19-20` | read |
| The attach client draws the bar after every `Data` and `Status` frame | `src/main.rs:2024-2038` | read |
| `Attach` callers: `src/main.rs:1523,1822`, `tests/daemon.rs:551,2103` | listed | `rg "attach_status\("` |
| `StatusInfo { .. }` literals: `src/daemon.rs:2451`, `tests/status.rs:10`, `tests/wire.rs:768` | listed | `rg "StatusInfo \{"` |
| The picker groups by the team's name string | `src/picker.rs:408` | read (not the private flag) |
| Existing prefix tests drop unknown bytes | `tests/prefix.rs:98-112` | read; `tests/prop_prefix.rs:19-45` has a command alphabet `d w [ s r` and its model must learn `1`-`9 n p` |
| The existing wheel test sends reports on row 5 | `tests/attach_cli.rs:148` | read |
| `request_sessions(&mut client)` returns `SessionSummary` with `team` | `src/main.rs:2771` (`request_sessions`), `2813-2819` (`resolve_session`), `src/wire.rs:335-346` | read |

Not probed: a session's team cannot change while attached; whether Claude Code and Codex turn mouse reporting on
(with `session_mouse` true and `top_rows > 0` the new path runs for them).

## Amendment 1 (2026-10-27, architect review of the first implementation; no BLOCKED)

Adds to section 3.6 "Mouse": a press that began on the line (`MousePress::Team`) is swallowed with its release even when
the release lands in the session grid; that release is not sent to the session. A grid release with no remembered
press is still passed through as before. Adds to section 5 acceptance: (a) `tests/daemon.rs` has a session of a
different team and a team-less session next to the two members, and the status `members` omits both (cross-team
leakage test); (b) `tests/attach_cli.rs`: after prefix `s` hides the line the session's output is on screen row 1
(row origin reset); a click on the member line sent in X10 encoding switches like the SGR one; with the picker open,
and in scroll mode, a press and release on row 1 switch nothing; (c) a test of the wait for the connection to close
after `Detach` (a fake daemon that sends frames after `Detach` before closing and sees the second attach only after
the close), or a stated reason it cannot be built in the existing fixtures. The three `// shortcut:` comments the
section demands (top rows fixed at attach, switch and toggle; failed switch not shown; split report unshifted with
`top_rows > 0` and `session_mouse`) are required, and the pre-existing `find_mouse_report` shortcut comment is
extended to say it now also covers X10. `attach_session` returns whether the target is in a team so `switch_session`
asks the daemon once; `with_row` converts the X10 row with a checked conversion (saturating at the byte maximum),
not an `as u8` cast. Scope is unchanged (the paths of section 0).

## 6. Review record

Not yet reviewed.
