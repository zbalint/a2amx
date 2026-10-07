# Spec 3l: team identity, private teams, and team-aware listings

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `79636b6`; gate notes in section 14;
consultant review `m_720` and `m_722`, all four blocking findings applied).
The owner asked for (a) private teams that cannot see or message agents outside the team, (b) a
way for a private team to talk to named teams, (c) a revised team-file schema with `prefix`
renamed to `team`, (d) team-aware `a2amx list` and `list_agents`, and (e) team and role in the
attach status line. Owner decisions are in section 2; the consultant reports `m_713`, `m_715` and
`m_717` (context `a2amx-private-teams`) are input, not review. This spec supersedes spec 3g (the
file-level `prefix`) and makes a team private by default (owner, 2026-10-07), and amends spec 3h D2 in one respect: the role is also shown to the attached
human in the status line (it stays out of `list`, `list_agents` and every message).

**Baseline:** develop at `79636b6`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-private-teams`. Public test seams: `messaging::visible` (pure), `team::parse` and
`team::plan` (`tests/team.rs`), `status::render` (`tests/status.rs`), the `a2amx list` binary
(`tests/refs_and_list.rs`), and the daemon through session-token and admin clients
(`tests/common`, new `tests/teams.rs`).

**Scope.** May edit exactly these files and no others.

- `src/messaging.rs`, `src/wire.rs`, `src/session.rs`, `src/daemon.rs`, `src/team.rs`,
  `src/main.rs`, `src/cli.rs`, `src/status.rs`, `src/mcp.rs` (the `list_agents` description
  string only)
- `tests/team.rs`, `tests/messaging.rs`, `tests/wire.rs`, `tests/status.rs`,
  `tests/mcp.rs` (the exact `list_agents` description string, lines about 219 to 228),
  `tests/refs_and_list.rs`, `tests/common/mod.rs`, and the new file `tests/teams.rs`
- Mechanical `team: None, role: None` additions to the `Request::NewSession` literals in
  `tests/codex.rs`, `tests/daemon.rs`, `tests/broker.rs`, `tests/delivery.rs`, `tests/quota.rs`,
  `tests/hook.rs`, `tests/attach_cli.rs` (which also gets the new picker test 10b),
  `tests/claude_channel.rs`, and to the
  `SessionSummary`, `AgentSummary`, `StatusInfo` and `TeamSession` literals (and the `Conflict`
  literals of `tests/team.rs`, which gain a reason) in the test files above
- `README.md`, `a2amx.toml.example`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/harness.rs`, `src/codex.rs`, `src/delivery.rs`, `src/store.rs`,
`src/channel.rs`, `src/hook.rs`, `src/bridge.rs`, every other test file, every other spec document, `AGENTS.md`, `docs/delivery.md`, the owner's
gitignored `a2amx.toml` (the architect migrates it after acceptance). No new dependency. Do not
commit, stage or merge: leave the diff uncommitted in the working tree.

## 1. Why

Several teams share one daemon, and every session sees every other session. A message from
another team's architect looks like an order from your own. The owner wants a team to be able to
be private, with an explicit, mutual way to open a channel to another team. The team file also
needs a real team identity: today only an optional name `prefix` exists, the daemon never learns
which team a session belongs to, and the key collides in name with the global `--prefix` (the
Ctrl-B key). This spec makes `team` the identity and the name prefix, adds `private` and
`allow`, enforces visibility in the daemon, and shows the team in the listings and the status
line. It closes backlog F3 (scoping of `list_agents`) and the open visibility line in
`docs/architecture.md`.

## 2. Decisions

- **D1. Team file.** File-level keys: `team` (string, replaces `prefix`), `private` (bool,
  default **true** when `team` is set: every team is private unless the file says
  `private = false`), `allow` (array of team names, default empty). Everything else is unchanged:
  `[[session]]` and `name`, `command`, `cwd`, `attach`, `reset`, `control_from`, `watch`,
  `heartbeat`, `role`. When `team` is set every session name becomes `<team>-<name>` (always
  prefixed, as `prefix` was). `prefix` is rejected with
  `prefix was renamed to team; see the README` (hard rename, no alias).
- **D2. Validation (all parse-time, in `team::parse`).** `team` must pass
  `messaging::validate_name` and must not end in `-` (the empty and trailing-dash errors of spec
  3g keep their wording with `team` for `prefix`). An explicit `private` (either value) without `team` is an error.
  `allow` with `private = false` is an error (`allow needs a private team`); `allow` without
  `team` is an error. Each `allow` entry must pass `validate_name`,
  must not equal the file's own `team`, and must not repeat. The final session names are still
  validated and checked for duplicates as today.
- **D3. References.** `watch` and `control_from` entries have three forms, decided by shape:
  `developer` names a session in this file; `other-team/architect` names the session
  `other-team-architect`; `/hand-started` (leading slash) names the global session
  `hand-started` as written. The `/` split is on the first `/` only; both parts (or the part after
  the leading slash) must pass `validate_name` and the joined final name must too. When `team`
  is set, a bare entry must name a session in this file, else
  `unknown session NAME in watch of SESSION` (or `control_from`), with the final session name.
  When `team` is not set, a bare entry is used as written (today's behavior); the other two forms
  still apply. This replaces spec 3g's "prefixed only if it matches a base name, else raw".
- **D4. Team scope.** `messaging::TeamScope { name: String, private: bool, allow: Vec<String> }`
  (`Debug, Clone, PartialEq, Eq, Serialize, Deserialize`). Every session started from a team
  file with `team` set carries `Some(scope)`; the flag form (`team up NAME=EXE`), `a2amx new`
  and every daemon-created session without it carry `None`. Only the team file can set it.
- **D5. Visibility.** `messaging::visible(a: Option<&TeamScope>, b: Option<&TeamScope>) -> bool`
  is true when (both are `Some` with equal `name`) or `a` is not private or `b` is not private
  (`None` is not private) or (`a.allow` contains `b.name` and `b.allow` contains `a.name`). It is
  symmetric. Examples: ungrouped and private: true; non-private team and private team: true;
  two private teams: true only with mutual `allow`; same team: true.
- **D6. Enforcement (daemon, session-token callers only; the admin token sees all).**
  `list_agents` returns only sessions visible to the caller, plus the caller itself.
  `send_message` to a session not visible to the sender fails exactly like an unknown
  recipient (`code::UNKNOWN_RECIPIENT`, message `unknown recipient`), checked right after the
  recipient is resolved and before the self, exited and rate checks. `reset_session` for a
  target not visible to the sender fails with `code::UNKNOWN_SESSION` (`unknown session`),
  checked right after `reset_target` and before the `control_from` check. Exit events
  (`observe_exit`) are queued only for watchers to which the exited session is visible, and a
  heartbeat digest (`heartbeat`) considers only watched peers visible to the watching session.
  If the caller's own session cannot be found while building the `list_agents` answer (a kill
  race), the reply is the plain error `sender session is unavailable`, never an unfiltered list.
  `message_status` and every other request are unchanged.
- **D7. Wire.** `Request::NewSession` gains `team: Option<TeamScope>` and `role: Option<String>`,
  both `#[serde(default, skip_serializing_if = "Option::is_none")]`. The daemon validates them at
  the wire boundary (`validate_name` for the team and each `allow` entry; `allow` only with
  `private`; the role by the shared `messaging::validate_role`, section 3). `SessionSummary`
  gains `team: Option<TeamScope>` (admin `list`, `team::plan`, `list --as`), with
  `#[serde(default, skip_serializing_if = "Option::is_none")]` like the fields around it. `AgentSummary` gains
  `team: Option<String>` (the name only; `private` and `allow` are never shown to agents) and
  `you: bool` (`#[serde(default, skip_serializing_if = "is_default")]`, true only on the caller's own
  entry); `AgentSummary.team` also uses `#[serde(default, skip_serializing_if =
  "Option::is_none")]`, so an ungrouped agent never gets `"team": null` and existing exact-JSON
  tests hold. `StatusInfo` gains `team: Option<TeamScope>` and `role: Option<String>`, both
  `#[serde(default, skip_serializing_if = "Option::is_none")]`. The role
  is not added to `SessionSummary` or `AgentSummary`. The Codex role environment variable path of
  spec 3h is untouched.
- **D8. Plan conflicts.** `team::plan` treats a running session whose `team` differs from the
  wanted session's `team` as a conflict, not `AlreadyRunning`. `Conflict` gains a `reason`
  (`Exited`, `TeamMismatch`). The exited message is unchanged
  (`session NAME has exited (ID); run a2amx kill NAME first`); the new one is
  `session NAME (ID) is running with a different team setting; run a2amx team down first`. The
  all-conflicts-before-any-spawn rule stays. Consequence the docs must state: a running session with the wanted name that was
  started by hand (no scope) or from a team file with other settings conflicts, so re-upping a
  team after adding `private = true`, or over a hand-started `a2amx-architect`, needs a
  `team down` first.
- **D9. `a2amx list` and the session picker (Ctrl-B w).** A `TEAM` column is inserted after `NAME`
  in the plain table, the `--details` table and the picker, only when at least one listed session
  has a team (so output for team-less users is byte-identical to today). The picker uses
  the same cell text, stays unfiltered and keeps list order, selection, the `(current)` marker and
  Enter unchanged; its `CWD` column stays last and keeps its left-shortening and drop-when-narrow
  behavior (with a TEAM column present the fixed prefix is wider, so CWD is hidden on narrower
  terminals than before; accepted). Column indexes and prefix widths are computed from the header
  slice length, not from literals. The value is the
  team name, followed by ` (private)` for a private team, or `-` for an ungrouped session. New
  flags: `--team NAME` keeps only sessions whose team name equals NAME; `--as NAME` keeps only
  sessions visible to the session named NAME (plus that session; NAME resolves like every other
  name argument, an unknown one is an error `unknown session NAME`). The filters apply to `a2amx list` only (the picker always lists every
  session, as the admin sees all) and combine with
  AND and run client-side on the `SessionSummary` list, using `messaging::visible`. No grouping or
  sorting change.
- **D10. `list_agents`.** Entries carry `team` and `you` per D7. The tool description (and only
  that, no input schema change) says that sessions outside the caller's visibility are not
  listed, that `team` is the session's team when it has one, and that `you` marks the caller.
- **D11. Status line.** After the activity and quota cells and before the harness cell the bar
  shows `role ROLE` (when a role is set) and `team NAME` (followed by ` (private)` for a private
  team), each as its own `│`-separated cell like the harness cell. Both are sanitized like the
  address. On a narrow terminal cells drop in this order until the bar fits: team, harness, role,
  quota, activity, pending, hint (today's order with team and role inserted; harness, quota,
  activity, pending and hint keep their relative order).
- **D12. Deliberately left out.** A system-prompt line telling the agent its team (the `you` marker
  and `team` field in `list_agents` give it the same fact without a harness change on three
  harnesses); agent-level `allow`; a `team` in the message envelope; `a2amx new --team`; grouping or sorting of `list`; moving the Codex role path onto the new wire
  field.

- **D13. One scope per team.** `create_session` rejects a session whose team name equals the
  team name of a live (not exited) session whose `private` or `allow` differs, with
  `team NAME is already running with different settings`. Without it one stray member of a
  team could silently defeat the guard (D5 treats same-name sessions as mutually visible, and a
  non-private member sees everything).

## 3. `src/messaging.rs`

Add `TeamScope` (D4) and `visible` (D5) as pure items, and
`pub fn validate_role(role: &str) -> anyhow::Result<()>` holding the role checks that
`validate_sessions` in `src/team.rs` performs inline today (non-empty, at most 64 `chars()`, no
control character, no leading or trailing whitespace; the same four sentences without the
`session NAME:` prefix). `team::validate_sessions` calls it and adds the `session NAME: ` prefix,
so every existing role error message is byte-identical.

## 4. `src/wire.rs`

The fields of D7. `use crate::messaging::TeamScope;` (messaging does not import wire, no cycle).
Every constructor and destructure in `src/` is updated: the `SessionSummary` literal and the
`AgentSummary` literal in `src/daemon.rs`, and the `StatusInfo` literal in
`attachment_status`.

## 5. `src/session.rs` and `src/daemon.rs`

`SessionSpec` and `Session` store `team: Option<TeamScope>` and `role: Option<String>` with
accessors `team(&self) -> Option<&TeamScope>` and `role(&self) -> Option<&str>`.
`create_session` (daemon.rs 257 onward) destructures the two new fields, validates them (D7) with
the same `Response::Error { message }` style as its neighbours, and passes them into `SessionSpec`.
Visibility checks (D6) use a small private helper on `Runtime` or a free function over two
`Session` references that calls `messaging::visible(a.team(), b.team())`; `Request::ListAgents`
(daemon.rs about 1697) needs the caller's session: it already has `role`
(`Role::Session(id)`); an admin caller gets every session and `you: false`.
`send_message` (about 691), `reset_session` (about 775), `observe_exit` (about 461) and
`heartbeat` (about 521) get the checks of D6 at the stated points. `Request::List` fills
`SessionSummary.team` from `session.team().cloned()`; `attachment_status` fills `StatusInfo.team`
and `.role`.

## 6. `src/team.rs`

`TeamFile` gains `team: Option<String>`, `private: Option<bool>` and `allow: Vec<String>` (all
`#[serde(default)]`; the scope's `private` is `private.unwrap_or(true)`, `parse` rejects an
explicit value without `team`), and a hidden `prefix: Option<toml::Value>` used only to emit the D1 error.
`TeamSession` gains `#[serde(skip)] pub team: Option<TeamScope>` (set by `parse` on every
session when `team` is present; `flag_sessions` sets `None`). `parse` applies D1 to D3 in place of
the current prefix block (lines 65 to 89): validate the file-level keys, resolve every
`watch`/`control_from` entry with one small private function implementing the three forms, apply
`<team>-` to every session name, set the scope, then run `validate_sessions` as today (it keeps
the final-name checks). `Conflict` and `plan` gain D8; `plan` compares `existing.team` (new
`SessionSummary` field) against `wanted.team`.

## 7. `src/main.rs` and `src/cli.rs`

`NewOptions` gains `team: Option<TeamScope>`; `run_new` passes `None`; `run_team_up` passes
`session.team` and `session.role` (role already reaches `NewOptions`; it now also goes to the
request); `create_session` puts both on the `Request::NewSession`. `run_team_up` prints the D8
message per conflict reason (`for conflict in conflicts`, main.rs about 561) and keeps returning
the existing error. `src/cli.rs`: `List` gains `--team <NAME>` and `--as <NAME>` (the field is named
`viewer`, since `as` is a Rust keyword, declared `#[arg(long = "as")]`). The list rendering in `main.rs`
(`session_value_rows` and the table printer around lines 1452 to 1500) inserts the `TEAM` column
per D9 in the plain and `--details` tables and in the picker. Required mechanics (least churn,
no change to the three header const arrays or to `session_value_rows`/`picker_value_rows`):
`column_widths` and `format_table` take slices (`&[&str]`, rows `&[R] where R: AsRef<[String]>`,
widths as `Vec<usize>`); one helper returns the team cells (`None` when no listed session has a
team) and one inserts `TEAM` and its cells at index 2 only when they exist, applied in the plain
and `--details` arms and in `session_rows` (picker, about line 2133), where every literal `10`
(`widths[..10]`, `2 * 10`, `PICKER_HEADERS[10]`, `row[10]`, `row[..10]`) becomes
`let cwd = headers.len() - 1` arithmetic. `--as` and `--team` filter the
`Vec<SessionSummary>` for `list` only.

## 8. `src/status.rs`

`render` reads `info.team` and `info.role` and `left_spans` gains the two cells per D11; the
shrink loop gets two more `show_*` flags in the D11 order. No change to widths or colors of
existing cells.

## 9. `src/mcp.rs`

The `list_agents` description per D10; nothing else (the response already serializes
`AgentSummary`).

## 10. Tests (failing first, expected values are literals)

`tests/messaging.rs`, pure:

1. `visible` truth table: (None, None) true; (None, private A) true; (non-private A, private B)
   true; (A, A) true; (private A, private B) false; (private A allow [b], private B allow []) false;
   (private A allow [b], private B allow [a]) true; and symmetry for each pair.
2. `validate_role` accepts `architect` and rejects empty, 65 characters, `a\nb`, ` architect`,
   `architect ` (existing team tests keep passing unchanged).

`tests/team.rs`, pure (`parse`, `plan`):

3. `team = "demo"` prefixes names; each session's `team` is
   `Some(TeamScope { name: "demo", private: true, allow: [] })` (private by default);
   `private = false` gives `private: false`; `allow = ["other"]` is kept in the scope. A file without `team` leaves names unprefixed and
   `team` `None`.
4. Errors, each naming the key: `prefix = "x"` (contains `renamed to team`); empty team; `team =
   "demo-"`; `private = true` and `private = false` each without team; `allow = ["x"]` with `private =
   false`; `allow = ["x"]` without team; `allow` entry equal to
   the team; duplicate `allow` entry; an invalid `allow` name.
5. References with `team = "demo"`: `watch = ["developer"]` becomes `demo-developer`;
   `"other/architect"` becomes `other-architect`; `"/hand"` becomes `hand`; bare `outsider`
   fails with `unknown session outsider in watch of demo-architect`; same for `control_from`;
   `"a/b/c"` and `"/"` and `"x/"` fail. Without `team`, bare `outsider` is accepted as written.
6. The spec-3g tests `parse_leaves_already_final_references_unchanged` and
   `parse_base_name_match_wins_over_already_final_spelling` are deleted or rewritten to the new
   rules; every other `prefix` literal in `tests/team.rs` becomes `team`.
7. `plan`: a running session with the wanted name and a different (or absent) scope yields a
   `TeamMismatch` conflict; same scope yields `AlreadyRunning`; an exited session still yields
   an `Exited` conflict.

`tests/status.rs`: 8. with `team` (private) and `role` set the visible bar text contains
`role architect` and `team demo (private)` in that order before the harness cell; at widths where
today's tests drop the harness, `team` drops first and `role` last of the three; without team or
role the output is byte-identical to today's.

`tests/wire.rs`: 9. round-trip of the new fields; a `NewSession` without them (old client JSON)
still decodes with `None`, and every ungrouped summary serializes byte-identically to today (no
`team`, `role` or `you` key).

`tests/refs_and_list.rs` (binary): 10. with one team session and one ungrouped session `a2amx
list` shows `TEAM` after `NAME` with `demo (private)` and `-`; with none, the header line is
byte-identical to today's; `--team demo` keeps one row; `--as NAME` keeps the sessions visible to
NAME; `--as nosuch` fails with `unknown session nosuch`.

`tests/attach_cli.rs`: 10b. in the style of `picker_switches_sessions_and_exit_status_is_reported`,
start a daemon, create one ungrouped session and one team session through `team up --detach -f
FILE` (`team = "demo"`, `command = ["sh", "-c", "sleep 30"]`; the CLI cannot
set a team on `new`), open the picker with Ctrl-B w and assert on substrings of the lines: the
header has `NAME` then `TEAM` in that order, the team row contains `demo (private)`, the
ungrouped row has `-` in the TEAM position. The existing no-team picker assertions (about lines
333 to 347 and 393 to 396) and `picker_clips_long_cwd_to_narrow_terminal` stay unchanged and
prove the column is absent without teams and CWD clipping still works.

`tests/teams.rs` (new, real daemon in a temp state dir on port 0, `tests/common`; sessions
created through a new `tests/common` helper `new_team_agent(admin, dir, name, harness, deliver,
Option<TeamScope>, watch: &[&str], control_from: &[&str])` that `new_agent` now calls with `None`
and empty slices; the peer that must exit in test 14 is a hand-built `Request::NewSession`
literal in `tests/teams.rs` running `sh -c "exit 0"`, as `tests/broker.rs` does): 11. two private teams `a` and `b`
without `allow`, plus an ungrouped session: an `a` session's `list_agents` lists itself (with
`you: true`) and the ungrouped session but not `b`; `send_message` from `a` to `b` fails with
code `unknown_recipient` and message `unknown recipient` (identical to sending to a nonexistent
name); ungrouped can message and list both. 12. With mutual `allow` the same sends succeed and
both see each other; with one-sided `allow` they do not. 13. `reset_session` from `a` to `b`'s
session (with `control_from` naming the `a` session) fails with `unknown_session`. 14. Exit
event: `a`'s session watches a `b` session by name, the `b` session exits: no `peer exited`
message is queued for the `a` session. A same-team watcher and peer pair runs in the same test
and its `peer exited` message is awaited first (admin `Request::ListMessages`) so the absence
assertion cannot pass before the observer task has run. The heartbeat filter has no test (a
timed digest would be slow and flaky); it is covered by review, and the spec says so.
15. `list_agents` for a team member carries `team` as the plain name and never `private` or
`allow`, and an ungrouped entry has no `team` key at all. 16. D13: a second session of team `a`
with different `private` or `allow` than a live `a` session is refused with the D13 message; the
same settings are accepted; after the first exits (exited sessions do not count) it is accepted.
17. `a2amx list --as NAME` parses (clap), proving the `--as` long flag.

## 11. Documentation

- `README.md` "Teams" (about lines 170 to 240): `prefix` becomes `team`; the example and the
  paragraph at 198 to 202 are rewritten for D1 to D3 (always prefixed, the three reference
  forms, unknown bare names are errors); new paragraphs for `private`, `allow` (mutual consent,
  team level), what a hidden session sees (nothing: not listed, `unknown recipient`), in the first paragraph
  for `private`, that a team is private unless `private = false`, that ungrouped sessions (started by hand) and
  teams with `private = false` see everything and are seen by everyone (so a team is protected
  only from other private teams), that an `allow` entry naming a team with `private = false`
  changes nothing, and that agents never see `private` or `allow`, the
  `team down`/`team up` requirement of D8, `list --team` and `--as`, the status-line cells, and
  the non-goal: a guardrail against mix-ups, not isolation from other processes of the same OS
  user. Fix the flag-form sentence ("has no prefix" becomes "has no team").
- `a2amx.toml.example`: header comment and `prefix` line for `team`, a commented
  `# private = false` line (teams are private by default), a commented `allow` line, one `/name` reference example.
- `docs/architecture.md`: line 202 (the open visibility rule) is resolved with a pointer to this
  spec's rules; the `team` paragraph near line 308 replaces `prefix`; the identity section gains
  the visibility rule; the status-line paragraph (about line 328) mentions team and role.
- `docs/backlog.md`: F3 moves to Closed as `C16`, `Team identity, private teams and team-aware
  listings`, `spec 3l`.

## 12. Out of scope

- Everything in D12, and from the picker: sorting or grouping by team, a team filter key, the role
  column, and marking which sessions the attached one can message (the consultant's report
  `m_722` gives the triggers: more sessions than fit on screen, teams interleaving in practice).
- The system prompt, `src/harness.rs`, `src/codex.rs`, the envelope.
- Persisting scope across daemon restarts (sessions do not survive one).
- Changing the global `--prefix` key option.
- The owner's live `a2amx.toml` (architect migrates it after acceptance).

## 13. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
rg -n 'prefix\s*=' a2amx.toml.example README.md
```

The first three pass with no warnings. The last must print nothing (exit status 1 is the pass
condition): no team-file `prefix =` key remains (the global `--prefix` option text does not
match the pattern). Known-flaky PTY tests (see spec 2n) are rerun
once and reported, not hidden. Also report `git status --short` showing only files in section 0's
scope list, and the exact bare `cargo test` result with its disposition if the `A2AMX_BIN`
contamination fails `generic_sessions_get_no_a2amx_bin`.

## 14. Pre-lock gate notes

- Baseline run on `79636b6`: `env -u A2AMX_BIN -u NO_COLOR cargo test --test team --test status
  --test messaging --test wire --test mcp --test refs_and_list` passes (13, 25, 9, 9, 27 and 22
  tests across the six result lines).
- Consultant review `m_720` (blocking B1 `tests/mcp.rs` scope, B2 serde skip attributes, B3 clap
  `long = "as"`, B4 test helper and positive control; advisories folded into D6, D8, D13, tests
  14 to 17, README and acceptance) and `m_722` (picker: column only, mechanics in section 7). The
  consultant's literal inventories (`NewSession`, `SessionSummary`, `AgentSummary`, `StatusInfo`,
  `TeamSession`, `Conflict`) were checked against section 0's scope: nothing missing.
- Content grep of old text: the team-file `prefix` appears in `src/team.rs`, `tests/team.rs`,
  `README.md` (lines about 180 to 236), `a2amx.toml.example`, `docs/architecture.md` (about 308)
  and `docs/backlog.md` (C11 row, historical, unchanged); the global `--prefix` key option is
  unrelated and stays. The `list_agents` description sentence appears in `src/mcp.rs`,
  `tests/mcp.rs` (in scope) and in locked specs 2 and 2c (untouched).
- Worked examples: team `a2amx` with default `private`, architect watching `developer` and
  `consultant` (same file, D3 local form), heartbeat set: all three sessions get the same scope,
  mutual visibility by D5 rule 1, watch names become `a2amx-developer` and `a2amx-consultant`.
  The same file re-upped against a hand-started `a2amx-architect` (no scope): D8 conflict listed
  with any other conflicts, nothing spawned. A private session watching `/hand` (ungrouped): D5
  gives true both ways, exit events and heartbeats flow. A second private team `b` without
  `allow` against `a`: hidden both ways (tests 11 to 14).
- Rules stated twice, diffed after the last edit: the private default (D1, D2, section 6, tests 3
  and 4, README bullet, example bullet) all say private unless `private = false`; the TEAM column
  rule (D9, section 7, tests 10 and 10b) all say after NAME, only when a listed session has a team.
- Feasibility: the hidden `prefix: Option<toml::Value>` field is accepted under
  `deny_unknown_fields` (it is a declared field); picker and table mechanics were checked by the
  consultant against `src/main.rs` (const-generic `column_widths` and `format_table`).
- Not probed before lock: the exact TOML error text for a wrong-typed `private`; that is serde's.
- Acceptance commands need the new code and run after implementation.
