# Spec 3er: an `agy` session accepts typed input only while its composer box is showing (spec 3 of 4)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `cd1b070` or later.
Shared task `context_id`: `agy-harness`. Spec 3 of the `agy` work; spec 4 (docs, team file) follows.

**Scope.** May edit exactly these files and no others.
- `src/harness.rs` (`ready`, one new public function `agy_composer_idle` and its private helpers)
- `src/session.rs` (`draft_is_stale`, one new private `stale_agy_draft`, and Amendment 2: the stale-draft check at the start of `begin_delivery`, `agy` only)
- `tests/messaging.rs` (screen tests), `tests/broker.rs` (stale-draft and delivery tests, and the `claude_reset_target` helper signature)
- `tests/daemon_cli.rs` (Amendment 1: only the `ExitFixture` script and its argv)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `src/daemon.rs`,
`src/delivery.rs`, `src/confine.rs`, `src/main.rs`, `src/statusline.rs`, every other file and spec. Do not
commit, stage or merge.

**Public test seams.** `harness::ready(Harness::Agy, &Screen, scrolled)`, `harness::agy_composer_idle`, a real
daemon through `Client` (`tests/broker.rs`). Screens are built the way `screen_from` does in `tests/messaging.rs`
(feed bytes through `Emulator`).

## 1. Why

Live probes of `agy` 1.5.9 (tester, 2026-11-15, isolated a2amx root, real model calls):

- **Unsafe auto delivery.** `Harness::ready` for `Agy` is only `bracketed_paste && !scrolled`. With
  `--deliver auto`, a message typed while a permission card was on screen (`MCP` card, `Allow calling this tool?`,
  highlighted row `> 1. Yes, allow tool call`) was typed as the envelope plus Enter; the Enter answered the card
  with the default row (tool approved) and the message body was lost, yet the message was recorded `submitted`.
  A delivery must never answer a card. (Default delivery for `agy` is hold, so only an explicit `--deliver auto`
  or a team file is exposed today.)
- **Stuck human-draft hold.** After a person types once, `Hold::HumanDraft` is never cleared for `agy`
  (`draft_is_stale` is `false`), even when the composer is empty again; only the manual release (prefix, then `r`)
  clears it. An `agy` team member then stops receiving.
- **No safe signal exists in a2amx today.** OSC 7501 (`state=blocked`) is emitted only when `AGY_CLI_OSC7501=on`
  is set, which a2amx does not set; the status-line payload carries `agent_state` and `tool_confirmation_pending`
  but only on render ticks (a later spec may use it as a second signal).

Decision: a whitelist of the composer's shape, plus a separate emptiness test. `ready` for `agy` is true only when
the screen positively shows the composer box (a block of rows between two full-width rules, with a visible cursor
inside it); a card, dialog, blank or unrendered screen is not ready, so nothing is typed into it. This follows the
Claude pair `claude_ready` (shape) and `claude_composer_empty` (emptiness, used only to release a stale draft hold).
`ready` must accept a composer that already holds the pasted envelope, because a delivery pastes, waits, then
re-checks `ready` before it sends Enter (`Delivery::recheck`, `src/session.rs:1407-1430`); an emptiness test in
`ready` would fail that recheck for every delivery and every `/clear` reset. Rejected: a blacklist of known card
texts (the file-edit card was not captured); setting `AGY_CLI_OSC7501` and parsing OSC (an unprobed side channel);
using the status-line `agent_state` alone (not delivered while idle); making `ready` an emptiness test (breaks the
recheck); refusing delivery while the model works (agy's own queue, `queuedMessages: queue`, holds the message and
submits it after the turn; measured).

Measured fixtures (120x40; `alt_screen` false; bracketed paste on). Rows below are exact; `R` is the composer row.
`RULE` is a full-width row of U+2500.

| State | Rows (relative to R) | Cursor |
| --- | --- | --- |
| Fresh start | R-1 `RULE`; R `> Auto mode: edits & safe commands auto-approved (shift+tab to cycle)`; R+1 `RULE` | row R, col 2, visible |
| Plan mode, fresh | R `> Plan mode: research & plan only (shift+tab to cycle)` | row R, col 2, visible |
| Idle after a turn | R-1 `RULE`; R `>`; R+1 `RULE`; below it the status-line text | row R, col 2, visible |
| One-line draft | R `> one line draft` | row R, col 16 |
| Multi-line draft | R `> line one`; R+1 `  line two`; R+2 `  line three`; then `RULE` | last line |
| Working | R-2 `⣷  Working...`; R-1 `RULE`; R `>` (spinner glyph rotates, text varies: `Generating...`, `Running command`, `Calling MCP tool`) | row R, col 2, visible |
| Working with a queued message | R-3 `⣷  Working...`; R-2 `▸ queued while working (alt+m to send immediately)`; R-1 `RULE`; R `>` | row R, col 2, visible |
| MCP card | `MCP`, `RULE`, tool name, `Allow calling this tool?`, `> 1. Yes, allow tool call`, ...options..., `  ↑/↓ Navigate · tab Amend` | hidden; no composer box |
| Shell card | `Command`, ..., `Run this command?`, `> 1. Yes, run command`, ... | hidden |
| Trust dialog | `Do you trust the contents of this project?`, `> Yes, I trust this folder`, `  No, exit` | hidden |
| Slash menu | R `> /`; below the bottom rule `> /add-dir ...` | row R |

Paste states (tester round 4; a2amx pastes the envelope without Enter, then re-checks): the composer grows with the
pasted lines (first row `> `, further rows two spaces; both full-width rules stay). A line longer than 117
characters wraps (a composer row is at most 119 cells: 2-character prefix plus 117 characters); every wrapped
continuation row starts with the same two spaces as a new logical line, at a word boundary where one exists (the
break consumes one space), at the 117th character otherwise. A real envelope wraps: its opening tag line (id,
from, subject) and the operator sentence of about 330 characters both exceed 117, a small paste is never collapsed, a paste of about 16 lines or more, or one line of about
1000 characters or more, collapses to the single row `> [Pasted text #N +L lines]` or `> [Pasted text #N C chars]`
(default attributes); `/clear` shows `> /clear` and a menu row below the bottom rule; pasting while the model
works is accepted (composer `> hello`, rules intact, spinner row above the top rule); pasting while a card is on
screen is buffered by agy, shows nowhere, leaves the card and the hidden cursor unchanged, and appears in the
composer after the card is cancelled. Redraw after a paste lands within about 25 ms.

Facts used: the prompt-echo box of a submitted prompt has 60-column rules (not full width); the composer's rules
are full width; the composer row index is not fixed; with a status line configured (a2amx always configures one)
`agy` draws the status-line text as the last row; a blank screen after an attach is possible until the next repaint.

## 2. `src/harness.rs`: composer shape, emptiness, `ready`

1. Private `fn agy_composer(screen: &Screen) -> Option<(u16, u16)>`: the rows `(top, bottom)` of the composer's
   two rules, or `None`. Conditions: `screen.cursor.visible`; the cursor row `R` has a rule row `top < R` as the
   nearest full-width rule above it and a rule row `bottom > R` as the nearest rule below it (`row_is_rule`:
   every cell `─`, so the 60-column prompt-echo box never matches), with no more than 40 rows between them;
   row `top + 1` has `>` in column 0 and a space or end of content in column 1; every row from `top + 2` to
   `bottom - 1` starts with two spaces (an empty row counts). `None` on a screen with no such block (a card, a
   dialog, a blank screen).
2. New `pub fn agy_composer_idle(screen: &Screen, scrolled: bool) -> bool` (emptiness, the analogue of
   `claude_composer_empty`): `ready(Harness::Agy, ..)` holds, the composer has exactly one row
   (`bottom == top + 2`), the cursor is on it at column 2, and the text from column 2 on is blank or exactly one of
   the two placeholders `Auto mode: edits & safe commands auto-approved (shift+tab to cycle)` /
   `Plan mode: research & plan only (shift+tab to cycle)` followed only by blanks. Everything else (any draft text, several rows) is `false`.
3. `ready`: the `Harness::Agy` arm is `screen.modes.bracketed_paste && !scrolled && agy_composer(screen).is_some()`
   (split out of the `Generic | Omp | Codex` arm; the other arms stay).
4. A `// shortcut:` comment above `agy_composer` states the ceilings: (a) a card that appears between the recheck
   and the Enter of a delivery is not seen (a window of a few milliseconds); upgrade by also requiring the
   status-line `tool_confirmation_pending` (not delivered while idle); (b) only the two known placeholders count
   as empty on a fresh screen (an `agy` in another mode is not "empty" until its first turn, which only delays the
   stale-draft release); (c) the composer is found around the visible cursor, so an `agy` that hides its cursor
   while idle would never be ready (measured: visible in every idle state).

## 3. `src/session.rs`: stale human-draft release

`draft_is_stale`: the `Harness::Agy` arm calls a new private `stale_agy_draft(state)` (move `Agy` out of the
`Generic | Codex | Agy` arm). `stale_agy_draft` is `stale_claude_draft` with `harness::agy_composer_idle` in place
of `harness::claude_composer_empty`: `hold == Some(Hold::HumanDraft)`, `restore.is_none()`, last human input older
than `STALE_DRAFT_MARGIN` (or none), and the composer is empty. To avoid copying the condition, extract the shared
part (`hold`, `restore`, margin) into one private helper if that is smaller than a duplicate; otherwise duplicate
with the one differing call.

## 4. Behaviour consequences (state them in the report, no extra code)

- Delivery and reset: a delivery pastes into a visible composer (any content), re-checks, and sends Enter; with a
  card, dialog or blank screen `ready` is false, so a new delivery is not started and an in-flight one ends
  `Unsubmitted` with the existing `unsubmitted_envelope` hold (text may sit buffered in agy; measured).
  The `/clear` reset step works on an idle composer.
- A message delivered while the model works is typed into the composer and queued by agy; this is unchanged.
- A human draft (any composer content) is still caught by `Hold::HumanDraft` from human input, as for Claude; the
  stale release now applies to `agy` once the composer is empty again.
- `Session::starting()` still excludes `Agy` (spec 3eq pins it). `session_working` / `session_activity_cause`
  (`src/daemon.rs`) use `ready`: an `agy` showing a card or a dialog reads as not ready/working in status and the
  picker, which is right.
- A session whose composer never matches stays pending (`not_ready`); the human can release or deliver by hand.

## 5. Tests (write first)

`tests/messaging.rs`: a 120x40 variant of `screen_from` (the existing helper is 40x10); bytes enable bracketed paste
(`\x1b[?2004h`) and the cursor (`\x1b[?25h`/`\x1b[?25l`) as needed; rules are 120 `─`. Assert `ready(Agy, ..)` and,
where stated, `agy_composer_idle` (both; the new function does not exist on the old code, so tests that call it
fail to compile there: put the `agy_composer_idle` assertions in their own tests and keep the `ready(Agy)`
assertions in separate tests so the red/contract split below is checkable).
1. `ready` true: a five-line pasted envelope whose first line is 162 characters and second 329 characters (rows of
   113, 47, 110, 115 and 102 characters after the two-space prefix, then three short rows, `> ` on the first row
   only, the closing rule full width, cursor at the end of the last row); idle composer (rule, `>`, rule, status text; cursor row R col 2) with R at 9, 25 and 37; fresh
   start with each placeholder (cursor col 2); a composer holding a one-line draft; a six-row draft; the collapsed
   `> [Pasted text #1 +40 lines]`; a pasted envelope (five rows); `> /clear` with a menu row below; working (spinner
   row above the top rule, `>` composer); working with a queued `▸` row above the rule.
2. `ready` false: MCP card (cursor hidden, `> 1. Yes, allow tool call`, no composer pair, header rule present);
   shell card; trust dialog; a screen where the cursor is visible but the only full-width rules belong to a card
   header (no `>` row after the top rule); blank screen (bracketed paste on); bracketed paste off; scrolled;
   cursor hidden on an otherwise idle composer; the 60-column prompt-echo box without a composer pair; a composer
   whose second row does not start with two spaces.
3. `agy_composer_idle` true: idle composer; fresh start with each placeholder; false: one-line draft, six-row
   draft, collapsed paste placeholder, placeholder text with the cursor at the end, a one-character change in a
   placeholder, working composer with draft text, card, blank screen.
`tests/broker.rs`: extend `claude_reset_target` (line ~1674, three callers) with a harness, a glyph and a post-input
output string (instead of a sibling helper), or add one small sibling helper for the redraw cases below; the 40-column
fixture has the rule pair at rows 1 and 3, `>` at row 2, cursor at col 2 (R >= 2 holds at row index 2).
`wait_claude_ready` and `wait_claude_hold` are not Claude-specific and are reused. Cases:
1. Stale draft clears: static child (`cat >/dev/null`, no redraw). A keystroke through an attachment sets
   `human_draft`; reset is rejected `draft_present` before the margin; after `Duration::from_millis(2200)` the hold
   clears and reset succeeds. Red on the old code.
2. A fixture whose composer shows `> draft` stays `draft_present` after the margin.
3. Delivery whose recheck screen holds the pasted text is `submitted`: the child prints the idle composer, waits for
   its first input byte (`head -c 1 >/dev/null`, the paste's opening bytes), then redraws the composer with
   pasted text (`printf '\033[3;3Hpasted envelope\033[3;18H'`) and `exec cat >/dev/null`; the redraw lands long
   before the paste gap (`PASTE_GAP`, `src/messaging.rs:49`). A mutant requiring an empty composer in `ready`
   ((d)) makes it `unsubmitted_envelope`.
4. Delivery whose recheck screen has no composer is `unsubmitted_envelope`, not submitted: the same child redraws
   a card instead (`printf '\033[2J\033[?25l'` and card rows). Mutant (a) reports `submitted`. This is the 7.4 abuse
   case, red on the old code.
How the test sends the message and reads its state follows the existing broker auto-delivery tests (the developer
names the helper used).

Classification on `cd1b070` (the `ready(Agy)` tests compile there): red: the card, shell card, trust dialog,
card-header-rule, blank screen (with bracketed paste on), cursor-hidden, echo-box and non-two-space cases of 5.2,
and the broker stale-draft clearing. Contract (green on the old code): every case of 5.1 and the scrolled and
paste-off cases of 5.2. The `agy_composer_idle` tests of 5.3 are new behaviour (no old-code run).

Mutation check (the developer, in the checkout, `sha256sum` restore): (a) `ready` for `Agy` back to
`bracketed_paste && !scrolled`; (b) accept a missing top rule (use row 0 as `top`), killed by the 5.2 case `> 1. Yes, allow tool call` row with the cursor visible, a full-width rule below it and none above; (c) drop `cursor.visible`; (d) require an
empty composer in `ready` (must fail 5.1 draft cases and the broker submitted-delivery case); (e) drop the
two-space continuation check; (f) `draft_is_stale` Agy arm back to `false`; (g) any non-blank text counts as empty in
`agy_composer_idle` (fails the draft negatives) and (g2) placeholders no longer count as empty (fails the placeholder
positives); name the failing test for each.

## 6. Out of scope

OSC 7501, `AGY_CLI_OSC7501`, the status-line `agent_state` / `tool_confirmation_pending` as a second signal (a
later spec, only if a gap shows); changing agy's default delivery from hold to auto; answering or detecting cards
by name; the file-edit card (not captured); scroll keys of agy (not found); `src/delivery.rs` and
`src/daemon.rs` (their delivery gate calls `ready`, which now checks shape, as for Claude);
documentation (spec 4: the idle-composer rule, the placeholder limit, `submitted` means "bytes typed").

## 7. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
2. `git status --short` lists exactly the five scoped files (Amendment 1 added `tests/daemon_cli.rs`).
3. Red and contract tests as classified in section 5; the mutation check names a failing test per mutant (the
   tester reruns it and adds its own).
4. Trust boundary (abuse-case tests, red on the old code): the card, shell-card, trust-dialog and card-header cases
   of 5.2 are not ready, so no delivery is typed into them; the broker test shows a delivery whose recheck screen
   has no composer ends `unsubmitted_envelope`, not submitted (mutant (a)).
5. Live check (tester, after implementation; the architect decides): an a2amx `reset` of the `agy` session
   succeeds and clears the conversation; an isolated daemon, an `agy` session started
   with `toolPermission: request-review` and an allow list; a message sent with `--deliver auto` while a card is
   on screen stays pending and the card is untouched; after the card is cleared the message is typed and
   submitted and the model answers; a message sent while the model works is queued by agy and answered; after a
   human types once and the composer is empty again, the next message is delivered with no manual release.

## 8. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `ready` for `Agy` is bracketed paste and not scrolled; `starting()` excludes `Agy` | `src/harness.rs:285-291`, `src/session.rs:587-591` | read |
| `draft_is_stale` returns false for `Agy`; Claude/OMP variants exist | `src/session.rs:985-1003` | read |
| `row_is_rule` requires every cell `─` across the full width | `src/harness.rs:139-149` | read |
| `ready` is used by restore (749, 771), the reset gate (1041), the delivery start (1154), the post-paste recheck before Enter (1416) and the readiness latch (575) | `src/session.rs` | grep, consultant read |
| Tests that already use `Harness::Agy` do not depend on agy readiness | `rg Harness::Agy tests`: `daemon.rs:891` (sleeping command, never ready), `harness_infer.rs`, `statusline.rs`, `agy_*`, `confine.rs` | grep |
| Card text, composer rows, rules, spinner, queued row, cursor visibility | tester round 2 and 3 reports (live `agy` 1.5.9, replay through `a2amx::emulator::Emulator` at 120x40) | tester measurement, not rerun by the architect |
| Auto delivery over a card approves it | tester round 2 (m_2 on card, `messages` shows `submitted`, body absent) | tester measurement |
| `claude_composer_empty` tests and `screen_from` live in `tests/messaging.rs`; the stale-draft test is in `tests/broker.rs` | `tests/messaging.rs:707, 876`, `tests/broker.rs:2247` | read |

## Amendment 1 (architect, after the developer's BLOCKED on the full suite)

The graceful-kill gate (`kill`, `tests/daemon_cli.rs` `ExitFixture`) uses `ready`; section 5 and the consultant review
wrongly said it does not. `kill_sends_agy_two_key_exit_sequence` failed because the fixture draws the Claude
composer (`❯` plus NBSP) for the agy case. Amendment: scope adds `tests/daemon_cli.rs`, limited to the `ExitFixture`
script and its argv. For harness `agy` the fixture draws the agy composer: the same rules at rows 3 and 5, row 4 is
`> ` (ASCII `>`, a space), cursor at column 3 (1-based), visible; other harnesses unchanged byte for byte; the
`not-ready` mode still draws no composer. Acceptance adds: that test passes and the other `ExitFixture` tests stay
green and unchanged. Lesson: grep tests for every caller of the changed predicate, including fixtures that draw a
screen for `--harness agy`, before locking.

## Amendment 2 (architect, after the tester's BLOCKED and live finding F1)

Section 3 released a stale draft only on the reset path (`reset_gate_reason`), so a pending message stayed held
(`human_draft`) after one human keystroke, or an Esc that dismissed a card, even once the agy composer was idle and
empty past `STALE_DRAFT_MARGIN`; only a manual Release delivered it. This contradicted section 4 ("the stale release
now applies to `agy`") and the live acceptance check. Amendment: scope adds the start of `begin_delivery`
(`src/session.rs`): under the existing state lock, when `self.harness == Harness::Agy` and
`self.draft_is_stale(&state)`, set `state.hold = None` before the hold test. Other harnesses are unchanged (their
delivery path does not consult the stale predicate; making it do so is a separate decision). A lone Esc still counts
as typing in `input_is_typing` (all harnesses, unchanged, backlog). Acceptance adds: a broker test where one attached
keystroke is followed by an idle empty composer for longer than the margin and a message then submits with no
Release; a `> draft` composer stays held; mutation: remove the release in `begin_delivery`, a test fails. Process
lesson: a spec that adds a release predicate must name every path (reset, delivery) that consults it.
