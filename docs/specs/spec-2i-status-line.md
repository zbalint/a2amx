# Spec 2i: a status line in the attach client

## 0. Status

**LOCKED.** Decisions D1 to D9 in section 2 were settled with the owner. Builds on spec 2h.
The work is done directly on `develop` in the main checkout, no worktree.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/status.rs` (new), `src/lib.rs`, `src/wire.rs`, `src/prefix.rs`, `src/client.rs`,
  `src/daemon.rs`, `src/main.rs`
- `tests/status.rs` (new), `tests/wire.rs`, `tests/prefix.rs`, `tests/daemon.rs`,
  `tests/broker.rs`, `tests/attach_cli.rs`
- `README.md`, `docs/architecture.md`

Does not touch: every other file under `src/` (in particular `src/emulator.rs`,
`src/session.rs`, `src/delivery.rs`, `src/harness.rs`), every other file under `tests/`
(`tests/common` included), `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the locked spec
documents. No new dependency. Do not commit and do not merge: leave the diff uncommitted in
the working tree.

In `tests/broker.rs`, `tests/daemon.rs` and `tests/wire.rs` the only permitted change to
existing code is adding `status: false` to `Request::Attach` literals (section 4); in
`tests/attach_cli.rs` it is the size literals of section 8.

## 1. Why

An attached human sees only the harness. Whether messages are waiting for this session, or a
message is held for the human (`human_draft` and the other hold reasons), is visible only by
running `a2amx list` in another terminal. A one-row bar at the bottom of the attach client shows
who this session is, how many messages are waiting, and a highlighted alert while a hold is
stopping delivery.

The architecture makes this a client-side feature. The client redraws from the emulator and the
daemon never forwards raw PTY bytes, so the client can reserve its last terminal row, tell the
daemon the PTY is one row shorter, and draw the bar itself. It already draws one such row
(`scroll_status`, `src/main.rs:1087`). The bar is outside the PTY, so it works for every
harness.

## 2. Decisions settled with the owner

- **D1.** Scope: the attach client only. No `a2amx status` command, no peer list, no
  detached-session display.
- **D2.** Content: the session's address on the left; on the right the pending-message count
  (shown only when above zero) and, while the session is held, an alert segment with the hold
  reason.
- **D3.** Always on by default, toggled with the prefix chord `s` (default `Ctrl-B s`). The
  toggle is per attach process: it survives switching sessions in the picker and is not
  persisted. While visible, the bar reserves terminal row `rows` and the PTY gets `rows - 1`.
  The bar is visible only when the terminal has at least 3 rows; below that it is hidden and
  the PTY gets all rows.
- **D4.** The daemon pushes a new stream frame `ServerFrame::Status` (tag `0x04`); the client
  does not poll. The daemon recomputes the status once a second and sends a frame only when it
  changed, plus one frame right after the first screen snapshot.
- **D5.** The client opts in with a new `status` field on `Request::Attach`. A daemon that
  never heard of the field ignores it, and a client that did not opt in never receives frame
  `0x04`. Reason: the installed binary is replaced while old daemons and old clients are still
  running; an old client that received an unknown tag would end its attachment.
- **D6.** Scroll mode keeps its own `[scroll: q to exit]` row. While scroll mode is active the
  status bar is not drawn (scroll mode's frames are full renders, which blank the row).
- **D7.** While the session picker is open the bar is not drawn; the picker owns the screen.
- **D8.** `pending` is the count the daemon already reports in `SessionSummary.pending`
  (messages in state `pending` or `delivering` for this session in the current boot). The hold
  is the `hold_reason` already computed for `SessionSummary` (`AnyChannel::for_session(..)
  .hold_reason()`).
- **D9.** `NewSession` keeps the full terminal size it sends today; the attach that follows
  resizes the PTY. No change to `new`.

## 3. `src/status.rs` (new) and `src/lib.rs`

`src/lib.rs`: add `pub mod status;` (alphabetical position, after `session`).

`src/status.rs` is a pure formatter. No I/O, no terminal types.

```rust
use crate::wire::StatusInfo;

/// Bytes that draw the status bar on terminal row `rows`, leaving the cursor where it was.
pub fn render(session: &str, info: Option<&StatusInfo>, cols: u16, rows: u16) -> Vec<u8>;
```

Rules, in order. Let `W = cols - 1`: the bar is `W` characters wide, never `cols`, because a
character written in the last column of the last row can leave the terminal in a pending-wrap
state; leave a comment saying so.

1. If `rows < 3` or `cols < 2`, return an empty `Vec`.
2. Text is sanitized before layout: every `char::is_control` character becomes `?`. The text
   comes from the daemon and goes to a terminal.
3. `left` is `" {address} "` where `address` is `info.address`, or `session` (the id passed in)
   when `info` is `None`.
4. `pending` is `" {n} pending "` when `info` is `Some` and `info.pending > 0`, otherwise empty.
5. `hold` is `" HELD {reason} "` when `info` is `Some` and `info.hold` is `Some(reason)`,
   otherwise empty.
6. Fit to `W`, measured in `char`s. If `left + pending + hold` is longer than `W`, drop
   `pending`. If `left + hold` is still longer than `W`: when `hold` alone is `W` or longer,
   truncate `hold` to its first `W` characters and drop `left`; otherwise truncate `left` to
   its first `W - hold` characters. Truncation has no ellipsis.
   `// shortcut: widths are char counts, not terminal cell widths; use unicode-width if wide
   addresses ever matter.`
7. Padding: spaces so that `left + padding + pending + hold` is exactly `W` characters.
8. Output, byte for byte:

   `ESC 7` `ESC [ {rows} ; 1 H` `ESC [ 0 m` `ESC [ 7 m` `left` `padding` `pending`
   then, only when `hold` is non-empty, `ESC [ 0 m` `ESC [ 1 ; 3 7 ; 4 1 m` `hold`;
   finally `ESC [ 0 m` `ESC 8`.

   (`ESC 7` and `ESC 8` save and restore the cursor; row `{rows}` is the terminal's last
   row, 1-based.)

Worked examples, `rows = 24`. `PRE` is `"\x1b7\x1b[24;1H\x1b[0m\x1b[7m"`, `POST` is
`"\x1b[0m\x1b8"`, `ALERT` is `"\x1b[0m\x1b[1;37;41m"`.

| # | `session`, `info`, `cols` | Output |
|---|---|---|
| 1 | `"s1"`, `None`, 40 | `PRE` + `" s1 "` + 35 spaces + `POST` |
| 2 | `"s1"`, address `agent-plan@host-a`, pending 2, hold none, 40 | `PRE` + `" agent-plan@host-a "` + 9 spaces + `" 2 pending "` + `POST` |
| 3 | same address, pending 2, hold `human_draft`, 40 | `PRE` + `" agent-plan@host-a "` + 2 spaces + `ALERT` + `" HELD human_draft "` + `POST` |
| 4 | same as 3, 20 | `PRE` + `" "` + `ALERT` + `" HELD human_draft "` + `POST` |
| 5 | `"s1"`, address `a\x1bb`, pending 0, hold none, 40 | `PRE` + `" a?b "` + 34 spaces + `POST` |

(Example 3 drops `pending`: 19 + 11 + 18 is longer than 39; 19 + 18 fits. Example 4: `W = 19`,
`hold` is 18 characters, so `left` is cut to 1.) `rows = 2` and `cols = 1` each return an
empty `Vec`.

## 4. `src/wire.rs`

- Add

  ```rust
  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  pub struct StatusInfo {
      pub address: String,
      pub pending: u32,
      #[serde(default)]
      pub hold: Option<String>,
  }
  ```

- `ServerFrame` gains `Status(StatusInfo)`. Encoding: tag `0x04` followed by the JSON of the
  struct (`serde_json::to_vec`). Decoding: a body longer than `MAX_SERVER_DATA_LEN` is an
  error (as for `Data`); invalid JSON is an error. The existing tags and their tests do not
  change.
- `Request::Attach` gains `status: bool` with
  `#[serde(default, skip_serializing_if = "is_default")]`, placed after `rows`. With the
  default value the JSON is byte-identical to today's; the existing assertion in
  `tests/wire.rs:96` keeps passing apart from the added `status: false` in its literal.
- Callers that build `Request::Attach` by literal and need `status: false` added:
  `src/client.rs:109`, `tests/wire.rs:96`, `tests/daemon.rs:701`, `tests/daemon.rs:742`,
  `tests/broker.rs:239`. The pattern in `src/daemon.rs:1028` gains `status`.

## 5. `src/client.rs`

`Client::attach(session, force, cols, rows)` keeps its signature and behavior (many tests call
it) and sends `status: false`. Add `Client::attach_status(session, force, cols, rows)` with
the same return type, sending `status: true`. Both delegate to one private function taking the
flag; do not duplicate the body.

## 6. `src/daemon.rs`

- `attach(...)` (`src/daemon.rs:1261`) gains a parameter `status: Option<Arc<Runtime>>`. The
  call at `src/daemon.rs:1039` passes `status.then(|| runtime.clone())`.
- After the snapshot `send_data` and the `exited` check (`src/daemon.rs:1342-1347`), when
  `status` is `Some`, compute the status and send it as `ServerFrame::Status`; remember it as
  `last`.
- In the main `loop`, add a `tokio::select!` arm on a one-second ticker
  (`tokio::time::interval_at(Instant::now() + period, period)` with
  `MissedTickBehavior::Delay`, created before the loop). On a tick, when `status` is `Some`,
  recompute; send `ServerFrame::Status` only when it differs from `last`, then update `last`.
  `// shortcut: one-second poll per attachment; push from the store's write path if latency
  or load matters.`
- The computation, as a private `async fn` next to `attach`: `address` from
  `messaging::address(session.name(), &session.id().0, &runtime.host_name)`; `pending` from
  `runtime.store.open_counts(&runtime.boot)` looked up by `session.id().0` (0 when absent);
  `hold` from `AnyChannel::for_session(session.clone()).hold_reason()` mapped with
  `str::to_owned`. These are the expressions `Request::List` uses (`src/daemon.rs:978-1006`);
  reuse them, do not restate the logic in a second shape. A store error propagates with `?`,
  as in `Request::List`.
- With `status` `None` the new ticker arm does nothing and no `0x04` frame is ever sent.

## 7. `src/prefix.rs` and `src/main.rs`

`src/prefix.rs`: `Command` gains `StatusLine`; after the prefix, `s` yields
`Action::Command(Command::StatusLine)` (flushing pending forwarded bytes first, like the other
commands). Update the module doc comment only if it lists the commands.

`src/main.rs`, attach client:

- `AttachmentState` gains `status_visible: bool` (initially `true`) and
  `status: Option<StatusInfo>` (initially `None`).
- A free function `pty_rows(rows: u16, visible: bool) -> u16`: `rows - 1` when `visible` and
  `rows >= 3`, otherwise `rows`. Every place that sends the terminal size to the daemon sends
  `pty_rows(rows, status_visible)` as the rows: the initial attach in `run_attachment`
  (`src/main.rs:274`, with `visible = true`), `restore_current` (`src/main.rs:450`), the
  picker's switch (`src/main.rs:513`), and the `ClientFrame::Resize` on `SIGWINCH`
  (`src/main.rs:588`). All three attaches use `Client::attach_status`. The state keeps the
  full `rows` for drawing.
- Drawing: a method that sends `status::render(&self.session, self.status.as_ref(), cols,
  rows)` to `self.output` when `status_visible`, there is no picker, and `scroll_mode` is
  false. It is called after every `ServerFrame::Data` is sent to the output (a full render
  clears the row) and when a `ServerFrame::Status(info)` arrives (store it first). It runs
  before the existing `scroll_status` call, which stays as it is.
- `PrefixCommand::StatusLine`: flip `status_visible`, then send
  `ClientFrame::Resize { cols, rows: pty_rows(rows, status_visible) }`. The daemon answers a
  resize with a full render, so the `Data` handler above draws or clears the row; no other
  drawing code is needed.
- Switching sessions in the picker sets `status` to `None` until the new session's first
  `Status` arrives (the bar then shows the session id).
- `ServerFrame::Status` in the `match` at `src/main.rs:601`; no other arm changes.

## 8. Tests

Through public interfaces. Expected values are literals.

- `tests/status.rs` (new): the five examples of section 3 and the two empty cases, byte for
  byte.
- `tests/wire.rs`: `ServerFrame::Status` round-trips through encode and decode; its encoding
  starts with `0x04`; a body of `MAX_SERVER_DATA_LEN + 1` bytes and a body that is not JSON
  both fail to decode; `Request::Attach` with `status: true` serializes with
  `"status":true` after `"rows"`; JSON without a `status` key deserializes with
  `status == false`.
- `tests/prefix.rs`: prefix then `s` is `[Action::Command(Command::StatusLine)]`; a split
  read (`prefix`, then `s` in the next `feed`) gives the same.
- `tests/daemon.rs`: attach through `Client::attach_status` to a running generic session
  that delivers `auto` and keeps a draft hold (the setup of
  `tests/attach_cli.rs:497`). The frame after the first `Data` is `Status` with the
  session's address (take it from `Request::List`), `pending` 0 and `hold` `None`. After the
  test sends `ClientFrame::Input(b"x".to_vec())` and then a message to the session
  (`Request::SendMessage` from a second session, as the delivery tests do), a later `Status`
  arrives within five seconds with `pending` 1 and `hold` `Some("human_draft")`. A second
  attachment through `Client::attach` (no status) to another session never yields a
  `Status` frame while the test reads for 2.5 seconds.
- `tests/attach_cli.rs`, through the real binary in a PTY:
  - the existing `custom_prefix_and_sigwinch_resize_reach_attached_session` now expects
    `23 80` after attach and `9 40` after the resize to 40x10;
  - the toggle, in this order (each size is new on screen, so `wait_for_text` cannot match
    an earlier print): after the resize to 40x10 (`9 40`), the toggle chord (the test's
    prefix then `s`) makes the script print `10 40` and the last screen row no longer
    contains the address; resize to 50x12 prints `12 50`; the chord again prints `11 50` and
    the last screen row contains the address again;
  - a session named `agent-plan` shows a last screen row containing `agent-plan@`;
  - the attached-listing literal at `tests/attach_cli.rs:525` becomes `80x23`; fix any other
    literal in this file that fails only because the attached PTY is one row shorter, and
    change nothing else in it.

## 9. Docs

- `README.md`: add `s` toggles the status line to the list of prefix commands
  (`README.md:122`), and one sentence saying what the bar shows (address, pending count, a
  HELD alert) and that it takes the terminal's last row.
- `docs/architecture.md`: a short "Status line" subsection under the implemented terminal
  core (after `docs/architecture.md:264`), covering D3 to D7 and the daemon's one-second
  status frame.

## 10. Out of scope

A command that prints the status text for tmux or a prompt; peer lists; a configurable
format, colors or position; persisting the toggle; hiding the bar from the command line or
an environment variable; a status bar for detached sessions; push-on-change from the store
(the one-second poll is deliberate); any change to `src/emulator.rs`, the delivery code, the
MCP tools, or the Claude, Codex or OMP launch paths; `NewSession` sizing (D9).

## 11. Acceptance

```sh
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short
```

All three cargo commands exit 0 with no warnings; `cargo test` shows no failures and no newly
ignored tests. `git status --short` lists only the files in section 0. Do not run the
installed daemon or touch `~/.a2amx`; every daemon the tests start uses a temp state dir.
