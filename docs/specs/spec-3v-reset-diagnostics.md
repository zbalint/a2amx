# Spec 3v: a reset rejection names its source, and a stale OMP draft hold no longer blocks it

## 0. Status

**LOCKED** (2026-10-10; pre-lock gate run against develop at `d6e797c`; consultant review `m_1143`, amended F1-F5). Owner-directed
investigation, context `answer-side-memory-hook-2026-10-10` (findings in SALTMDB memories `7f0141d5`
and `169d3e09`). Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `0ad824b` (`d6e797c` plus the example-toml line; source anchors are unchanged).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `answer-side-memory-hook-2026-10-10`. Public test seam: the daemon API under `tests/broker.rs`
(`Request::Reset`, `BridgeUp::State`, `ClientFrame::Input` over `Client::attach`).

**Scope.** May edit exactly these files and no others.

- `src/session.rs`
- `src/delivery.rs` (visibility of `hold_name` only)
- `tests/broker.rs`
- `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/wire.rs`, `src/daemon.rs`, `src/mcp.rs`, `extension/omp.ts`, every other test file, every
spec document. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

`reset_session` on an OMP developer was rejected at step 1 with `busy` four times on 2026-10-10 while
`list_agents` showed the developer idle, and with `draft_present` on 2026-10-03 and 10-04 on an empty composer.
Each rejection says only `reset gate rejected step 1 of 2`, so the cause of a stuck rejection cannot be read from
it. Code reading (see the memories) found two separate mechanisms:

- `busy` comes from the OMP extension's last reported state (`idle`, `pending` stored as `in_flight`) or a
  waiting native delivery (`Session::reset_native_reason`). Which one held on 10-10 is unknown.
- `Hold::HumanDraft` is set by any attached-client byte that is not a focus or mouse report
  (`note_human_input`) and cleared only by `release` (Ctrl-b r) or `note_submit`, which only the Claude
  UserPromptSubmit hook calls. An OMP session therefore keeps `draft_present` after one stray keystroke, with
  an empty composer, though the extension reports the real composer text state as `draft`.

This spec adds the diagnostics (D1, D2) and fixes the second mechanism (D3). The root cause of the `busy` case is
out of scope; the diagnostics will identify it on the next occurrence.

## 2. Decisions

- **D1. Named source.** A rejection message becomes `reset gate rejected step {n} of {total}: {source}`; the
  error `code` values do not change. `{source}` is, in the gate's order:
  - `exited`;
  - `hold {name}` with `name` from `delivery::hold_name` (`human_draft`, `unsubmitted_envelope`,
    `corrupted_submissions`);
  - `screen not ready`;
  - `native bridge refused` or `native bridge not connected` (code `not_ready`; test `refused` first, because
    `native_refuse` also clears `connected`);
  - `native draft=true` (code `draft_present`);
  - `native busy: idle={b} in_flight={b} waiting={b}` (code `busy`), where `b` is `true` or `false`.
  The second rejection site (`begin_delivery` returning `None`, line 733) uses the same `{source}` of the gate's
  current reason; when the gate has none by then, the code stays `held` and the source is `begin_delivery refused`.
  The messages `reset is already running` (`begin_reset`), `step {n} of {total} was not submitted`, `failed` and
  `did not become ready` are unchanged.
- **D2. Timing in the native detail.** `NativeSlot` gains `idle_since: Option<Instant>` (the time of the last change of the
  `idle` value in either direction, and of the first report; printed as `idle_for` even while `idle=false`, where
  it then means time spent not idle) and `reported_at: Option<Instant>` (set on every `native_state`). A bridge reconnect (`native_accept`) does not
  reset them.
  Every `native ...` source of D1 (not the hold, screen or exited ones) ends with
  ` idle_for={s}s reported={s}s ago`, whole seconds, or `none` in place of the number when never set.
  The message reaches the caller through `failed(error.code, &error.message)` (`src/daemon.rs` reset path, no
  change); `reset_audit` still logs only the code and step.
- **D3. Stale OMP draft hold.** `State` gains `last_human_input: Option<Instant>`, set by `note_human_input` on every
  typing input (each call that passes `input_is_typing`, not only the one that inserts the hold), so the margin
  counts from the last keystroke. A new `Session::clear_stale_omp_draft(&self)` clears `hold` (sets it to
  `None`) when all hold at once, under one lock: harness is `Omp`; `hold == Some(Hold::HumanDraft)`;
  `restore.is_none()` (a hold with a restore text protects human text the daemon still has to retype, and is
  never cleared); the bridge is connected and not refused; `native.draft == false`; `native.idle == true`;
  `last_human_input` is `None` or older than `STALE_DRAFT_MARGIN` = 2 s. It is called at the top of
  `reset_gate_reason`, before the hold is read, so reset steps, the settle loop and the graceful-kill gate share it.
  The clear is lazy: it happens at a gate, and `list` keeps showing `human_draft` until one runs. It does not call
  `message_notify` (OMP delivers over the native path, not through a held PTY queue); that is intended.
  Why a margin and not the next frame: the extension sends a state frame only when a value changes
  (`extension/omp.ts` `reportState`), so a bare Esc produces no frame at all; a real keystroke into the OMP editor
  flips `draft` to `true` within its 500 ms poll, so after 2 s `draft == false` means the composer is empty.
  Accepted risk: `safeDraft` in `extension/omp.ts` returns `false` when `getEditorText` throws, so in that
  failure D3 would clear a real hold and the reset would type onto the human's text. A missing `getEditorText` is
  refused at bridge hello (`missing`), so only a throwing one is exposed; the extension is out of scope.
  `// shortcut:` comment: fixed 2 s margin, replace with a per-keystroke acknowledgement if the extension polls
  slower. Message delivery (`begin_delivery`) does not call it (section 5).
- **D4. Old behavior kept.** `draft=true` still blocks (`draft_present`); a hold with a restore text still blocks;
  a Claude or Codex session is unaffected (D3 is OMP only); the 2 s margin means a reset within 2 s of a human
  keystroke still returns `draft_present`.

## 3. `src/session.rs`

Anchors are at `d6e797c`.

- `NativeSlot` (line 106): add the two fields of D2; `Default` stays derived.
- `State` (line 98 area) and its construction (line 216 area): add `last_human_input: Option<Instant>`,
  initial `None`.
- `note_human_input` (line 449): set `last_human_input = Some(Instant::now())` in the typing branch.
- `native_state` (line 578): maintain `idle_since` and `reported_at` per D2.
- `reset_native_reason` (line 655) and `reset_gate_reason` (line 671): return the code together with the D1
  `{source}` text (a tuple or small struct; callers at lines 726, 734, 771 and 946 use the code, the two reset
  sites also the text). Extend the existing function rather than adding a second one.
- New `clear_stale_omp_draft` per D3; the `STALE_DRAFT_MARGIN` constant sits beside `RESET_SETTLE` and friends
  (line 27).
- Reset rejection sites (lines 726-740): build the message per D1.

`src/delivery.rs`: change `hold_name` (line 392) from private to `pub(crate)`; nothing else.

## 4. Tests (failing first; literals as expected values)

All in `tests/broker.rs`, built on the shape of `reset_rejects_an_in_flight_native_delivery_as_busy`
(line 1512): an OMP session whose `sh` child prints `\033[?2004h`, a bridge connection that sends
`Hello` and `State`, and a controller listed in `control_from`. Give each target `reset: vec![]` (default
`/clear`) unless noted.

**Synchronisation.** `BridgeUp::State`, `ClientFrame::Input` and `Request::Reset` travel on separate connections,
so a test must not assume a frame has been processed. (a) After sending `ClientFrame::Input`, wait until the list
seam shows `hold_reason == "human_draft"` for the target (`common::eventually`, as `tests/daemon.rs:240-258` does).
(b) Tests 1-3 can never succeed, and a rejection has no side effects, so poll `Request::Reset` with
`common::eventually` until the expected code and message text appear. (c) A success leg may be polled the same way
(the first success ends the polling). (d) Where a test needs `draft: true` to be seen before a margin wait, send
it before the first reset and poll test-2 style until `draft_present` is returned; only then start the wait.

1. **Busy names the flags.** Bridge reports `idle: false, pending: false, draft: false`: `Request::Reset` fails
   with code `busy` and a message containing `native busy: idle=false in_flight=false waiting=false`. Then
   `idle: true, pending: true`: the message contains `in_flight=true`.
2. **Draft names the flag.** Bridge reports `draft: true`: code `draft_present`, message contains
   `native draft=true`.
3. **Hold is named.** Attach a client to the target (`Client::attach`; see `tests/daemon.rs:240` for sending
   `ClientFrame::Input`), send `ClientFrame::Input(b"x".to_vec())`; bridge reports `idle: true, pending: false,
   draft: true`. Reset fails with `draft_present` and a message containing `hold human_draft`.
4. **Stale hold clears (the regression).** Attach, send `ClientFrame::Input(vec![0x1b])` (a bare Esc); bridge
   reports `idle: true, pending: false, draft: false`. Reset at once: fails `draft_present` (inside the 2 s
   margin; the `human_draft` wait of (a) makes the Esc processed, and the first reset follows within 1 s of it).
   After 2.2 s past the Esc, reset again: succeeds with `Response::Reset { steps: 1 }`.
5. **A real draft survives.** As test 4 but the bridge then reports `draft: true`, and after the margin reset
   still fails `draft_present`; after the bridge reports `draft: false` again and the margin passes, a reset
   succeeds. (The hold stays set while `draft` is true; no second keystroke is needed.)
6. Existing reset tests (`broker.rs` `reset_*`, `daemon_cli.rs` `reset_*` and `team_reset_*`, `delivery.rs`
   `human_draft_*`, `bridge.rs` draft tests, `attach_cli.rs` `attached_input_sets_hold_and_prefix_release_clears_it`)
   pass unchanged. They already pass on the old code (contract tests).

If any existing test fails because of this change, stop and report `BLOCKED — SPEC ADJUDICATION REQUIRED` with
the test name; do not edit it.

## 5. Out of scope

- The root cause of the 10-10 `busy` beyond diagnostics; the extension, `extension/omp.ts` and the protocol.
- Making the flags readable in `list --details`: it needs a field on the wire session summary and a column
  (`src/wire.rs`, `src/main.rs`, the picker); recorded in `docs/backlog.md` instead.
- Applying D3 to message delivery (`begin_delivery`): a held peer message on an OMP session has the same
  stale-hold problem; recorded in the backlog, to be specified separately.
- Claude and Codex draft handling, the hook, the picker, `team reset`.
- Renaming or changing any error code.

## 6. Documentation

- `docs/architecture.md`: where the reset gate and `draft_present` are described (about lines 263-266 and 422), two
  sentences: a rejection names its hold or native flag; on OMP a stale human-draft hold clears when the extension
  reports an idle, empty composer 2 s after the last human keystroke.
- `docs/backlog.md`: under the section for ideas or known gaps, two items (flags in `list --details`; stale hold
  for message delivery on OMP).

## 7. Acceptance

Run fresh on the final tree, from the main checkout, with the daemon untouched (the spec file and the consultant
`a2amx.toml.example` change are committed before the assignment, so `git status` starts clean):

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short
```

`git status --short` shows only the files of section 0. Known load flakes:
`exited_claude_session_never_reports_needs_input` (daemon) and `mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
(attach_cli); on a failure rerun it alone and report both results. The new tests 1-5 are the red phase: they fail
on the old code (no `native ...` text, no clearing) and pass after.

## 8. Gate notes

Run at `d6e797c`: `cargo test --test broker reset` 3 passed; `cargo test --test daemon_cli reset` 13 passed.
Grep of the old text `rejected step` across `src tests docs README.md AGENTS.md`: only `src/session.rs:730,738`
(and the specs 2s, 3t, 3u as history); no test asserts the message text, only codes. Callers of
`reset_gate_reason`: `src/session.rs` lines 726, 734, 771, 946. Consultant review m_1143 dispositioned: F1 synchronisation, F2 clean tree (example toml committed first), F3-F5 wording, fail-open risk accepted in D3.
