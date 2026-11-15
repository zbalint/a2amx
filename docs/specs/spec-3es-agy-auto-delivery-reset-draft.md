# Spec 3es: `agy` delivers automatically by default and reset refuses a non-empty composer (spec 3b of the `agy` work)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `65e1ac2` or later.
Shared task `context_id`: `agy-harness`. Follows spec 3er; spec 4 (docs, team file) follows.

**Scope.** May edit exactly these files and no others.
- `src/harness.rs` (`default_deliver`)
- `src/session.rs` (`reset_gate_decision` and `begin_delivery`: one agy condition each)
- `tests/harness_infer.rs` (the agy default assertion), `tests/broker.rs` (reset and delivery tests, and the `agy_reset_target` helper signature); any other test file only for fixture bytes of a screen drawn for `--harness agy` that the new condition rejects (list them in the report)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `src/messaging.rs`
(`input_is_typing`), `src/daemon.rs`, `src/delivery.rs`, every other file and spec. Do not commit, stage or merge.

## 1. Why

Owner decisions (2026-11-15): `agy` delivers `auto` by default, and the two open items from spec 3er belong to
the work.

1. **Default.** Spec 3er made typed delivery safe: a delivery needs the visible composer shape and never answers a
   card; a human keystroke holds delivery and the hold is released automatically once the composer is empty.
   `Harness::default_deliver` still returns `Hold` for `Agy`. Change it to `Auto` (Claude, OMP, Codex are `Auto`).
2. **Reset over queued text.** Live: `reset` while agy held its own queued-message text in the composer (no a2amx
   hold, `ready` true) typed `/clear` after that text and submitted the merged prompt; the conversation was not
   cleared. `reset_gate_decision` only checks hold and `ready`. For agy the gate must also require the composer to
   be empty.
3. **Lone Esc (decision, no code).** A lone Esc counts as typing (`input_is_typing`, all harnesses) and sets
   `HumanDraft`. With spec 3er the hold is released automatically 2200 ms after the last input once the agy composer
   is empty, so Esc over a card no longer blocks delivery for good. `input_is_typing` stays as is: changing it
   would alter Claude, OMP and Codex. Rejected alternative: exclude a lone `0x1b` (an Esc in an unknown composer
   state may edit a draft).

## 2. `src/harness.rs`

`default_deliver`: `Self::Claude | Self::Omp | Self::Codex | Self::Agy => Deliver::Auto`; `Self::Generic => Deliver::Hold`.
`tests/harness_infer.rs` line 24: expect `Deliver::Auto` for agy.

## 3. `src/session.rs`: reset gate

In `reset_gate_decision`, after the `ready` check passes and before the native reason: when `self.harness ==
Harness::Agy` and `!harness::agy_composer_idle(screen, scrolled)`, return
`Some((messaging::code::DRAFT_PRESENT, "composer not empty".to_owned()))`. Other harnesses unchanged. A fresh agy
showing the placeholder is idle (spec 3er), so reset works on it.

Consequences to keep (not to route around): `reset_gate_decision` is shared by reset (before every step and in the wait
after each step), by graceful `kill` (`graceful_exit`) and by `explain`'s reset preview. An agy with text in its
composer therefore also skips the Ctrl-D Ctrl-D exit (it falls back to the non-graceful kill, which is right: Ctrl-D
over text is not an exit) and `explain` shows reset not allowed, `composer not empty`. The wait after `/clear` now
needs an idle composer; the screen agy draws after `/clear` is the fresh banner with an empty composer (tester, live,
3er), so it passes; Acceptance 4 checks it live.

`begin_delivery` (same function the reset steps use): for `Harness::Agy` the delivery also requires
`harness::agy_composer_idle`, evaluated before the paste (never at the recheck, which must tolerate the pasted text,
spec 3er). Why: with auto as the default, a composer holding agy's own text (a restored or queued message, no a2amx
hold) would get the envelope pasted after it and the merged prompt submitted, the same defect as the reset. A working
agy still shows an empty `>` composer, so messages sent while the model works are still typed and queued by agy.
A human draft is already held by `Hold::HumanDraft`. The test seam is the existing broker agy delivery tests.

## 4. Tests (`tests/broker.rs`)

1. Default: `agy_reset_target` takes an `Option<Deliver>` (no second helper); spawn agy with `None` and assert via
   explain that the hold is not `deliver_hold`, and that a message is submitted. `tests/harness_infer.rs` pins the value.
2. Reset on an agy screen showing `> queued text` (cursor column does not matter), no hold: rejected `draft_present`
   with text `composer not empty` (`reset_rejection_admin`), at step 1 before any delivery; the helper's child is
   `cat`, so "nothing typed" is shown by the rejection itself and by the mutant below.
3. Delivery over the same screen: a message stays pending (`not_ready` / not submitted); over an empty `>` composer it
   is submitted (exists). Reset on an idle empty composer still succeeds (existing tests). The fresh-placeholder
   composer is covered by the idle unit tests of spec 3er in `tests/messaging.rs`; add a broker case only if the
   helper can take a width of at least 72 columns without a new helper.
Mutation: remove the reset branch, test 2 fails; remove the `begin_delivery` condition, test 3 fails; revert
`default_deliver`, test 1 and `tests/harness_infer.rs` fail.

## 5. Out of scope

`input_is_typing`, docs/CHANGELOG (architect), the status-line `tool_confirmation_pending` signal, spec 4.

## 6. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
2. `git status --short` lists exactly the scoped files (four, plus any fixture-only test file named in the report).
3. The mutation check names a failing test per mutant; the tester reruns it.
4. Live (tester, real agy, at most 6 model turns, isolated root, uninstall the plugin afterwards): reset succeeds on a
   fresh agy and after a turn; a message sent to a fresh agy session with the default deliver mode is typed and
   answered; reset with queued or typed text in the composer is rejected `draft_present`; record which agy screen, if
   any, leaves queued text in the composer (Esc after a turn, a stopped turn) and what delivery does there.
   Residual: the first delivery can arrive before the plugin's tools are connected (Claude has the same race).

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review); developer OMP openai-codex/gpt-5.6-luna; tester OMP opencode-go/deepseek-v4.1-flash.
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | the tightened reset gate also changes graceful `kill`, `explain` and the wait after `/clear`; not stated | stated in the spec, live check added |
| Draft review | consultant | test seams wrong (`agy_reset_target` hard-coded `Deliver::Auto`, child is `cat`, helper width too small for the placeholder) | spec text corrected |
| Draft review | consultant | a composer holding agy's own text would also merge with an auto delivery (not only reset) | `begin_delivery` agy condition added |
| Implementation | developer | delivery regression asserted `pending` immediately and could pass before the retry loop ran | test waits 1100 ms first |
| Tester pass (live) | tester | collapsed `▸` queue rows above an empty composer let `reset` run; `/clear` cancels the in-flight turn and queued messages | not fixed here, backlog G15 |

Not a second consult after the `begin_delivery` addition: it reuses the 3er predicate and is covered by the live and broker tests.
