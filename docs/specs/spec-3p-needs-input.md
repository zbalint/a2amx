# Spec 3p: a needs-input warning for sessions stuck on a prompt

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `9dc715a`; gate notes in section 10;
consultant review `m_764`).
Follow-up idea 5 from the consultant triage `m_733` (context `a2amx-followups`) and backlog item
I4, approved by the owner ("all the ideas sound good, work them out"). Architect decisions are in
section 2. Independent of specs 3m, 3n and 3o; it edits the same listing code as spec 3l, which is
already committed (`9dc715a`).

**Baseline:** develop at `9dc715a`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-needs-input`.
Public test seams: `harness::needs_input` (pure, `tests/harness_infer.rs` style on a built `Screen`),
the daemon `List` and attach status through the admin client with a fake Claude and a fake Codex
(`tests/daemon.rs`, `tests/codex.rs`), and `status::render` (`tests/status.rs`).

**Scope.** May edit exactly these files and no others.

- `src/harness.rs`, `src/session.rs`, `src/daemon.rs`
- `tests/harness_infer.rs`, `tests/daemon.rs`, `tests/codex.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/codex.rs`, `src/wire.rs`, `src/status.rs`, `src/main.rs`, `src/cli.rs`, `src/mcp.rs`,
`src/delivery.rs`, `src/messaging.rs`, every other test file, every spec document, `AGENTS.md`,
the owner's gitignored `a2amx.toml`. No new dependency, no wire change. Do not commit, stage or
merge: leave the diff uncommitted in the working tree.

## 1. Why

A team that runs unattended stalls silently when one session sits on a human prompt: a Claude
permission or trust dialog, a Codex "Update available" dialog that leaves no thread loaded, a Codex
approval request. Messages to it wait (`not_ready`, `no_thread`) and nothing in `a2amx list` says a
person is needed. Backlog I4 planned this for the Codex update dialog only.

## 2. Decisions

- **D1. Reuse the HELD cell.** No new column, no wire field. `a2amx list` and the picker already
  render `SessionSummary.hold_reason` in HELD and the attach status line already renders
  `StatusInfo.hold` as `HELD reason`. A session that needs input reports its label through those two
  existing strings when it has no real hold: in `Request::List` and `attachment_status`,
  `hold_reason = channel.hold_reason().or(session.needs_input())`. A real hold always wins. The two
  consumers that decide behavior from a hold (`send_message`'s `recipient_hold`, delivery gating,
  activity `Busy`, the status-bar `r` hint) keep using the real hold only and never see the label.
  `SessionSummary.held` follows the displayed `hold_reason` (true while the label shows) because it
  already means "HELD cell is not empty"; nothing else reads it for behavior (verify with `rg
  "\.held"` at implementation time and report).
- **D2. Labels.** `needs_input` for a signal that comes from a harness protocol (Codex), and
  `input?` for a screen heuristic (Claude). The question mark is deliberate: the Claude markers also
  appear in ordinary menus (`/model`, `/resume`, a permission-mode picker) and the label must not
  claim certainty. Both names contain no spaces; the list and picker columns size themselves to the longest value.
- **D3. Signals (per harness).**
  - **Codex:** the `Link` reason (`Link::reason`, already `pub(crate)`, `src/codex.rs` line about 265,
    read as `session.codex().and_then(|link| link.reason())`) is `no_thread` or `waiting_on_approval`.
    A pure `pub fn label_for_codex_reason(reason: Option<&'static str>) -> Option<&'static str>` in
    `src/harness.rs` maps exactly those two to `needs_input` and every other reason (`in_flight`,
    `app_server_down`, `thread_error`, `None`) to `None`. Risk, stated: the Codex poller also sets
    `no_thread` for any unknown thread status (`codex.rs` about line 466) and there is no real-Codex smoke
    test (backlog U4); if a real idle Codex 0.160 session is ever seen reporting `no_thread`, remove
    `no_thread` from the map (the table test pins the current choice).
  - **Claude:** `harness::needs_input(harness, screen) -> bool`, pure, true for `Harness::Claude`
    exactly when **one screen row contains both** markers of `DIALOG_MARKERS` (`harness.rs` line 71;
    the real dialog footer is one row, ` Enter to confirm · Esc to cancel`, as
    `tests/fixtures/fake_claude.sh` prints it). The looser any-row-any-marker check that `claude_ready`
    uses stays as it is: it only makes a session not ready, while this label must not stick on a session
    that merely prints those words (a diff or a search hit). Reuse `row_contains_marker`; do not copy it.
  - **OMP and generic:** never (no reliable signal).
- **D4. Persistence.** A signal must hold continuously for at least 10 seconds before the label
  appears and clears the moment the signal is gone. The existing one-second `sample_activity` task
  (`daemon.rs`, about line 580) evaluates the signal on every tick and keeps the first-seen
  `Instant` in the session state; the label is reported while `now - first_seen >= NEEDS_INPUT_AFTER`
  (`const NEEDS_INPUT_AFTER: Duration = Duration::from_secs(10)`). Startup transients (the channel
  development dialog the daemon accepts within seconds, a Codex thread that loads in a second or two)
  therefore never show. An exited session never shows the label: `Session::needs_input` returns `None`
  when `exit_code()` is set, and a tick that no longer sees the signal clears the first-seen instant.
- **D5. No suppression for an attached human.** The label shows regardless of `ATTACHED`; the
  persistence rule is the false-positive guard and `list` already shows ATTACHED beside it.
- **D5b. Status line side effect.** `status::render` hides the activity cell while `StatusInfo.hold` is set,
  so a session showing the label loses its activity cell in the attach status line; accepted (the
  label is the more important fact) and documented. The `r` hint is absent because `hint_allowed` lists only
  real hold reasons.
- **D6. Not an input event.** The label never sends keys, never auto-dismisses a dialog (AGENTS.md:
  never send input that destroys human work) and never holds or fails messages; it is display only.

## 3. `src/harness.rs`

`pub fn needs_input(harness: Harness, screen: &Screen) -> bool` (D3), reusing `has_dialog_marker`
(make it `pub(crate)` or call it from the new function in the same file; do not duplicate the marker
loop). A doc comment names the heuristic and its false-positive case.

## 4. `src/session.rs`

`src/session.rs`: a field in the session state (next to `last_change`, behind `self.lock()`) for
the first-seen instant and the signal (`needs_input_since: Option<(Instant, &'static str)>`),
`pub(crate) fn note_needs_input(&self, signal: Option<&'static str>, now: Instant)` that sets, clears
or restarts it (a changed signal restarts the clock), and `pub fn needs_input(&self) ->
Option<&'static str>` returning the label once `NEEDS_INPUT_AFTER` has passed and the session has not
exited. `Session::codex()` already exists (about line 340); no accessor is added.

## 5. `src/daemon.rs`

`sample_activity` computes the signal each tick (Codex: `label_for_codex_reason` of the link reason;
Claude: `harness::needs_input` giving `input?`; otherwise none) and calls `note_needs_input`; the
labels for the two harnesses are the constants of D2. `Request::List` and `attachment_status`
use D1's expression. `NEEDS_INPUT_AFTER` is a const next to `ACTIVITY_POLL`. Add the `// shortcut:`
comment: the Claude signal is a screen heuristic shared with `claude_ready`; replace it with a
protocol signal if Claude Code ever exposes one.

## 6. Tests (failing first, expected values are literals)

`tests/harness_infer.rs`, pure:

1. `needs_input(Claude, screen)` is true for a screen with one row ` Enter to confirm · Esc to cancel`,
   false when the two markers are on different rows, false for a row with only one marker, false for a
   ready composer screen and for an empty screen; false for `Omp`, `Codex` and `Generic` on the same
   dialog screen. `label_for_codex_reason` table: `Some("no_thread")` and `Some("waiting_on_approval")`
   give `Some("needs_input")`; `Some("in_flight")`, `Some("app_server_down")`, `Some("thread_error")` and
   `None` give `None`.

`tests/daemon.rs` or `tests/codex.rs` (the file whose existing fakes already drive the screen/Codex
state; reuse them, no new fixture style):

2. A fake Claude session (`tests/fixtures/fake_claude.sh`, `MODE=dialog_after_paste` style, or a `sh -c`
   that prints the one-row footer) whose screen shows ` Enter to confirm · Esc to cancel`: before 10 seconds `list` shows HELD
   `-`; after 10 seconds (use a short test hook if the suite has a clock seam, otherwise accept a
   real 11 second wait in one test and say so) HELD is `input?` and `held` is true; when the screen
   changes to a ready composer the HELD cell returns to `-` on the next sample.
3. A fake Codex session reporting `no_thread` (the fake app-server the codex tests already start
   without creating a thread; `set_threads` and `wait_reason` in `tests/codex.rs`): HELD becomes
   `needs_input` after 10 seconds and `-` once a thread loads. (`waiting_on_approval`, `thread_error`
   and `in_flight` are covered by the table in test 1, not by scenarios.)
4. A real hold wins: a Claude session with a human draft hold (`human_draft`) and a dialog marker shows
   `human_draft`, not `input?`, and `send_message`'s `recipient_hold` is unchanged by the label.
5. The attach status frame carries the label in `hold` while shown (`attachment_status` path through
   the existing status-frame test style, `tests/daemon.rs` about line 1320). 5b. An exited session shows
   no label (kill the child while the label is showing; HELD returns to `-`).

`tests/status.rs`: 6. none (the status bar already renders any `hold` string; one assertion that
`HELD input?` renders without the `r` hint, because `hint_allowed` lists only real hold reasons).

## 7. Documentation

- `README.md`: HELD description gains the two labels, what they mean and the false-positive caveat
  for `input?`, and that nothing is sent to dismiss a dialog.
- `docs/architecture.md`: the human-controls bullet and the status-line paragraph mention the labels.
- `docs/backlog.md`: I4 moves to Closed as `C20`, `Needs-input label for stuck sessions (Claude screen
  heuristic, Codex protocol signal)`, `spec 3p`; note that OMP and generic have no signal.

## 8. Out of scope

Auto-dismissing or answering a dialog; mentioning the label in the heartbeat digest of watched peers
(the place where an unattended architect would learn a developer is stuck; a likely next step); a push notification; new columns or wire fields; a signal for
OMP or generic sessions; Claude Code version probing; the status-bar hint for the label; changing
real holds.

## 9. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list.

## 10. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` passes on `9dc715a` (404 passed, 27 suites, 3
  ignored; tree unchanged since).
- Consultant review `m_764` (with a correction to its own `m_733`: the Codex reasons did not reach HELD before,
  `CodexChannel::hold_reason` is `None`, so the label is new information): B1 exited sessions (applied),
  B2 status-bar side effect (D5b), B3 both markers on one row (D3, test 1). Verified facts: HELD consumers
  are display only (`main.rs` list cell and picker rows, `status.rs`); `SessionSummary.held` is read only by
  tests; heartbeat, `recipient_hold`, `message_info` and activity `Busy` call `hold_reason()` directly, so
  the real-hold-only rule of D1 holds; `Session::codex()` and `Link::reason` already exist; the sampler runs
  for every session and the label reads the emulator screen, so a channel-mode Claude on a dialog reaches
  it; no clock seam exists, tests use real waits of about 11 seconds (`tests/daemon.rs` about line 157
  sleeps 10.2 s the same way).
- Not verified (stated in the spec): a real idle Codex session reporting no reason; the real Claude trust
  dialog footer text beyond the fixture.
- Rules stated twice, diffed: labels (D2, tests 2-4, README bullet), signals (D3, section 5), the
  real-hold rule (D1, test 4).
- Acceptance commands need the new code and run after implementation.
