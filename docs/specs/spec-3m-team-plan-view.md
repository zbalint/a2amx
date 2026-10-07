# Spec 3m: `team status`, `team up --dry-run` and a note about messages lost in a restart

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `9dc715a`; gate notes in section 9;
consultant review `m_762`, all three blocking findings applied).
Follow-up idea 2, 6 and 7a from the consultant triage `m_733` (context `a2amx-followups`),
approved by the owner ("all the ideas sound good, work them out"). Owner decisions: none beyond
that approval. Architect decisions are in section 2. Depends on spec 3l (accepted, `9dc715a`):
`team::plan`, `Conflict` with its reason, `SessionSummary.team`.

**Baseline:** develop at `9dc715a` (the commit that accepted spec 3l). **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-team-plan-view`.
Public test seams: `team::inspect` and `team::plan` (`tests/team.rs`), the `a2amx` binary against a
temp-state daemon (`team status`, `team up --dry-run`, `tests/team.rs` style `run_binary`), and
`redact_argv` through the binary output only.

**Scope.** May edit exactly these files and no others.

- `src/team.rs`, `src/cli.rs`, `src/main.rs`
- `tests/team.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/daemon.rs`, `src/wire.rs`, `src/harness.rs`, `src/omp.rs`, `src/store.rs`,
`src/status.rs`, `src/mcp.rs`, every other test file, `AGENTS.md`, every spec document, the
owner's gitignored `a2amx.toml`. No new dependency, no wire change, no daemon change. Do not commit,
stage or merge: leave the diff uncommitted in the working tree.

## 1. Why

With private teams, "why did `team up` refuse?" and "what exactly will it start?" become common
questions: a conflict can come from an exited session or from different team settings, and the
launch command carries a merged `--settings` and a system prompt the operator never sees. Today the
only way to find out is to run `team up` and read the error, which may already have started some
sessions. This spec adds a read-only `team status`, a `team up --dry-run` that prints the plan and
the redacted launch commands without starting anything, and a one-line note when the daemon
restart lost messages.

## 2. Decisions

- **D1. `a2amx team status [--file FILE]`.** Reads the team file (default `a2amx.toml`, same rules
  as `team up`: relative to the invoking directory, `cwd` untouched) and asks the daemon for its
  session list (`Request::List` only, nothing is written or started). It prints one row per wanted
  session, in file order: `NAME`, `TEAM` (team name, ` (private)` suffix for a private team, `-`
  when none), `STATE`, `NOTE`. `STATE` is `missing` (would start), `running` (with the session id
  in `NOTE`), `exited` (id in `NOTE`) or `team-mismatch` (id in `NOTE`). The `NOTE` of the two
  conflict states is the part of the `team up` message after the semicolon (`run a2amx kill NAME
  first`, `run a2amx team down first`); both commands build their sentences with one shared function
  `conflict_text(reason, name, id) -> String` (the full sentence is `session NAME has exited (ID); run
  a2amx kill NAME first` / `session NAME (ID) is running with a different team setting; run a2amx
  team down first`). `running` means the same name and the same team scope, not that the command,
  cwd or role match the file; the README says so. Exit status is 0 when `team up`
  would proceed and 1 when it would refuse (any conflict); a daemon that is not running (it needs `Request::List`; `--dry-run` has the same requirement), an
  unreadable or invalid file are errors as for `team up`. `team status` returns its exit status
  without printing an `a2amx:` error line (section 4). No `NAME=EXECUTABLE` form.
- **D2. `team::inspect`.** New pure function
  `inspect(wanted: &[TeamSession], existing: &[SessionSummary]) -> Vec<Entry>` with
  `Entry { name: String, state: EntryState }` and
  `EntryState::{Missing, Running { id }, Conflict { id, reason: ConflictReason }}`, one entry per
  wanted session in order. `plan` is rewritten on top of `inspect` so the conflict rule exists once;
  `plan`'s signature, return values and every existing test are unchanged.
- **D3. `a2amx team up --dry-run`.** Same file reading, same daemon `List`, same `plan` and the same
  conflict messages and error as `team up`; then, instead of spawning, prints per wanted session in
  order: `would start NAME` or `already running NAME ID`, followed for a session that would start by
  indented lines: `harness`, `cwd` (the resolved path), `team` (`NAME`, `private` or `public`,
  `allow` list), `role`, `attach`, `watch`, `control_from`, `heartbeat` (each only when set) and
  `command` (the argv a2amx would send, redacted by D4). Nothing is started, attached or installed;
  `--dry-run` conflicts with `--detach` and with `NAME=EXECUTABLE` items (clap). Exit status as
  `team up` (1 on conflicts).
- **D4. Redaction.** The printed `command` shows the executable (`argv[0]`) verbatim. A later
  element is shown verbatim only when the part before any `=` has the exact shape of a flag name,
  `-x` or `--long-name` (`^--?[A-Za-z][A-Za-z0-9-]*$`), and no `--` element precedes it; when it had
  an `=` it is followed by `=<redacted, N bytes>`. Every other element, including anything that
  starts with `-` but is not shaped like a flag and everything after a `--` element, is printed as
  `<N bytes>` (N is the UTF-8 byte length). Example: `claude --model sonnet --settings {"a":1}` is
  shown as `claude --model <6 bytes> --settings <7 bytes>`; `claude -- -secret` as `claude --
  <7 bytes>`; `claude -token123` as `claude <9 bytes>`. Residual disclosure, stated in the README:
  `argv[0]`, flag names and byte counts. This is deliberate: the merged `--settings`
  (spec 3k), the system prompt and any user value can hold secrets, and a dry run is often
  pasted or read by an agent. Environment variables are never printed.
- **D5. Which command is shown.** For a Claude session the real wired argv (what `create_session`
  would send, after `wire_claude_channel_argv`, including the merged `--settings`), redacted. For
  Codex and generic sessions the command as given. For an OMP session the command as given, and the
  line `omp extension install skipped (dry run)`: the OMP wiring installs files under the state
  directory (`a2amx::omp::install`) and a dry run must not write anything.
- **D6. Shared code.** The harness is resolved first with `Harness::infer(&command)` (the dry run does this itself; `team
  up` always passes no harness). The `let command = match harness { ... }` block of `create_session`
  (main.rs about lines 449 to 489) moves unchanged into a helper
  `async fn wired_command(home: &Path, command: Vec<String>, harness: Harness, cwd: &Path, role:
  Option<&str>, no_authorize_peers: bool, no_channel: bool, install_omp: bool) ->
  anyhow::Result<Vec<String>>` (clone `cwd` and `role` inside for the blocking closures, since
  `create_session` uses both again afterwards) used by `create_session` (with
  `install_omp = true`, byte-identical behavior) and by the dry run (with `false`, where the OMP
  arm returns `command` unchanged). Reading a `--settings` file in the Claude arm is a read and is
  allowed in a dry run; it already runs on a blocking thread.
- **D7. Lost-message note.** After the plan is computed, `team up`, `team up --dry-run` and `team
  status` ask, on their own short-lived `Client::connect(&home)` (never the connection that later
  spawns sessions: a response above the 1 MiB frame limit leaves a client unusable), for
  `Request::ListMessages { session: None, state: Some("undeliverable") }` and count the entries whose
  `detail` is `daemon_restarted` and whose `to` has a local part (before `@`) equal to a wanted
  session name. When the count is not zero they print one line on stderr: `note: N message(s) to
  this team were lost in a daemon restart; see a2amx messages --state undeliverable`. Nothing is
  printed for zero. Every failure (connect, I/O, a response too large, an error response) is swallowed:
  the note is skipped, never fails the command and never changes the exit status. `team up` prints it
  before the first spawn or attach; `status` and `--dry-run` print it after their output. Retention is
  the existing 7 days (messages from every restart in that window count), so the note disappears when
  rows are purged. No resend (a later spec).
- **D8. Out of the first cut.** `team status` for the flag form, `--json` output, watching
  (`--follow`), per-session health beyond the four states.

## 3. `src/team.rs`

Add `Entry`, `EntryState` (D2) and `inspect`; `ConflictReason` keeps its two variants. Rewrite the
body of `plan` to call `inspect`: any `Conflict` entry becomes a `Conflict` (same `name`, `id`,
`reason`), otherwise `Missing` becomes `Action::Start(Box::new(session.clone()))` and `Running`
becomes `Action::AlreadyRunning`. A doc comment says `inspect` is the single place where the
conflict rule lives.

## 4. `src/cli.rs` and `src/main.rs`

`TeamAction` gains `Status { file: Option<PathBuf> }` and `Up` gains `#[arg(long, conflicts_with_all
= ["detach", "items"])] dry_run: bool`. Dispatch: `TeamAction::Status { file } => return run_team_status(home, file).await`, the pattern of
`DaemonAction::Status => return run_daemon_status(home).await` (main.rs about line 89 and 257): the
function returns `anyhow::Result<std::process::ExitCode>` (`SUCCESS`, or `ExitCode::from(1)` on a
conflict), so no `a2amx: error` line is printed and the early return bypasses the final
`result.map(|_| SUCCESS)`. `team up --dry-run` on conflicts keeps returning `Err("team has session
conflicts")` exactly as `team up` does. `run_team_up`
gets the `dry_run` flag: after `plan` succeeds it either spawns (today) or prints (D3). Conflict
printing is moved into one function used by `team up`, `team up --dry-run` and `team status`, built
on the shared `conflict_text` (D1). Add
`redact_argv(&[String]) -> String` (D4) as a private function in `src/main.rs` (no new module). The
lost-message note (D7) is one small private async function `note_lost_messages(home, wanted)`.

## 5. Tests (failing first, expected values are literals)

`tests/team.rs`, pure:

1. `inspect` returns, for wanted `[a, b, c, d]` and existing sessions (a running with the same scope,
   b exited, c running with a different scope, d absent): `[Running{a-id}, Conflict{b-id, Exited},
   Conflict{c-id, TeamMismatch}, Missing]`.
2. `plan` is unchanged: the pre-existing `plan` tests pass without edits, and `plan` over the same
   input returns the conflicts of test 1 in file order.

`tests/team.rs`, binary against a temp daemon (the file's existing `run_binary` helpers):

3. `team status` on a file with two sessions, one running (started with `team up --detach`) and one
   missing: stdout has a header `NAME TEAM STATE NOTE` row and one row per session in file order with
   states `running` (id in the note) and `missing`; exit 0 and an empty stderr; no session is added (a second `a2amx
   list` shows the same sessions).
4. `team status` with an exited session (a child that exited: `new --detach --name ... -- sh -c
   "exit 0"`, waited on with `common::eventually` as the existing exited-conflict test does): state
   `exited`, exit 1, note contains `run a2amx kill`, and stderr has no line starting `a2amx:`. With a
   running session of the same final names but different team settings (`team up --detach` of a file
   with `team = "x"`, then `team status` of a file with `team = "x"` and `private = false`):
   `team-mismatch`, exit 1, note contains `team down`.
5. `team up --dry-run` on a Claude, a generic and an OMP entry of a file (use a fake `claude` and
   `omp` executable on `PATH` as the existing wiring tests do, or entries whose `harness` is inferred
   from `command[0]`): stdout contains `would start` for each, no session exists afterwards
   (`a2amx list` empty), and no OMP extension files were created (`!state_dir.join("omp").exists()`). The Claude entry's `command` line contains `--mcp-config`,
   `--settings` and `--append-system-prompt` flag names and no value text: a literal `UNIQUE-SECRET`
   placed in the file's command as `--settings {"x":"UNIQUE-SECRET"}` never appears anywhere in
   stdout or stderr. Exit 0.
6. `team up --dry-run` against an exited conflicting session prints the same stderr line as `team
   up` and exits non-zero without starting anything.
7. `redact_argv` through the binary: the exact line for `["claude", "--model", "sonnet", "--x=ab",
   "pos"]` is `claude --model <6 bytes> --x=<redacted, 2 bytes> <3 bytes>`; for `["claude", "--",
   "-secret"]` it is `claude -- <7 bytes>`; for `["claude", "-token123"]` it is `claude <9 bytes>`
   (neither secret text appears in the output).
8. Lost-message note: start a daemon in a temp dir (`common::start_daemon_in`), create a `Deliver::Hold`
   generic session named like a wanted session and a sender with `common::new_agent`, send a message
   with `Client::connect_addr(addr, token)` + `Request::SendMessage`, shut the daemon down and start
   another on the same dir (the restart pattern of `tests/broker.rs` about lines 1679 to 1723; the
   next open marks the message `undeliverable` / `daemon_restarted`), then `team status` for a
   file naming that session prints the one-line note on stderr with `1 message(s)`; a file naming
   other sessions prints no note; with no lost message it prints nothing.
9. clap rejects `--dry-run` with `--detach` and with `NAME=EXECUTABLE` items.

## 6. Documentation

- `README.md` "Teams": `team status` and `team up --dry-run` (what they print; exit 1 only when `team up`
  would refuse, 0 when sessions are merely missing; both need a running daemon; `running` does not mean
  up to date; values are redacted and why, the residual disclosure of `argv[0]`, flag names and byte
  counts; the OMP extension is not installed in a dry run) and the lost-message note.
- `docs/architecture.md`: one paragraph beside the `team up`/`team down` description (about line 306).
- `docs/backlog.md`: Closed row `C17`, `team status, team up --dry-run, lost-message note`, `spec 3m`.

## 7. Out of scope

D8; any daemon or wire change; resending lost messages (idea 7b); a machine-readable format; the
envelope; agent-level `allow`, `team reset` and the needs-input warning (their own specs).

## 8. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list.

## 9. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` passes on `9dc715a` (404 passed, 27 suites, 3
  ignored, run by the architect when accepting spec 3l; the tree has not changed since).
- Consultant review `m_762`: B1 (redaction allowlist, applied in D4 and test 7), B2 (note on its own
  short-lived connection, skipped when too large, applied in D7), B3 (exit code through an early
  `return` like `DaemonAction::Status`, applied in section 4 and test 4). Advisories folded in: shared
  `conflict_text`, harness inference in D6, helper signature, `running` semantics, notes limited to
  wanted sessions and "lost in a daemon restart" wording, test 8 split, `!state_dir.join("omp").exists()`.
- Facts checked by the consultant against the code: `omp::install` has one caller (`main.rs`
  about 481) and the Claude wiring arm only reads the user's `--settings` file; `store::list` with no
  session has no boot filter; `MessageInfo` has no body field; `Action::Start` holds a `Box` so
  rewriting `plan` on `inspect` keeps `tests/team.rs` valid; no other test constructs `Action` or
  `Conflict`.
- Rules stated twice, diffed after the last edit: redaction rule and examples (D4, test 7, README bullet);
  exit status (D1, section 4, tests 3, 4, 6); note text and filter (D7, test 8); conflict sentences (D1,
  D3, section 4).
- Not probed before lock: interactive behavior of the dry run on a real Claude install (it reads only
  a `--settings` file). Acceptance commands need the new code and run after implementation.
