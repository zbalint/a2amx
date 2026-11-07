# Spec 3dd: Claude readiness accepts an empty composer whose cursor block is not drawn, as when the terminal has no focus

## 0. Status

LOCKED (architect, 2026-11-07), after the consultant's pre-lock review (`m_5957`, one round, all findings applied). Section 3's trade-off was decided by the architect under the owner's delegation; the owner confirms on return.

Scope (files that may be edited): `src/harness.rs` (`claude_cursor_shown` only), `tests/messaging.rs` (two existing cases change, new cases are added). Does not touch: `claude_ready`'s other conditions, `claude_composer_empty` (its blank-row branch already accepts this shape), `src/session.rs`, `src/delivery.rs`, `src/daemon.rs`, the attach client, any input sent to a session, `docs/` and `CHANGELOG.md` (the architect updates them), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id `a2amx-rc2-2026-11-06`. Public seam: `a2amx::harness::ready` and `a2amx::harness::claude_composer_empty` (both pure).

## 1. Why

On a default-settings Claude Code (v2.1.319, a fresh `CLAUDE_CONFIG_DIR`) `a2amx explain` reads `ready: no (screen not ready)` and `reset: refused (not_ready)` for long stretches while the composer is empty. Spec 3db made `claude_cursor_shown` accept Claude's own drawn cursor (an `INVERSE` cell at the tracked cursor) in place of the terminal cursor. The probe now shows Claude stops drawing that cell too.

Evidence (probe session `probe-claude`, owner and architect, 2026-11-07; events a9bdfbbf, 71c08f12, a55d10f8, 0cafbca8, 2155e510 in the context above):
- Detached after the owner's clean `Ctrl-B d`: tracked cursor (19,2) then (12,2), terminal cursor hidden, the cell at the cursor `INVERSE` (attrs `0x0010`): ready.
- Owner attached: `explain` said `ready: no` on every sample for minutes; the screen text was identical.
- Forced takeover (the probe's attach, which sends no focus report), then detached: tracked cursor (12,2), terminal cursor hidden, the cell at the cursor attrs `0x0000` (no `INVERSE`), composer row `❯` U+00A0 then blanks between two full rules: not ready, 24 of 24 samples over 2 minutes plus 16 over 6 s.
- Focus capture: with the owner attached and the readiness logged every 0.4 s, ready was yes while the terminal window had focus (19:30:33-19:30:49, 19:31:17-19:31:36 on 2026-11-07) and no while the owner's window was unfocused (19:30:49-19:31:12), and yes after a clean detach.
Cause (consultant `m_5949`, supported by the timing, not by reading Claude's source): the daemon forwards the attached terminal's focus reports (DECSET 1004) to Claude, and Claude draws its `INVERSE` cursor only while the terminal has focus. The cell attribute itself cannot be read while a client is attached (the probe refuses), so the link between focus and the attribute is inferred from the readiness timing.

The consultant's recommended change (and why not the alternatives), architect verified the anchors in `src/harness.rs`:
- `claude_cursor_shown` (line 112) gets a third accepted case, ORed after the two existing ones: the tracked cursor is at column 2 and every cell from column 2 to the last column of that row is `' '` or `'\u{a0}'`. The clauses "terminal cursor hidden" and "no `INVERSE` cell at the cursor" are left out on purpose: after `visible || INVERSE at the cursor` they narrow nothing, and writing them would only create a mutant that cannot be killed. The other conditions of `claude_ready` (bracketed paste, not scrolled, the glyph then a space or U+00A0 in columns 0 and 1, a full `─` rule above and below, no dialog marker) are unchanged.
- Removing the cursor test altogether was rejected: a menu row such as `❯ 1. Yes` between two full rules, with a hidden cursor and no `INVERSE` cell, passes every text check in `claude_ready`; only the cursor test keeps it not ready (spec 3db case 1(t)). The new case keeps that row not ready because the row has text after the glyph.
- Sending a focus-in report to the session on detach or takeover was rejected: it is synthetic input to the session, it does not cover an attached terminal that has no focus, and it fakes the harness's state.
- Treating the cursor block as an idle-versus-busy signal does not apply: Claude Code keeps drawing the composer while a turn runs and queues input sent mid-turn (docs/delivery.md, and the Spec 2 probe, memory adf00b22 findings 3 and 7), and `claude_ready` has no busy test.

Menu screens checked against this rule (probe on `probe-claude`, 2026-11-07, typed into the throwaway session): the `/` command picker (the composer row keeps the typed `/` at column 2, so the row is not blank), the `/mcp` view and the `/model` picker (the selected row is `  ❯ ...`: column 0 is a blank, the `❯` sits in column 2, and `claude_ready`'s glyph test already rejects them: `glyph_col0=false`). `/config` and `/help` were not captured (an earlier attempt typed them as chat prompts); the same shape is assumed.

## 2. Changes

1. `src/harness.rs`, `claude_cursor_shown` (line 112): return true also when `screen.cursor.col == 2` and all cells of the cursor row from column 2 on are `' '` or `'\u{a0}'` (no condition on `cursor.visible` or `INVERSE`: they are already covered by the first two cases). Reuse the existing cell helpers; do not copy the blank-row loop of `claude_composer_empty` (line 220): extract one private helper that both call, or call one from the other. Add a `// shortcut:` comment only if a corner is cut on purpose.
2. `tests/messaging.rs`: `claude_readiness_rejects_each_non_ready_fixture` (line 804): `f4` (the idle composer with the terminal cursor hidden by `ESC[?25l`) now reads ready, so it moves out of the rejected list into a positive assertion (a new test `claude_readiness_accepts_an_empty_composer_with_a_hidden_cursor`, ready true and empty true). The hidden-cursor-without-`INVERSE` case at lines 957-958 (`hidden_no_inverse`, the `❯ ` composer with `ESC[?25l` and no inverse cell) flips the same way: its `ready` assertion and its `claude_composer_empty` assertion both become true. That is two cases and three assertions (`f4` ready, line 958 ready, line 958 composer empty). The flips are the point of this spec, not a regression; the developer states them in the plan and in the report.

## 3. Behavior change the owner must accept

An attached session whose terminal has no focus, with an empty composer, now reads ready. Before, it read not ready by accident (the cursor block was missing). Every behavior that `ready` gates becomes live for such a session (consultant review `m_5957`, each site read at the cited lines):
- PTY delivery (`begin_delivery`, `src/session.rs:1136`; `src/delivery.rs:148`) and draft restore (`src/session.rs:731`, `:753`): both type into the composer, so text can now be pasted into an unfocused window. Native-channel delivery never typed into the composer and does not change.
- Reset: `reset_gate_decision` (`:1023`) no longer returns `not_ready` for it. `stale_claude_draft` (`:973`) calls `claude_composer_empty`, which is now true on this shape, so a stale `human_draft` hold of an unfocused session clears at the reset gate after `STALE_DRAFT_MARGIN`, which is spec 3db's intent.
- `kill`'s graceful Ctrl-D (`:1287-1296`, through `reset_gate_reason`): before, an unfocused session skipped it with `not_ready`; now it gets the 0x04.
- Watcher heartbeat digests (`src/daemon.rs:865`, `:1010`, `watcher_may_receive`): an unfocused watcher now receives them.
- Status display: `observe_ready` latches `ready_latched` (`:568`), so `starting` ends sooner; `session_working` and `session_activity_cause` (`src/daemon.rs:2679`, `:2691`) treat not-ready as working, so an unfocused idle session now shows idle where it showed working, and `explain`'s `ready:` line (`:2429`) says yes.
Unchanged, and worth knowing: if focus is lost during a PTY delivery, the recheck (`:1394`) sees the pasted text on a row with no `INVERSE` cell, reads not ready and puts the session on `Hold::UnsubmittedEnvelope`, before and after this fix; the fix covers only the empty composer. An unfocused composer that holds text still reads not ready, because the row is not blank.

This is the same exposure a visible-cursor session already has, and spec 3db accepted a hidden-cursor session receiving PTY deliveries (its section 3).

Owner decision: the owner was asked on 2026-11-07, went away before answering, and delegated ("you are working on my behalf"); the architect took the recommended option (accept; no attach-state logic, which would need the session to track attach state) and logged it as a decision (event 99d859a4). The owner confirms or reverses on return.

## 4. Out of scope

`claude_composer_empty` and the stale-draft rule, anything that sends input to a session, the attach client's focus forwarding, the status line, OMP and other harnesses, the `/config` and `/help` screens, an empty composer with suggestions turned off (not captured), docs.

## 5. Tests

Expected values are literal frames, built with the same escape-sequence method as the cases at `tests/messaging.rs:918-1045` (`rules` then `❯ ` rows).
1. (a) Hidden cursor, no `INVERSE` anywhere, `❯` then a space, blank row, tracked cursor at column 2 between two full rules: `ready` true and `claude_composer_empty` true (this is the old `f4` and the old line-958 case). (b) The same with U+00A0 after the glyph: both true. (c) Hidden cursor, no `INVERSE`, row `❯ x` (plain `x`), tracked cursor at column 2: ready false. (d) Hidden cursor, no `INVERSE`, blank row, tracked cursor at column 4: ready false (the cursor is not at the composer start). (e) Same as (a) but scrolled: false. (f) Same as (a) with a dialog marker visible: false. (g) Same as (a) with bracketed paste off: false. (h) The marker-less menu row `❯ 1. Yes` (line 1042): ready false. (i) a regression guard that never reaches the new branch (the `❯` is in column 2, so the row is not blank): the `/mcp` shape: a blank in column 0, `❯` in column 2 on a row between rules, hidden cursor at column 2: ready false. (j) The `/` picker shape: `❯ /` with the cursor at column 3, hidden: ready false. (k) A blank row with one plain character near the right edge (column 38 of 40), tracked cursor at column 2, no `INVERSE`: ready false (kills an end-bound off-by-one in the shared helper). (l) The suggestion frame of `tests/messaging.rs:985` with the `ESC[7m` on its first cell removed (plain `c`, then the dim words), terminal cursor hidden, tracked cursor at column 2: ready false. This documents a residual: an unfocused composer that still shows a grey suggestion stays held; whether Claude keeps drawing the suggestion while unfocused is not confirmed (the probe's default install showed blanks).
2. All other existing Claude readiness and composer cases keep their results; the two flipped cases (three assertions) are the only changes.

## 6. Acceptance

1. `cargo test --test messaging` passes; before the change, test 1(a) and 1(b) fail (red), and with the old `f4` and line-958 assertions left in place those two fail after the change (so the flip is deliberate).
2. Mutation by restore-by-checksum in the checkout: (a) drop the `col == 2` test in the new branch: 1(d) fails; (b) drop the blank-row test: 1(c) fails; (c) make the new branch unconditional: 1(c), 1(d), 1(h) and 1(j) fail; (b) is also killed by 1(h); (a) is killed by 1(d) because the blank check starts at column 2, not at the cursor; shift the blank check's end bound by one: 1(k) fails. No equivalent mutant is expected.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient environment of a hosted session. Known flakes I7, I14, I18: rerun once and record as pre-existing.
4. Live: on the installed build with the fix, `probe-claude` (default config) attached and the window unfocused reads `ready: yes`; the `/mcp` + arrow keys + Esc reset on an idle session still passes. Scratch files stay under `target/scratch/3dd/` and are removed before the report.
