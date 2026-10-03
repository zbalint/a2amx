# Spec 2o: `a2amx team up` and `team down`

## 0. Status

**LOCKED.** Decisions D1 to D11 in section 2 were settled with the owner, except D10 (the
`toml` dependency), which is the architect's choice and is flagged for the owner's review.
Requires spec 2m to be implemented first (session names resolving to ids, the `harness` and
`cwd` fields on `SessionSummary`); it is applied to the same uncommitted tree. The work is done
directly on `develop` in the main checkout, no worktree. Shared task `context_id`:
`a2amx-team`. Public test seams: the pure functions in `src/team.rs` (`parse`, `plan`,
`flag_sessions`) and the `a2amx` binary against a real daemon in a temp state dir
(`tests/common`).

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/team.rs` (new), `src/lib.rs`, `src/cli.rs`, `src/main.rs`
- `Cargo.toml`, `Cargo.lock` (the `toml` dependency only)
- `tests/team.rs` (new), `tests/refs_and_list.rs` (only if a helper must be shared; prefer a
  private copy in `tests/team.rs`)
- `README.md`, `docs/architecture.md`, `AGENTS.md` (one row in the Modules table)

Does not touch: every other file under `src/` (`src/daemon.rs`, `src/wire.rs` and
`src/session.rs` included: the daemon and the wire do not change), every other file under
`tests/`, `tests/common`, `scripts/`, the locked spec documents. Do not commit and do not
merge: leave the diff uncommitted in the working tree.

## 1. Why

The owner starts a working pair (an architect on `claude`, a developer on `omp` or `codex`) by
launching each session by hand with `a2amx new --detach --name ... -- <command>`. `team up`
starts a described set in one command, and `team down` ends it.

## 2. Decisions settled with the owner

- **D1.** `a2amx team up` and `a2amx team down`. No `status` subcommand: `a2amx list` covers it.
- **D2.** Two ways to name the sessions: a TOML file, or `NAME=EXECUTABLE` arguments.
- **D3.** `team up` starts each missing session exactly as `a2amx new --detach --name N --
  <command>` does (same harness inference, same wiring), in file order.
- **D4.** A session that is already running is skipped and reported. `up` never restarts or
  replaces a running session.
- **D5.** Preflight before any spawn: validate the input, then classify each name against the
  daemon's current sessions. An exited session with that name is a conflict: `up` stops
  before spawning anything and names each conflicting session. The check and the spawns are
  not atomic (accepted by the owner, mark with a `// shortcut:` comment).
- **D6.** If a spawn fails after others started, the started sessions stay, the failure is
  reported, `up` starts no further session, and the exit status is non-zero. No rollback.
- **D7.** Attach: the file may mark at most one session `attach = true`; in the flag form the
  first session is attached. `--detach` suppresses attaching. The daemon is reached the way
  `new` reaches it.
- **D8.** No PATH check of executables; a bad command fails at spawn like `new` does. No
  per-session harness field (inference covers it), no roles, no initial prompts, no restart,
  no quota handling.
- **D9.** The default file is `a2amx.toml` in the current directory; a missing file is a clear
  error.
- **D10.** TOML needs a parser and the standard library has none, so add the `toml` crate (the
  latest release `cargo` resolves, serde support, default features). `serde_json` is not used
  because a hand-edited file benefits from comments. **Flag the chosen version in the report.**
- **D11.** `team down` kills the sessions named by the file or the names given, running or
  exited, and reports names that have no session.

## 3. `src/team.rs` (new) and `src/lib.rs`

`src/lib.rs`: add `pub mod team;` in alphabetical position. This module is pure: no I/O, no
`tokio`, no daemon calls.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamSession {
    pub name: String,
    pub command: Vec<String>,
    /// Relative paths are resolved by the caller against the team file's directory.
    pub cwd: Option<String>,
    pub attach: bool,
}

pub fn parse(text: &str) -> anyhow::Result<Vec<TeamSession>>
pub fn flag_sessions(specs: &[String]) -> anyhow::Result<Vec<TeamSession>>
pub fn plan(wanted: &[TeamSession], existing: &[SessionSummary]) -> Result<Vec<Action>, Vec<Conflict>>
```

`parse` reads the file format below. Unknown keys are an error
(`#[serde(deny_unknown_fields)]`). Validation, each failure an `anyhow` error that names the
offending session by name or 1-based position: `name` is required and passes
`messaging::validate_name` (`src/messaging.rs:168`); names are unique within the file;
`command` is a non-empty array of strings; at most one `attach = true`; at least one session.

```toml
[[session]]
name = "architect"
command = ["claude"]
attach = true            # optional, default false

[[session]]
name = "developer"
command = ["omp"]
cwd = "."                # optional; relative paths are relative to this file's directory
```

`flag_sessions` parses `NAME=EXECUTABLE` items: split at the first `=`; the name passes
`validate_name`; the executable is non-empty and contains no whitespace (error text says a
command with arguments needs a team file); `command = [executable]`, `cwd = None`; names
unique; the first session has `attach = true`.

`plan(wanted, existing)` returns, in `wanted` order, one `Action` per session, or all the
conflicts:

```rust
pub enum Action { Start(TeamSession), AlreadyRunning { name: String, id: String } }
pub struct Conflict { pub name: String, pub id: String }   // an exited session holds the name
```

A name with no session in `existing` is `Start`; a session with that `name` and
`exit_code == None` is `AlreadyRunning`; with `exit_code` set it is a `Conflict`. If any
conflict exists, `plan` returns `Err` with all of them and no `Action`s.

## 4. `src/cli.rs`

Add to `Command`:

```rust
    /// Start or stop a set of sessions described by a team file or NAME=EXECUTABLE arguments.
    Team {
        #[command(subcommand)]
        action: TeamAction,
    },
```

```rust
#[derive(Debug, Subcommand)]
pub enum TeamAction {
    /// Start the sessions that are not running.
    Up {
        /// Team file (default: a2amx.toml). Cannot be combined with NAME=EXECUTABLE items.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Do not attach to any session.
        #[arg(long)]
        detach: bool,
        /// NAME=EXECUTABLE; the first session is attached.
        items: Vec<String>,
    },
    /// Kill the team's sessions, running or exited.
    Down {
        #[arg(long)]
        file: Option<PathBuf>,
        /// Session names (default: every name in the team file).
        names: Vec<String>,
    },
}
```

`--file` together with items (up) or names (down) is an error before any daemon call.

## 5. `src/main.rs`

**Reuse `run_new`'s core.** `run_new` (`src/main.rs:~262`) builds the argv wiring for the
harness, the environment, sends `Request::NewSession`, and returns the session id. Extend it
rather than copy it: split the part from `let harness = ...` through the `Created` match into
`async fn create_session(home: &Path, client: &mut Client, options: NewOptions) -> anyhow::Result<String>`
where `NewOptions` carries `name`, `harness: Option<Harness>`, `deliver`, `no_authorize_peers`,
`no_channel`, `command`, and `cwd: PathBuf`, and `cols`/`rows`. `run_new` calls it with
`cwd = std::env::current_dir()?`; its observable behavior and output do not change.

**`run_team_up`.**
1. Resolve the sessions: `--file` or `a2amx.toml` (read the file; a missing file's error names
   the path) via `team::parse`, with each relative `cwd` joined to the file's directory and a
   missing `cwd` set to the current directory; or `team::flag_sessions` with the current
   directory.
2. Connect, `request_sessions`, call `team::plan`. On `Err(conflicts)` print to stderr, one
   line per conflict, `session <name> has exited (<id>); run a2amx kill <name> first`, and
   exit non-zero. Nothing was spawned.
3. For each `Action`, in order: `AlreadyRunning` prints `already running <name> <id>`;
   `Start` calls `create_session` and prints `started <name> <id>`. On the first spawn error,
   print `failed <name>: <error>` to stderr and stop; exit non-zero. Add
   `// shortcut: the conflict check and the spawns are not atomic; a name taken in between
   fails the spawn. A daemon-side batch request if concurrent clients make this matter.`
4. After the loop, if some session is marked `attach`, `--detach` was not given, and stdin is a
   terminal, attach to it (running or just started) with `run_attachment`. If stdin is not a
   terminal, do not attach and do not fail.

**`run_team_down`.** Resolve names (given names, else the file's); `request_sessions`; for each
name kill its session by id with `Request::Kill` and print `killed <name> <id>`; a name with
no session prints `no session <name>` and does not fail the command. A kill error is reported
as `failed <name>: <error>`, the loop continues, and the exit status is non-zero at the end.

## 6. Tests (`tests/team.rs`, new)

Pure, through `a2amx::team`:
1. `parse` accepts the example in section 3 and returns exactly the two sessions.
2. `parse` rejects: a missing `name`; an invalid name (`Bad_Name`); a duplicate name; an empty
   `command`; two `attach = true`; an unknown key; an empty file.
3. `flag_sessions(["architect=claude", "developer=omp"])` gives two sessions, the first with
   `attach`. It rejects `"architect=claude -p"` (whitespace in the executable), `"architect"`
   (no `=`), `"=claude"` (empty name), `"architect="` (empty executable), and
   `["a=cat", "a=cat"]` (duplicate name). `"a=b=c"` is accepted as name `a`, executable `b=c`.
4. `plan`: absent name gives `Start`; running gives `AlreadyRunning` with its id; exited gives
   `Err` with that conflict; two conflicts are both returned and no actions.

Binary, real daemon in a temp state dir (as `tests/attach_cli.rs` does, with its own
`run_binary`): 
5. `team up --detach --file team.toml` with two `sh -c "sleep 30"` sessions starts both;
   `list` shows both names running; output has `started architect s1` and
   `started developer s2`.
6. Running it again prints `already running` for both and starts nothing (`list` still has two
   sessions).
7. Exited conflict: start a team whose command exits immediately (`["sh","-c","exit 0"]`),
   wait until `list` shows `exited`, run `team up` again: it exits non-zero, stderr names the
   session and says `a2amx kill`, and no new session was created.
8. Flag form: `team up --detach architect=cat developer=cat` starts two sessions.
9. `team down --file team.toml` kills both; a second `team down` prints `no session` for both
   and succeeds.
10. A missing default file (`team up` in a directory with no `a2amx.toml`) fails and the error
    names `a2amx.toml`. `--file` with items fails before any daemon call.
11. A relative `cwd = "sub"` in a file in a temp directory starts the session in that
    directory (observe with `list --details` from spec 2m).

## 7. Documentation

`README.md`: add to the command block
`a2amx team up [--file F] [--detach] [NAME=EXE ...]   # start the sessions of a2amx.toml (or NAME=EXE)`
and
`a2amx team down [--file F] [NAME ...]`
and a short subsection `Teams` showing the file example of section 3 and stating D4, D5 and
the bare-executable limit of the flag form. `docs/architecture.md`: a short paragraph under the
CLI description stating that `team` is a client-side feature (the daemon and wire are
unchanged), the preflight rule, and the accepted non-atomicity. `AGENTS.md` Modules table: add
`| \`team\` | The team file and flag formats and the up-plan (pure; the commands live in \`main\`) |`
after the `cli` row if that keeps alphabetical grouping, otherwise at the end.

## 8. Out of scope

- Any daemon, wire or MCP change; a batch-create request.
- A harness field, roles, initial prompts, restart policies, quota checks, PATH checks.
- `team status`, `team list`, editing a team file from the CLI.
- Rolling back started sessions on failure.
- Removing exited sessions automatically.

## 9. Acceptance

After implementation, from the main checkout, on the final tracked snapshot:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported,
not hidden. These need the new code, so they run after implementation. Also report the
resolved `toml` version, and a manual transcript: a temp state dir daemon, a two-session
`team.toml`, `a2amx team up --detach`, `a2amx list`, a repeated `team up`, `a2amx team down`.
