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
  waited for (line 339).
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
