# Spec 3ch: move the auto-mode decisions out of `run_team_up` and `dry_run_start` into the modules that own them

## 0. Status

**LOCKED** (2026-11-03). Architect-directed fix for a failing Sonar gate on `0633233` (spec 3cf). Context
`a2amx-auto-consent-complexity-2026-11-03`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `0633233`; `env -u A2AMX_BIN -u NO_COLOR cargo test` 837 passed, 3 ignored; clippy and fmt clean.
The scratch Sonar scan of that commit reports two open `rust:S3776` issues on new code (gate value 20 against 14):
`dry_run_start` (`src/main.rs`, 32 against a limit of 30) and `run_team_up` (45).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the `a2amx` binary (`tests/team.rs`, `tests/confine.rs`: dry-run and `team up` output, the consent
tests) for unchanged behavior; unit tests at the public API of `team`, `confine` and `grant` for the moved decisions.

**Scope.** May edit exactly: `src/main.rs`, `src/team.rs`, `src/confine.rs`, `src/grant.rs` and, in those files, only
the items named in section 3 plus their unit tests. Leave the diff uncommitted.

## 1. Why

Spec 3cf grew both functions past the limit, but the count is a symptom. `run_team_up` now decides which grants are new,
merges auto-detected grants into a session's request, builds escape notices and runs the consent prompt, all inline in
the CLI dispatch file. Those are pure decisions about types that `team`, `confine` and `grant` already own, and they
have no unit test of their own; only whole-binary tests pin them. `dry_run_start` renders a confinement plan as text in
`main.rs`, next to `confine::notice`, which renders the same plan for agents. The refactor puts each decision where its
types live, tests it at that seam, and leaves `main.rs` with the I/O (HOME, the blocking scan, stdin, the state file,
stderr). A lower complexity number is a consequence, not the aim.

## 2. Rule: behavior is unchanged

Same stdout and stderr bytes in the same order, same file writes, same errors and exit codes. No change to `shortcut:`
comments or to any signature outside section 3. Moves are verbatim where possible; the three deliberate rewrites are
named in section 3. Order of the flow in `run_team_up` stays exactly: auto-grant merge, validate sessions, load accepted
grants, compute new grants, escape notices to stderr, consent (only when not a dry run), auto summaries, then the
dry-run stdout report or the starts. `--yes` prints the consent lines and neither extends nor saves the accepted set.
Consent never runs in a dry run, and the dry-run report still receives the accepted set as loaded.

## 3. Target shape

Pure decisions (no I/O, unit-tested at the public seam):

1. `team.rs`: `pub fn new_auto_grants(actions: &[Action], accepted: &BTreeSet<String>) -> Vec<(String, String)>`.
   Moves the collection loop: auto sessions with a confine request only, grants not in `accepted`, first occurrence per
   grant, as (grant, reason). Tests: dedupe across sessions keeps the first reason; accepted grants skipped; sessions
   that are not auto are skipped.
2. `confine.rs`: `impl ConfineRequest { pub fn apply_auto(&mut self, found: Vec<(&'static str, String)>) }` moves the
   merge: append each found grant the request lacks, set `auto = true`, and set `reasons` to those it added. Tests: a
   grant already requested is not duplicated and gets no reason; `auto` is set when nothing new is found.
   `pub fn explicit_grants(&self) -> Vec<String>`: `grants` minus those named by `reasons` (the filter now in
   `run_team_up`). Test: auto-added grants are excluded.
3. `team.rs`: `pub fn escape_notices(actions: &[Action], rows: &[crate::grant::Grant]) -> Vec<String>`: one entry per
   started session that has notices, the lines of `grant::escape_notice` for its `explicit_grants()`, each line
   newline-terminated and joined, so `main.rs` writes each entry as today (same bytes, same number of writes). Test: only
   explicit grants appear; a session with none yields no entry.
4. `confine.rs`, next to `notice`: `pub fn dry_run_lines(plan: &Plan, resolved_cwd: &Path, accepted: &BTreeSet<String>) ->
   String`: the report lines from `confinement: auto|manual` through the skipped-missing loop, moved verbatim, including
   the "not yet accepted" line. Keep `resolved_cwd`; do not infer it from `plan.cwd`. Tests: an auto plan lists the
   not-yet-accepted grants; the working directory is not listed as writable.
5. `grant.rs`, beside `CONSENT_QUESTION`: `pub fn consent_refusal(new: &[(String, String)]) -> String`: the non-terminal
   refusal text now built inline. Test: names every grant, in order, with the `--yes` hint.

I/O stays in `main.rs`:

6. `async fn resolve_auto_grants(actions: &mut [team::Action], cwd: &Path, origin: TeamOrigin<'_>) -> anyhow::Result<()>`:
   reads HOME once, runs the blocking `auto_grants_for` scan per auto session, calls `apply_auto`.
7. `async fn accept_new_grants(home: &Path, accepted: &BTreeSet<String>, new_grants: &[(String, String)], yes: bool) ->
   anyhow::Result<()>`: writes the consent lines; on `yes` stops there; otherwise asks via `confirm` with
   `grant::consent_refusal`, bails with the existing "not accepted; nothing started" error on a no, and saves the union
   of `accepted` and the new grants. Nothing reads the set afterward, so it is taken by reference (one local union is
   the only rewritten line). `run_team_up` keeps the `!dry_run && !new_grants.is_empty()` test.
8. `dry_run_start` calls `confine::dry_run_lines` for a present plan and keeps `confinement: off`, the `command:` line
   and the omp line. `write_auto_summaries` stays as it is.

Out of scope, recorded as backlog: `dry_run_start` and `validate_team_session` both build the host and plan from HOME and
XDG_RUNTIME_DIR; a shared `confine_plan_for` is a separate dedupe.

**Not allowed:** a consent state struct or a stdin-mocking trait; prompt, stdin or save I/O in `grant`, `team` or
`confine`; the plan renderer in `grant.rs`; extracting the start/attach loop of `run_team_up`; any suppression comment;
any change made only to move a number.

## 4. Acceptance

Run from the main checkout, as an ordinary user:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
scripts/prepush-fast.sh
git status --short   # only the four files of Scope
```

All 837 baseline tests still pass unchanged and unedited (`git diff` shows no removed or altered test), plus the new unit
tests named in section 3 (at least 11). Hand count per function in the completion message, Sonar rules: +1 for each
`if`, `else if`, `else`, loop, `match` (once, not per arm) and each run of `&&`/`||`, plus the nesting depth at that
point; a closure adds one nesting level; `?`, `let ... else` and method chains add nothing. Expected: `run_team_up`
about 21, `dry_run_start` about 6, `dry_run_lines` about 16; every function at 25 or less. The architect confirms with the
Sonar scan, and the developer reports the count calibrated against Sonar's earlier values (32 for `dry_run_start` and 45
for `run_team_up` at `0633233`).

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (exact model not exposed to the architect, not
fresh because its reset was refused); developer OMP `openai-codex/gpt-5.6-luna`; tester on OMP (model not reported).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| After the push gate of spec 3cf (pre-push scan) | architect, Sonar | Spec 3cf's review never hand-counted the two functions it grew; both went past 30 | This spec |
| Draft review | consultant | The first draft only moved code into private helpers in `main.rs`, which the owner rejected; its own 25 target was likely missed (about 27) | Spec rewritten around pure decisions in `team`, `confine` and `grant`, scope widened, tests named |
| Draft review | consultant | Scope, "no new pub item" and "test count equals baseline" contradicted the new shape; the counting rule was wrong for `match` and closures | Sections 0, 2 and 4 rewritten |
| Implementation | developer | Clippy flagged a `let ... else` in `escape_notices` | Replaced with `?` |
| Tester pass | tester | No test pinned a mixed accepted set in `dry_run_lines` | Test added |
| Architect review | architect | New items glued to neighbours without blank lines | Fixed |
| Consultant design read | consultant | `accept_new_grants` suggested `--yes` saves; the auto-and-confine condition was written out three times | Renamed `confirm_new_grants` with a doc line; `TeamSession::auto_confine` added |
| Architect review | architect | The addendum said "remove the repeated test" for a repeated condition; the developer had to ask | Clarified (`m_5340`) |
