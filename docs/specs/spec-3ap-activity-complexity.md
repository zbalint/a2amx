# Spec 3ap: split `validate_sessions` and `dry_run_report` into named checks

## 0. Status

**LOCKED** (2026-10-26; pre-lock gate run against develop at `241dd2d`; consultant review `m_3942`, findings adopted).
Architect-directed, backlog I4 follow-up, context
`a2amx-sonar-s3776-2026-10-26`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `241dd2d`; `env -u A2AMX_BIN -u NO_COLOR CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test --test team`
passes 78/78.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `team::parse` and `team::flag_sessions` (every message of `validate_sessions` is reached
through them) and the `a2amx team up --dry-run` binary output (`dry_run_report`); both are already exercised by
`tests/team.rs`. The new helpers are private and are not tested directly.

**Scope.** May edit exactly these files and no others.

- `src/team.rs`: `validate_sessions` (line 771, the last item of the file) and new private helpers placed directly
  after it, at the end of the file
- `src/main.rs`: `dry_run_report` (line 926) and new private helpers placed directly after it
- `tests/team.rs`: only the characterization tests of section 4

Does not touch: every other source and test file, `docs/`, `CHANGELOG.md`, `README.md`, `Cargo.toml`, `Cargo.lock`,
`sonar-project.properties`. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

The CI SonarQube gate on `develop` is red at `241dd2d`: `rust:S3776` (cognitive complexity over 30) flags
`validate_sessions` and `dry_run_report`, both at 35 (a hand count of the current code gives 35 for each, which
matches Sonar). Both functions are old; spec 3ao (`23a3086`) added the activity checks and the loop report to the
first and the activity detail line to the second, which took them over the limit. The architect's review of 3ao
asked for the activity additions to be split out of `validate_sessions` but did not count the total, so the
function stayed at 35. The local `scripts/prepush-sonar.sh` scan did not report either finding, because Sonar
anchors the issue on the unchanged `fn` line and a scan against `develop` reports only changed lines. No local
command measures S3776, so section 6 uses a hand count and the CI gate.

The point of this spec is not to pass the gate. Both functions are long checklists that mix many unrelated rules in
one body, so a reader cannot see the order of the checks or add one without risk. Naming each check, and writing the
dry-run detail lines as data instead of seven copies of one `if let ... push_str` pattern, makes the order explicit
and removes duplicated code. Behavior must not change.

## 2. Rule: behavior is unchanged

Same inputs give the same `Ok`/`Err`, the same error text byte for byte, the same check order (the first failing
check wins), and the same dry-run bytes. No public signature changes, no new `pub` item, no change to
`validate_activity`, `activity_cycle`, `team_detail`, `redact_argv` or `wired_command`.

The one text change that is not observable: the loop report today converts the cycle to owned strings and handles a
two-name cycle in its own branch; the replacement (3.2) gives the same text for every cycle of two or more names.
`activity_cycle` returns at least two names, because a session that watches itself is rejected earlier in the loop.

## 3. Target shape

### 3.1 `src/team.rs`: `validate_sessions`

`validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()>` keeps its signature and owns the only state that
spans sessions: the `names` set and the `attached` flag. Its body is: the empty-team `bail!`; the loop; the loop
report. The loop body is exactly this sequence, in this order, and the order is the contract (a team that fails two
checks reports the earlier one):

1. `validate_session_name(index, session)?`
2. the duplicate-name check, inline (`names.insert`)
3. `validate_session_command(session)?`
4. the one-attach check and `attached |= session.attach`, inline
5. `validate_session_reset(session)?`
6. `validate_session_links(session)?`
7. `validate_session_heartbeat(session)?`
8. `validate_activity(session)?` (existing)
9. `validate_session_role(session)?`

After the loop: `if let Some(cycle) = activity_cycle(sessions)` then `bail!("activity notices would loop between
{}", join_with_and(&cycle))`.

New private helpers, each `fn name(session: &TeamSession) -> anyhow::Result<()>` unless stated, each one
responsibility, each moving its current code and message unchanged:

- `validate_session_name(index: usize, session: &TeamSession)`: `validate_name` with the
  `session {index+1} ({name}): {error}` context. Kept as a helper, although it only wraps one call, so the loop
  reads as one uniform list of `validate_session_*` steps.
- `validate_session_command`: empty command, then whitespace in `command[0]`.
- `validate_session_reset`: the `Some(reset)` case: empty `reset`, then `validate_reset_steps`.
- `validate_session_links`: every `control_from` entry, then every `watch` entry including the watches-itself
  check, in that order.
- `validate_session_heartbeat`: heartbeat needs a non-empty `watch`, then `parse_interval`.
- `validate_session_role`: `validate_role` with the `session {name}: {error}` context.
- `join_with_and(names: &[&str]) -> String`: a slice pattern, `[]` gives an empty string, `[only]` gives it, and
  `[rest @ .., last]` gives `"{rest joined by \", \"} and {last}"`. It replaces the owned-string conversion, the
  two-name branch, the `pop` and the unreachable `return Ok(())`. Two names give `a and b`, three give
  `a, b and c`.

### 3.2 `src/main.rs`: `dry_run_report`

`dry_run_report` keeps its signature. Its body is the `for` over `actions` and a `match`: the `AlreadyRunning` arm is
unchanged, the `Start` arm appends `dry_run_start(home, cwd, file_dir, &session).await?` to `output`.

- `async fn dry_run_start(home: &Path, cwd: &Path, file_dir: &Path, session: &TeamSession) ->
  anyhow::Result<String>`: resolves the harness and cwd, calls `wired_command` first (as today, so its error
  discards the output), then builds the text in the current order: `would start {name}`, `  harness:`, `  cwd:`,
  one `  {label}: {value}` line per entry of `session_details(session)`, `  command:`, and the
  unindented `omp extension install skipped (dry run)` line for the OMP harness.
- `fn session_details(session: &TeamSession) -> Vec<(&'static str, String)>`: the optional detail lines as data, in
  the current order: `team` (from `team_detail`), `role`, `attach` (value `true`, only when set), `watch` and
  `control_from` (joined with `", "`, only when non-empty), `heartbeat`, `activity`. Built as an array of
  `Option<(&'static str, String)>` and flattened, with no `if` chain.
- `fn activity_summary(activity: &team::TeamActivity) -> String`: `working_after {v}` and `idle_after {v}` for the
  directions that are set, joined with `", "`, in that order.

Helper parameters are the data they use (a session, an activity), not the fields spread out.

## 4. Safety net

Before the first edit the developer adds characterization tests to `tests/team.rs` and runs them on the unchanged
code, where they pass. Expected values are literals observed on the unchanged code, never computed, and the report
states each observed message.

- **One test per message** of `validate_sessions` and its helpers that no test asserts in full today: invalid
  session name, empty command, whitespace in `command[0]`, second attach, empty `reset`, invalid reset step, invalid
  `control_from` entry, invalid `watch` entry, heartbeat with an invalid interval, invalid role. Use the full text
  or a substring unique to that message.
- **Precedence**: a table-driven test where each row is a team that fails two checks and the expected message is the
  earlier one. Rows: duplicate before command, command before attach, attach before reset, reset before links,
  links before heartbeat, heartbeat before activity, activity before role; and inside a helper, `control_from`
  entry before `watch` entry, and heartbeat's empty-`watch` check before its interval check (a heartbeat of an
  invalid interval on a session with no `watch`); plus one cross-session row where session 1 fails its last check
  (role) while session 2 fails its first (name), expecting session 1's message. Name before duplicate is not a row:
  a duplicate needs an earlier session of the same name, which already passed `validate_name`, so no team fails
  both and the order cannot be observed (it stays name first, as in 3.1). Construction rules: use no team scope and
  no `/` in references, because `resolve_references` runs over all sessions before `validate_sessions` and its
  errors would win; for command before attach put both failures in session 2 (session 1 is valid with
  `attach = true`; session 2 has `command = []` and `attach = true`), otherwise session 1 fails first and the row
  tests nothing.
- **Dry-run order**: one test with a session that sets every optional field, including `team`, asserting the exact
  block from `would start` through `  command: ` in order (`harness`, `cwd`, `team`, `role`, `attach`, `watch`,
  `control_from`, `heartbeat`, `activity`). A `team = "t"` file names the session `t-NAME` and requires `watch` and
  `control_from` to name sessions in the file, which print as `t-other`; use an absolute cwd (`/`, as the existing
  exact test does) and a generic command such as `cat`, so the bytes are exact. Two more tests assert
  `  activity: working_after 2m\n` for a session with only that direction and `  activity: idle_after 30s\n` for
  only the other. The existing dry-run tests assert substrings only, which does not pin the order.

## 5. Out of scope

Renames, message changes, `validate_activity`, `activity_cycle`, `wired_command` and its three boolean arguments
(noted, not fixed here), `start_planned_session`, `run_team_up`, `plan`, any other function or Sonar finding, and the
changelog (internal refactor, nothing user-visible).

## 6. Acceptance

Post-implementation, in the main checkout, with the memory-safe form that earlier crashes require:

```sh
export CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2
env -u A2AMX_BIN -u NO_COLOR cargo test --test team
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/team.rs, src/main.rs, tests/team.rs
```

- The characterization tests pass on the unchanged code first (the developer's report says so) and after the
  change; `cargo test --test team` passes every baseline test plus the added ones.
- Complexity, hand-counted with Sonar's rules (`if`, loops, `match` and `&&`/`||` runs each +1 plus their nesting
  depth; `?`, closures and method chains +0): `validate_sessions` 8, `validate_session_command` 2,
  `validate_session_reset` 3, `validate_session_links` 4, `validate_session_heartbeat` 3,
  `validate_session_role` 1, `validate_session_name` 0, `join_with_and` 1, `dry_run_report` 3, `dry_run_start` 2,
  `session_details` 0, `activity_summary` 0. The tester recounts each new function and reports any above 15. Final proof is the CI
  SonarQube gate on `develop` after the architect pushes; the architect reads it per the Sonar guidance.
- Tester pass with a mutation check in a scratch copy: swap each adjacent pair of calls in the loop of 3.1 except
  name and duplicate (an equivalent pair, section 4), swap the two checks inside `validate_session_links` and
  inside `validate_session_heartbeat`, swap `team` and `role` and swap two other entries of `session_details`,
  drop one `bail!` and one detail line, change `join_with_and`'s separator; each mutant must fail a named test.
- Known flaky: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (PTY timing). Rerun once
  on a failure; if it then passes, record it as a pre-existing flake. Any other failure is new unless it reproduces
  on a clean `241dd2d`.

## 7. Claims checked against the tree

| Claim | Where | Verified how |
| --- | --- | --- |
| `validate_sessions` is 35 by Sonar's rules | `src/team.rs:771`; CI issue `AaF41d3VxgATYJd-7WY9` says 35 | hand count 35, matches the issue |
| `dry_run_report` is 35 | `src/main.rs:926`; CI issue `AaF41d4xxgATYJd-7WY-` says 35 | hand count 35, matches |
| 3ao grew both | `git show 23a3086` hunks in `validate_sessions` and `dry_run_report` | read |
| A cycle has at least two names | self-watch bails in the loop before `activity_cycle` runs (`src/team.rs`, loop) | read |
| No existing helper joins names with "and" | `rg '" and \{'` finds only `src/team.rs` | grepped |
| Tests pinning the loop report | `tests/team.rs:2236` (`a and b`), `:2258` (`a, b and c`) | read |
| Dry-run tests pin substrings and two exact reports | `tests/team.rs:1426`, `:1486`, `:1510`; `:1557` (redaction, substrings, unaffected) | read |
| Name before duplicate is not observable | `src/team.rs:778-782`: a duplicate needs an earlier session with the same, already valid, name | read (consultant and architect) |
| `session_details` order puts `team` before `role` | current `dry_run_report`, `src/main.rs:958-961` | read |
| `command = [""]` passes `validate_sessions` but `flag_sessions` rejects it | `src/team.rs:630` | read by the consultant; out of scope, backlog note |
| `validate_sessions` callers | `src/team.rs:463` (`parse_with`), `:649` (`flag_sessions`) | grepped |
| Baseline `cargo test --test team` is 78 passed | the command in section 0 | run at `241dd2d`, exit 0 |

## Review record

**Models.** Architect: Claude Sonnet 5.5. Consultant: a Claude Code session, model not exposed to the architect.
Developer and tester: OMP sessions reporting `openai-codex/gpt-5.6-luna`; their planning, working and advisor
slot models are not exposed to the architect. **BLOCKED/amend cycles:** 0.

| Found in | Found by | Problem | Resolution |
| --- | --- | --- | --- |
| After the push of spec 3ao | CI SonarQube gate | `validate_sessions` and `dry_run_report` were at cognitive complexity 35 (limit 30) after 3ao; the architect's 3ao review asked for part of the code to be split out but did not count the total, and the local `prepush-sonar.sh` scan did not report them because Sonar anchors the issue on the unchanged `fn` line | This spec; the hand count per function is now part of acceptance; backlog F25 for a local proxy |
| Draft review | Consultant | Scope said to place the helpers "before `validate_activity`", which precedes `validate_sessions` in the file | Helpers go at the end of the file |
| Draft review | Consultant | Name before duplicate is an equivalent pair: no team fails both, so the precedence row and the swap mutant could not exist | Row and mutant dropped, reason stated in section 4 |
| Draft review | Consultant | The dry-run order test omitted the `team` line, so a `team`/`role` swap would survive | Team line required, with the construction rules |
| Draft review | Consultant | The architect's worked example (`command = [""]`) is accepted by `validate_sessions`, so it does not show the command message | Example replaced; the `[""]` gap went to backlog F26 |
| Draft review | Consultant | Missing rows: `control_from` before `watch`, heartbeat's empty-watch check before its interval check; cross-session row needed construction rules | Rows and rules added |
| Tester pass | Tester | Indenting the OMP notice line still passed: the existing test matched a substring | Architect changed the assertion to `\nomp extension install skipped (dry run)\n` |
| Tester pass | Tester | One precedence row failed only one check, so it tested no pair | Row removed (architect); the message keeps its own test |
| Design read | Consultant | The precedence test repeated the assertion loop; rows had no labels; ten tests were named after private helpers; two dry-run tests did not assert the exit status | One developer round, tests only; names follow the `parse_characterizes_*` convention |
| Design read | Consultant | Style notes on the unchanged production shape (let-else in two helpers, `command.first()`, "links" against the codebase's "references", six repeated session-context `map_err`s) | Not changed; backlog F27 |
