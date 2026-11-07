# Spec 3db: Claude readiness and the empty-composer rule accept Claude's own drawn cursor and its grey suggestion, and a stale `human_draft` hold clears at the reset gate

## 0. Status

LOCKED.

Scope (files that may be edited): `src/harness.rs` (the cursor test of `claude_ready`, one new pure function), `src/session.rs` (the stale-draft
rule and its two call sites), `src/daemon.rs` (`explain_of`, the `hold` line only), `tests/messaging.rs` (or
the existing test file that holds the Claude ready frames, the developer names it in the plan),
`tests/broker.rs`, `tests/explain.rs`. Does not touch: `src/messaging.rs` (`input_is_typing` stays as it is),
`src/hook.rs`, `src/delivery.rs`, the OMP rule's behavior (only the shape of its call sites changes; `explain` output for OMP and Codex sessions with a latch changes on purpose, section 2.3), the status line, `docs/` and `CHANGELOG.md` (the
architect updates them), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seams: `a2amx::harness::claude_composer_empty` (pure), the `reset_session`
request and `a2amx explain` through a real daemon with a Claude stub session.

## 1. Why

Several times in a few days `reset_session` on the Claude consultant was refused with `draft_present`,
`hold human_draft`, while its composer was empty. The owner reproduced it on purpose on rc.1: in an idle
Claude session with an empty composer they typed `/mcp`, Enter, Down twice, Up twice, Esc to close the view;
the next reset was refused, and `a2amx explain` printed `hold: none` next to `reset: refused (draft_present):
hold human_draft`. They cleared it with `^B r`.

Cause (read in code): `messaging::input_is_typing` (`src/messaging.rs:599`) counts every input byte except
focus events and mouse reports as typing, and `Session::note_human_input` (`src/session.rs:640`) then sets
`Hold::HumanDraft`. Only `note_submit` (the `UserPromptSubmit` hook path, `src/session.rs:660`) or `release()`
clears it. A Claude Code local command such as `/mcp` fires no such hook (the hook adapter handles only
`UserPromptSubmit`, `src/hook.rs:55`), so the latch stays with an empty composer. OMP does not have the
problem: `stale_omp_draft` (`src/session.rs:925`) clears the hold when the OMP extension reports `draft =
false` and the session is idle; Claude has no such report. The second defect: `explain`'s `hold:` line comes
from `AnyChannel::hold_reason()` (`src/daemon.rs:2316`), and `NativeChannel::hold_reason` always returns
`None` (`src/delivery.rs:198`; a channel event never touches the composer), while the reset gate reads the
session's own `state.hold` (`src/session.rs:956` and following). The two lines of one command disagree.

Second defect, found by probing a fresh default install (Claude Code v2.1.319, read-only render probe, owner,
2026-11-07; the three captures are in section 4): that session shows `ready: no (starting)` for good. The only
`claude_ready` condition that fails is `screen.cursor.visible`: Claude Code draws its own cursor, an inverse
(`INVERSE`) cell at the cursor position, and hides the terminal cursor, with a draft and without one. The tracked
cursor row and column still follow the composer. The consultant's session, with custom settings, keeps the terminal
cursor visible and is ready, so the two kinds of session coexist. While a session reads not ready, PTY delivery and
the reset gate hold it (native-channel delivery does not use `ready`). The same install shows a grey prompt
suggestion in an empty composer: the first character is `INVERSE` (Claude's cursor sits on it) and the rest of the
suggestion is `DIM`; typed text has no attribute and Claude's cursor is an `INVERSE` blank after it.

Decision (consultant advice, architect verified the code anchors): make the latch stale for a Claude
session when the screen shows an empty composer, and apply it exactly where OMP's rule applies, the reset
gate. It must never type over a real draft (AGENTS.md: never send input that destroys human work), so every
doubt keeps the hold. `claude_ready` (`src/harness.rs:184`) already requires bracketed paste, not scrolled, a
visible cursor (section 2.1 relaxes this to Claude's own cursor cell), the `❯` glyph then a space or U+00A0 in columns 0 and 1, a full `─` rule directly above and
directly below the cursor row, and no dialog marker; a screen that passes it has a one-row composer, so a
multi-line or wrapped draft cannot read as empty. A pasted-text or image chip (`[Pasted text #1 ...]`,
`[Image #1]`) is visible characters and keeps the hold.

Rejected, with reasons: ignoring navigation keys in `input_is_typing` (it does not fix the repro, because
`/mcp` is printable text; and Up in an empty composer recalls history into it, which creates a draft, so
arrows are not draft-neutral); treating Enter as clearing the latch (a backslash-Enter continuation adds a
line; Enter in a permission dialog or a completion menu leaves the draft in place); a hook for local commands
(Claude Code reports none that this code handles); keeping the hold for any placeholder text (the first draft's fail-safe choice, dropped: a probe shows an empty composer in a default install often carries a grey suggestion, which would leave the fix inactive there; the suggestion is now recognised by its attributes, and anything that does not match exactly keeps the hold).

## 2. Changes

1. `src/harness.rs`, two changes that share one cell test.
   a. `claude_ready` (line 184) replaces `!screen.cursor.visible` by a private `claude_cursor_shown(screen)`: true when
      `screen.cursor.visible`, or when the cell at the tracked cursor row and column has the `INVERSE` attribute
      (Claude's own cursor). Nothing else in `claude_ready` changes; a hidden cursor with no inverse cell at the
      cursor position still reads not ready. Add the attribute read next to `cell_char` (a `cell_attrs` helper, or
      the `Cell` itself, as the existing helper style allows).
   b. Add `pub fn claude_composer_empty(screen: &Screen, scrolled: bool) -> bool`: true only when
      `claude_ready(screen, scrolled)` is true, `screen.cursor.col == 2`, and the cursor row from column 2 to the last
      column matches one of two shapes: (blank) every cell is `' '` or `'\u{a0}'`, whatever its attributes (an
      inverse blank is Claude's cursor); or (suggestion) the cell at column 2 is not blank and has `INVERSE`, every
      other non-blank cell has `DIM`, and at least one non-blank cell has `DIM` (so a one-character draft with the
      cursor moved to the start, which is one inverse character and nothing dim, does not read as a suggestion).
      Anything else keeps the hold. The suggestion shape is also refused when the row text from column 2 starts with `[Pasted text #` or `[Image #` (a chip may be drawn dim; no capture exists). Leave a `// shortcut:` comment: a typed one-character prefix of the suggestion with the cursor moved to the start is the same cells as the suggestion, so one typed character can be lost; the upgrade trigger is a capture showing whether Claude keeps ghost text after a typed prefix. A draft made only of spaces with the cursor moved to the start reads empty (only whitespace is lost; accepted). It uses the existing helpers; no second copy of the glyph or rule checks.
2. `src/session.rs`: replace the OMP-only call sites with one rule that dispatches on the harness. Add
   `fn stale_claude_draft(state: &State) -> bool`: `state.hold == Some(Hold::HumanDraft)`,
   `state.restore.is_none()`, `state.last_human_input` is `None` or at least `STALE_DRAFT_MARGIN` old (the
   same constant, 2 s), and `harness::claude_composer_empty(&state.emulator.screen(),
   state.emulator.is_scrolled())`. Rename or wrap so that `clear_stale_omp_draft` (line 937; remove its early `if self.harness != Harness::Omp { return; }`, or the Claude branch never runs there) and the check inside `reset_gate_decision` (line 965) call one function, for example `fn draft_is_stale(&self, state:
   &State) -> bool`, that returns `stale_omp_draft(state)` for `Harness::Omp`, `stale_claude_draft(state)` for
   `Harness::Claude` and `false` otherwise. `reset_gate_reason` keeps clearing the hold (line 955) and the
   preview (`reset_gate_preview`) keeps ignoring it without clearing, as today for OMP. The PTY delivery path
   is not touched: nothing clears the latch there.
3. `src/daemon.rs` `explain_of` (line 2316): order of the `hold` line: `deliver_hold` first, then the channel's `hold_reason()`, then, when both are none and the session's own `state.hold` is `Some(h)`, `h`'s name (`delivery::hold_name`) with no explanation. The fallback applies to every channel that returns `None`: the Claude native channel, OMP (`src/delivery.rs:99`) and Codex (`:270`); on purpose, their `explain` output changes too when a latch is set (the OMP stale-draft preview test asserts only `info.reset`, `tests/explain.rs:476`, and no existing test asserts `hold: none` for a latched session). The explanation is omitted because `messaging::hold_explanation("human_draft")` says delivery resumes when the draft is submitted, which is false on a native channel and would contradict `reset: gate open`. A stale Claude latch that the reset gate would ignore is still shown as the hold (it is registered); the `reset:` line says the gate is open. Do not change the `ExplainInfo` wire shape. The PTY channel already returns `state.hold` (`src/delivery.rs:142`) and is unchanged.

## 3. Out of scope

`input_is_typing`, the hook adapter, delivery behavior over the native channel (a draft never blocked a channel
delivery and still does not), clearing the latch outside the reset gate, placeholder-text detection, the
status line's `HELD human_draft` display, OMP behavior, other harnesses, and any docs. Known and accepted: with the readiness relaxation, a hidden-cursor Claude session on the PTY channel now receives PTY deliveries (before, it was held for good); text that reaches the composer without attach input (a prompt Claude restores by itself) is not latched and a PTY delivery would paste over it, because `begin_delivery` checks `ready`, not `claude_composer_empty`; a visible-cursor session already has that exposure, so this is not a new kind of risk and the developer must not try to close it here.

## 4. Tests

Expected values are literal frames and strings, not recomputed with the code under test.
1. Pure: `harness::claude_composer_empty` over literal screens built like the Claude ready frames in the
   existing tests (the developer finds them: `rg -n 'claude_ready|Claude' tests/messaging.rs` and names the helper in the plan): (a) empty composer, cursor at column 2: true; (b)
   `❯ hello` with the cursor after it: false; (c) `❯ hello` with the cursor moved to column 2: false (the row is
   not blank); (d) a two-row composer: false; (e) scrolled: false; (f) a dialog marker visible: false; (g) typed text `xy` after the glyph with the cursor at column 2 on the `x` (`x` `INVERSE`, `y` plain): false; (j) the glyph, a blank row and the cursor at column 4 (a draft made of spaces): false; (h) the glyph followed by U+00A0 instead of a space with the rest
   blank: true; (i) bracketed paste off: false; (k) cursor hidden and an `INVERSE` blank at column 2, rest blank (an empty composer drawn the s5 way): ready true, empty true; (l) the s5 suggestion capture, cursor hidden, tracked cursor at column 2: `c` `INVERSE`, `heck`, `message_status`, `for`, `m_5806` letters `DIM`, the spaces between them plain: ready true, empty true; (m) the s5 draft capture `xyz`, cursor hidden, tracked cursor at column 5, `INVERSE` blank at column 5: ready true, empty false; (n) the s5 capture `\u{a0} xzya`-style draft that starts with a space, tracked cursor at column 7: ready true, empty false; (o) cursor hidden and no `INVERSE` cell at the tracked cursor position: ready false; (p) `x` `INVERSE` at column 2 and nothing else on the row, tracked cursor at column 2: empty false; (q) `x` `INVERSE` at column 2, `y` plain, `z` `DIM`: empty false; (r) `[` `INVERSE` at column 2 then `Pasted text #1 +4 lines]` all `DIM` (a chip that Claude might draw like a suggestion, not captured): empty false; (s) the same with `[Image #1]`: empty false; (t) a marker-less menu row `❯ 1. Yes` between two full rules, hidden cursor and no `INVERSE` cell at the tracked cursor: ready false. The existing `claude_readiness_rejects_each_non_ready_fixture` (`tests/messaging.rs:755`, its `f4` is a hidden-cursor frame whose cell (3,3) is not `INVERSE`) already is case (o) and stays unchanged; the helpers are `screen_from` (a 40x10 emulator, `tests/messaging.rs:655`) and `f1_bytes` (`:660`). Build the screens by feeding escape sequences (`ESC[?25l`, `ESC[7m`, `ESC[2m`, `ESC[0m`) into the public emulator, or by the existing literal-screen builder; the developer names the method in the plan.
2. `tests/broker.rs`, in the style of `reset_stale_omp_draft_hold_clears_after_margin` (line 1988): a `Harness::Claude` session whose stub draws the empty composer frame, enables bracketed paste, then runs `stty raw -echo` (as `tests/fixtures/fake_claude.sh` does: without it the tty echoes the input onto the composer row, the row is not blank and the gate stays refused on the new code too) and `exec`s `sleep` or `cat`. The test first waits for `explain` to show `ready: yes`, sends input that sets the latch through an attach (`Client` attach plus `ClientFrame::Input`, as at `tests/explain.rs:439`), then waits with `common::eventually` until `explain` shows `reset.reason == "hold human_draft"` (as `tests/explain.rs:457` does) before it asserts anything: `reset_rejection` (`tests/broker.rs:1598`) panics on a success, and a reset that races ahead of the input frame would succeed and paste `/clear`. Then it observes `reset_session` refused inside the margin, then accepted after the margin (measure the 2 s from the input) with the hold cleared. A second case: the stub draws `❯ draft` (not empty): the reset stays refused after the
   margin. Red on the old code: the first case is refused forever.
3. `tests/explain.rs` (a stub like `start_inferred_claude`, line 95, extended to draw the frame and run `stty raw -echo`; it must be a Claude session WITH the native channel, started through the CLI start path or with `A2AMX_CLAUDE_CHANNEL=1`, because a Claude session without the channel gets the PTY channel, whose `hold_reason` already returns the latch and the red phase would not exist; the input comes from an attach as in test 2, the CLI helpers alone cannot set the latch): with the latch set, `explain` shows `hold: human_draft` and, after the margin with the empty frame, `reset: gate open` (the text asserted at `tests/explain.rs:148` and `:165`). With a
   non-empty composer both lines say refused. Red on the old code for the `hold:` line (it printed `none`). A second case on a `Harness::Omp` session with a latch (reuse the fixture of `stale_omp_draft_preview_does_not_clear_the_hold`, `tests/explain.rs:354-479`; a generic session never gets a native channel, `src/delivery.rs:96-104`, so it would already be green) asserts the new fallback line and the absence of the explanation text.
4. A OMP regression: the existing OMP stale-draft tests keep passing unchanged.

## 5. Acceptance

1. `cargo test --test broker --test explain` and the file that holds test 1 pass; before the change the
   developer shows tests 1 (does not compile), the first case of 2 and the `hold:` assertion of 3 failing.
2. `rg -n 'stale_omp_draft|clear_stale_omp_draft' src` shows one dispatching function used by both call sites
   and no duplicated rule bodies.
3. Mutation check by restore-by-checksum edits in the checkout (the tree must equal the developer's own diff
   afterward): (a) drop the cursor-column test in `claude_composer_empty`, 1(j) fails; (b) drop the blank-row test, 1(c) and 1(g) fail; (c) drop the `scrolled` argument, 1(e) fails; (c2) replace the `INVERSE` test in `claude_cursor_shown` by `true`, 1(o) fails; drop it (visible cursor only), 1(k), 1(l), 1(m) and 1(n) fail; (c3) drop the `every other non-blank cell is DIM` requirement, 1(q) fails; (c5) drop the chip-prefix refusal, 1(r) and 1(s) fail; (c4) drop the `at least one DIM` rule, 1(p) fails; (d) drop the margin test in
   `stale_claude_draft`, the inside-margin refusal in test 2 fails; (e) drop `state.restore.is_none()`: `restore` is set only by `note_corrupted` on the hook path, which a broker stub cannot reach, so this mutant is expected to survive and the developer reports it as such; (f) make `explain_of` keep `hold: none`, test 3
   fails. The developer names the failing test for each.
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient
   environment of a hosted session. Known flakes I7, I14 (fixed by spec 3cx once it lands), I18: rerun once
   and record as pre-existing.
5. Live composer evidence (2026-11-06 and 2026-11-07, owner captures; the first is the idle `❯ ` composer of the architect's own session): the composer row is `❯ ` with nothing after the glyph, between two full `─` rules, status lines below; no placeholder text. The render probe added: s5 default install, suggestion showing: tracked cursor row/col on the composer at column 2, terminal cursor hidden, `c` `INVERSE`, rest of the suggestion `DIM`; s5 with `xyz` typed: tracked cursor at column 5, hidden, text plain, `INVERSE` blank at column 5; the same with a leading space: tracked cursor at column 7; the consultant (custom settings), composer empty: terminal cursor visible, row blank. Owner capture 2026-11-07, composer emptied (draft deleted, `^B r`): the suggestion came back at once, so an empty composer in this configuration carries the suggestion (same cells as the first capture, tracked cursor at column 2, terminal cursor hidden). NOT captured: an empty composer with suggestions turned off; case 1(k) (an `INVERSE` blank at column 2 and nothing else) is an assumption. If the real cell differs, that session reads not ready (readiness stays broken for a user who disabled suggestions) but nothing becomes unsafe; the owner captures it with suggestions off on the next rc, and the spec is amended if it differs. After acceptance, the architect repeats the owner's reproduction (`/mcp`, Enter, arrows, Esc on an idle
   consultant, then `reset_session`) on the installed build once the owner has installed the next rc; the
   spec is not closed until that passes. Scratch files stay under `target/scratch/3db/` and are removed
   before the report.
