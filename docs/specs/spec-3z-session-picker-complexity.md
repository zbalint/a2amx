# Spec 3z: split `Session::spawn` and `PickerParser::feed` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14; pre-lock gate run against develop at `a6e1824`; consultant review `m_1649`, findings 1-5 adopted). Architect-directed, backlog I5 part 1 plus the session.rs finding
Sonar reported after the backlog was written, context `a2amx-sonar-s3776-2026-10-14`. Implementation goes to
`a2amx-developer` only.

**Baseline:** develop at `a6e1824`; `cargo test --test picker` and full-suite counts are recorded by the developer before the first edit.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `PickerParser::feed` and `PickerParser::set_filter_mode` through `tests/picker.rs`; `Session::spawn`
through the daemon and CLI tests (`tests/daemon.rs`, `tests/daemon_cli.rs`, `tests/attach_cli.rs`). New helpers are
private.

**Scope.** May edit exactly these files and no others.

- `src/picker.rs`: `PickerParser::feed` (line 32) and new private functions directly after the `impl` block
- `src/session.rs`: `Session::spawn` (line 181) and new private functions directly before the `impl Session` block
- `tests/picker.rs`: only characterization tests of section 4

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `Session::spawn` (45) and `PickerParser::feed` (72) against 30. `spawn` contains three thread bodies
(PTY writer, PTY reader, child monitor) inline; `feed` mixes a plain-key table, a pending-escape table and the
buffering rules in one loop.

## 2. Rule: behavior is unchanged

Same actions for the same byte streams (including split reads), same thread start order, same shutdown order, same
log messages and levels, same `Shared` state transitions. No `pub` item changes. Keep every `shortcut:` comment with the
code it annotates.

## 3. Changes

### 3.1 `src/picker.rs`

Keep `feed`'s signature, the `escape_alone` early return and the `pending` buffering. Today's per-byte order inside
the loop, which must not change: (1) if `pending.len() == 1`: a following `[` or `O` is pushed and the loop continues;
any other byte clears `pending` and is then handled below as a plain key in the same iteration (so `ESC j` gives
`Move(1)` and, in filter mode, `ESC b` gives `Char(b)`; `ESC ESC` leaves one ESC pending); (2) if `pending` is empty:
`plain_key` result pushed and the loop continues; ESC pushed onto `pending` and the loop continues; any other byte
(tab, `x` in normal mode, other controls) is dropped silently by falling through to (3), where nothing matches and the
recovery rule clears an empty or non-ESC `pending`; (3) otherwise the byte is pushed onto `pending`; (4) the
escape-table lookup runs on `pending`, and only when it finds no match does the recovery rule (`pending.len() > 3 ||
first != ESC` clears) run. An unknown 3-byte ESC sequence therefore stays pending and swallows the next byte
(`ESC [ Z j` produces nothing); `ESC [ 1 ; 5 A` clears at `ESC [ 1 ;` and then yields `5` and `A` as plain keys.
Keep all of that.

Extract two private free functions:

1. `plain_key(byte: u8, filter_mode: bool) -> Option<PickerAction>`: the key tables for an empty `pending`, except
   ESC. In filter mode printable `0x20..=0x7e` gives `Char`, `0x7f`/`0x08` give `Backspace`; outside filter mode ` `,
   `h`, `l`, `/` give `Toggle`, `Fold`, `Unfold`, `StartFilter`; in both modes `\r` gives `Select`, and outside
   filter mode `q`, `j`, `k` give `Cancel`, `Move(1)`, `Move(-1)` (in filter mode they are `Char`, so the shared arms
   are unreachable for them). ESC stays in `feed` (it pushes onto `pending`). Returning `None` means "not a plain key".
2. `escape_action(pending: &[u8]) -> Option<PickerAction>`: the escape-sequence table (`ESC[A`/`ESCOA` Move(-1),
   `B` Move(1), `ESC[5~` Page(-1), `ESC[6~` Page(1), `ESC[H`/`ESCOH`/`ESC[1~` Home, `ESC[F`/`ESCOF`/`ESC[4~` End,
   `ESC[C`/`ESCOC` Unfold, `ESC[D`/`ESCOD` Fold). No sequence is in two arms and no entry is a prefix of another.
   `feed` clears `pending` when it returns `Some` and pushes the action.

### 3.2 `src/session.rs`

Keep `spawn`'s signature, validation, env, `tty::new`, fd clones, `Shared` construction, `Ok(Self { .. })`. Replace the three
`std::thread::spawn(move || { .. })` bodies with calls to private functions; the `thread::spawn` calls stay in `spawn`,
in the same order (writer, reader, monitor):

1. `writer_loop(writes: mpsc::Receiver<Option<Vec<u8>>>, writer: File)`.
2. `reader_loop(reader: File, state: Arc<Shared>, replies: mpsc::Sender<Option<Vec<u8>>>)`.
3. `monitor_loop(..)` for the child monitor: it receives `pty`, `pid`, the two `AtomicBool` clones, the state, the
   writer stop sender and the reader `JoinHandle`. A small private struct for these arguments is allowed.
4. `exit_code_from_status(status: i32) -> i32`: `(status & 0x7f) == 0` gives `status >> 8`, otherwise `128 + (status & 0x7f)` (the monitor stores the result in `State::exit_code`).

Parameters that call `blocking_recv`, `write_all`, `read` or `next_child_event` are declared `mut` (`mut writes`, `mut writer`, `mut reader`, `mut pty`). `replies` and `stop_writer` are clones of `input` made before `input` moves into `Self`; `reader_thread` is the `JoinHandle` and exists only after the second spawn, so the monitor spawn stays third. `monitor_loop` takes a private argument struct (its natural argument list is exactly 7, the `clippy::too_many_arguments` limit, so a struct avoids a lint break); destructure it at the top (`let MonitorArgs { mut pty, .. } = args;`) so `drop(pty)` last is not a partial move. Other types follow the existing variables (the developer reads them from the source). The reader's
`Err(raw_os_error == Some(5))`, `Interrupted` and other-error arms, the permit-retaining double notify, the 3 second
kill escalation with `Errno::SRCH` ignored, the reader join log and the final `drop(pty)` stay as written.

## 4. Safety net

Checked against `tests/picker.rs` at `a6e1824` (find by name; line numbers drift). Already asserted: escape sequences
`ESC[A`, `ESC[H`, `ESCOH`, `ESC[1~`, `ESC[F`, `ESCOF`, `ESC[4~`, `ESC[5~`, `ESC[6~`, `ESC[C`, `ESC[D`, `ESCOC`,
`ESCOD`; in normal mode `j`, `\r`, space, `h`, `l`, `/`; in filter mode printables (including `q j k h l /` and
space) as `Char`, `0x7f` and `0x08` as `Backspace`, `\r` as `Select`, `ESC[A`, and `ESC b` as `Char(b)`; split sequences,
alone-ESC cancel, `ESC[9~` cleared. NOT asserted: normal-mode `q` (Cancel) and `k` (Move(-1)); sequences `ESC[B`,
`ESCOA`, `ESCOB`. Before the first edit run `cargo test --test picker` and record the count, then add one test per
missing input above, passing on the unchanged code, plus two pinning tests: an unmapped byte in normal mode (`x`,
tab) produces no action, and `ESC [ Z j` produces no action. Expected values are literals from the unchanged code.

`Session::spawn` has no unit seam; the daemon and CLI suites cover it. The developer reports which test reaches the
reader's `Interrupted` and `EIO` arms (likely none; say so) and which reaches the 3 second kill escalation.

## 5. Out of scope

Everything else in both files, `daemon.rs` (spec 3aa onward), renames.

## 6. Acceptance

```sh
cargo test --test picker
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/picker.rs, src/session.rs and, if added, tests/picker.rs
```

Final proof of the complexity fix is the Sonar rescan after the owner pushes. Known flaky:
`tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`: rerun once, record as pre-existing if it
then passes. Session tests spawn real PTYs; run the full suite twice and report both counts, since a lost wakeup or
reordered thread start would show as an intermittent hang.
