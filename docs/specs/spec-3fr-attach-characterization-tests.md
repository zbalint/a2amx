# Spec 3fr: characterization tests for the attach screen

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seams: the `a2amx`
binary under a PTY (`tests/common/pty.rs`), the real daemon (`common::start_daemon`) and the fake daemon in
`tests/attach_cli/main.rs`.

Scope (edit):
- `tests/attach_cli/main.rs`: a new `FakeDaemon` option, the shared helper moved in from `viewer.rs`
  (section 2.0) and the `mod` lines.
- `tests/attach_cli/viewer.rs`: only the removal of `start_fake_daemon` (moved to `main.rs`) and what the move
  forces on its callers (an unchanged call syntax, because `use super::*` already reaches `main.rs`).
- New files `tests/attach_cli/chunked_input.rs`, `tests/attach_cli/failed_switch.rs`,
  `tests/attach_cli/mouse_and_resize.rs`.

Does not touch: everything under `src/` (the mutation check edits `src/main.rs` and `src/prefix.rs` temporarily
and restores them, section 3), `Cargo.toml`, `Cargo.lock`, every other test file, `docs/` other than this spec,
`CHANGELOG.md`.

## 1. Why

The attach screen (`src/main.rs`, `AttachmentState` and its loop) is about to be redesigned in later specs: its
input handling will move into a pure model driven one step at a time, and the redesign must not change
behaviour except through listed fixes. Today's suite cannot show that.

How the existing code reads input, per this spec: `handle_input` (`src/main.rs` 2842-2896) walks one terminal
read left to right. `prefix_batch_len` (2919-2934) ends a batch at the prefix byte and feeds the byte after it
alone, because a command can change how the remaining bytes must be interpreted: after `C-b w` the picker
takes the following bytes one at a time (2875-2885), after `C-b [` scroll mode does (2883-2885), and after
`C-b d` the rest of the read is dropped (2979). Every prefix command in `tests/attach_cli/` is sent as its
own write (`[2, b'w']`), so a read that mixes a command with the bytes after it is untested. The failure
paths of a session switch (3325-3378) and several mouse and resize combinations are untested too. This spec
adds those tests first, against the unchanged code.

Rejected: writing the tests together with the redesign (a behaviour change would hide inside a large diff);
unit tests of private functions (AGENTS.md: test through public seams, and the code is private to the binary);
keeping tests for behaviour that is already covered (the rule-row and grid-to-team-row cases below were
dropped or sharpened for that reason).

Prior lesson, stated here because specs cannot cite memory: a test that cannot fail does not pin behaviour.
Each test below names an observation that changes when the behaviour changes, and section 3 requires a
mutant that kills it.

## 2. Tests to add

Every test passes on the current code (contract tests: there is no red phase). If a test cannot pass on the
current code with the stated expectation, do not weaken the assertion and do not change `src/`: send
`QUESTION` to the architect with the observed behaviour. Use `WAIT` from `main.rs`, a temp home, no fixed
ports. Names and hosts are fictional (`s1`, `demo-alpha`).

Notation: `C-b` is byte `0x02` (default prefix); `Enter` is `\r` (the picker selects on `\r` only). "One write"
means a single `PtyHarness::send` call with all the bytes; the client's stdin reader normally gets them in
one read, and the expected result must also hold if the OS splits the write, so assert outcomes, not chunk
boundaries. Wait for the attach screen (a marker the session printed) before the first send in every test.

### 2.0 Shared helper

Move `start_fake_daemon` from `tests/attach_cli/viewer.rs` into `tests/attach_cli/main.rs` unchanged (two new
modules need it; backlog F94 already called for the move when a second module uses it). Keep every existing
caller compiling. `mod` order in `main.rs` (rustfmt sorts it): `attach_session`, `chunked_input`,
`failed_switch`, `list_and_messages`, `mouse_and_resize`, `picker`, `scroll_and_mouse`,
`status_and_team_line`, `viewer`.

Byte capture: a session PTY is in canonical mode, so bytes without a newline stay in the line buffer and the
file stays empty. Sessions that capture bytes run
`sh -c 'stty raw -echo; exec cat > /abs/path/FILE'` (precedent: `attach_session.rs` line 50) and every test
waits for FILE's content with a positive marker, never for emptiness, then reads it.

### `chunked_input.rs` (real daemon)

1. `command_and_following_keys_in_one_write_reach_the_picker`: sessions `s1` and `s2` running
   `sh -c 'printf ALPHA; exec cat'` and `sh -c 'printf BETA; exec cat'`; attach `s1`; send `C-b w j Enter` in one
   write. Expect the picker to open, `j` to move to `s2` and Enter to attach it (screen shows `BETA`).
   If the picker's initial selection is not `s1`, so that `j` does not reach `s2`, send `QUESTION`.
2. `scroll_command_and_quit_in_one_write_return_to_live`: a `cat` session; send `C-b [ q` in one write, then
   `Z\n` in a second write. Expect `Z` echoed by the session (live again). The `q` must not reach the
   session: use a raw-capture session instead of `cat` and expect FILE to equal `Z\n` exactly.
3. `detach_drops_the_rest_of_the_write`: raw-capture session; send `A`, wait for FILE == `A`, then send
   `C-b d x y z` in one write. Expect exit code 0 and, after a short wait (at most 500 ms), FILE still equal to
   `A`.
4. `literal_prefix_then_command_letter_in_one_write_is_forwarded`: raw-capture session; send `C-b C-b d` in one
   write. Expect FILE to equal the bytes `0x02 'd'` and the client still attached (a following `C-b d` write
   prints `[detached from`).
5. `bracketed_paste_with_the_prefix_byte_is_forwarded_verbatim`: raw-capture session; send
   `ESC[200~ a C-b d b ESC[201~` in one write. Expect FILE to equal exactly those bytes and the client still
   attached.

### `failed_switch.rs` (fake daemon)

New `FakeDaemon` option `refuse_attach: Option<String>` (session id): `Request::Attach` for that session is
answered with `Response::Error { message: "session refused" }` and is not recorded in `attach_viewers`, and
does not advance `attach_count`.

6. `picker_enter_on_a_session_that_refuses_keeps_the_picker_open_with_the_error`: attach `s1`, `C-b w`, move to
   `s2`, `Enter`. Expect the picker footer to show `session refused` and the picker to stay open; then `Esc`
   (separate write) returns to `ALPHA` with `s1` attached. `attach_viewers()` equals
   `[("s1", false), ("s1", false)]` (first attach and restore; no `s2` entry).
7. `team_next_to_a_session_that_refuses_restores_silently_and_keeps_input_on_the_current_one`: team fixture;
   attach `s1`; `C-b n`; then, as a separate write, `hello`. Expect no error text on screen, `BETA` never to
   appear, `attach_viewers()` equal to `[("s1", false), ("s1", false)]`, and the concatenation of the
   `ClientFrame::Input` payloads recorded after the refusal to equal `hello`. The refused `s2` never reaches
   the serve loop, so those frames can only belong to the restored `s1`.

### `mouse_and_resize.rs` (fake daemon, 2 team rows; the fake data enables mouse mode 1000)

8. `team_row_press_on_another_member_released_on_the_grid_does_not_switch`: find the column of the `demo-beta`
   label on the rendered team line (read the PTY screen text, never the library), SGR press at row 1 on that
   column, release at row 5. Expect `attach_viewers()` to stay `[("s1", false)]` and no `ClientFrame::Input`
   recorded for the press or the release.
9. `grid_press_released_on_the_team_row_is_forwarded_shifted`: session mouse on; press at row 5 column 10,
   release at row 1 column 10 (SGR). Expect the recorded `ClientFrame::Input` frames to equal
   `ESC[<0;10;3M` (the press shifted by the two team rows) followed by `ESC[<0;10;1m`, and no switch.
10. `resize_in_live_scroll_and_picker_modes_sends_the_pty_rows`: three tests or one with per-case frame slices.
    Team fixture, 2 team rows granted, status bar visible, harness resized to 12 rows. Different widths per case
    (live 60, scroll 61, picker 62) so earlier frames cannot satisfy later checks. Expect
    `ClientFrame::Resize { cols: W, rows: 8 }` (12 rows minus 2 status rows minus 2 team rows; the literal 8
    comes from this hand example). Picker case: the screen shows the first 62 characters of the picker header
    (`a2amx sessions: Enter attach/fold, ...`, clipped by the client) and not the header's full text. Live case,
    which is timing sensitive by design: take the raw output length before the resize, wait for the `Resize`
    frame, `expect_raw_lacks` the session address text after that point, then wait for the scheduled data
    (`FakeDaemon::schedule_data_after`, 2500 ms as in `viewer.rs`) and expect the status bar to be repainted.
11. `status_toggle_when_team_rows_are_refused_reattaches_on_every_show`: fake daemon `with_top_rows(team,
    Some(0))`, harness 80x24. On attach expect a corrective `Resize { cols: 80, rows: 22 }`. `C-b s` (hide):
    expect `Resize { 80, 24 }` and no new attach (`attach_viewers()` length 1). `C-b s` (show): expect a second
    attach (length 2) and `Resize { 80, 22 }`. This pins today's behaviour, which is probably wrong (a granted 0
    never equals the desired 2, so every show re-attaches); the redesign specs decide whether to fix it. Do not
    fix it here.

Dropped from the first draft because the existing suite already covers them: a rule-row press and release
(`status_and_team_line.rs` lines 237-240) and the grid-to-team-row release forwarding without its frame
literals (lines 246-247); test 9 adds the exact frame values.

## 3. Mutation check

For each numbered test the developer breaks the behaviour it pins, in the checkout (AGENTS.md scratch rule:
no other location), runs that test, records the failing test, and restores the file by checksum. Files that may
be mutated temporarily: `src/main.rs` and `src/prefix.rs`. Mutants (verified to change the result on each
test's exact input; use an equivalent if one does not apply, and report a mutant no test kills):

| Test | Mutant |
|---|---|
| 1 | in `prefix_batch_len`, never set `self.after_prefix = true` (`C-b w j Enter` then forwards `j\r` to the session) |
| 2 | same mutant (the `q` is then forwarded) |
| 3 | skip the `return Ok(Some(...))` after `Command::Detach` in `handle_prefix_actions` |
| 4 | in `src/prefix.rs` (literal prefix branch) drop `forwarded.push(byte)` for a literal prefix |
| 5 | in `src/prefix.rs`, make the paste start marker never advance (`d` inside the paste then detaches) |
| 6 | in `switch_session`, replace `state.footer = Some(error.to_string())` with a no-op |
| 7 | in `switch_session`, drop `self.attachment = Some(restored)` |
| 8 | replace `Some(MousePress::Team) => {}` (grid release arm) with forwarding the bytes |
| 9 | make `team_rows` return 0 (the press then arrives unshifted) |
| 10 | delete the `ClientFrame::Resize` send in the SIGWINCH arm of `run_attachment_loop` |
| 11 | make `team_rows` return 0 |

After the pass `git diff --stat -- src` must be empty.

## 4. Out of scope

Any change under `src/` that stays; fixing bugs found (report them; they are spec S3 material); the known
issues the architect already holds for S3 (picker and scroll mode with session mouse on, DECSET parameter
lists, picker refresh error handling, the always-reattaching status toggle in test 11) get their failing
tests there; refactoring existing attach tests beyond the helper move; a second fake daemon.

## 5. Acceptance

Run from the main checkout; each judged by its own output.

- `cargo test --test attach_cli --no-fail-fast`: all pass. Baseline at 4a40384 (run by the architect, 75 s):
  49 passed, 0 failed, 0 ignored (48 in `tests/attach_cli`, 1 in `tests/common/fake_discord.rs` compiled in
  through `mod common`). After the change: 49 plus the 11 tests added (10 may be one or three tests; the
  developer states both numbers and the architect reruns).
- `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`: pass, no
  warnings.
- `git status --short` lists only `tests/attach_cli/main.rs`, `tests/attach_cli/viewer.rs` and the three new
  files (the spec itself is committed before the assignment).
- `git diff --stat -- src Cargo.toml Cargo.lock` is empty.
- New test files are under 500 lines each; no `#[ignore]`; no sleep above 500 ms except test 10's scheduled
  data and the existing waits.
- Mutation table in the completion report as in section 3.
- Known flaky tests: `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error` (backlog I29),
  `scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode` (I7) and
  `status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session` (I18): rerun a failure once and
  record it as pre-existing if it passes.
- The tester (assigned by the architect afterwards) reruns a mutation pass; the architect reviews the diff.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| Every existing prefix command is sent as its own write | `rg 'send\(&\[2' tests/attach_cli` | grep by the architect and the consultant |
| Prefix commands `d w [ s v r 1-9 n p` | `src/prefix.rs` 142-180 | read (consultant) |
| A literal prefix forwards the byte; paste Inside forwards verbatim | `src/prefix.rs` 131-141 | read |
| After `C-b d` the rest of the read is dropped | `src/main.rs` 2979 | read |
| Failed switch keeps the picker open with a footer; restores the session | `src/main.rs` 3359-3372 | read |
| `select_member` ignores the current session | `src/main.rs` 3072 | read (consultant) |
| `FakeDaemon.frames` is a flat list; `attach_viewers` records every attach | `tests/attach_cli/main.rs` 161, 263-266, 334-337 | read |
| A refused attach error text is the footer text | `src/client.rs` 206 | read (consultant) |
| Session PTY is canonical; `stty raw -echo` makes bytes pass | `attach_session.rs` 50; tty semantics | precedent plus standard tty behaviour, not run |
| Baseline 49 passed | `cargo test --test attach_cli --no-fail-fast` | run, 2026-11-20 |
| Picker initial selection and `j` reaching `s2` with the real daemon (test 1) | `src/picker.rs` | NOT verified: the developer checks and sends QUESTION |
| Member column of `demo-beta` in the 2-member fake line (test 8) | `src/status.rs` | NOT verified: read from the screen |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant a Claude Code session (its model is not
reported to the architect); developer Antigravity CLI, Gemini 4 Hydrogen (Medium); tester OMP,
opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0 (one FIX round after the tester pass).

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | `cat > FILE` in a canonical-mode PTY holds bytes without a newline, so tests 4 and 5 would fail and test 3 pass vacuously | sessions run `stty raw -echo`; positive file marker |
| Draft review | consultant | test 7 asserted a per-connection frame record the fake daemon does not keep; "ALPHA visible" observed nothing | reworded to `attach_viewers` plus trailing Input bytes, "BETA never appears" |
| Draft review | consultant | test 8 clicked the current member (`select_member` returns early), so it could not fail | click `demo-beta` |
| Draft review | consultant | tests 9 and 10 were already covered in `status_and_team_line.rs` 237-247; the "untested" claim was false for them | test 10 dropped, test 9 sharpened to frame literals |
| Draft review | consultant | four suggested mutants did not change the result on the tests' input; tests 7 and 9 had no mutant | table rewritten and walked per test |
| Draft review | consultant | test 11 frames accumulated across cases; literals missing for the status toggle; `start_fake_daemon` private in `viewer.rs` conflicted with the edit scope; flaky list and line anchors incomplete | per-case widths, literals, helper moved into `main.rs`, list and anchors fixed |
| Tester pass | tester | the developer's "mutant 3 survives, equivalent at every seam" claim was wrong: the full `attach_cli` module kills it through the fake daemon's `Detached` frame; only the single test cannot | mutation table corrected; classification KILLED |
| Tester pass | tester | test 8 used a byte offset as the column, and its 200 ms assertions ran before a switch could complete (the kill was incidental to the final detach text) | FIX round: character column, assertions moved after `wait_exit` |
| Tester pass | tester | test 10's picker wait was satisfied by the pre-resize screen | FIX round: `wait_for_raw_contains` on the 62-column text |
| Tester pass | tester | the tester's own row-7 mutant duplicated a section 3 mutant | tester reran a distinct mutant (killed) |
| Architect review | architect | my first full-suite run overlapped the tester's temporary mutation of `src/main.rs` (shared checkout) and was discarded | reran after the tester reported the restore and checksums matched; lesson: never run the suite while a mutation pass is open |

Test 3 (`detach_drops_the_rest_of_the_write`) cannot fail from a client mutation on its own: neither daemon reads
frames after a Detach, so it documents the end result and the existing suite carries the mutation kill.
