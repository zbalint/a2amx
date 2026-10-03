# Spec 2r: `a2amx screen`, a read-only view of a session's screen

## 0. Status

**LOCKED** (2026-10-03, pre-lock gate run against the tree at the baseline below). Decisions D1 to D8 follow from the owner's go-ahead on 2026-10-03
("if it helps it is worth to build"). The owner reviewed them the same day: D1, D2 fine; D5 fine
for a first version though strict; D8 keep the script for now; D3 and D6 are to be proven by
tests (tests 7 and the wide-character test). Gate findings folded in: the only exhaustive
`match request` is `src/daemon.rs:993`; the session-token refusal list is in `tests/broker.rs`
(now in scope); wire literals are fixed in D3.

**Baseline:** develop at the commit that adds this spec, on top of the picker-columns change
(`9edefa7`, context `a2amx-picker-columns`). **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-screen`.
Public test seams: the `a2amx` CLI against a real daemon (`tests/common`, as
`tests/daemon_cli.rs` does) and the wire request/response round trip (`tests/wire.rs`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs`, `src/emulator.rs`, `src/daemon.rs`, `src/client.rs` (only if the request needs a
  helper there), `src/cli.rs`, `src/main.rs`
- `tests/daemon_cli.rs`, `tests/wire.rs`, `tests/broker.rs` (one request added to the session-token
  refusal list)
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

No new dependency. Do not commit, stage or merge: leave the diff uncommitted for review.

## 1. Why

To read what a session's screen shows (for example a status line with `5h 0% left`, or whether a
peer is stuck on a prompt) there is only `scripts/screen-probe.py`. It attaches in a pty, so it
resizes the session by one row (the status bar) until the next attach, it must never be used on a
session someone is attached to, and it needs python. The daemon already holds the rendered
screen (`session.lock().emulator.screen()`, used by `session_quota`). A native command reads it
directly: no attach, no resize, safe for attached and detached sessions.

## 2. Decisions

- **D1.** `a2amx screen <session> [--rows N]`. `<session>` is an id or a name, resolved
  client-side with `resolve_reference` like `attach` and `kill`. Unknown session: the same error
  and exit status as `kill` on an unknown session.
- **D2.** Output is the plain text of the session's visible screen (the active buffer, so the
  alternate screen of Codex or Claude when they use it). No colors, no escape sequences, no
  scrollback. One line per screen row, trailing spaces trimmed, trailing blank rows dropped;
  blank rows in the middle are kept. `--rows N` keeps only the last N lines of that result.
  Without `--rows` the whole result is printed. Nothing else is printed to stdout (no header);
  failures go to stderr.
- **D3.** New control request `Request::Screen { session }` and response
  `Response::Screen { lines: Vec<String> }` in `src/wire.rs`, serialized like the existing
  variants, with exactly these JSON shapes: request `{"type":"screen","session":"s1"}`, response
  `{"type":"screen","lines":["first","second"]}`. The daemon sends the full trimmed text (D2 before `--rows`); `--rows` is applied by
  the CLI. A session that exited still answers with its last screen (exited sessions stay in the
  daemon's session map until killed; the developer proves it with test 7 and, if the emulator is
  not retained, stops and reports instead of choosing a behavior); an unknown id answers
  `Response::Error` like `Kill` does.
- **D4.** Read-only and side-effect free: no attach, no resize, no input to the session, no change
  to attachment, hold or pending state, nothing written to disk. It works the same while a client
  is attached.
- **D5.** Authorization: admin tokens only. The existing allow-list for session tokens
  (`Request::SendMessage | ListAgents | MessageStatus | ReportPrompt | BridgeAttach`) stays as is,
  so a session token gets `not permitted for a session token`. Add a test for this. The owner
  accepts this as strict but fine for a first version; an MCP tool that lets agents read each
  other's screens is out of scope (a separate owner decision, likely later).
- **D6.** Text extraction lives in `src/emulator.rs` (the only module naming terminal-emulation
  types): a `Screen` method returning the trimmed lines of D2, built from `Cell.ch`. Wide-character
  spacer cells (`Attrs::WIDE_SPACER`) must not produce a duplicate or stray character, and a
  wide character counts as one character in the line; cover it with a test through the public
  seam (a session printing a wide character, for example a CJK character, between ASCII).
- **D7.** Lock discipline: take the session lock only long enough to copy `emulator.screen()`,
  never across an `.await`, and never while another session or runtime lock is held (spec 2l
  deadlocked computing quota inside `Request::List` while holding the session lock). Render the
  text after releasing the lock.
- **D8.** `scripts/screen-probe.py` stays untouched until the owner confirms `screen` works as
  intended in real use; a later task deletes it. Docs may mention the command next to the
  script.

## 3. Changes

1. `src/wire.rs`: the request and response variants of D3.
2. `src/emulator.rs`: the text method of D6.
3. `src/daemon.rs`: handle `Request::Screen` (D3, D4, D7); leave the session-token allow-list
   unchanged (D5).
4. `src/cli.rs`, `src/main.rs`: the `screen` subcommand, `--rows`, reference resolution (D1).
5. Docs: `README.md` command list, `docs/architecture.md` request list, `docs/backlog.md` if it
   lists the screen command as an idea (and add a backlog item: delete `scripts/screen-probe.py`
   once the owner confirms `screen`).

## 4. Tests (failing first, public seams only)

In `tests/daemon_cli.rs`, with a real daemon and fake sessions started via `new --detach`:

1. A session that prints known lines then sleeps: `a2amx screen s1` prints exactly those lines
   (literals in the test), trailing blank rows and trailing spaces gone, a blank row in the middle
   kept.
2. By name: `a2amx new --name worker ...` then `a2amx screen worker` gives the same text.
3. `--rows 2` prints only the last two lines.
4. A session on the alternate screen (prints `\x1b[?1049h` then text) prints the alternate-screen
   text.
5. Side-effect free: `a2amx list` shows the same `SIZE` for the session before and after
   `a2amx screen`, and an attached client (pty) stays attached and is not resized.
6. Unknown session: stderr message and non-zero exit, same as `kill`.
7. An exited session still prints its last screen.
8. A session token is refused: add `Request::Screen { session: sender_id.clone() }` to the list
   of refused requests in `tests/broker.rs` (the loop around line 258 that expects
   `not permitted for a session token`).

In `tests/wire.rs`: the new request and response serialize to exactly the two JSON literals of D3
(extend `request_json_uses_exact_tagged_shapes` and add the response beside the existing
response-shape test).

## 5. Out of scope

Scrollback, colors or attributes, a follow/watch mode, an MCP tool, per-row ranges, changing
`list_agents`, deleting `scripts/screen-probe.py` (D8), anything under `src/session.rs`,
`src/harness.rs`, `src/quota.rs`.

## 6. Acceptance

Fresh, on the final tree, no warnings: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`; diagnostics clean on changed files; `git diff --stat` shows only the files of
section 0. Run cargo under `env -u A2AMX_BIN` if the session exports it (an OMP-launched session
breaks `omp_wiring::generic_sessions_get_no_a2amx_bin`). Do not touch `~/.a2amx/bin` or the live
daemon.
