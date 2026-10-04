# Spec 3d: ACTIVITY column in the session picker

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `a06da9a`; gate notes in section 6).
The owner saw that the `Ctrl-b w` picker lacks the ACTIVITY column that `a2amx list` has and said
they want it. Decisions D1 to D3 are the owner's or the architect's, marked accordingly.

**Baseline:** develop at `a06da9a`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-picker-activity`. Public test seam: the attach PTY tests in `tests/attach_cli.rs`.

**Scope.** May edit exactly these files and no others.

- `src/main.rs` (`PICKER_HEADERS`, `picker_value_rows`, `session_rows`)
- `tests/attach_cli.rs`
- `docs/backlog.md` (add a Closed row for this spec)

Does not touch: `session_value_rows`, `SESSION_HEADERS`, `DETAIL_HEADERS`, `src/daemon.rs`,
`src/wire.rs`, `README.md`, `docs/architecture.md`, any other test. No new dependency. Do not
commit, stage or merge.

## 1. Why

Spec 2y added ACTIVITY to `a2amx list`, the details table and `list_agents`, and told the developer
to leave the picker unchanged. The owner now wants it in the picker too. The picker shows the same
cells as `list` (minus none) plus CWD, so the fix is to stop dropping the cell.

## 2. Decisions

- **D1. (owner)** The picker shows ACTIVITY.
- **D2. (architect)** Column order matches `a2amx list`: ID, NAME, HARNESS, STATE, ACTIVITY,
  ATTACHED, PENDING, HELD, QUOTA, SIZE, then CWD last. The picker row is the ten
  `session_value_rows` cells unchanged, plus the CWD cell.
- **D3. (owner)** When the terminal is too narrow for everything, CWD is the column that is
  shortened (and omitted below its existing minimum). ACTIVITY is never shortened or dropped.
  This is the picker's existing CWD clipping rule with one more fixed column in front of it; no new
  clipping logic.

## 3. `src/main.rs`

- `PICKER_HEADERS` becomes `[&str; 11]`: the ten `SESSION_HEADERS` in order, then `"CWD"`.
- `picker_value_rows` returns `Vec<[String; 11]>`. Delete the destructure that drops `_activity`;
  build each row from the ten `session_value_rows` cells followed by
  `session.cwd.clone().unwrap_or_else(|| "-".to_owned())`.
- `session_rows`: the fixed (non-CWD) columns are now ten, so every index and count that assumed
  nine fixed columns moves by one: `widths[..9]` and `2 * 9` become `widths[..10]` and `2 * 10`;
  `PICKER_HEADERS[9]` becomes `PICKER_HEADERS[10]`; `row[9]` becomes `row[10]`; the
  `PICKER_HEADERS[..9]`, `widths[..9]` and `row[..9]` slices become `[..10]`. Nothing else in the
  function or in `shorten_left` changes.

## 4. Tests (failing first)

In `tests/attach_cli.rs`:

- `picker_switches_sessions_and_exit_status_is_reported`: add `"ACTIVITY"` to the header list
  waited for (line 339) and, per Amendment 2, remove `"CWD"` from that list.
- Extend that test or add one: with a running session, the opened picker shows a row containing an
  activity literal (`idle`, `working` or `busy`) in the ACTIVITY column, and an exited session
  shows `-` there. Use literals, no recomputation with the code under test.
- `picker_clips_long_cwd_to_narrow_terminal`: see Amendment 1. The terminal becomes 100x10; the
  literal `…-picker-path (current)` and every other assertion stay as they are.

## 5. Out of scope

Changing `session_value_rows`, the list or details output, the daemon, the wire format, colors, a
per-width column drop order beyond CWD, terminal cell-width handling (existing `shortcut:` stays),
docs prose (the README and architecture mention the picker only as "session picker").

## 6. Acceptance and gate notes

Post-implementation commands (they need the new code, so they were not run at lock time):

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short   # only the three scoped files
```

Gate notes, at `a06da9a`: `rg "PICKER_HEADERS|picker_value_rows"` hits only `src/main.rs`; the
picker header text appears in tests only at `tests/attach_cli.rs:339`; `rg "\[9\]|\[\.\.9\]"` in
`src/main.rs` shows the `session_rows` uses listed in section 3; README and architecture do not
describe picker columns. The old-text search for `ACTIVITY` in tests shows only `list` output
assertions (lines 475, 488, 637), unaffected.

## Amendment 1 (2026-10-04, after developer BLOCKED m_376)

Evidence, verified against `src/main.rs:2065-2093` and `tests/attach_cli.rs:377-428`: at a 90x10
terminal the attached session reports size `90x9`, so the ten fixed widths are
`[2,4,7,7,8,8,7,4,5,4]` (sum 56) plus 20 separator columns = 76. CWD room is
`90 - (1 marker + 76 + 10 for " (current)")` = 3, below the existing 4-cell minimum, so CWD is
omitted and the literal cannot appear. This contradicted section 4's old "must still pass
unchanged" at 90 columns. It does not contradict D3: CWD is the column that yields.

Decision: in `picker_clips_long_cwd_to_narrow_terminal`, change the terminal from 90x10 to 100x10.
That is, `resize(90, 10)` becomes `resize(100, 10)`, the waited text `"9 90"` becomes `"9 100"`,
and the line-width bound `<= 90` becomes `<= 100`. At 100 columns CWD room is 13, the same as the
test had before this spec, so `…-picker-path (current)` still appears verbatim. No other assertion
changes, and the production code is unchanged by this amendment. The test's own 80x24 start
(`"23 80"`) is untouched.

Gate: `rg -n "90" tests/attach_cli.rs` in the test's range shows only those three uses.

## Amendment 2 (2026-10-04, after developer BLOCKED m_378)

Causal review: Amendment 1 was found by checking the one test with its own width; the gate did not
enumerate every test that opens the picker at the PTY default of 80 columns (`tests/common/pty.rs`
`COLS`). Enumerated now: the picker is opened in `tests/attach_cli.rs` at lines 290, 337, 356,
386 and 440. Only line 339 (inside the test opened at 337) waits for the CWD header; the others
wait for `a2amx sessions:` only, and `tests/refs_and_list.rs:313` tests `list --details`, which
this spec does not touch.

Evidence (developer, verified against the code): at 80 columns the ten fixed widths sum to 57,
plus 20 separators is 77, so the CWD header needs `1 + 77 + 3 = 81` columns and is omitted. This
is D3 working as specified, not a defect.

Decision: in `picker_switches_sessions_and_exit_status_is_reported` the header list waited for
becomes `["NAME", "HARNESS", "ACTIVITY", "PENDING", "HELD", "QUOTA"]` (no `"CWD"`). In
`picker_clips_long_cwd_to_narrow_terminal` (100x10 after Amendment 1) add one assertion: some
screen line contains the header `CWD`. At 100 columns the header fits (81 <= 100), so this keeps
header coverage. No production change from this amendment, no other test change.

Gate: `rg -n "CWD" tests` hits `tests/attach_cli.rs` (the two tests above) and
`tests/refs_and_list.rs:313` only.

## Amendment 3 (2026-10-04, after developer BLOCKED m_383)

Causal review: Amendment 1 computed CWD room at 100 columns as 13 by reusing the 4 character
SIZE cell of a `90x9` session. At 100 columns the SIZE cell is `100x9` (5 characters), so the fixed
widths are `[2,4,7,7,8,8,7,4,5,5]` = 57, plus 20 separators = 77, and CWD room is
`100 - (1 + 77 + 10)` = 12, giving `…picker-path`, not `…-picker-path`. The developer's observed
screen (`100x9  …picker-path (current)`) confirms the 12.

Decision: the test terminal becomes 101x10, not 100x10. The waited text is `"9 101"`, the line
bound is `<= 101`, and everything else stays as in Amendments 1 and 2. At 101 columns the SIZE cell
is `101x9` (still 5 characters), so CWD room is `101 - 88` = 13 and the literal
`…-picker-path (current)` appears verbatim. The CWD header (needs 81 columns) still fits. The
literal stays unchanged. No production change.

The room depends on the SIZE cell width, which depends on the column count; a width of 100 to 109
columns all give `NNNx9` (5 characters) and 101 is the one that yields 13.

