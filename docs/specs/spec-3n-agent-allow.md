# Spec 3n: agent-level `allow` between private teams

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `9dc715a`; gate notes in section 11;
consultant review `m_764`).
Follow-up idea 1 from the consultant triage `m_733` (context `a2amx-followups`), approved by the
owner ("all the ideas sound good, work them out"). Architect decisions are in section 2. Depends on
spec 3l (accepted, `9dc715a`): `TeamScope`, `messaging::visible`, the six enforcement sites.

**Baseline:** develop at `9dc715a`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-agent-allow`.
Public test seams: `messaging::visible` (pure, `tests/messaging.rs`), `team::parse` and
`team::plan` (`tests/team.rs`), the daemon through session-token and admin clients (`tests/teams.rs`),
and `a2amx list --as` (`tests/refs_and_list.rs`).

**Scope.** May edit exactly these files and no others.

- `src/messaging.rs`, `src/session.rs`, `src/daemon.rs`, `src/team.rs`, `src/main.rs`
- `tests/messaging.rs`, `tests/team.rs`, `tests/teams.rs`, `tests/refs_and_list.rs`,
  `tests/wire.rs`, `tests/status.rs` (mechanical: every `TeamScope { ... }` literal in these files
  gains `agents: Vec::new()`, and every `TeamSession { ... }` literal in `tests/team.rs` gains
  `allow: Vec::new()`)
- `README.md`, `a2amx.toml.example`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/wire.rs` (no new field: the list rides on `TeamScope`, which is already on
`Request::NewSession`, `SessionSummary` and `StatusInfo`), `src/cli.rs`, `src/mcp.rs`,
`src/status.rs`, `src/harness.rs`, every other test file, every other spec document, `AGENTS.md`,
the owner's gitignored `a2amx.toml`. No new dependency. Do not commit, stage or merge: leave the
diff uncommitted in the working tree.

## 1. Why

Team-level `allow` (spec 3l) opens every member of one private team to every member of the other.
The real cross-team use is narrower: one architect talking to another architect, with the
developers and consultants staying hidden. An agent-level list lets two named agents see and
message each other across private teams while everything else about both teams stays private.

## 2. Decisions

- **D1. Team file.** A `[[session]]` may have `allow = ["other-team/architect"]`: the agents of
  other teams this session consents to see and message. Each entry has exactly the shape
  `team/agent` (one `/`, both parts valid names, the joined final name `team-agent` valid); the
  bare, `/global` and `a/b/c` shapes are errors. The entry's team must differ from the file's own
  team and entries must not repeat; at most 16 entries (the team-level `allow` of spec 3l has no limit; the cap bounds the wire
  payload only). A session-level `allow` needs a private team:
  with `private = false` (or no `team`) it is an error (`allow needs a private team`). It is
  independent of the file-level `allow` (team level) and the two add up.
- **D2. Scope.** `messaging::TeamScope` gains `agents: Vec<String>` with
  `#[serde(default, skip_serializing_if = "Vec::is_empty")]`: the final names this session consents
  to. `team::parse` now gives each session its own scope (team name, `private` and team-level
  `allow` as before, plus its own `agents`). Because the field rides on `TeamScope`, no wire type
  gains a field and `team::plan`'s scope comparison also notices a changed list (a running session
  with other `agents` is a `TeamMismatch` conflict). The daemon's one-scope-per-team check (spec
  3l D13) compares only `private` and `allow`, never `agents`.
- **D3. Visibility.** `messaging::visible` changes from `(Option<&TeamScope>, Option<&TeamScope>)`
  to `(Party, Party)` with `pub struct Party<'a> { pub name: Option<&'a str>, pub team:
  Option<&'a TeamScope> }`. It is true when the 3l rule holds (same team, or either side not
  private, or mutual team-level `allow`) **or** both sides have a scope and a name, `a.team.agents`
  contains `b.name` and `b.team.agents` contains `a.name`. It stays symmetric. Mutual consent is
  required for the pair, as for team-level allow. A session sees exactly the agents it is allowed
  by name plus whatever the 3l rule already gives; the rest of the other team stays hidden.
- **D4. Enforcement.** The six sites of spec 3l D6 (`list_agents`, `send_message`,
  `reset_session`, exit events, heartbeat, and the client-side `list --as`) and nothing else call
  `visible`; each builds a `Party` from the session (new `Session::party(&self) -> Party<'_>`) or,
  for `list --as`, from a `SessionSummary` (`name.as_deref()` and `team.as_ref()`). No other
  behavior changes.
- **D5. Daemon validation.** `create_session` validates `scope.agents` at the wire boundary: each
  entry passes `validate_name`, none repeats, none equals the session's own name, at most 16, and a
  non-empty list requires `scope.private` (same message `allow needs a private team`). It does not
  require the entry to be a currently existing session.
- **D6. Display.** Nothing new is shown to agents (`AgentSummary` is unchanged); `a2amx list`,
  `--details` and the picker show no `agents` (the TEAM cell is unchanged); `list --as NAME` applies
  D3 (it needs the viewer's name as well as id and scope: keep all three in `run_list`).
- **D7. Out of scope.** Wildcards (`other-team/*`), role-based entries, agent-level `allow` for a
  session of the same team (no-op), changing a running session's list (needs `team down`, as for
  every scope change), exposing `agents` in any listing.

## 3. `src/messaging.rs`

`TeamScope.agents` (D2); `Party`; the new `visible` (D3) with a doc comment stating the full rule in
three lines. The 3l truth-table test is rewritten for `Party` (section 7).

## 4. `src/session.rs` and `src/daemon.rs`

`Session::party`. Every `messaging::visible(a.team(), b.team())` call from spec 3l becomes
`messaging::visible(a.party(), b.party())` (send, reset, `ListAgents` filter, `observe_exit`,
`heartbeat`). `create_session` gets the D5 checks next to the 3l `allow` checks.

## 5. `src/team.rs` and `src/main.rs`

`TeamSession` gains `#[serde(default)] pub allow: Vec<String>` (the raw entries; the table key is
`allow`); `parse` resolves each entry to the final name `team-agent`, validates D1 per session with
errors naming the session (`session NAME: allow ...`), and builds a per-session `TeamScope`
(`agents` = resolved names) instead of one cloned scope. `flag_sessions` gives `allow: Vec::new()`.
`run_list` (`--as`) builds `Party` values for the viewer and each session. Nothing else in `main.rs`
changes (the scope already travels on the request).

## 6. Documentation

- `README.md` "Teams": the session-level `allow` key (shape `team/agent`, mutual consent, needs a
  private team, adds to team-level `allow`), with the file-level and session-level `allow` examples
  side by side because the same key name means team names at file level and `team/agent` at
  session level; that changing a session's `allow` makes it a `team-mismatch` so the session (for the
  architect, the human's live one) must be taken down with `team down` before the next `team up`
  (`team status`, spec 3m, shows it); and that a cross-team `control_from` entry (`other-team/agent`) works
  only when the two agents can see each other, so mutual `allow` lets one architect reset the other if
  that session lists it in `control_from`.
- `a2amx.toml.example`: a commented session-level `allow` line next to the commented file-level one.
- `docs/architecture.md`: the visibility sentence added by 3l gains the agent-level clause.
- `docs/backlog.md`: Closed row `C18`, `Agent-level allow between private teams`, `spec 3n`.

## 7. Tests (failing first, expected values are literals)

`tests/messaging.rs`, pure (`Party { name, team }`):

1. The 3l truth table, unchanged in outcomes, rewritten for `Party`.
2. Ungrouped parties (`team: None`, with or without a name) are visible to and from every party. Agent consent: `a2amx-architect` (team `a2amx`, private, `agents = ["saltmdb-architect"]`) and
   `saltmdb-architect` (team `saltmdb`, private, `agents = ["a2amx-architect"]`): visible both ways;
   `a2amx-developer` (team `a2amx`, `agents = []`) and `saltmdb-architect`: false both ways; one-sided
   consent (only a2amx lists saltmdb-architect): false both ways; a party with `name: None` never
   matches an `agents` entry; symmetry for every pair.

`tests/team.rs`, pure:

3. `parse` with `team = "a2amx"` and a session `allow = ["saltmdb/architect"]`: that session's scope
   has `agents == ["saltmdb-architect"]`, a sibling session's `agents` is empty, and both have the
   same name/private/team-level allow.
4. Errors, each naming the session and `allow`: bare entry `architect`; `/architect`; `a/b/c`;
   `saltmdb/` ; an entry of the file's own team; a repeated entry; 17 entries; `allow` with
   `private = false`; `allow` in a file without `team`.
5. `plan`: a running session whose scope differs only in `agents` is a `TeamMismatch` conflict; the
   same `agents` is `AlreadyRunning`.

`tests/teams.rs` (real daemon, temp state dir; reuse the 3l helper `new_team_agent`, extending its
`TeamScope` argument as needed):

6. Two private teams `a` and `b`, architects with mutual `agents`, plus a developer in each:
   architect `a` lists itself (`you`), its team's members and exactly `b`'s architect (not `b`'s
   developer); it can message `b`'s architect and the reply works; it gets `unknown_recipient` for
   `b`'s developer; `a`'s developer cannot reach `b`'s architect. One-sided consent: nothing is
   visible. Exit event: `a`'s architect watching `b`'s architect receives `peer exited` when it
   exits; `a`'s developer watching it does not.
7. D5: the daemon refuses a session whose `agents` contains its own name, a repeat, 17 entries, or a
   non-private team with a non-empty list; and still accepts a second session of an existing team
   with a different `agents` list (D13 ignores `agents`).

`tests/refs_and_list.rs`: 8. `a2amx list --as NAME` for an architect with mutual `agents` shows the
other team's architect and not its developer. `tests/wire.rs`, `tests/status.rs`: 9. a `TeamScope`
without `agents` in JSON decodes with an empty list and serializes without the key (old payloads
and the 3l tests keep passing).

## 8. Out of scope

D7 of section 2; anything in `src/wire.rs`, `src/cli.rs`, `src/mcp.rs`; the other follow-up ideas
(`team status`, `team reset`, needs-input warning).

## 9. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list.

## 10. Rules stated twice

Reviewed against each other after the last edit: the entry shape (D1, section 5, test 4), the
`agents` meaning (D2, D3, D5, test 7), the six enforcement sites (D4, section 4).

## 11. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` passes on `9dc715a` (404 passed, 27 suites, 3
  ignored; tree unchanged since).
- Consultant review `m_764`: no blocking finding. Verified: `messaging::visible` call sites are exactly
  the six of D4 (`daemon.rs` about 551, 626, 789, 891, 1816 and `main.rs` about 742); `TeamScope` literals
  are in `src/team.rs`, `tests/messaging.rs`, `tests/teams.rs`, `tests/team.rs`, `tests/wire.rs`,
  `tests/status.rs` (all in scope); `TeamSession` literals gained to the mechanical sentence;
  `team::plan` compares full scope equality and the daemon's one-scope-per-team check compares only
  `private` and `allow`; serde default/skip keeps old JSON valid; the 3l parse loop is the only place that
  assumed one shared scope. Advisories folded in (viewer name, `(None, any)` test line, README items).
- Worked example through the six sites: `a2amx-architect` and `saltmdb-architect` with mutual `agents`
  see and message each other, exit events and heartbeats flow between them, `saltmdb-developer` stays
  hidden both ways (tests 6 and 8).
- Rules stated twice, diffed: entry shape (D1, section 5, test 4), `agents` semantics (D2, D3, D5, test 7).
- Acceptance commands need the new code and run after implementation. Implementation order advice
  (consultant): last of the four follow-up specs, because it changes the `visible` signature.
