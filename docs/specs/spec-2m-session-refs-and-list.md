# Spec 2m: sessions by name, and a leaner `a2amx list`

## 0. Status

**LOCKED.** Decisions D1 to D8 in section 2 were settled with the owner. Builds on spec 2l
(committed `6ec9761`). The work is done directly on `develop` in the main checkout, no
worktree. Shared task `context_id`: `a2amx-refs-and-list`. Public test seams: the `a2amx` binary
(`tests/attach_cli.rs` `run_binary`, `tests/daemon_cli.rs` `run`), and the wire types through
`tests/wire.rs`.

**Scope.** May edit exactly these files and no others.

- `src/main.rs`, `src/cli.rs`, `src/wire.rs`, `src/daemon.rs`, `src/session.rs`,
  `src/harness.rs`
- `tests/attach_cli.rs`, `tests/daemon_cli.rs`, `tests/wire.rs`, `tests/refs_and_list.rs` (new)
- `README.md`, `docs/architecture.md`

Does not touch: every other file under `src/` (in particular `src/mcp.rs`, `src/emulator.rs`,
`src/delivery.rs`, `src/messaging.rs`), every other file under `tests/` (`tests/common`
included), `scripts/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the locked spec documents. No
new dependency. Do not commit and do not merge: leave the diff uncommitted in the working
tree.

In existing test files the only permitted changes are the mechanical ones in section 7.

## 1. Why

Every session command takes a session id (`s1`, `s2`), although a name is unique on a host and
is how peers address each other. `a2amx list` also prints each session's full command, which
for a Claude session is several hundred characters, and it shows neither the harness nor the
working directory. The owner asked for: sessions addressable by name, a default `list` without
the command, a `--details` flag that adds it back, and HARNESS and CWD columns.

## 2. Decisions settled with the owner

- **D1.** `attach`, `kill`, and `messages --session` accept a session name or an id.
- **D2.** The CLI resolves a name to an id itself, from the `List` response it already can
  request. The daemon, `lookup`, and the wire requests do not change. The picker, status bar
  and terminal title keep using ids.
- **D3.** A reference that matches no session name is passed on unchanged. An unknown name or
  id therefore gets the daemon's existing `unknown session <ref>` error, and
  `messages --session` still accepts an id of a session no longer in the registry.
- **D4.** Names cannot look like ids (`s12`), so a reference is never ambiguous. No new
  validation.
- **D5.** `a2amx list` shows these columns by default, in this order: `ID`, `NAME`, `HARNESS`,
  `STATE`, `ATTACHED`, `PENDING`, `HELD`, `QUOTA`, `SIZE`. `a2amx list --details` appends `CWD`
  then `COMMAND`. `COMMAND` stays last because it is the longest cell.
- **D6.** `HARNESS` shows the harness the session runs with, as `claude`, `codex`, `omp` or
  `generic`. `CWD` shows the directory the session was started in, or `-` when none was
  recorded. Neither cell is truncated.
- **D7.** The attach picker (`w`) is unchanged, command column included. `list_agents` and
  `send_message` (`src/mcp.rs`) are unchanged.
- **D8.** The session summary on the wire carries `harness` and `cwd`.

## 3. `src/cli.rs`

Change the `List` variant (the doc comment line stays) to:

```rust
    /// List sessions.
    List {
        /// Also show each session's working directory and command.
        #[arg(long)]
        details: bool,
    },
```

No other change in this file. `attach`'s `session`, `kill`'s `session` and `messages`'
`--session` keep their types; their doc comments gain nothing.

## 4. `src/wire.rs`, `src/session.rs`, `src/harness.rs`, `src/daemon.rs`

`src/harness.rs`: add to `impl Harness` a method returning the same strings serde writes:

```rust
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Generic => "generic",
            Self::Omp => "omp",
            Self::Codex => "codex",
        }
    }
```

`src/wire.rs`: `SessionSummary` (`src/wire.rs:241`) gains two fields after `quota`:

```rust
    #[serde(default, skip_serializing_if = "is_default")]
    pub harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
```

`Harness` is already imported in this file for `Request::NewSession`. A summary whose harness
is `Generic` omits the key, like `address` and `pending` omit their defaults.

`src/session.rs`: `Session` keeps the directory it was started in. Add a field
`cwd: Option<std::path::PathBuf>` to `Session` (after `argv`), set from `spec.cwd` in
`Session::spawn`'s constructor, taking a clone before `spec.cwd` is moved into the PTY
configuration (`working_directory: spec.cwd`, `src/session.rs:137`). Add
`pub fn cwd(&self) -> Option<&std::path::Path>` next to `name()`.

`src/daemon.rs`: in the `Request::List` arm (the `SessionSummary { ... }` literal at
`src/daemon.rs:1008`) add `harness: session.harness(),` and
`cwd: session.cwd().map(|path| path.to_string_lossy().into_owned()),`.

## 5. `src/main.rs`

**List.** `Command::List { details }` replaces `Command::List` (`src/main.rs:75`); `run_list`
takes `details: bool`.

- `SESSION_HEADERS` becomes the nine default headers of D5 and `session_value_rows` returns
  `[String; 9]` rows: the existing cells, with the `HARNESS` cell (`session.harness.as_str()`)
  inserted after `NAME`, and without the command cell.
- Add `const DETAIL_HEADERS: [&str; 11]` (the nine, then `CWD`, `COMMAND`) and make
  `session_value_rows` produce 11-cell rows when details are requested. Reuse the one
  function: change it to return `Vec<Vec<String>>`, or build a second thin function that
  extends each 9-cell row with `cwd.clone().unwrap_or("-")` and `argv.join(" ")`. Whichever is
  smaller, `format_table` and `column_widths` (const-generic over the array length today) are
  extended or called through a slice-based variant so both widths work. The picker's
  `PICKER_HEADERS` / `picker_value_rows` / `session_rows` do not change.
- `format_session_table(sessions, details)` picks the header set.
- Output rule unchanged: cells separated by two spaces, the last column unpadded.

**References.** Add one function:

```rust
/// A session reference from the command line: a name resolves to its id, anything else
/// (an id, or a name no session has) is returned unchanged for the daemon to judge.
fn resolve_reference(sessions: &[SessionSummary], reference: &str) -> String
```

It returns the `id` of the session whose `name` equals `reference`, else `reference.to_owned()`.
Add an async helper `resolve_session(home, reference)` that connects, calls the existing
`request_sessions` (`src/main.rs:1413`), and returns `resolve_reference(...)`. Use it in:

- `run_kill` (`src/main.rs:354`): resolve before sending `Request::Kill`.
- `run_attach_command` (`src/main.rs:394`): resolve before `run_attachment`, so the status bar,
  terminal title, and picker comparison (`selected == self.session`) all see the id.
- `run_messages` (`src/main.rs:363`): resolve `session` when it is `Some`.

`new --detach` continues to print the id (`s1`). Do not change that output.

## 6. Documentation

`README.md`: line 94 block becomes `a2amx list [--details]`; `attach <id|name> [--force]`;
`kill <id|name>`. Append to the paragraph at `README.md:128` (it ends by explaining that ATTACHED means a
human client is attached) the sentence `The default list omits each session's working directory and command; a2amx list --details adds CWD and COMMAND.` and add one line under the command block: `Commands that take a session accept its name or its id.`

`docs/architecture.md`: in the human-controls bullet at line 360 add that `a2amx list`
gains HARNESS and, with `--details`, CWD and COMMAND, and that the CLI resolves a session name to an id
before attach, kill and messages (the daemon still sees ids only).

## 7. Mechanical changes to existing tests

Every other existing assertion stays. Found with `rg '"list"' tests` and
`rg "SessionSummary \{" tests`:

- `tests/wire.rs` lines 111 and 414: the two `SessionSummary` literals do not use `..`, so
  add `harness: Default::default(),` and `cwd: None,` to both. Their JSON expectations do not change
  because both new keys are omitted at their defaults.
- `tests/daemon_cli.rs` lines 189 and 213: use `["list", "--details"]`, because the command
  is no longer in the default output.
- `tests/attach_cli.rs`, the three header assertions and two row assertions, become (exact
  literals, the second listing has no rows):
  - line 242: `"ID  NAME  HARNESS  STATE    ATTACHED  PENDING  HELD  QUOTA  SIZE\n"`
  - line 244 row: `"s1  -     generic  running  no        0        -     -      80x24\n"`
  - line 254: `"ID  NAME  HARNESS  STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE\n"`
  - line 403: `"ID  NAME        HARNESS  STATE    ATTACHED  PENDING  HELD  QUOTA  SIZE\n"`
  - line 405 row: `"s1  agent-plan  generic  running  no        0        -     -      80x24\n"`
  - line 529 (the held row, command `sh -c stty raw -echo; cat`): run it with
    `["list", "--details"]` and assert with `contains` the cell text
    `"s1  -     generic  running  yes       0        human_draft  -      80x23  "` followed by
    the temp directory path the test started the session in, two spaces, and
    `"sh -c stty raw -echo; cat\n"`. The harness cell there is `generic`.
  - lines 54, 76 and 535 assert only fragments (`ID  NAME`, `s1`, `running`, and
    `s1  -     running  yes       0        -`). The last fragment becomes
    `s1  -     generic  running  yes       0        -`; the others stay.

## 8. Tests to add (`tests/refs_and_list.rs`, new)

Start a real daemon through `tests/common` and drive the `a2amx` binary as
`tests/attach_cli.rs` does (copy the small `run_binary` / `cli_args` pattern; do not edit
`tests/common`). One behavior at a time, test first:

1. `kill` by name: `new --detach --name agent-plan -- sh -c "sleep 30"`, then
   `kill agent-plan` succeeds and `list` no longer shows `agent-plan`.
2. `attach` by name: with a PTY (as `tests/attach_cli.rs` does with `PtyHarness`), attaching
   `agent-plan` reaches the same session as `attach s1`; the terminal shows `[detached from s1]`
   after `Ctrl-B d`.
3. `messages --session agent-plan` lists the same rows as `messages --session s1` after one
   message to the session (send through the admin client as `tests/broker.rs` does).
4. Unknown name: `kill nobody` fails with stderr containing `unknown session nobody`.
5. `list` omits the command by default and `list --details` includes `CWD` and `COMMAND`
   cells: the session started in a temp directory shows that path and its command.
6. A session started with `--harness omp -- sh` shows `omp` in the HARNESS cell; an unnamed
   `sh` session shows `generic`.
7. `resolve_reference` has no public seam; the above covers it through the binary.

## 9. Out of scope

- Name resolution inside the daemon, in `lookup`, or in any wire request.
- Changing the picker, `list_agents`, `send_message`, or `message_status`.
- Truncating or wrapping long cells; a `--wide`/`--json` output mode.
- Removing exited sessions in bulk; renaming a session.
- Any change to `new` output, to naming rules, or to address formats.

## 10. Acceptance

All of these, on the final tracked snapshot, from the main checkout:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All three must pass with no warnings. `cargo test` includes the known-flaky PTY tests
(`attach_cli status_line_toggles_and_shows_the_session_address`, the `delivery` suite); a
failure in them is rerun once and reported with its output, not hidden. These checks need the
new code, so they run after implementation, not before.

Manual proof for the report: start a daemon in a temp state dir, run
`a2amx new --detach --name demo -- sleep 30`, then `a2amx list`, `a2amx list --details` and
`a2amx kill demo`, and paste the output.
