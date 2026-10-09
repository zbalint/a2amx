# Spec 3u: the session picker groups sessions into foldable team sections and filters

## 0. Status

**LOCKED** (2026-10-09, pre-lock gate run against develop at `597ae93`; consultant advice `m_905` and
pre-lock reviews `m_922` and `m_924`, context `a2amx-picker-teams`; gate notes in section 9). Owner request: the picker is too dense with several teams of several agents; the
rows themselves stay as they are. Spec 3t (scrolling, page keys, the pure `picker` module) is committed;
this spec builds the team sections, folding and a filter on it. Owner decisions: rows unchanged; other
teams start folded. Architect decisions are in section 2.

**Baseline:** develop at `597ae93`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-picker-teams`. Public
test seams: `a2amx::picker` (pure view model and parser, `tests/picker.rs`) and the `a2amx` binary under a
PTY (`tests/attach_cli.rs`).

**Scope.** May edit exactly these files and no others.

- `src/picker.rs`, `src/main.rs`
- `tests/picker.rs`, `tests/attach_cli.rs`
- `AGENTS.md` (the `picker` module row), `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/lib.rs`, `src/wire.rs`, `src/daemon.rs`, `src/messaging.rs`, `list` output and its
formatting helpers (`format_session_table`, `insert_team_column`, `session_team_cells`,
`session_value_rows`, `picker_value_rows`, `column_widths`, `format_row`), every other test file, every
spec document. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

With several teams the picker shows one long table with a TEAM column. Finding a session means scrolling
through every agent of every team. Sections with a one-line summary per team, folded by default except where
the user already is, plus a filter, make a large fleet navigable, while a small fleet (no teams) looks exactly
as it does today.

## 2. Decisions

- **D1. View model in `src/picker.rs`.** A pure `PickerView` owns the sections, the fold state, the filter
  and the selection. `src/main.rs` keeps the row formatting and the I/O. `PickerView` takes `PickerEntry`
  values (not `SessionSummary`) so it stays independent of the wire types.
- **D2. Layout.** If no entry has a team, the picker is flat with no headers, exactly the 3t layout.
  Otherwise sessions are grouped: team sections sorted by team name (byte order), then one section for entries
  without a team, labelled `(no team)`, last. Inside a section, sessions keep the order the daemon returned.
  The TEAM column is no longer shown in the picker (the header carries it); `list` is unchanged.
- **D3. Header text.** Expanded: `[-] NAME (N)`; folded: `[+] NAME (N): SUMMARY` (no `: SUMMARY` when the
  summary is empty). `NAME` is the team name, followed by ` (private)` when the team is private, or `(no team)`.
  `N` is the number of sessions in the section (under a filter: the number shown). The summary lists, in this
  order and omitting zero counts, `K working`, `K busy`, `K idle` (running sessions by activity; a running
  session with unknown activity is counted in none of them), `K held` (running sessions with a hold, counted in
  addition to its activity), `K exited`, joined by `, `.
- **D4. Selection identity.** The selection is `Session(id)` or `Header(team key)` (the key is the team name,
  `None` for the `(no team)` header), never a line index; the line index is derived. After any change that
  hides the selected line (a fold, or `replace` with no active query) the selection becomes, in order: for a hidden session whose
  section is folded, that section's header; otherwise the line at the previous line index clamped to the last
  line; with no lines, nothing. A `replace` while the query is non-empty uses D7's rule for a hidden selection.
- **D5. Initial state.** Count the distinct named teams. With two or fewer, every section starts expanded.
  With more, only the section that contains the current session starts folded-open; all others start folded.
  The selection starts on the current session (hidden never: its section is open); if the current session is not
  in the list, on the first line. Fold state survives `replace` (the periodic or error refresh) by team key and
  is not kept between picker openings.
- **D6. Keys, no filter active.** `Space` or `Enter` on a header toggles its fold; `Left` and `h` fold,
  `Right` and `l` unfold: on a header they act on that section, on a session `Left`/`h` folds its section and
  moves the selection to the header, `Right`/`l` do nothing. `Enter` on a session attaches it (as today). `/`
  starts a filter. `q`, `Esc`, `j`, `k`, arrows, PageUp/PageDown, Home/End keep their 3t meaning (Move,
  Page, Home, End are navigation over lines, headers included).
- **D7. Filter.** `/` enters filter mode with an empty query. In filter mode every printable ASCII byte
  (`0x20..=0x7e`, including `q j k h l /` and space) is appended to the query, `0x7f` and `0x08` remove the
  last character, arrows/PageUp/PageDown/Home/End still navigate, `Enter` attaches the selected session. `Esc`
  (a lone `0x1b` read) clears the query and leaves filter mode; the next `Esc` cancels the picker. The
  match is an ASCII case-insensitive substring test against the session id, the session name, and the team
  name. While the query is non-empty: only matching sessions are shown, grouped under their headers (a header
  shows only if it has at least one shown session), every shown section is expanded whatever its fold state
  (the fold state itself is not changed), fold keys and `Enter` on a header do nothing, and a team name
  match shows all of that team's sessions. When the query changes and the selection is no longer shown, the
  selection becomes the first shown session, or the first line when there is none. When the query becomes empty
  (by backspacing or `Esc`) D4 applies. An empty query in filter mode shows everything with the folds as they
  are.
- **D8. Scrolling and position.** The 3t window (`scroll_into_view`, `target`) works over the lines
  (headers and sessions); the footer position is `{selected line + 1}/{number of lines}` when the lines do not
  fit. In filter mode the footer shows `/QUERY`, followed by two spaces and the position when the lines do not
  fit. A footer error keeps priority over both, as today.
- **D9. Row layout.** Marker column first (`>` selected, space otherwise), as today. A header line is the
  marker followed by the header text. A session line in a sectioned picker is the marker, two spaces of indent,
  then the existing row. The column header gets the same two-space indent. Flat pickers have no indent. Column
  widths are computed over all sessions (not only the visible ones) so the table does not change shape while
  folding. The CWD clipping arithmetic counts the indent as part of the fixed prefix, both in the per-row `available`
  width and in the `header_has_cwd` test (`1 + indent + fixed + len("CWD")`).
- **D10. Out of scope behavior.** `Enter` on the session you are already attached to returns to it, unchanged.
  Live refresh is not added.
- **D11. Footer error.** A footer error (a failed switch) is cleared by the next byte the picker receives, which
  also redraws it, so the filter prompt and position come back. This replaces 3t's D6b (the error persisting
  until the picker closes). No automated test by choice (a failed attach is possible, as the switch test shows, but proving the error is
  gone needs a wait-for-absence helper that `tests/common` does not have and this spec does not add); the
  tester probes it.

## 3. `src/picker.rs`

Additions (all `pub`, no I/O, no async; keep `PickerParser`, `scroll_into_view` and `target` as 3t left them
except where this section says; `target`'s `match` gains arms for the new `PickerAction` variants, which all
return `selection` unchanged like `Cancel` and `Select`):

```
pub enum EntryState { Working, Busy, Idle, Unknown, Exited }
pub struct PickerEntry { pub id: String, pub name: String, pub team: Option<String>, pub private: bool,
                         pub state: EntryState, pub held: bool }
pub enum Line { Header { key: Option<String>, text: String }, Session { id: String } }
pub enum Outcome { Attach(String), Cancel }
pub struct PickerView { /* private fields */ }
impl PickerView {
    pub fn new(entries: Vec<PickerEntry>, current: &str) -> Self;       // D5
    pub fn replace(&mut self, entries: Vec<PickerEntry>);              // D4, D5 (fold state kept)
    pub fn lines(&self) -> Vec<Line>;                                    // D2, D3, D7
    pub fn selected_line(&self) -> Option<usize>;
    pub fn selected_session(&self) -> Option<&str>;
    pub fn filter_mode(&self) -> bool;
    pub fn query(&self) -> &str;
    pub fn apply(&mut self, action: PickerAction, height: usize) -> Option<Outcome>;
}
```

`PickerEntry.name` is the empty string for an unnamed session. `apply` handles every `PickerAction`:
`Move/Page/Home/End` set the selection from `target(selected_line, action, height, lines.len())`; `Toggle`,
`Fold`, `Unfold` per D6 (no-ops while the query is non-empty); `StartFilter`, `Char`, `Backspace` per D7;
`Select` returns `Some(Outcome::Attach(id))` on a session, toggles on a header (no-op while the query is
non-empty);
`Cancel` returns `Some(Outcome::Cancel)` unless filter mode is on, in which case it clears the query, leaves
filter mode and returns `None`. Everything else returns `None`.

`PickerAction` gains `Toggle`, `Fold`, `Unfold`, `StartFilter`, `Char(u8)`, `Backspace`.
`PickerParser` gains `pub fn set_filter_mode(&mut self, on: bool)` (default off) and these mappings:

- Filter off: `' '` gives `Toggle`; `h` and `ESC [ D` and `ESC O D` give `Fold`; `l` and `ESC [ C` and
  `ESC O C` give `Unfold`; `/` gives `StartFilter`. All 3t mappings stay.
- Filter on: a byte in `0x20..=0x7e` (when no escape sequence is pending) gives `Char(byte)`; `0x7f` and `0x08`
  give `Backspace`; `\r` gives `Select`; arrows, PageUp/PageDown, Home/End and a lone ESC keep their meaning;
  `q j k h l /` and space are `Char`.

Precedence: with no escape sequence pending, filter mode tries the printable-`Char` and `Backspace` rules
before anything else (so `q j k h l /` are text); with only an ESC pending (one byte), the next byte decides: `[` or `O`
continues the sequence, any other byte clears the pending ESC and is then handled as if nothing were pending
(this also applies with the filter off: ESC then `j` gives `Move(1)`). Once the sequence continues (`ESC [` or `ESC O`), the pending buffer swallows any byte up to 3t's
existing limit (cleared when more than 3 bytes are pending without a match), exactly as 3t does; do not build
a whitelist.

`ESC [ C` and `ESC [ D` are three bytes, so the existing "clear when more than 3 bytes pending" rule is
unchanged.

## 4. `src/main.rs`

- `PickerState`: replace the `selection` index with a `PickerView` built from the session list (a small
  private function maps `SessionSummary` to `PickerEntry`: name `""` when none, team name and `private` from
  `team`, state from `exit_code` then `activity` (`Idle`, `Working`, `Busy`, otherwise `Unknown`), `held` from
  the summary's `held`); keep `sessions` for row formatting. `refresh` calls `view.replace`. After every
  `apply` call `parser.set_filter_mode(view.filter_mode())` before the next byte is fed (the binary feeds
  one byte per call, so `/be-2` arriving in one read must parse as `/` then text).
- `handle_picker_input` clears `footer` at its start (D11) and redraws once if it was set.
- `selected_id` returns `view.selected_session()`. `set_target`/`move_selection` become `view.apply(...)`; its
  `Outcome::Attach(id)` takes the place of the current `Select` flow (the flow itself is unchanged), and
  `Outcome::Cancel` the place of `Cancel`. Offsets are recomputed from `view.selected_line()` and the line
  count after every change, refresh and resize.
- `render`: build `view.lines()`; draw a header line as marker plus text, a session line as marker plus
  indent (D9) plus that session's formatted row; window and footer per D8; the title hint becomes
  `a2amx sessions: Enter attach/fold, Esc or q cancel, / filter, Space fold, j/k move` (tests wait only for the
  `a2amx sessions:` prefix).
- `session_rows`: drop the `selection` parameter and the marker, add an `indent: usize` parameter (2 when the
  picker is sectioned, else 0), stop calling `insert_team_column`, and return the column header and one
  formatted row per session in the order of `sessions` (no marker; the header keeps its single leading space
  and gains the indent). The row for a session is looked up by id when rendering. `insert_team_column` and
  `session_team_cells` stay (used by `list`).

## 5. Tests (failing first, expected values are literals)

Fixture for `tests/picker.rs` (entry order is the daemon order): `s1` name `base`, no team, `Idle`;
`s2` `al-1`, team `alpha` private, `Working`; `s3` `al-2`, `alpha` private, `Idle`, held; `s4` `be-1`, `beta`
(not private), `Working`; `s5` `be-2`, `beta`, `Exited`; `s6` `ga-1`, `gamma`, `Idle`. Current is `s1`.

1. `PickerView::new(fixture, "s1").lines()` equals, in order:
   `Header{key:Some("alpha"), text:"[+] alpha (private) (2): 1 working, 1 idle, 1 held"}`,
   `Header{key:Some("beta"), text:"[+] beta (2): 1 working, 1 exited"}`,
   `Header{key:Some("gamma"), text:"[+] gamma (1): 1 idle"}`,
   `Header{key:None, text:"[-] (no team) (1)"}`, `Session{id:"s1"}`; `selected_line() == Some(4)`.
2. Same view, `apply(Home, 5)` then `apply(Select, 5)` (toggle): lines become `[-] alpha (private) (2)`,
   `s2`, `s3`, then the beta and gamma headers (still folded), the open `[-] (no team) (1)` header and `s1`;
   `selected_line() == Some(0)`. `apply(Move(1), 5)` selects `s2` (line 1); `apply(Unfold, 5)` and
   `apply(Toggle, 5)` on `s2` change nothing (same lines, `s2` still selected); `apply(Fold, 5)` then folds
   alpha again and `selected_line() == Some(0)`.
3. Two teams (entries `s2..s5`, current `s2`): every section starts expanded (no `[+]`). No teams (entry `s1`
   only, plus another unteamed entry `s7` name `solo`): `lines()` is exactly `[Session s1, Session s7]`, no
   header.
4. Current in a folded-by-default team: `new(fixture, "s5")` has beta expanded (header `[-] beta (2)`, `s4`,
   `s5`), alpha, gamma and `(no team)` folded, selection `s5`.
5. Filter: from test 1's view, `apply(StartFilter, 5)`, then `Char(b'b')`, `Char(b'e')`, `Char(b'-')`,
   `Char(b'2')`: `lines()` is `Header{key:Some("beta"), text:"[-] beta (1)"}`, `Session{id:"s5"}`, and
   `selected_session() == Some("s5")`; `Backspace` once (query `be-`) gives `[-] beta (2)`, `s4`, `s5` and
   the selection stays `s5`; `apply(Cancel, 5)` returns `None`, `filter_mode()` is false, `query()` is `""`,
   `lines()` is test 1's lines again (beta folded, so the selection moved to the beta header,
   `selected_line() == Some(1)`); a second `apply(Cancel, 5)` returns `Some(Outcome::Cancel)`.
6. Team-name match: query `alpha` shows `Header{key:Some("alpha"), text:"[-] alpha (private) (2)"}`, `s2`,
   `s3` only; query `GA` (uppercase) shows `[-] gamma (1)`, `s6` only; query `zz` shows no lines and
   `selected_line() == None`.
7. `apply(Select, 5)` on a session returns `Some(Outcome::Attach("s1".into()))`; `apply(Cancel, 5)` with no
   filter returns `Some(Outcome::Cancel)`.
8. `replace`, two cases that each start from a fresh `PickerView::new(fixture, "s1")` with `Home`,
   `Select` (alpha open) and `Move(1)`, `Move(1)` (selection `s3`, `selected_line() == Some(2)`):
   (a) `replace` with the fixture minus `s3`: alpha stays open (`lines()[0].text` is
   `[-] alpha (private) (1)`), beta stays folded, and the removed selection falls back to the line at the
   previous index (D4): `selected_line() == Some(2)` and `lines()[2]` is
   `Header{key:Some("beta"), text:"[+] beta (2): 1 working, 1 exited"}`; (b) `replace` with the unchanged
   fixture keeps the selection (`selected_session() == Some("s3")`).
9. Parser (one byte at a time, filter off): `b' '` gives `Toggle`; `h`, `ESC [ D`, `ESC O D` give `Fold`;
   `l`, `ESC [ C`, `ESC O C` give `Unfold`; `/` gives `StartFilter`; `j` still `Move(1)`.
   Filter on (`set_filter_mode(true)`): `q j k h l / x` and space each give `Char(that byte)`; `0x7f` and
   `0x08` give `Backspace`; `\r` gives `Select`; `ESC [ A` still `Move(-1)`; a lone ESC (`escape_alone=true`)
   still `Cancel`; `set_filter_mode(false)` restores the unfiltered mappings. ESC followed by another byte in one read
   (`[0x1b, b'b']`, `escape_alone=false`) with the filter on gives `[Char(b'b')]`; with the filter off
   `[0x1b, b'j']` gives `[Move(1)]`; `[0x1b, b'[', b'A']` still gives `[Move(-1)]`.
10. Existing 3t tests in `tests/picker.rs` still pass unchanged.

`tests/attach_cli.rs`, PTY (replace `picker_shows_team_column_when_a_team_session_exists` with
`picker_groups_sessions_into_team_sections`; it keeps its two sessions: `plain` and the team session `worker`
of team `demo`): the screen has no `TEAM` column header, shows `[-] demo (private) (1)`, `worker`,
`[-] (no team) (1)` and `plain` (two teams or fewer: everything open), with `NAME` before the rows.
New `picker_folds_teams_and_filters`: start `base` (`new --detach --name base -- sh -c "sleep 30"`) and three
team files (teams `alpha`, `beta`, `gamma`; sessions `al-1 al-2`, `be-1 be-2`, `ga-1 ga-2`, each
`sh -c "sleep 30"` except `be-2`, which is `sh -c "printf BE2-READY; sleep 30"`) with `team up --detach --file`, attach to `base`, open the picker: the screen shows
`[+] alpha (private) (2)`, `[+] beta (private) (2)`, `[+] gamma (private) (2)`, `[-] (no team) (1)` and
`base`, and not `al-1`. Send `Home` (`ESC [ H`) then `\r`: wait for `[-] alpha (private) (2)`, the screen shows
`al-1` and `al-2` and not `be-1`. Send space: wait for `[+] alpha`, no `al-1`. Send `/`, then `be-2`: wait for
`[-] beta (private) (1)`, the screen shows `be-2` and not `al-1`, and the footer shows `/be-2`. Send `\r`: wait
for the text `BE2-READY` (the attach switched to `be-2`; the existing switch test at
`tests/attach_cli.rs` lines 304-404 verifies a switch the same way, by the target's output).
Wait for text before every screen assertion.

Existing PTY picker tests (the switch test, the clip test, the mouse test, the 3t scrolling test) have no
teams, so they stay flat and must pass unchanged. If any other existing test fails because of this change,
stop and report `BLOCKED — SPEC ADJUDICATION REQUIRED` with the test name.

## 6. Documentation

- `AGENTS.md`: the `picker` row becomes `The session picker's key parser, view model (team sections, folds,
  filter) and scroll window (pure)`.
- `README.md`: the attach keys paragraph (around line 152) and the sentence near line 174 about filters not
  affecting the picker: describe the sections, `Space`/`Enter` fold, `h`/`l`/`Left`/`Right`, `/` filter and
  `Esc`; say that other teams start folded when there are more than two and that the picker is still the
  unfiltered operator view.
- `docs/architecture.md`: the picker description and line 472's "same optional TEAM column" (the picker now
  shows sections, not a column).
- `docs/backlog.md`: add `| C25 | Session picker groups sessions into foldable team sections and filters | spec 3u |`
  after the last row.

## 7. Out of scope

Escape sequences longer than four bytes typed in filter mode (F5 is `ESC [ 1 5 ~`, Ctrl-Right `ESC [ 1 ; 5 C`,
split mouse reports): their tail leaks into the query as text because the parser ends a sequence by length,
not at the CSI final byte. Accepted (a shortcut the 3t parser already has); upgrade trigger: report of junk
in the filter, fix is to end a CSI at a byte in `0x40..=0x7e`.

Live refresh while the picker is open; mouse; multibyte filter input; fold state kept between openings; a
protocol or daemon change; the `list` command and its TEAM column; changing columns or their content; the
cell-width fix; any change to how the picker is opened or to `attach`.

## 8. Acceptance

Post-implementation commands (new code required), with `A2AMX_BIN` and `NO_COLOR` unset:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short   # only files listed in section 0 scope (the committed spec is not listed)
```

All pass with no warnings. Known load flakes: `attach_cli::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
and `daemon::exited_claude_session_never_reports_needs_input`; if one fails, rerun it alone and report both
results.

## 9. Gate notes (pre-lock)

- Baseline: develop `597ae93`, clean tree; `cargo test` at the 3t acceptance: 452 passed, 0 failed, 3 ignored.
- Consultant reviews applied: `m_922` (E1-E7: test sequencing and the `(no team)` wording, D4 versus D7
  selection rules, `target` arms, PTY marker `BE2-READY`, parser precedence and the ESC-then-byte rule,
  footer error cleared on the next byte), `m_924` (W1 pending-ESC wording and no whitelist, W2 footer-clear
  test note, W3 long-sequence leak listed in section 7). The consultant hand-worked tests 1-8 against the
  fixture and verified anchors, team file defaults (private) and name validity.
- Scope reconciliation: files named in sections 3-6 are all in section 0 scope; `tests/common` is not
  edited. Other users of `session_rows` and `PickerAction`: only `src/main.rs` and `tests/picker.rs`
  (consultant grep).
