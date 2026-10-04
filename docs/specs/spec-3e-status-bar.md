# Spec 3e: a richer attach status bar

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `5d9ccde`; gate notes in section 7).
The owner called the bottom status line of the attach client "barebone" and asked for something
more appealing, leaving the content to the architect. The architect asked the consultant what an
attached user needs and adopted its advice (D2 to D6). Decisions D1 to D7 are the architect's,
marked **(architect)**, made under the owner's delegation while away.

**Baseline:** develop at `5d9ccde`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-status-bar`. Public test seams: `a2amx::status::render` (`tests/status.rs`), the daemon
attach status frames (`tests/daemon.rs`), the wire codec (`tests/wire.rs`).

**Scope.** May edit exactly these files and no others.

- `src/status.rs`, `src/wire.rs` (`StatusInfo`), `src/daemon.rs` (`attachment_status`),
  `src/quota.rs`, `src/main.rs` (`draw_status`, the one `quota_cell` call site, the
  `quota_cell` definition)
- `tests/status.rs`, `tests/daemon.rs`, `tests/wire.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/session.rs`, `src/harness.rs`, `src/delivery.rs`, `src/emulator.rs`, any
other test file, the picker, scroll mode's own row. No new dependency. Do not commit, stage or
merge.

## 1. Why

Today the bar is an inverse-video row with the session address on the left, `N pending` and a red
`HELD reason` alert on the right. The daemon already knows each session's harness, activity
(`idle`, `working`, `busy`) and quota, and `a2amx list` shows them, but the attached user cannot
see any of it without leaving the session. The owner wants a more appealing and more useful bar.

## 2. Decisions

- **D1. (architect)** Styling: a dark-grey bar instead of reverse video, bold name, dim separators,
  a colored activity dot. Exact bytes in section 4.
- **D2. (architect)** Content, left to right: `NAME`, then optional details in this order
  `● ACTIVITY`, `QUOTA`, `HARNESS`, each introduced by `│`; on the right `N pending`, then the
  `HELD` alert. HELD keeps today's highest priority.
- **D3. (architect)** The activity detail is shown only when the session has no hold
  (`info.hold` is `None`) and `info.activity` is `Some`. The word is `idle`, `working`, or, for
  `busy`, `resetting` (a hold also makes the daemon report `busy`, and the HELD alert already says
  that; with no hold `busy` can only mean a reset is running). Dot colors: idle 78, working 221,
  busy 209 (256-color foreground).
- **D4. (architect)** The quota detail is shown only when it is low: `info.quota` is `Some`
  and (`limit_reached`, or `five_hour` or `weekly` is `Some(n)` with `n <= 15`). The text is
  `quota::cell` (the same text `a2amx list` shows). Color: red (203) when `limit_reached`, amber
  (221) otherwise. Reason: for Claude and Codex the quota is read from the session's own last rows,
  which the attached user already sees; showing it always would repeat the screen and could go
  stale.
- **D5. (architect)** The harness detail is `info.harness.as_str()` when `Some`.
- **D6. (architect)** The HELD alert gets a key hint: ` HELD {reason} · ^{L} r ` where `{L}` is the
  prefix's control letter (`b'A' + prefix - 1`, so Ctrl-B is `B`), when the prefix byte is `1..=26`;
  with any other prefix byte there is no hint. `StatusInfo.hold` is always one of
  `human_draft`, `unsubmitted_envelope`, `corrupted_submissions` (`hold_name` in
  `src/delivery.rs`; the native and Codex channels never report a hold, and strings such as
  `channel_down` are unready reasons, not holds), and `r` releases all three. As a guard, the
  renderer shows the hint only for those three reasons, so a future hold that `r` does not release
  gets no hint; tests exercise that branch with a synthetic reason (`channel_down`), which a real
  daemon never sends.
- **D7. (architect)** Narrow terminals: while the total width exceeds the available width, remove
  in this order: the harness detail, the quota detail, the activity detail, `N pending`, the HELD
  key hint; after that the existing truncation rules apply unchanged (section 3).

## 3. `src/status.rs`

New signature: `render(session: &str, info: Option<&StatusInfo>, prefix: u8, cols: u16, rows: u16)
-> Vec<u8>`. Keep the early return (`rows < 3 || cols < 2` returns an empty vector) and
`width = cols - 1` (the last column is never written).

Visible text, in characters, with control characters replaced by `?` in every daemon-supplied
string as today:

- `name` = `info.address`, or `session` when `info` is `None`.
- `left` = `" " + name + " "` followed by each present detail as `"│ " + text + " "`, in the
  order activity, quota, harness. The activity text is `"● " + word`. With `info` `None` there are
  no details.
- `pending` = `" {n} pending "` when `n > 0`, else empty.
- `hold` = `" HELD {reason} "` or, with the D6 hint, `" HELD {reason} · ^{L} r "`; empty when
  there is no hold.
- Fit: while `len(left) + len(pending) + len(hold) > width`, remove one thing per the D7 order and
  re-check. Then the existing rules: if `len(left) + len(hold) > width` and `len(hold) >= width`,
  truncate `hold` to `width` characters and clear `left`; if `len(left) + len(hold) > width` and
  `len(hold) < width`, truncate `left` to `width - len(hold)` characters. Padding is
  `width - len(left) - len(pending) - len(hold)` spaces between `left` and `pending`.
  The key hint is removed only through the D7 order, and details are never truncated (they are
  all removed before truncation can start).
- Styled spans under truncation: `left` is a character sequence made of spans (name in bold,
  separators, dot, quota text, plain text). When truncation cuts `left`, each span keeps its SGR
  open and close sequences around the characters that remain of it and is omitted entirely when no
  character of it remains (so a cleared `left` emits no `ESC[1m`). The same applies to the
  truncated alert text, which is one span.
- Update the existing `shortcut:` comment: widths are character counts; `●`, `│` and `·` are East
  Asian Ambiguous width and misalign on terminals that render them wide.

Bytes (all of them inside one cursor save and restore, as today):

```
PRE   = ESC 7, ESC [ {rows} ;1 H, ESC [0m, ESC [38;5;252;48;5;236 m
POST  = ESC [0m, ESC 8
FG    = ESC [38;5;252m                      (restores the bar foreground)
name  = ESC [1m {name} ESC [22m
sep   = ESC [38;5;240m │ FG                 (so "│" is dim, then FG)
dot   = ESC [38;5;{78|221|209}m ● FG
quota = ESC [38;5;{221|203}m {cell} FG       (as the detail text)
alert = ESC [0m ESC [1;37;41m {hold text}    (unchanged from today, then POST)
```

A detail is `sep` + `" "` + its text + `" "`. The pending text and the padding are plain bar text.
Everything else is as today.

## 4. Worked examples (these are the test expectations)

Name `agent-plan@host-a` (17 characters), so `" " + name + " "` is 19. Details: activity
`"│ ● working "` is 12, `"│ 5h 92% wk 12% "` is 16, harness `"│ claude "` is 9. `" 2 pending "` is 11.

`visible()` below means the bytes with `ESC 7`, `ESC 8` and every `ESC [` ... final byte
(`0x40..=0x7E`) sequence removed. `⎵` marks one space in a padding run, written as `{n}⎵`.

| # | info | cols (width) | visible text |
|---|---|---|---|
| E1 | activity working, harness claude, quota 5h 92 wk 42 (not low), pending 2 | 80 (79) | ` agent-plan@host-a │ ● working │ claude ` + 28 spaces + ` 2 pending ` |
| E2 | as E1 but quota 5h 92 wk 12 | 80 (79) | ` agent-plan@host-a │ ● working │ 5h 92% wk 12% │ claude ` + 12 spaces + ` 2 pending ` |
| E3 | as E2 | 60 (59) | ` agent-plan@host-a │ ● working │ 5h 92% wk 12% ` + 1 space + ` 2 pending ` |
| E4 | as E2 | 50 (49) | ` agent-plan@host-a │ ● working ` + 7 spaces + ` 2 pending ` |
| E5 | as E2 | 36 (35) | ` agent-plan@host-a ` + 5 spaces + ` 2 pending ` |
| E6 | as E2 | 28 (27) | ` agent-plan@host-a ` + 8 spaces |
| E7 | as E1 plus hold `human_draft`, prefix 2 | 100 (99) | ` agent-plan@host-a │ claude ` + 35 spaces + ` 2 pending ` + ` HELD human_draft · ^B r ` |
| E8 | as E7 | 50 (49) | ` agent-plan@host-a ` + 5 spaces + ` HELD human_draft · ^B r ` |
| E9 | as E7 | 40 (39) | ` agent-plan@host-a ` + 2 spaces + ` HELD human_draft ` |
| E10 | address and hold only, hold `human_draft`, prefix 2 | 20 (19) | ` ` + ` HELD human_draft ` |
| E11 | address and hold only, hold `human_draft`, prefix 2 | 8 (7) | ` HELD h` |
| E12 | address `agent-plan@host-a`, synthetic hold `channel_down` (never sent by a real daemon), prefix 2 | 60 (59) | ` agent-plan@host-a ` + 21 spaces + ` HELD channel_down ` (no hint) |
| E13 | address `a`, quota `limit_reached` only, activity idle | 40 (39) | ` a │ ● idle │ limit ` + padding to 39 |
| E14 | address `a`, quota 5h 15 (boundary, shown) | 40 (39) | ` a │ 5h 15% ` + padding to 39 |
| E15 | address `a`, quota 5h 16 (hidden) | 40 (39) | ` a ` + padding to 39 |
| E16 | address `a`, activity busy, no hold | 40 (39) | ` a │ ● resetting ` + padding to 39 |
| E17 | address `a`, activity working, hold `human_draft`, prefix 2 | 60 (59) | ` a ` + padding + ` HELD human_draft · ^B r ` (no activity detail while held) |

Arithmetic checks, so the developer does not have to trust them: E1 `40 + 28 + 11 = 79`; E2
`56 + 12 + 11 = 79`; E3 `47 + 1 + 11 = 59` (harness removed); E4 `31 + 7 + 11 = 49` (quota
removed too); E5 `19 + 5 + 11 = 35` (activity removed); E6 `19 + 8 = 27` (pending removed);
E7 `28 + 35 + 11 + 25 = 99` with the 25-character alert `" HELD human_draft · ^B r "`;
E8 `19 + 5 + 25 = 49` (harness, then pending removed); E9 `19 + 2 + 18 = 39` (hint removed);
E12 `19 + 21 + 19 = 59`; E13 left `3 + 9 + 8 = 20` (`"│ ● idle "` is 9, `"│ limit "` is 8), padding 19;
E14 left `3 + 9 = 12` (`"│ 5h 15% "` is 9), padding 27; E15 left 3, padding 36; E16 `"│ ● resetting "`
is 14, left 17, padding 22; E17 left 3, alert 25, padding 31. E9 and E10 walk the removal order:
a step that removes something absent (activity is already hidden while held) is skipped.

Byte-exact expectations (a few, not all):

- B1: `render("s1", None, 2, 40, 24)` is `PRE(24)` + `" "` + `ESC[1ms1ESC[22m` + `" "` + 35 spaces + `POST`.
- B2: `address "a"`, activity `Working`, nothing else, prefix 2, cols 40, rows 24 is `PRE(24)` +
  `" "` + `ESC[1maESC[22m` + `" "` + `ESC[38;5;240m│ESC[38;5;252m` + `" "` +
  `ESC[38;5;221m●ESC[38;5;252m` + `" working "` + 24 spaces + `POST`.
- B3: `address "a"`, synthetic hold `channel_down`, prefix 2, cols 40, rows 24 is `PRE(24)` + `" "` +
  `ESC[1maESC[22m` + `" "` + 17 spaces + `ESC[0mESC[1;37;41m HELD channel_down ` + `POST`.
- B4: the dot colors idle 78 and busy 209, and the quota colors 221 (low) and 203 (limit), each as
  a contains-the-SGR check on the matching case.
- B5 (truncation spans): E10's bytes are `PRE(24)` + `" "` + `ESC[0mESC[1;37;41m HELD human_draft `
  + `POST` (the name's bold span is omitted because no name character remains in `left`, which is
  one space); E11's bytes are `PRE(24)` + `ESC[0mESC[1;37;41m HELD h` + `POST`.
- Tiny terminals: `rows < 3` or `cols < 2` give an empty vector, as today.
- Control characters in the address, the reason and `session` become `?` as today (keep the three
  existing assertions' inputs, with new expected text).

## 5. Other files

- `src/wire.rs`: `StatusInfo` gains `#[serde(default)] pub harness: Option<Harness>`,
  `#[serde(default)] pub activity: Option<Activity>`, `#[serde(default)] pub quota:
  Option<QuotaInfo>`, after `hold`. All three types already derive `Serialize`, `Deserialize`,
  `Clone`, `PartialEq`, `Eq`. Update the comment in `ServerFrame::encode` that says `StatusInfo`
  contains only strings, a `u32` and an `Option`: it still cannot fail to serialize, say so with
  the new types.
- `src/daemon.rs` `attachment_status`: set `harness: Some(session.harness())`,
  `activity: session_activity(session)`, `quota: session_quota(session)`. Both helpers are in the
  same file; do not change them.
- `src/quota.rs`: add `pub fn cell(quota: Option<QuotaInfo>) -> String` moved verbatim from the
  private `quota_cell` in `src/main.rs`; `src/main.rs` deletes `quota_cell` and calls `quota::cell`
  at its one call site. No behavior change for `a2amx list`.
- `src/main.rs` `draw_status`: pass the client's prefix byte (the existing `prefix_byte` field of
  the attachment state) as the new `prefix` argument.
- `README.md`: the sentence about the status line (around line 151) lists what the bar shows
  now. `docs/architecture.md` "Status line": add one sentence on the new fields and that the daemon
  still sends only changes. `docs/backlog.md`: a Closed row `C9`, `Richer attach status bar`,
  `spec 3e`.

## 6. Tests (failing first, expected values are literals)

- `tests/status.rs`: rewrite the file's tests to the new signature and the section 4 cases
  (E1 to E17 through a small local `visible()` helper, and B1 to B4 byte-exact). Remove the old
  byte-exact strings; they encode the old styling.
- `tests/daemon.rs` `opted_in_attachments_receive_initial_and_changed_status_only`: the two
  whole-`StatusInfo` equalities become field checks: `address`, `pending`, `hold` as before,
  `harness == Some(Harness::Generic)`, `quota == None`, `activity.is_some()`. Do not compare
  `activity` to a literal. The fixture `cat` never enables bracketed paste, so its activity is a
  steady `working` (not ready) and the "no repeated status in 2.5 seconds" assertion stays valid;
  run that test three times to confirm it is stable.
- `tests/wire.rs` `status_frames_and_attach_opt_in_preserve_wire_compatibility`: add the three new
  fields to its literal, and add one assertion that an old payload
  `{"address":"a","pending":0}` (as a `0x04`-tagged frame) decodes with `hold`, `harness`,
  `activity` and `quota` all `None`.
- Unchanged and must still pass: `tests/attach_cli.rs`
  `status_line_toggles_and_shows_the_session_address` (the last row still contains
  `agent-plan@`; the name stays leftmost and is never dropped before the other parts).

## 7. Out of scope, and gate notes

Out of scope: a clock, session id, working directory, a general hint row, changing the poll rate,
the picker, scroll mode's row, `a2amx list`, colors beyond section 3.

Post-implementation commands (they need the new code, so they were not run at lock time):

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short   # only the scoped files
```

Gate notes, at `5d9ccde`: `rg "status::render"` hits `src/main.rs:909` and `tests/status.rs` only;
`rg "StatusInfo"` hits `src/wire.rs`, `src/daemon.rs`, `src/main.rs` (type import and field),
`tests/status.rs`, `tests/daemon.rs`, `tests/wire.rs`, all in scope. `quota_cell` has exactly one
call site (`src/main.rs:1437`) and no test references it. The other tests that read the status row
(`tests/attach_cli.rs:967`) only check the address substring. `Activity`, `QuotaInfo` and `Harness`
all derive `Eq` and serde. A baseline `cargo test` at `5d9ccde` passed (26 suites, 353 passed,
0 failed, 3 ignored). Section 4 was checked by hand arithmetic only.
