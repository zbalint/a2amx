# Spec 3ea: an attached session is sized to the client in viewer mode too (F87)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, after spec 3dz is committed (both edit `tests/attach_cli.rs`). Context: `a2amx-viewer-sizing-2026-11-11`. Governing contract: this spec and the owner's decision of 2026-11-11 (phone, m_6742): "the sizing should work no matter its viewer or normal mode; if it is attached it should not overlap". Backlog row F87 holds the three sightings and the measurement.

Scope (files that may be edited): `src/daemon.rs`, `src/main.rs`, `src/client.rs` (expected to need no edit: `attach_viewer` has no comment about the old rule), `tests/presence.rs`, `tests/attach_cli.rs`, `docs/cli.md`, `docs/remote-messaging.md`, `docs/architecture.md` (expected to need no edit; the grep in section 2.5 decides). The architect edits `docs/backlog.md` (F87 to Closed) and `CHANGELOG.md` in the accepting commit.

Does not touch: `src/viewer.rs`, `src/wire.rs`, the wire format, the VIEW marker or overlay, presence rules, `src/session.rs`, and the specs 3dp and 3dq (history; this spec supersedes the sentences named in section 1).

Public test seams: the control API through `Client` and a real daemon (`tests/presence.rs`), and the attach client against the fake daemon (`tests/attach_cli.rs`, `start_fake_daemon`, `PtyHarness`). No private functions.

## 1. Why

F87 (three sightings, 2026-11-10 and 2026-11-11): the OMP footer and the a2amx status line overwrite each other. In the third sighting the owner measured, in one terminal of 62 rows, a session screen of 62 lines when it overlaps and 58 when it looks normal; 58 is the terminal height minus the two status rows and the two team rows (`pty_rows`, `src/main.rs:2483`). The client reserves rows at the bottom and top for its own lines, and the session is only told the smaller height when something resizes it.

Cause, read from the code. A viewer attach never resizes the session: `attach` skips `resize_locked` when `viewer` is true (`src/daemon.rs:3705-3709`), and the frame loop ignores `ClientFrame::Resize` while the attachment is a viewer (`src/daemon.rs:3849-3852`, guard `!guard.viewer`). The client follows: in viewer mode it sends no `Resize` on a terminal resize (`src/main.rs:3300-3302`), on a status-line toggle that keeps the team rows (`src/main.rs:2909-2911`) or after an attach (`src/main.rs:3082`, `!viewer`). The automatic switch to viewer mode after 300 seconds alone does not leave the session unsized, because it runs inside an interactive attach that already sized it (`src/main.rs:2436-2454,3315-3321`). The overlap needs viewer mode plus one of: a terminal resize (`src/main.rs:3300-3302`), a status-line toggle that keeps the team rows (`2909-2911`), or a re-attach in viewer mode (a picker or team-click switch, or a team-row toggle, through `attach_session`, `3059-3070`): the re-attach carries the viewer's size in the `Attach` request, and the daemon drops it today (`src/daemon.rs:3705`). Whether a session reset at the full height matters was not checked. Spec 3dq planned a `// shortcut:` comment for this case (docs/specs/spec-3dq-viewer-mode.md:56: "a viewer shows the session at the daemon's size, which can differ from the terminal's") but it never reached the code (`rg` finds none in `src/`). The hypothesis is read from code and fits the measurement; the owner's cheap test (prefix key then `v`, which sends a `Resize`) has not been run, so the cause is not confirmed by a live run.

Decision (owner, 2026-11-11): the session is sized to the client's usable rows whenever a client is attached, in viewer mode as well as interactive mode. This supersedes the rule "a viewer does not resize the session" in specs 3dp (section on the viewer attach) and 3dq (owner decisions, "Resize") and the `// shortcut:` comment. What stays read-only in viewer mode: terminal input and release frames are still ignored; the viewer attach still does not count as owner presence; resizing is not presence and does not change the owner-present state.

Rejected: cropping or scrolling the snapshot in the client (hides rows of the agent's screen, more client code); sizing only at the first viewer attach (a later terminal resize in viewer mode would overlap again, and the owner asked for no overlap whenever attached); a new wire field.

## 2. Mechanical changes

2.1. `src/daemon.rs`, `attach` (about 3705-3709): the viewer attach sizes the session like an interactive attach; remove the `if viewer { Ok(()) }` branch so `session.resize_locked(&mut state, size)` runs in both modes. `row_origin` handling is unchanged.

2.2. `src/daemon.rs`, frame loop (about 3849-3852): `ClientFrame::Resize` is honoured in viewer mode; remove the `!guard.viewer` condition from that arm and keep `state.exit_code.is_none()`. `ClientFrame::Input` and `ClientFrame::Release` keep their `!guard.viewer` guards. Add a one-line why-comment at each site only if the code reads unclearly without it; there is no existing comment to update.

2.3. `src/main.rs`, the attach client: in viewer mode it sends `ClientFrame::Resize { cols, rows: pty_rows(...) }` exactly where interactive mode does, and then redraws its status line and the viewer decorations as it does today:
- on a terminal resize (the `sigwinch` arm, about 3300-3312): send the `Resize`, and in viewer mode also redraw status and decorations;
- on a status-line toggle whose team rows do not change (`toggle_status_line`, about 2909-2918): send the `Resize` and, in viewer mode, redraw status and decorations;
- after an attach (`attach_session`, about 3082): the condition `!viewer &&` is removed so a viewer attach whose team rows were refused also sends the corrective `Resize` (`pty_rows(.., 0)`). The current daemon never refuses the rows at 9 or more terminal rows (`src/daemon.rs:3643-3652`), so this case is reachable only against an older daemon or the fake one (`start_fake_daemon(Some(0))`); the test for it uses the fake with a viewer switch.
The attach request already carries the client's size (`attach_viewer`, `src/main.rs:3062-3068`), and it now sizes the session on the daemon side (2.1). Leaving viewer mode keeps sending `Mode(false)`, `Resize`, `Redraw` in that order (spec 3dq), unchanged.

2.4. `src/client.rs`: no change expected (no signature or comment change); touch it only if the developer finds a comment that states the old rule.

2.5. Docs: `docs/cli.md` (about line 193, "Viewer mode does not send terminal input, resize, or release frames to the session") and `docs/remote-messaging.md` (about line 23, "terminal input, resize, and release") say that input and release are not sent and that the client still sizes the session to the terminal. Search `rg -n -i "viewer" docs/architecture.md docs/cli.md docs/remote-messaging.md` for any other sentence that states the old rule and fix it; the architect adds the CHANGELOG Unreleased line (stating the reversal explicitly: a viewer now sizes the session; the released 0.3.0 entry stays as it is) and closes F87. Specs 3dp (lines 18, 35, 51 and the security table row at 70) and 3dq state the old rule; they are history and stay. Every behaviour sentence is read from the code.

2.6. Tests (flip the ones that pin the old rule; keep every other viewer assertion; write the flips and the new tests first and record their red runs on the old code). Literal row counts below come from `pty_rows` over the PTY size the helper uses (default 24 rows; confirm the fake daemon's team grant and `COLS`/`ROWS` in `tests/common/pty.rs` before writing the literals):
- `tests/presence.rs::viewer_attach_does_not_resize_or_accept_input_until_mode_changes` (about 489): rename it to say what stays true (input ignored until the mode changes). The session is created at 40x10 and the viewer attaches at 80x20: assert 80x20 after the attach (the attach-site change); the viewer's later `Resize` (about line 517) uses a third size, 100x30, which the test asserts afterwards (the frame-loop change); `Mode(false)` sends no `Resize`, so 100x30 stays and the final assertion at about 578 changes from (40,10) to (100,30). Input and release stay ignored; presence stays unchanged. This test is also the F87 case on a real daemon; add no second copy.
- `tests/attach_cli.rs::viewer_toggle_blocks_input_and_shows_overlay` (2261): its `attached.resize(90, 24)` in viewer mode (about 2279 to 2292) now yields exactly one `Resize { cols: 90, rows: pty_rows(24, true, top) }` (20 rows with the fake granting team rows: confirm). This is the SIGWINCH test; write no separate one.
- `viewer_status_toggle_same_top_rows_redraws_without_resize` (2610, `Some(0)` with hidden teams): rename it (it now sends a resize) and expect `Resize { cols: 80, rows: 24 }` after the hide and `rows: 22` after the show (confirm the constants).
- `assert_viewer_mouse_is_not_forwarded` (2729, run with `None` and `Some(0)`): for `Some(0)` the `s` toggle takes the same-top branch and now sends one `Resize` (`rows: pty_rows(24, false, 0)` = 24): expect the resize count plus one there; for `None` the count stays equal. Keep a separate check that mouse events send no `Resize`.
- `viewer_picker_switch_and_status_toggle_keep_viewer_attach` (2669): not flipped. Its `s` toggle changes the team rows and takes the `switch_session` path, and a re-attach carries the size in the `Attach` request, so it still sends no `Resize` frame: keep it as the guard that a viewer re-attach adds no extra frame.
- `attach_viewer_sends_no_resize_frame` (2219) tests `Client::attach_viewer`, which does not change: keep it unchanged.
- New, `tests/attach_cli.rs`, fake daemon with `start_fake_daemon(Some(0))`: a viewer session switch whose team rows the daemon refuses sends the corrective `Resize` (2.3, third bullet).

2.7. Effects of sizing from a viewer, stated so nobody is surprised (they already happen on an interactive attach): `resize_locked` changes the PTY window size, the agent gets SIGWINCH and redraws (`src/session.rs:1215-1235`); the redraw updates `last_change` (`src/session.rs:251`), which postpones a heartbeat digest by up to one interval (`src/daemon.rs:1181`); a viewer paging through sessions in the picker now resizes each session it visits, and readiness may look not-ready for the moment of the redraw. Presence, the `human_draft` hold, receipts and `Release` do not depend on the resize (`src/daemon.rs:3737-3744,3818-3825`). One new error path: a viewer attach with zero columns or rows used to succeed and now fails with "session dimensions must be nonzero", and a viewer `Resize` of zero ends the attach with that error; the real client never sends zero (`pty_rows` has `.max(1)`, `src/main.rs:2491`).

## 3. Out of scope

Cropping in the client, any change to presence or the viewer timer, multiple simultaneous attachments, the OMP footer itself, the picker, and changing specs 3dp and 3dq. The CHANGELOG and backlog edits (architect).

## 4. Trust-boundary note

A viewer attachment is a client of the admin token (attach is not a session-token action). The change lets that client set the session size, which an interactive attach already can; input and release stay ignored for a viewer (the defence in depth of spec 3dp). The daemon test for the abuse case stays: a viewer's `Input(b"x")` and `Release` change nothing; a viewer's `Resize` now does, by decision (it supersedes the security-table row of spec 3dp, line 70). The size is bounded as before by `resize_locked` (nonzero dimensions).

## 5. Acceptance

Baseline at draft time on the unchanged tree: `cargo test --test presence viewer_attach` and `cargo test --test attach_cli viewer` pass (the architect runs them before locking). Run single targets, never the full suite in a loop.

1. Red first: write the flipped tests and the new test of 2.6 before the source change; they fail on the old code. Record each failure (the session keeps its old size; no `Resize` frame arrives).
2. After the change they pass, and so do the flipped tests; `cargo test --test presence`, `cargo test --test attach_cli`, `cargo test --test viewer` pass.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass (the full suite once, with a timeout of at least 1700 seconds).
4. Mutation check (developer and tester, each restored by checksum, no second copy, each run limited to the relevant test target): the viewer attach skipping the resize again; the frame-loop guard restored; the client not sending `Resize` in viewer mode on SIGWINCH; the client not sending it on the status toggle; `Input` accepted in viewer mode (positive control that the input guard still has a test). Expected killers: attach-site mutant by the presence test (80x20 assertion); frame-loop guard mutant by the presence test (100x30); SIGWINCH mutant by `viewer_toggle_blocks_input_and_shows_overlay`; status-toggle mutant by the toggle test and `assert_viewer_mouse_is_not_forwarded` with `Some(0)`; input mutant by the presence test and the existing input test. Each mutant is killed by a named test.
5. `git diff --stat` shows only files in the scope list; no deleted assertion without an equivalent.
6. Live check (architect or owner, after the push; the developer cannot run it): in a 62-row terminal, a viewer attach to a session started at another size gives `amx screen` of 58 lines, and the overlap does not return. If the owner can still make it overlap, F87 stays open with the new capture.
7. Scratch under `target/scratch/3ea/`, deleted before reporting.

## Review record

Models: architect Claude Sonnet 5.5 (claude-sonnet-5-5); consultant on the claude harness (model not recorded); developer on OMP, working slot openai-codex/gpt-5.6-luna (advisor slot not exposed); tester on OMP, opencode-go/glm-5.3-flash. BLOCKED/amend cycles: 0.

| Where found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The test list missed `viewer_toggle_blocks_input_and_shows_overlay` (it breaks) and listed `viewer_picker_switch_and_status_toggle_keep_viewer_attach` (it does not) | 2.6 rewritten; the toggle test is the SIGWINCH test |
| Draft review | consultant | Section 1 said spec 3dq's `// shortcut:` comment exists in the code; it was only planned | Reworded |
| Draft review | consultant | The automatic 300-second switch alone does not leave the session unsized; the example scenario was overstated | Section 1 rewritten |
| Draft review | consultant | The presence test's `Resize` used the attach size, so it could not show the frame-loop guard was removed; the SIGWINCH side effects (`last_change`, readiness, zero size error) were unstated | Third size 100x30; section 2.7 added |
| Draft review | consultant | The red-first order covered only the new tests, not the flips | Section 5.1 covers both |
| Tester pass | tester | Survivor in committed code: removing the `exit_code` guard in `resize_locked` (`src/session.rs:1220`) is not detected by any test | Not in this diff; a tests-only hardening item, not scheduled |
| Full suite | developer | `gateway_up::sandbox_hides_state_host_and_limits_writes_to_gateway` fails in a confined session | Known I24, unrelated; CI (unconfined) is the judge |
