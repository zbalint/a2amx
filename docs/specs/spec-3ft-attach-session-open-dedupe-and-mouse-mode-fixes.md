# Spec 3ft: one session-open path for the attach screen, and wheel reports in scroll mode and the picker

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seams: the PTY suite in
`tests/attach_cli/` (real daemon and `FakeDaemon`) and the existing `tests/attach_cli/main.rs` helpers.

Scope (edit): `src/attach.rs`, `src/attach/state.rs`, `src/attach/event_loop.rs`, `tests/attach_cli/scroll_and_mouse.rs`
(new tests only). Does not touch: everything else, `src/main.rs`, `src/prefix.rs`, `src/picker.rs`,
`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md` (the architect adds the changelog line when committing), docs.

## 1. Why

Spec 3fs moved the attach screen into the library unchanged. The consultant's design read and the architect's
own read of the moved code found debts that are small, real and cheap to fix before the larger redesign (S4)
rewrites the state type: (a) the "open a session and negotiate its top rows" handshake is written twice
(`attach::run`, `AttachmentState::attach_session`), (b) two branches of the event loop that mean "stdin closed"
are copies, (c) in scroll mode and with the picker open the wheel reports are only recognised when the session has not asked
for mouse reports, so with the mode on the report bytes are fed to those modes as keys: typed into the picker
filter, and the wheel does not scroll in scroll mode.

Decided and rejected while grilling (one line each): the status-toggle re-attach on a refused team row (pinned by
spec 3fr test 11) stays as is, because the daemon may grant rows later and fixing it needs the
granted-versus-desired split of the S4 model; `after_prefix` versus `PrefixMachine` pending state stays, because
merging them changes how a pasted prefix byte splits a forward and belongs to the model redesign; the
`restore_current` swallowed refresh error stays (it follows an attach failure already shown in the footer), and
the opposite case in `switch_session`'s restore branch (`state.refresh().await?` ends the screen on a transient
list failure after a successful restore) is listed for S4 because a test needs a daemon whose list request fails.
Mouse-mode lists (`ESC[?1000;1006h`) are not a client bug: the daemon renders each mode as its own sequence.

Prior lessons: a fix is tested failing first; a test that passes on the old code is named a contract test and
stays; a pure dedupe is proven by the unchanged suite plus a structural check (section 6).

## 2. Target shape

One free function in `src/attach.rs`, used by `run` and by `AttachmentState::attach_session`:

```rust
struct Geometry { cols: u16, rows: u16, status_visible: bool }
struct Opened { attachment: Attachment, top_rows: u16, in_team: bool }
async fn open_attachment(
    client: Client, home: &Path, session: &str, force: bool, viewer: bool, geometry: Geometry,
) -> anyhow::Result<Opened>
```

Responsibility: resolve whether the session is in a team, request the team rows the geometry allows, attach
(viewer or not), and when the daemon granted no top rows although some were requested send the single
`Resize` with the full-height pty rows. `run`
passes `force` and `viewer = false` and `geometry { cols, rows, status_visible: true }`; `attach_session` passes
`force = false`, `viewer = self.viewer.is_viewer()` and its own geometry. The `// shortcut:` comment above the
`Resize` moves with it. `Geometry` and `Opened` are private to the `attach` root. `attach_session` stays and
becomes a thin call that returns what its three callers use today (`(Attachment, u16, bool)`); nothing else about
the callers changes.

`run` keeps its signature and the `client` it receives from `main.rs`; `attach_session` connects `Client::connect(&self.home)` and passes it. Section 5 states what is deferred.

`run` calls `open_attachment` before `Output::start` and `TerminalGuard::enter`, as today, so an attach error
still exits before the alternate screen. `attach_session` now connects its client before the team lookup instead of
after; the only effect is which error wins when both fail, and it is accepted.

Event loop: the two identical "stdin closed" branches (input channel closed, and the reader reported end of input)
become one: `let Some(bytes) = input.flatten() else { ...one detach path... }`. The path sends `Detach` to the
current attachment if there is one and returns the same `[detached from {session}]` outcome as today.

## 3. Behaviour fixes (each tested failing first)

### 3.1 Reports are recognised in scroll mode and with the picker open even when the session has mouse on

`handle_input` (`src/attach/state.rs:112-170`) skips `find_mouse_report` when the picker is open or scroll mode
is on and `session_mouse` is true, so the SGR bytes of a wheel or click report reach the picker parser and the
scroll parser as keystrokes. New rule: when the picker is open or scroll mode is on, mouse reports are always
found and routed to `handle_mouse_button` (which drops them in the picker and scrolls in scroll mode), whatever
`session_mouse` says.

Tests (each first run on the old code, red expected):
1. `typed_filter_is_not_polluted_by_a_wheel_report_with_session_mouse_on`: session script prints
   `\033[?1000h\033[?1006h` then `ready; sleep 30`; `prefix w` opens the picker, `/` starts the filter, send
   `\x1b[<64;10;5M`; the picker footer or filter line must not contain `4;10;5M` (old code types the tail
   into the filter: `plain_key` maps 0x20-0x7e to characters in filter mode); then Esc Esc closes it.
2. `wheel_scrolls_in_scroll_mode_with_session_mouse_on`: model `mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
   (`scroll_and_mouse.rs:65`): the script prints 100 numbered lines and the mouse-on sequences, `prefix [`
   enters scroll mode, the wheel-up report scrolls the history (the scrolled-back line appears), `q` returns.
   Old code: the report bytes are fed to the scroll parser as keys and the wheel does not scroll.
The developer shows both red before the fix (the daemon renders the program's mode lists as single modes, see the
claims table, so the tests use two separate private modes). If a test is already green on the old code, the
developer stops and sends QUESTION before changing anything.

## 4. Tests that already pin the moved paths (contract tests, unchanged)

The dedupe of section 2 is covered by `tests/attach_cli/` as a whole, in particular: `failed_switch.rs` (both
tests, the switch and restore paths), `status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session`,
`viewer.rs` (viewer attach keeps the viewer attach on a switch and a toggle), `mouse_and_resize.rs::status_toggle_when_team_rows_are_refused_reattaches_on_every_show`
(the refused-rows `Resize`), and the attach-time `Resize` when `forced_top_rows` is `Some(0)`
(`mouse_and_resize.rs` lines 166-182). They must pass unchanged.

## 5. Out of scope

Everything in section 1's rejected list; `main.rs` (the `client` parameter of `run` stays); renaming `Output`,
`attach::picker`; the `PickerState::open` argument count; split mouse-mode sequences across output chunks; the
typed row interface of `table`; any new public item. The architect owns docs, the changelog line and the
review record.

## 6. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at `10d2b69` (architect, 2026-11-20): 63 suites, 1367 passed, 0
  failed, 3 ignored. After: 1367 plus the new tests; the developer states the numbers.
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- Red-then-green evidence: for each new test the developer runs it on the old code before the fix and reports
  fail or pass (contract test). Command: `cargo test --test attach_cli <test name>`.
- Dedupe check (outside judge, architect): `rg -c 'attach_status|attach_viewer' src/attach.rs src/attach/state.rs`
  prints `src/attach.rs:2` and nothing for `state.rs` (today attach.rs 1, state.rs 2); `rg -n 'Resize' src/attach.rs
  src/attach/state.rs` shows one in `attach.rs` and three in `state.rs` (viewer toggle, status toggle twice); the
  SIGWINCH `Resize` lives in `event_loop.rs`, unchanged; `rg -c 'detached from' src/attach/event_loop.rs` prints 2
  (today 3: the merged stdin-closed path and the attachment-stream-ended path). The architect reads the diff for any
  other change.
- `wc -l`: no file over 1,000 lines. `git status --short` lists only the section 0 files.
- Mutation check (developer, mutate in the checkout, restore by checksum): in `handle_input`, restore the
  `(!session_mouse).then(...)` guard (killed by both new tests); in `open_attachment`, drop the granted-top
  `Resize` (killed by `status_toggle_when_team_rows_are_refused_reattaches_on_every_show`); in `open_attachment`,
  pass `force` as `false` in `run` (name the killing test; if none, say so).
- Known flaky: `tests/presence` `viewers::viewer_attach_keeps_input_read_only_while_sizing_session` (timing under
  load), `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error`,
  `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode`,
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session`: rerun a failure
  once; if it passes record it as pre-existing.
- The tester (assigned afterwards) runs an independent mutation pass and reads the diff; a consultant reads the
  design (refactor spec).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| `run` and `attach_session` negotiate the same way | `src/attach.rs:36-60`, `src/attach/state.rs:533-569` | read: both compute team rows, attach, resize on a refused request; differences: `run` takes `force` and a given client, `attach_session` picks viewer or status attach and connects its own client |
| `run`'s `client` comes from `main.rs` at three sites | `src/main.rs` calls to `attach::run` | read in 3fs (lines 973, 2056, 2588) |
| The stdin-closed branches are identical | `src/attach/event_loop.rs:31-46` | read |
| The client never sees a mode list: Data frames are emulator renders with one private mode per sequence | `src/emulator.rs:790-827`, `src/daemon.rs:3706,3799` | read by the consultant; so a list-parsing fix in `scan_session_mouse` is not a bug and is out of scope |
| `handle_input` drops the report search in picker and scroll mode when session mouse is on | `src/attach/state.rs:122-125` | read |
| In the picker with filter off the report tail is ignored; with the filter on it is typed into the filter; in scroll mode the report is harmless as keys but the wheel does not scroll | `src/picker.rs:32-70`, `src/attach/input.rs:48-63`, `plain_key` | read by the consultant (hand trace): both new tests are red; the developer confirms by running |
| A picker report is dropped; in scroll mode button 64 scrolls up and 65 scrolls down; other buttons are ignored | `handle_mouse_button`, `src/attach/state.rs:410-437` | read |
| Baseline 1367 passed | architect full run after 3fs | run, `target/final-3fs.log` (deleted) |

## Review record

(Appended by the architect at acceptance.)
