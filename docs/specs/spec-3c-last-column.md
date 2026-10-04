# Spec 3c: the attach client must not erase the last column

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `3dbb1e1`; gate notes in section 5).
The owner reported on 2026-10-04 that in every session started through a2amx the last column of
the screen is not visible, so the last character of a long line cannot be seen. Decisions D1 to D3
are the architect's, marked **(architect)**, made under the owner's delegation while away.

**Baseline:** develop at `3dbb1e1`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-last-column`. Public test seam: `Emulator::render_full` and `render_update`
(`tests/emulator_render.rs`).

**Scope.** May edit exactly these files and no others.

- `src/emulator.rs` (`render_line`)
- `tests/emulator_render.rs`
- `docs/backlog.md` (add a Closed row for this spec)

Does not touch: `src/status.rs`, `src/main.rs`, `src/session.rs`, the snapshot serializer's other
paths, any other test file. No new dependency. Do not commit, stage or merge.

## 1. Why

`render_line` (`src/emulator.rs:484`) draws a row cell by cell and then always appends
`ESC[0m ESC[K` (erase to end of line). When the row's last glyph is written in the last column,
the terminal is in the pending-wrap state: the cursor still sits on that last column. In xterm
and the terminals that follow it (to our knowledge including Windows Terminal), `ESC[K` in that
state erases from the cursor column inclusive, so it erases the character just written in the last
column. That matches the owner's symptom exactly: a long line loses its last character, in every
attached session, because the client redraws every row through this function. `src/status.rs:8`
already avoids the same trap ("leave the last column untouched to avoid pending wrap").

This cause is **not confirmed in the owner's terminal**. The repository's own emulator
(alacritty) does not erase it, so no unit test can show the bug; a probe on 2026-10-04 mirrored a
10 column full-width line through `render_full` into a second emulator and the last character
survived. The fix is safe regardless: there is nothing to erase to the right of the last column.

## 2. Decisions

- **D1. (architect)** In `render_line`, do not emit the trailing `ESC[K` when the last glyph reached
  the last column. The test is the loop's own bookkeeping: after the cell loop, if `output_col`
  (the column the next glyph would go to) is `>= cols`, skip the `ESC[K`. This covers a wide
  glyph whose second half is in the last column. Keep the `ESC[0m` reset before it.
- **D2. (architect)** The first early-return branch (a row with no drawn cell, `last` is `None`)
  keeps `ESC[0m ESC[K`: nothing was written, so no pending wrap.
- **D3. (architect)** A row shorter than the screen keeps `ESC[0m ESC[K` exactly as today, so
  stale cells to the right of the last glyph are still cleared. No other output byte changes.

## 3. `src/emulator.rs`

`render_line`, the final `render.extend_from_slice(b"\x1b[0m\x1b[K")` (line 528 at the baseline):
emit `ESC[0m`, and `ESC[K` only when `output_col < cols` (D1). Nothing else in the function
changes.

## 4. Tests (failing first)

In `tests/emulator_render.rs`, through `render_full`, with literal expected bytes:

1. A 10 column, 3 row emulator fed `abcdefghij`: the rendered bytes contain
   `\x1b[1;1H\x1b[0mabcdefghij\x1b[0m\x1b[2;1H` (the full row, then the reset, then the next row's
   cursor move with no `\x1b[K` in between). This is the red test before the change.
2. The same emulator fed `abcdefghi` (one short): the bytes contain
   `\x1b[1;1H\x1b[0mabcdefghi\x1b[0m\x1b[K`, unchanged behavior (D3).
3. A 10 column emulator fed eight narrow characters then one wide character (for example
   `abcdefgh` followed by a wide CJK character) fills exactly 10 columns: no `\x1b[K` after the
   glyph (D1, wide case).
4. The existing mirror tests (`incremental_render_mirrors_*`) still pass.

## 5. Acceptance and gate notes

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass. After acceptance the owner checks in a real terminal that a line of exactly the
terminal's width shows its last character (not a gating step, and the architect cannot do it).

Gate notes at lock time (baseline `3dbb1e1`): the two `ESC[K` emissions are at
`src/emulator.rs:499` and `:528`, and no test file contains the byte sequence `[K` (searched
`tests/`), so no existing assertion depends on it. `render_line` is called from `render_screen`
and `render_update`'s damage loop only. The probe described in section 1 ran in a throwaway
worktree outside the checkout and was removed.
