# Spec 3ce: show the confinement mode and live grants everywhere a user or a peer looks

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context
`a2amx-confine-visibility-2026-11-03`. Public seams: the `a2amx` binary (`list`, `team status`,
`team up --dry-run`, `confine show`), the control protocol types in `a2amx::wire`, the MCP
`list_agents` result, `a2amx::suggest`, `a2amx::status`.

Scope (may be edited): `src/wire.rs`, `src/suggest.rs`, `src/confine.rs`, `src/session.rs`,
`src/daemon.rs`, `src/team.rs` (only the `ConfineRequest` literal near line 703), `src/status.rs`, `src/main.rs`, `src/cli.rs`, `src/mcp.rs` (only if the tool
description needs the new field named), `tests/wire.rs`, `tests/status.rs`, `tests/suggest.rs`,
`tests/attach_cli.rs`, `tests/refs_and_list.rs`, `tests/team.rs`, `tests/auto_confine.rs`,
`tests/confine.rs`, `tests/mcp.rs` (or the test file that pins `list_agents`; the developer names
it), a new `tests/confine_show.rs`; `docs/confinement.md`, `docs/cli.md`, `docs/teams.md`.
Does not touch: `docs/backlog.md`, `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md` (the architect
edits them), `docs/specs/*`, the picker, any delivery or messaging module.

## 1. Why

Confinement is a safety net against accidents, and the owner's first priority for it is
disclosure: a user must be able to tell what is running now. Today a session is `[confined]` in the
status line whether the user wrote its grants or `auto` added them, `list`, `team status` and
`team up --dry-run` say nothing about confinement, no command prints the grants of a running
session, and a peer reading `list_agents` cannot tell whether another agent can write a path, so
agents assume wrongly what the other can reach. Auto mode is silent by design (it adds grants the
user never named), so it must be open about what it did and why.

Decisions (owner, 2026-11-03): auto mode is open about what it does and why, per session, with the
reason for each grant; peers see each other's confinement so an agent does not assume another can
reach something it cannot. Deferred by the architect, not decided by the owner: a drift check
between the team file and running sessions, a tool for a session to re-read its own grants, and a
picker column (backlog F68, F60); the owner can reopen them.

## 2. The data: `ConfineInfo` (`src/wire.rs`)

Add one type and thread it from the request to every view.

```rust
pub struct GrantReason { pub grant: String, pub reason: String }
pub struct ConfineInfo {
    pub auto: bool,                 // auto confinement applied
    pub grants: Vec<String>,        // the session's grants, sorted by name, including the implicit harness-config
    pub write: Vec<String>,         // the operator's `write` entries as given (not the defaults)
    pub reasons: Vec<GrantReason>,  // one per grant auto ADDED (not already set); empty unless auto
}
```

`grants` is `Plan.grants` (the request's grants, sorted) plus `harness-config` when the plan opens
the harness config implicitly (`effective_grants` in `src/confine.rs`, which today never reaches
`Plan.grants`), sorted by name, no duplicates. `auto` stays true when auto ran and added nothing.

- `ConfineRequest` (`src/confine.rs`, serialized on the wire) gains `#[serde(default)] auto: bool`
  and `#[serde(default)] reasons: Vec<GrantReason>`. It is built as a full literal in `src/team.rs`
  and in `tests/confine.rs`; update those literals (`..Default::default()` is fine, the type
  derives `Default`).
- `Plan` keeps `auto`, `reasons` and the request's `write` entries so the session can report them; `Session` exposes
  `confine_info(&self) -> Option<ConfineInfo>` next to `confined()` and the two agree
  (`confine_info().is_some() == confined()`).
- `SessionSummary` and `AgentSummary` each gain `#[serde(default, skip_serializing_if =
  "Option::is_none")] confine: Option<ConfineInfo>`. `StatusInfo` keeps `confined: bool` and gains
  `#[serde(default, skip_serializing_if = "is_default")] confine_auto: bool` (the status line
  needs only the mode).
- Old clients and daemons stay compatible: every new field is defaulted, and an old payload
  without them parses.

## 3. Reasons from the detector (`src/suggest.rs`, `src/main.rs`)

`auto_grants_for` (`src/main.rs`) returns names only today, and `suggest::detect` and
`suggest::git_remote_grants` return names only. Add one pure function to `src/suggest.rs`,
`explain(has_file, git_config, has_host_path) -> Vec<(&'static str, String)>`, with the same inputs as
`grants`, returning the same grants in table order, each with its reason; `grants` stays (used by
`confine suggest`). `auto_grants_for` returns that list. The merge into `ConfineRequest.grants` keeps
its behaviour (a grant already present is skipped) and also sets `auto = true` and `reasons` to the
reasons of the grants it actually added, so the reasons agree with the stderr line. A grant the
operator already set shows `set` in `confine show`, even when auto would have found it. The reason
text, worded exactly:

| Grant source | Reason text |
| --- | --- |
| a marker file | `<marker> found` (the first marker present, for example `Cargo.toml found`) |
| git remote is ssh | `git remote uses ssh` |
| git remote is https and `~/.config/gh` exists | `git remote uses https and ~/.config/gh exists` |
| git remote is https and a git credentials file exists | `git remote uses https and a git credentials file exists` |
| `cache` | `always added by auto` |

A grant with several markers (for example `java`) shows the first marker present in the grant's `detect` order. The one stderr line per session
(`auto confinement for session <name>: grants <a, b>`) stays and is unchanged in wording.

## 4. Where the mode shows

1. Status line (`src/status.rs`): `[confined auto]` for an auto session, `[confined]` otherwise,
   same styling.
2. `a2amx list` (`src/main.rs`, `SESSION_HEADERS`, `DETAIL_HEADERS`): a `CONFINE` column right after
   `HARNESS` with the values `off`, `manual` or `auto`. The picker's `PICKER_HEADERS` do not change.
   An exited session shows its recorded value.
3. `a2amx team status`: a `CONFINE` column after `STATE`, the running session's value, blank when the
   session is not running.
4. `a2amx team up --dry-run`: every session that would start prints `confinement: auto` or
   `confinement: manual` and, for auto, one line per reason (`  cache: always added by auto`); an
   `already running <name> <id>` line gains ` (confine <off|manual|auto>)`.
5. New command `a2amx confine show [SESSION]` (`src/cli.rs`, `src/main.rs`): one session by name or
   id (the same lookup `screen` uses), or every session with no argument. Per session it prints a
   header `<name> <id> <harness> confine <off|manual|auto>` and, when confined, a table
   `GRANT  SOURCE` with `auto: <reason>` for grants auto added and `set` for the others, then a
   line `write: <entries>` when the operator listed any. It reads the
   daemon, never the team file, and prints a plain `no sessions` line when none exist. A SESSION that
   matches nothing is an error that names it.
6. MCP `list_agents` (`src/daemon.rs` `list_agents`, forwarded as is by `src/mcp.rs`): each agent
   carries `confine` as in section 2, including `write`, so a peer can tell what the agent may write
   beyond its working directory (`cwd` is already in the result); the field is absent for an
   unconfined agent. The daemon fills it from its own record of the session.
7. The picker keeps its columns. `session_value_rows` (`src/main.rs:2780`) feeds both the tables and
   the picker; add the `CONFINE` cell only to the table paths so `PICKER_HEADERS` and picker rows do
   not shift.

## 5. Docs

- `docs/confinement.md`: describe the status tag, the `CONFINE` column, `confine show`, and that
  auto reports its reasons; add one troubleshooting row: "I do not know what a session may touch" →
  `a2amx confine show`.
- `docs/cli.md`: the `CONFINE` column, `confine show`, the new dry-run lines.
- `docs/teams.md`: the `team status` column and the dry-run lines.

## 6. Out of scope

- Drift detection, a session re-reading its own grants, the picker, session-level `auto`.
- A warning or confirmation before auto starts sessions, and marking which grants can open an
  escape (owner direction of 2026-11-03: auto mode only, the rule for remembering a confirmation is
  undecided). That is a separate spec after the escape audit (backlog F71).
- Showing the writable, hidden and protected path lists of a running session (the dry run before
  launch keeps doing that).
- Any change to which grants auto picks, to the status line colors, or to `confine list` and
  `confine suggest` output.
- The version number and changelog (the architect).

## 7. Trust boundaries

`list_agents` crosses from one session to another: a session now reads a peer's mode, grant names
and operator `write` entries. Disposition: constrained. The values come from the daemon's own
session record, set at spawn from the operator's `ConfineRequest`. Only a spawning client (the
operator's CLI) supplies `auto` and `reasons`; a session token cannot spawn
(`session_token_permitted`, `src/daemon.rs`) and `Reset` rebuilds nothing from caller input. The
result is limited to the peers `list_agents` already shows (`messaging::visible`). Grant names,
reasons and `write` entries are operator-chosen paths and marker names, no secrets. Abuse case
(Acceptance 8): a session token cannot spawn, and a session-token `Reset` leaves the reported
`confine` unchanged.

## 8. Acceptance

Baseline (existing, run before editing): `env -u A2AMX_BIN -u NO_COLOR cargo test` passes.
Final: all three commands from `AGENTS.md` pass with no warnings:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Behaviors (each pinned by a test through a public seam; the judge is the named test failing when the
code is broken):

1. `wire`: an old `SessionSummary`, `AgentSummary` and `StatusInfo` JSON without the new fields
   parses with `confine: None` and `confine_auto: false`; a payload with them round-trips
   (`tests/wire.rs`; the old-payload test passes on the old code, a contract test).
2. `suggest::explain`: each row of the section 3 table is produced for its input; `java` with both
   `pom.xml` and `build.gradle` present shows the first marker in `detect` order
   (`tests/suggest.rs`). With the scenario of section 3 (global `auto = true`, `grants = ["cargo"]`,
   `Cargo.toml`, an ssh remote) the request carries `auto: true`, grants `cache`, `cargo`, `ssh`
   and reasons for `ssh` and `cache` only, and `confine show` prints `cargo` as `set`
   (`tests/auto_confine.rs`).
3. Status line shows `[confined auto]` for an auto session and `[confined]` for a manual one
   (`tests/status.rs`; the `[confined]` assertion already exists).
4. `list` and `list --details` print the `CONFINE` column with `off`, `manual` and `auto`
   (`tests/attach_cli.rs`, `tests/refs_and_list.rs`; update the four pinned header lines).
5. `team status` prints the column; `team up --dry-run` prints the mode and, for auto, the reason
   lines; an `already running` line carries the mode (`tests/team.rs`, `tests/auto_confine.rs`; the
   existing pinned `already running` expectations are updated).
6. `confine show` prints the header and table for a confined session, handles no sessions and an
   unknown SESSION (`tests/confine_show.rs`, real daemon in a temp state dir on port 0).
7. `list_agents` carries `confine` for a confined agent and omits it for an unconfined one.
8. Abuse case: a request made with a session token to spawn is refused, and the `confine` reported
   for a session is unchanged after a session-token `Reset` (`tests/daemon_cli.rs` or the test file
   that already covers `session_token_permitted`; the developer names it). The first assertion
   passes on the old code (contract test).
9. `rg -n 'confined: bool' src/wire.rs` still matches `StatusInfo` (the field stays for old
   clients).

Known flaky tests: none new; if a test in the flake family `F37` of `docs/backlog.md` fails once,
rerun it once and record it as pre-existing if it passes.

## 9. Claims (draft; verified how)

| Claim | Where | Verified |
| --- | --- | --- |
| `StatusInfo.confined` exists and drives `[confined]` | `src/wire.rs:472`, `src/status.rs:430` | read |
| `SessionSummary` and `AgentSummary` have no confinement field | `src/wire.rs:341`, `:408` | read |
| `Session::confined()` is `spec.confine.is_some()` | `src/session.rs:382`, `:499` | read |
| `Plan` already holds `grants: Vec<String>` | `src/confine.rs:155` | read |
| Auto merges grants into the request after `auto_grants_for`, then prints the stderr line | `src/main.rs:675`, `:1433-1452` | read |
| `team status` builds its table from `inspect` over the session list | `src/main.rs:980-1000` | read |
| Tests pin the `list` header lines and the `already running` lines | `tests/attach_cli.rs:787,819,968`, `tests/refs_and_list.rs:367`, `tests/team.rs:1863,2014` | grep |
| `Plan.grants` is the request's grants sorted and excludes the implicit `harness-config` (a local `effective_grants`) | `src/confine.rs:1649-1650`, `:998-1029` | read (consultant, architect) |
| `ConfineAction` is defined at `src/cli.rs:176` and dispatched at `src/main.rs:80-87` | read | read |
| `ConfineRequest` is built as full literals in `src/team.rs:703` and in `tests/confine.rs` (171, 240-254, 283-374) | grep | grep |
| Session tokens cannot spawn: `session_token_permitted` allows only `SendMessage`, `Reset`, `ListAgents`, `MessageStatus`, `ReportPrompt`, `BridgeAttach` | `src/daemon.rs:2153-2162` | read |
| `tests/team.rs:1449` pins the header `NAME TEAM STATE NOTE\n` | `tests/team.rs:1449` | grep |
| `session_value_rows` feeds the tables and the picker | `src/main.rs:2780`, `:2811`, `:2932`, `:3487` | read |
| Wire derives have no `deny_unknown_fields`; new fields follow the `Option`+`skip_serializing_if` and `is_default` style | `src/wire.rs` | read (consultant) |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5 (not fresh: its session could not be reset by the architect); developer and tester OMP, working slot openai-codex/gpt-5.6-luna, advisor slot openai-codex/gpt-5.6-luna (the developer's advisor model was not exposed). BLOCKED/amend cycles: 1 (the developer's BLOCKED was a misreading of three function signatures, retracted without an amendment).

| Found at | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant, Opus 5.5 | Section 1 listed the deferred items as rejected while grilling, but no owner turn rejected them | Reworded as the architect's deferrals |
| Draft review | consultant, Opus 5.5 | `ConfineRequest` is built as a full literal in `src/team.rs`, outside the scope list | `src/team.rs` added to scope |
| Draft review | consultant, Opus 5.5 | "grants, table order" contradicted `Plan.grants` (sorted by name); the implicit `harness-config` never reached `Plan.grants` | Sorted by name, implicit grant included |
| Draft review | consultant, Opus 5.5 | The auto plus explicit `cargo` scenario would show `cargo` as auto-found, contradicting the stderr line | Reasons only for grants auto added |
| Draft review | consultant, Opus 5.5 | `suggest::detect` returns names only; Acceptance 2 named an input that cannot occur; Acceptance 8 was not a real test; `tests/team.rs:1449` pin unlisted | New `suggest::explain`, real abuse tests, pin listed |
| Draft review | consultant, Opus 5.5 | Peers need the operator `write` entries to know what another agent can reach | `write` added to `ConfineInfo` |
| Implementation | developer, gpt-5.6-luna | BLOCKED claimed three test files held struct literals; they hold function signatures | Retracted, no amendment; the architect had not run the full struct-literal inventory before lock (gate step 10) |
| Architect review | architect, Sonnet 5.5 | `explain` copied the git-remote parser instead of reusing `git_remote_grants` | Rebuilt on `detect` and `git_remote_grants` (fix round 1) |
| Architect review | architect, Sonnet 5.5 | A stray doc-comment hunk in `src/wire.rs`; the section 5 docs were missing from the first report | Removed; docs added |
| Tester pass | tester, gpt-5.6-luna | Mutation survivor: `session_confinement_mode` forced to `off` passed the suite; four gaps (default `confine_auto`, grant sorting and implicit grant, reset with auto reasons) | Tests added in fix round 1 |
| Architect review | architect, Sonnet 5.5 | Mutation survivor: `auto` set only when grants were added passed the suite | Test added in fix round 2 |
| Tester pass | tester, gpt-5.6-luna | Linker SIGBUS: `/tmp` was 93% full | Target directory moved to `~/.cache`; not a spec problem |

Follow-up, not fixed here (backlog F72): `confine show` labels the implicit `harness-config` grant `set`, which the operator did not set.
