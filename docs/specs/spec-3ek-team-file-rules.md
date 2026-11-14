# Spec 3ek: one rule set for team files

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from `0ff86a4` (the 3ej commit).
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. This is spec 5a of 7 in the owner's configuration
redesign (3eg root and layout, 3eh `daemon.toml`, 3ei `gateway.toml`, 3ej `client.toml`, this spec, 5b per-project
auto grants, migration, presence per session). **It breaks the team file format and one default**
(section 1); the architect writes the changelog entry.

Public test seams: `a2amx::team::parse` and `a2amx::team::team_name_for` in `tests/team.rs`; the `a2amx` binary
(`team up --dry-run`, `team status`, `team up`) in `tests/teams.rs` and `tests/daemon_cli.rs`; the documentation
example parsers in `tests/docs_examples.rs`.

**Scope** (files that may be edited):

- `src/team.rs`, `src/main.rs`, `src/cli.rs` (help text of the four `--file` options only).
- `tests/team.rs`, `tests/teams.rs`, `tests/daemon_cli.rs`, `tests/docs_examples.rs`, `tests/confine.rs`,
  `tests/grant.rs`, `tests/suggest.rs`, `tests/refs_and_list.rs`, `tests/confine_show.rs`,
  `tests/auto_confine.rs` (call-site and expected-name updates caused by sections 2 to 4; no new behavior).
- `docs/teams.md`, `docs/confinement.md`, `docs/getting-started.md`, `docs/cli.md`, `docs/install.md`,
  `docs/remote-messaging.md`, `a2amx.toml.example`, and `README.md` only where a team-file example uses `allow`, relies on an omitted `team`, or relies on confinement
  needing `enabled = true`.

Does **not** touch: `src/daemon.rs` (its wire-side scope validation keeps its texts), `src/messaging.rs`,
`src/confine.rs`, `src/grant.rs`, `src/suggest.rs`, `src/layout.rs`, `src/config.rs`, the `auto` key and
`state/auto-grants` (spec 5b), migration, `AGENTS.md`, `CHANGELOG.md`, `docs/architecture.md`, `docs/backlog.md`,
earlier specs.

## 1. Why

Team files have two rule sets. A local `a2amx.toml` without `team` gives ungrouped sessions that see everyone;
the global `config/team.toml` derives a team name from the directory; `--file` has no name at all. Confinement needs
`[confine] enabled = true` in a table whose existence already says "confine". `allow` means a team name at file
level and a team/agent pair in a session, under one key. `prefix`, `confine = true` and the `confine_*` session keys
are rejected with a custom message long after the formats they replace stopped being written. The owner decided
(grilling, 2026-11-14):

1. Every team file has a team. `team` omitted means the name derived from the project directory, in a local file,
   a `--file` file and the global file alike. The global file may never set `team`. Ungrouped sessions exist only
   in the flag form (`team up a=claude b=omp`). Private by default follows, so an old local file without `team`
   becomes a private team: **breaking**.
2. `allow` becomes `allow_teams` (file level) and `allow_agents` (session level). A file that still says `allow`
   is refused with a message naming the new key. No alias.
3. A `[confine]` or `[session.confine]` table means confinement on unless it says `enabled = false`; the session
   table overrides the team table; no table anywhere means off.
4. The project directory of a `--file` file is the file's directory: it names the team and anchors relative paths
   and an omitted `cwd`. A local file's and the global file's project directory is the invoking directory, as today.
5. The legacy rejections (`prefix`, `confine = true`, `confine_write`, `confine_hide`, `confine_unhide`,
   `confine_no_default_hides`, `confine_allow_containers`) are removed. The strict parser still refuses the keys,
   with its generic message that names the key.

Rejected: keeping a local file ungrouped (two rules for two file kinds); accepting `allow` as an alias (a
security-relevant key with two spellings); deriving the `--file` team from the invoking directory (the same file
would give different teams depending on where you stand). `auto` in any file is spec 5b, because accepting it
anywhere is only safe once acceptance is per project.

## 2. `src/team.rs`

Anchors are at `0ff86a4` (the 3ej commit; this spec is locked from there).

1. **API.** `parse(text)` and `parse_with(text, default_team: Option<&str>)` (482-486) become one function
   `pub fn parse(text: &str, dir: &Path, global: bool) -> anyhow::Result<Vec<TeamSession>>`. `dir` is the
   project directory (section 3); when the file omits `team`, `parse` derives the name with
   `team_name_for(dir, remedy)`, choosing the remedy from `global` (section 2.7). A file that sets `team` never
   derives, so its directory name cannot make it fail. `global` is true only for the global file. `parse_with`
   is removed. The test `parse_with_applies_derived_private_team...` (`tests/team.rs:195`) is renamed.
2. **Team name.** In the body (497-501) replace `default_team.is_some()` by `global`. `team` set in a global file
   keeps the exact error `the global team file must not set team; the team is named after the directory`.
   `build_scope` (770-816) is called with a present team always: delete its `else` branch (the `private requires
   team` and `allow requires team` errors) and make the parameter `String`.
3. **Renames.** `TeamFile.allow` (97) becomes `allow_teams`; `TeamSession.allow` (49) becomes `allow_agents`.
   `messaging::TeamScope.allow` (`src/messaging.rs:78`) is the wire field and keeps its name: `build_scope` maps
   `allow_teams` into it, and `resolve_agent_allow` fills `TeamScope.agents`. `src/daemon.rs` and `src/messaging.rs`
   are not edited. Before the strict parse, a pre-scan (the pattern of `reject_old_confine_format`, which it
   replaces) refuses an old key with these exact messages: top level `allow was renamed to allow_teams`; session N
   `session N: allow was renamed to allow_agents` (N is the 1-based index). The pre-scan is silent when the text
   is not valid TOML (the strict parse then reports it, as today).
4. **Message wording.** Every message in `build_scope` and `resolve_agent_allow` that says `allow` says the new key:
   `allow_teams needs a private team`, `allow_teams entry {entry:?}: ...`, `allow_teams entry must not be the team
   itself`, `allow_teams entry {entry:?} is repeated`, `session {name}: allow_agents needs a private team`,
   `session {name}: allow_agents has at most 16 entries`, `session {name}: allow_agents entry {entry:?} must be
   TEAM/AGENT`, `session {name}: allow_agents entry {entry:?}: {error}` (three places, `team.rs:866, 869, 876`),
   `session {name}: allow_agents entry {entry:?} must name another team`, `... is repeated`. The removed branch (2)
   also removes the `session {}: allow needs a private team` text at 546-548.
5. **Confinement default.** In `merge_confine` (710-713) `enabled` is decided per table: the session table, when
   present, decides (its `enabled`, default true); otherwise the team table, when present (its `enabled`, default
   true); otherwise false: `own.map(|t| t.enabled.unwrap_or(true)).or(team.map(|t| t.enabled.unwrap_or(true))).unwrap_or(false)`.
   So `[confine] enabled = false` plus a `[session.confine]` table without `enabled` confines that session.
   The error at 739-745 now fires only when the session's own table has options and
   `own.enabled == Some(false)`; its text becomes `session {name}: [confine] options are
   set but enabled = false; remove the options or the enabled = false`. The check `[confine] auto needs enabled =
   true` (680-682, condition `table.enabled != Some(true)`) becomes `table.enabled == Some(false)` with the text
   `[confine] auto needs confinement; remove enabled = false`; `[confine] auto = true` alone is valid. The other
   table validations (syntax, `keep_env`, `grants`, the global-only `auto` error) run as today.
6. **Legacy rejections.** Delete `reject_old_confine_format` (571-622), its call (487), the `prefix` field (99), its
   destructuring (493) and its check (502-504). The strict parser then refuses these keys with its generic
   message. Note the loss: a session-level `confine = true` now reads `session N: invalid type: boolean ...,
   expected struct ConfineTable` and does not name `confine` in that message beyond the type name; this is accepted
   and test 6 pins what the message contains.
7. **`team_name_for`** (271-304) becomes `pub fn team_name_for(dir: &Path, remedy: &str)`; **both** error branches
   (no final component, line 272-276; invalid name, 297-302) end `; {remedy}`. The remedy is
   `create an a2amx.toml here, or pass --file` when `global`, else `set team in the file`. It stays pure over the
   path (no filesystem access). The derivation rules are unchanged.

## 3. `src/main.rs`

1. `TeamSource` (1197-1202): `default_team: Option<String>` is replaced by `global: bool`. `dir` is the **canonical**
   project directory for all three kinds (it names the team and anchors relative paths), computed on the blocking
   thread with `std::fs::canonicalize` (error naming the directory):
   - `--file F`: `path = invoking_cwd.join(F)`, `dir` = the canonical parent of `path`, `global = false`. An
     unreadable file keeps its existing error from `read_team`; a missing parent directory now fails at
     canonicalize first (accepted).
   - local `a2amx.toml` (1238-1246): `dir` = canonical invoking directory (it was the raw one), `global = false`.
   - global (1255-1278): as today (canonical, `check_global_allowed`, eager `team_name_for` for the stderr line and
     the early failure), `global = true`.
2. `read_team` (1294-1302) calls `team::parse(&text, &source.dir, source.global)`.
3. The four omitted-`cwd` sites (1074, 1532, 1604, 1696: `.map_or_else(|| cwd.to_owned(), |path| origin.dir.join(path))`)
   use `origin.dir.to_owned()` instead of the invoking `cwd`. For `--file` the session starts in the file's directory.
   For a local or global file `dir` is now the canonical invoking directory, so under a symlinked invoking
   directory an omitted `cwd` resolves to the link target instead of the link path (accepted).
4. The `using global team file {path} as team {name}` stderr line (1281-1290) is kept, for the global file only.
5. `team up` with inline `NAME=EXECUTABLE` items (`flag_sessions`) is untouched: no team.

## 4. `src/cli.rs`

The four `--file` help strings (284, 290, 307, 319) keep their text and add: `A file without team is named after its directory.`

## 5. Docs and examples

- `docs/teams.md`: rewrite "Team file" and "Visibility": every file has a team (explicit or derived); the global
  file never sets `team`; `--file`'s directory rule; `allow_teams` and `allow_agents`; the rename sentence
  replaces "The old `prefix` team-file key was renamed to `team`." (delete that sentence); `[confine]` means on
  unless `enabled = false`, a session table overrides the team table, no table means off; the sentence "`enabled`
  defaults to false" and the "options on a disabled session" paragraph are corrected; the `auto` paragraph is
  unchanged. State the breaking changes in one short paragraph: omitted `team` now makes a private team named after the directory; the `allow` keys; a `[confine]` table (even one holding only `auto = false`) now confines; sessions started from the old file keep their old unprefixed names and `team up` starts new prefixed ones next to them, so stop the old sessions first (`a2amx team down` with the old names or `a2amx kill`).
- `docs/install.md:44` and `docs/remote-messaging.md:191` (the sentence that a team file must name a team): corrected.
- `docs/confinement.md` (lines 71, 155-158 and the examples with `enabled = true` that remain valid but redundant): correct
  the statement that confinement needs `enabled = true`.
- `docs/getting-started.md`, `docs/cli.md`, `a2amx.toml.example`, `README.md`: only the `allow` examples and any
  text that says a file without `team` is ungrouped.
- Every TOML block that `tests/docs_examples.rs` parses must still parse (call sites pass `Path::new("/work/demo")` and `false`, or
  `true` where they used `Some("demo")` before; every team name those blocks assert is `demo`, so the directory is named `demo`).

## 6. Tests (public seams)

Expected values are literals. Existing tests that asserted unprefixed names from a file without `team`, or the
removed messages (`tests/team.rs:357`, `364`, `372`, `3140-3172`; `223` and `334` keep their text), are rewritten to
the new rules; the daemon wire-text tests `tests/teams.rs:1124` and `1180` (`allow needs a private team`) stay as they are; none is deleted without an equivalent that pins its behavior.

1. `parse(text, Path::new("/work/proj"), false)` for a file without `team` gives session names `proj-a`, team `proj`, private
   (a file without `team`).
2. A file with `team = "mine"` and dir `/work/proj` gives `mine-a` (explicit wins); with dir `/` it still works (no derivation).
3. `parse(text_with_team, Path::new("/work/proj"), true)` fails with the exact `must not set team` text; without `team` it works.
4. Top-level `allow = ["x"]` fails with exactly `allow was renamed to allow_teams`; a session `allow = [...]` as the
   second session fails with exactly `session 2: allow was renamed to allow_agents`; `allow_teams = ["x"]` and
   `allow_agents = ["x/y"]` work as the old keys did (the existing visibility tests, renamed).
5. `private = false` with `allow_teams` fails with `allow_teams needs a private team`; a session with
   `allow_agents` on a public team fails with `session a: allow_agents needs a private team`.
6. Using `format!("{error:#}")` (the full chain; `to_string()` is only `invalid team file`): `prefix = "x"` fails and
   the text contains `prefix`; a top-level `confine = true` fails and contains `confine`; a session with
   `confine_write = [...]` fails and contains `confine_write`; a session `confine = true` fails and contains
   `ConfineTable`. None contains `renamed` or `replaced`.
7. `[confine]` alone gives every session a confine request; `[confine] enabled = false` gives none;
   `[confine] enabled = false` plus a `[session.confine] write = [...]` without `enabled` gives that session a request;
   `[confine]` plus `[session.confine] enabled = false` on one session gives that session none and the others one;
   `[confine] auto = true` in the global file (no `enabled`) parses with `auto` true; with `enabled = false` it fails with the exact new text;
   a session `[session.confine] write = [...]` with no team table gives that session a request; no table at all gives none;
   `[session.confine] enabled = false` with `write = [...]` fails with the new `options are set but enabled = false` text.
8. `team_name_for(Path::new("/"), "R")` (no final component) and `team_name_for(Path::new("/work/___"), "R")` (invalid name) both end with `; R`; on `/work/My_Project` returns
   `my-project` (existing derivation tests, signature updated).
9. CLI, local file without `team` in a fixed-name subdirectory of the test's temp directory (for example `proj_one`, so `NAME` is `proj-one`; a bare temp directory gives a random name): `team up --dry-run` rows and
   `team status` rows show `NAME-<session>` and `NAME (private)`.
10. CLI, `--file F` where F is in another directory D than the invoking one, no `team`, no `cwd`:
    the dry run shows the team derived from D's name and `cwd:` equal to D.
11. CLI, `--file F` with a `team` set, in a directory D whose name cannot derive a team (a temp directory named
    `___`): works. Without `team` in D: fails with the `set team in the file` text.
12. CLI, a file with the old `allow` key: exit 1, error contains the file path and `allow was renamed to allow_teams`.
13. `docs_examples` still parses every block (a regression check: the blocks change to the new keys, so it does not pass on the old code).
14. `team up` with inline items still gives ungrouped sessions (contract test, passes before and after).
15. Fixtures that need literal session names and have no `team` get an explicit `team = "..."` line, or live in a fixed-name subdirectory; a teamless file is now a private team, so a test that messages between file sessions and `a2amx new` sessions may change by visibility: the developer reports each such test.

Mutants the developer must run and map to a killing test: derived name ignored for local files (test 1); global
`team` check removed (3); old-key pre-scan removed (4); `auto` check kept as `!= Some(true)` (7); team `enabled` consulted before the session table (7); `allow_agents` check on public team removed (5);
`unwrap_or(false)` restored in `merge_confine` (7); session override order swapped (7); `origin.dir` replaced by
`cwd` at one omitted-`cwd` site (10); `--file` parent not canonicalized (10 with a `..` path); remedy chosen from the wrong flag (8, 11).

## 7. Out of scope

`auto` in any file and per-project acceptance (5b); `state/auto-grants`; migration of old files (a local
`a2amx.toml` is not migrated; the error from section 2.3 is the migration); splitting `src/team.rs` (about 1,400
lines; a follow-up for the owner); name collisions between two directories with the same final component (documented
limitation, `docs/teams.md`); the daemon's wire validation texts at `src/daemon.rs:655-690`; a notice that a derived
team was used; `team.toml` template writing (the global file stays hand-written).

## 8. Acceptance

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test team
cargo test --test teams
cargo test --test docs_examples
cargo test --test daemon_cli
rg -n "parse_with|reject_old_confine_format|private requires team|allow requires team|prefix was renamed" src tests
rg -n "^\s*allow = |^# allow = " docs/teams.md docs/confinement.md docs/getting-started.md a2amx.toml.example README.md
```

The two `rg` commands match nothing. Test 14 passes on the old code (contract test); the rest fail first.
In a confined session the three read-only-HOME tests of earlier specs (`confine real_bwrap*`, `gateway_up
sandbox_hides...`) fail on `$HOME`; rerun them with `HOME` under `target/scratch/3ek/` and report. Mutation check:
the developer mutates in the checkout, names the failing test per mutant, restores by checksum; the tester repeats it.

## 9. Claims verified against the tree (at `5bbb429`)

| Claim | Where | How |
| --- | --- | --- |
| `parse`, `parse_with` are the only public parse entry points | `src/team.rs:482-486` | read |
| `parse_with` callers outside `team.rs`: `main.rs:1298`, `tests/docs_examples.rs:31`, plus `team::parse` in `tests/team.rs` (96 uses), `confine.rs`, `grant.rs`, `suggest.rs`, `docs_examples.rs` | grep `team::parse\(\|parse_with\(` | grepped |
| Global file already derives the name and refuses `team` | `team.rs:497-501`, `main.rs:1255-1278` | read |
| Four omitted-`cwd` sites use the invoking dir | `main.rs:1074,1532,1604,1696` | read |
| `--file`'s `dir` is `path.parent()` of `invoking_cwd.join(file)`, not canonicalized | `main.rs:1212-1219` | read |
| Local file's `dir` is `invoking_cwd`, not canonical | `main.rs:1238-1246` | read |
| `confine` defaults to off when the table exists without `enabled` | `team.rs:710-713` | read |
| Daemon-side scope validation uses its own texts and the scope's field names | `daemon.rs:655-690` | read; left alone |
| `allow` appears as a file key only in team files and docs; the CLI has no `--allow` flag | `src/cli.rs`, `src/wire.rs` (grep `allow`: only `#[allow(clippy..)]` and `allowed`) | grepped |
| Tests pinning removed messages | `tests/team.rs:357,364,372,3140-3172`, the `auto needs enabled` test at `3288-3296`; `tests/teams.rs:1124,1180` pin the daemon's wire text and stay | grepped (consultant) |
| `TeamScope.allow` is the wire field | `src/messaging.rs:78`, `daemon.rs:662-673` | read (consultant) |
| `team_name_for` is pure over the path | `team.rs:271-304` | read |
| Only `tests/attach_cli.rs` outside scope runs `team up --file`, all four set `team` | `attach_cli.rs:1823,1881,1945,2152` | grepped (consultant) |
| `tests/docs_examples.rs` parses doc and README team blocks | `docs_examples.rs:24-95` | read |

Not verified before locking: the exact text serde and `toml` give for the removed keys (test 6 asserts only the names above); which tests in `tests/daemon_cli.rs`, `refs_and_list.rs`, `confine_show.rs`, `auto_confine.rs` change by visibility (the developer reports them).

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (model not recorded); developer OMP, openai-codex/gpt-5.6-luna; tester OMP, opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0 (one informational ruling, amendment 1: test 6.5's `session a:` literal means the final name, `session proj-a:`).

| Found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | `enabled` formula contradicted the owner's decision (per table); `auto needs enabled` check not re-read after the default flipped; over-engineered `Result<String,String>` API; `team_name_for` has two error branches; further M2-M7, L1-L4 | Spec rewritten before the lock |
| Tester pass | tester, deepseek-v4.1-flash | Section 6 tests 9-12 absent; seven mutants survived (M9, M10, M11, M15, M16, M17, M19) | Tests added in fix round 1; developer reran each mutant |
| Tester pass | tester | Two status tests lost their behavior; assertions weakened in three tests; one test renamed to match nothing | Restored in fix round 1 |
| Tester pass | tester | Unrequested duplicate validator `validate_sessions_before_prefix` changing error literals | Removed in fix round 1 |
| Tester pass | tester | `docs/teams.md` auto paragraph garbled, breaking-changes paragraph missing, examples pointed outside the team | Fixed in fix round 1 |
| Implementation | developer, gpt-5.6-luna | Claimed a mutant mapping without running mutants; one mutant survived until a no-table assertion was added | Mutants run in isolation in fix round 1 |
| Architect review | architect, Sonnet 5.5 | Watch/control_from resolver order swapped; duplicate global-team pre-scan; four unused `_cwd` parameters; a repeated `team_name_for` call | Removed in fix round 2 |
| Spec | architect | Test 6.5 literal used an unprefixed name | Amendment 1 |
