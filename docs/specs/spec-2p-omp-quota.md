# Spec 2p: report an exhausted OMP session

## 0. Status

**LOCKED.** Decisions D1 to D8 in section 2 follow from the owner's observation of a real
exhausted OMP session on 2026-10-03 and the owner's agreement to write this spec. Builds on spec
2l (`6ec9761`). Work is done directly on `develop` in the main checkout, no worktree. Shared task
`context_id`: `a2amx-omp-quota`. Public test seams: `quota::read` (pure, over an `Emulator`
screen) and the daemon through `Client` requests as `tests/quota.rs` does.

**Scope.** May edit exactly these files and no others.

- `src/quota.rs`, `src/wire.rs`, `src/daemon.rs`, `src/main.rs`
- `tests/quota.rs`, and any existing test file that builds a `QuotaInfo` literal (found with
  `rg "QuotaInfo \{" tests`; the only permitted change is adding `..Default::default()`)
- `README.md`, `docs/architecture.md`

Does not touch every other file under `src/` and `tests/`, `scripts/`, `Cargo.toml`,
`Cargo.lock`, `AGENTS.md`, the locked spec documents. No new dependency. Do not commit and do
not merge: leave the diff uncommitted in the working tree.

## 1. Why

An OMP session on the Codex provider that has used up its quota cannot act on a message, and
today `a2amx list` shows `-` for it and `send_message` reports nothing. Spec 2l reads Claude's
and Codex's status line, and OMP's footer has no quota text (it is `π · <model> · <cwd> · <branch>
· <context%> · ...`). What OMP does show is an error in the conversation area, directly above the
editor. Captured from a real exhausted session (169 columns, 51 rows; rows 46 to 50 are the
editor frame, the empty prompt, the frame, the footer and the session-name line):

```
41|  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)
43|  Error: Retry failed after 1 attempts: Provider requested 1071751ms wait, exceeds retry.maxDelayMs (300000ms). Original error: Codex error event: The usage limit has
44|  been reached (code=usage_limit_reached)
46| ───────────── Implement Session PTY Team Specs ─
47| ❯
48| ─────────────────────────────────────────────────
49|  π · ◑ GPT-6.1 Sol 🙈 · 📁 ~/workspace/a2amx · ⑂ develop · ◫ 60.4%/272K ⟲ · S7.10 + 👁 1.51
50|  developer@desktop-aavo022
```

An exhausted session produces no later output, so the error stays just above the editor until
the quota returns and the session speaks again. The error is also longer than a row and wraps, so
`code=usage_limit_reached` can sit on the second row. OMP also prints `Warning: advisor: Advisor
"Advisor" quota exhausted — pausing until reset.` when only its advisor model is out; that does
not mean the session itself is out and must not count.

Report only, as in 2l. Holding delivery (option B) stays held by the owner.

## 2. Decisions

- **D1.** Report-only. Delivery, holds and message states do not change.
- **D2.** Only `Harness::Omp` sessions get this detection, only while running. Claude and Codex
  keep the 2l status-line reading unchanged (D2 to D3 of 2l); Generic and exited sessions report
  nothing.
- **D3.** The signal is an OMP error row: a row whose text, after trimming leading and trailing
  spaces, starts with `Error: ` and which, together with the row directly below it, contains the
  text `code=usage_limit_reached`. The row below is included so a wrapped error still matches.
  Text that does not start a row with `Error: ` never counts, so conversation that quotes the
  code does not.
- **D4.** Scanned region: the last 12 rows of the screen (fewer when the screen has fewer). The
  match is in that region only.
- **D5.** The advisor warning, and any `Error:` row without `code=usage_limit_reached`, do not
  count.
- **D6.** `QuotaInfo` gains `limit_reached: bool`, true when D3 matched. It does not claim a
  window or a percentage, and `five_hour`/`weekly` stay `None` for OMP.
- **D7.** Exhausted means `limit_reached` or a recognised window at 0%. The report text for
  `limit_reached` alone is `usage` (so `send_message`'s `recipient_quota` becomes `usage quota
  exhausted`); when a window is also at 0% the existing window text wins.
- **D8.** The value can be stale: after the quota returns the error stays on screen until the
  session prints something else. Documented, not corrected (same as 2l D9). The daemon never
  sends input to learn it.

## 3. `src/wire.rs`

`QuotaInfo` gains

```rust
    /// The harness reported a usage limit in the conversation, with no window named.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub limit_reached: bool,
```

`exhausted`: the existing four window cases first; when none applies and `limit_reached`, return
`Some("usage")`. Keep the return type.

## 4. `src/quota.rs`

Extend the signature rather than adding a second reader:

```rust
pub fn read(screen: &Screen, harness: Harness) -> Option<QuotaInfo>;
```

For `Harness::Claude | Harness::Codex` behave exactly as today. For `Harness::Omp` apply D3 to D5
and return `Some(QuotaInfo { limit_reached: true, ..Default::default() })` or `None`. For every
other harness return `None`. Share the existing row-to-text conversion (cells to chars, U+00A0 to
space, trailing spaces trimmed); do not copy it. Add a `// shortcut:` comment on the 12-row
region: a long multi-line draft in the editor can push the error out of it; upgrade by anchoring
on the editor frame if that is seen.

Worked examples (`rows = 51`, `cols = 169`; text placed with `ESC [ <row> ; 1 H`):

| # | Content | Result (`Harness::Omp`) |
|---|---|---|
| 1 | row 41 `  Error: Codex error event: The usage limit has been reached (code=usage_limit_reached)` | `limit_reached` |
| 2 | row 43 `  Error: Retry failed after 1 attempts: ... Original error: Codex error event: The usage limit has` and row 44 `  been reached (code=usage_limit_reached)` | `limit_reached` |
| 3 | row 19 (the 33rd from the bottom) holding the example 1 text | `None` (outside the region) |
| 4 | row 41 `  Warning: advisor: Advisor "Advisor" quota exhausted — pausing until reset.` | `None` |
| 5 | row 41 `  Error: Codex error event: overloaded (code=server_error)` | `None` |
| 6 | row 41 `  the log said Error: Codex error event: usage (code=usage_limit_reached)` | `None` (does not start with `Error: `) |
| 7 | row 41 `Error: x` and row 43 `(code=usage_limit_reached)` (not the next row) | `None` |
| 8 | the example 1 text with `Harness::Generic` | `None` |
| 9 | row 24 of a 24-row screen `gpt-5 · weekly 12% left · 5h 0% left` with `Harness::Codex` | five_hour 0, weekly 12 (unchanged) |
| 10 | an empty screen with `Harness::Omp` | `None` |

## 5. `src/daemon.rs`, `src/main.rs`

`session_quota` passes the session's harness to `quota::read` and now admits `Harness::Omp` in
its guard; Generic and exited sessions still return `None`. No other daemon change.

`quota_cell` (`src/main.rs`): when `limit_reached` and neither window is present the cell is
`limit`; when windows are present it is unchanged. (A value with both is not produced by any
reader today; show the windows.)

## 6. Tests

Failing test first, one behavior at a time.

- `tests/quota.rs`: one test per row of the section 4 table, with expected values as literals;
  `QuotaInfo::exhausted` for `limit_reached` alone (`Some("usage")`), `limit_reached` with
  `five_hour: Some(0)` (`Some("5h")`), and `limit_reached` false with no windows (`None`).
- Daemon tests as 2l's: an `Harness::Omp` session running `sh -c "printf '\\033[41;1H  Error:
  Codex error event: x (code=usage_limit_reached)'; exec sleep 30"` at 169 by 51: eventually
  `Request::List` reports `quota` with `limit_reached` true and `Request::ListAgents` the same;
  a message from a second session to it is accepted with `recipient_quota` `Some("usage quota
  exhausted")`; the same script as `Harness::Generic` reports no quota. The message state after
  sending is unchanged by quota (assert as 2l does).
- A `quota_cell` check through the `a2amx list` output is not required.

## 7. `README.md`, `docs/architecture.md`

`README.md`: extend the QUOTA sentence added by 2l to say that for OMP it shows `limit` when the
session's last conversation rows show a Codex usage-limit error.

`docs/architecture.md`: extend the 2l quota paragraph in the document's style: for running OMP
sessions the daemon looks in the last 12 rows for an `Error:` row naming `usage_limit_reached`
(wrapped errors included), reports it as `limit_reached`, never changes delivery because of it,
and it may be stale until the session prints again.

## 8. Out of scope

Option B (holding delivery), reading OMP's logs or extension events, the time to reset from
`Provider requested N ms wait`, other OMP providers' error shapes, the advisor's own exhaustion,
an `a2amx screen` command, alerts in the attach status bar, `src/harness.rs`, `src/session.rs`,
`src/emulator.rs`.

## 9. Acceptance

Run all three, no warnings, with `A2AMX_BIN` unset (`env -u A2AMX_BIN`):

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`git diff --stat` shows only the files in section 0; `git diff --check` is clean.
