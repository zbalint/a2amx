# Spec 3bt: the `harness-config` grant makes the session's own harness config directory persistent and writable

## 0. Status

**LOCKED** (2026-10-31, one consultant review `m_4893` applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F58; owner request of this date: "we should provide a grant that allows the agent to read and write its config dir if the user decided he wants that"). Depends on spec 3bs (the overlay it switches off for the granted harness): assign it only after 3bs is accepted and committed.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `347758c` (spec 3bs accepted); full suite 739 passed, 0 failed, 3 ignored; run as in spec 3bs section 0. Spec 3bs moved the `src/confine.rs` anchors below (it added the overlay fields, `harness_overlay_parents`, `overlay_protected_entries` and the mount kinds); re-anchor every line number against the tree before editing, and the plan step goes after `bind_harness_data` and before `bind_operator_writes`, not after the overlay computation in `finish()`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `grant::all`, `grant::lookup`, `grant::render_list`, `confine::plan`, `Plan::wrap` argv, `Plan::overlay_parents()` (`tests/grant.rs`, `tests/confine.rs`).

**Scope:** `src/grant.rs`, `src/confine.rs` (including its unit test at `:1696`), `tests/grant.rs`, `tests/confine.rs`, `tests/team.rs`, `docs/confinement.md`, `docs/teams.md` (one word: `valid runtime names`). Not `CHANGELOG.md`. No new dependency. Leave the diff uncommitted.

## 1. Why

After spec 3bs a confined Claude or OMP session can renew its login, but its config directory (`~/.claude`, `~/.omp`) is a temporary overlay: settings, hooks, skills, plugins and instructions stay read-only and anything new is discarded at exit. A user who wants the agent to change its own configuration for good (edit `~/.claude/settings.json`, add a skill, change `~/.omp/config.yml`) has only `write = ["~/.claude"]` today: an unnamed path entry that the docs do not tie to the harness and that does not exist for OMP's two directories. The owner's philosophy (memory `88c8bc9c`) is that a user toggles an integration by name, the docs say plainly what it exposes, and the user owns the risk. This spec adds one such grant.

## 2. The grant (`src/grant.rs`)

`Grant` (`:7-17`) gains `pub own_config: bool`, false on every existing row (`:45-96`). A new row, appended to the table (stable order; `static GRANTS` becomes `[Grant; 8]`):

- `name: "harness-config"`
- `exposes: "the session's own harness config directory, read-write and persistent: settings, hooks, instructions and programs under it; running sessions of that harness pick up changes immediately"`
- `team_default_ok: false`, `sockets: NO_PATHS`, `env: NO_PATHS`, `own_config: true`.

`render_list` (`:150-167`) widens its name column to the longest name plus 2 (`harness-config` is 14 characters, the current `{:<12}` would fuse it with the scope column). `lookup` and the closest-match rule are unchanged (`harness_config` is distance 1, so it is named as the closest match).

## 3. Planning (`src/confine.rs`)

`plan_with_path` already validates the grant names (`:734`). In `PlanBuilder`, after `bind_harness_data` (`:742`) and **before** `bind_operator_writes` (`:743`, whose re-protection of `~/.a2amx`, the `PATH` directories and the team file checks `writable` at that moment), when any granted name has `own_config`:

- Claude: the directory `~/.claude`. OMP: the directory `~/.omp`. Generic: error `confinement grant "harness-config" needs a Claude or OMP session`; Codex is refused earlier.
- If the directory does not exist: `skipped` (the dry run shows it), no error.
- The directory is already validated by `validate_harness_data_paths` (`:430-452`, which checks the roots `~/.claude` and `~/.omp`); reuse it, add no second check. Add the canonical directory to `writable`.
- Drop from the plan every harness data directory and data file whose path lies inside the granted directory (`harness_data`'s dirs and files for that harness): the writable directory covers them, and with the single-file credential bind gone a harness rename of `.credentials.json` is atomic and a host-side rename is seen live (probed by the consultant, `m_4893`). Hides inside and the spec 3bo read-only entries stay and win by depth (probed: `~/.claude/ide` empty, `~/.claude/local` read-only).
- The conflict checks run in `finish()` after `apply_operator_hides` (`:748`, body `:1089-1099`), because operator hides arrive there, not at the grant: if the directory is in `hidden` the plan fails with `grant "harness-config" conflicts with the hide of <path>`; if it is in `readonly` through an `unhide` at the same path (readonly is inserted after writable in `mounts()` and wins, `:1200-1205`) it fails with `grant "harness-config" conflicts with the unhide of <path>`. Neither `~/.claude` nor `~/.omp` is a default hide (`DEFAULT_HIDES`, `:71-102`; only `~/.claude/ide` inside is), so say "operator hide" in the message.
- Spec 3bs excludes a parent inside a writable path from the overlay, so a granted harness gets no overlay; one test (section 5).
- `~/.claude.json` is unchanged: its parent is the home, so saves to it still fail (backlog F59); the docs say so.
- The agent notice lists the directory among the writable paths already; add one line to the notice: "<dir>: harness config, writable and persistent (grant harness-config)" (Amendment 1: not to the dry run).

## 4. Docs (`docs/confinement.md`, `docs/teams.md`)

In "Grants" (`:58-72`): add the row to the table and reword the sentence "Each runtime is a separate opt-in" (`:70-71`) so it does not say the grants are only runtimes. State plainly what the new row exposes: the agent can change the settings, hooks, instructions and programs under that directory, **and a running session of that harness (including your own unconfined ones) picks changes up immediately**, so a hook it adds runs with your full rights at once; the directory holds programs too (`~/.claude/local/claude`, anything under `~/.omp`), and a launched `omp` whose argv0 lies there is run by the daemon's quota probe outside the sandbox within minutes; symlinked entries (`hooks`, `agents`, `CLAUDE.md` linked into another directory) can be replaced for good, which breaks a setup managed elsewhere; if `~/.claude` is itself a symlink (a dotfiles repository, a Windows-shared path on WSL) the grant opens its canonical target. Open it only for an agent you trust with that. In "Harness login and state files" (spec 3bs): replace the pointer to the grant "not yet available" with the grant name and the recipe `grants = ["harness-config"]` in the session's `[session.confine]`. A team-level table cannot grant it (the existing message applies). `docs/teams.md:116`: `valid runtime names` becomes `valid grant names`.

## 5. Tests

1. `tests/grant.rs`: add the row to the table test at `:6-62` (sockets and env empty, `own_config` true, `team_default_ok` false; no other row has `own_config`), rename that test since it no longer names only runtimes, and update every assertion that counts or enumerates the table; pin the literal `render_list` line for `harness-config` (name, separator, scope, exposes) so the column widening is tested, as `:97-116` only checks `starts_with` and `contains`.
2. `tests/confine.rs::harness_config_grant_opens_the_own_directory`: a Claude fixture with the grant has `~/.claude` in `writable`; its data directories and files inside (the credential file, `history.jsonl`, `projects`) are not separate mounts in the wrap argv; `Plan::overlay_parents()` is empty; the hides inside (`~/.claude/ide`) and a 3bo read-only entry still follow it in the argv; the same plan without the grant keeps the overlay of spec 3bs. For OMP `~/.omp` is writable with the same properties and no overlay for `~/.omp` or `~/.omp/agent`.
3. `harness_config_grant_refuses_generic_hide_and_unhide`: Generic with the grant fails with the sentence of section 3; `hide = ["~/.claude"]` and `unhide = ["~/.claude"]` with the grant on a Claude fixture fail with the conflict sentences (abuse case: neither an explicit hide nor an unhide is overridden or lost silently).
4. Extend the existing team-level refusal test at `tests/team.rs:2871-2875` (the path is generic, `src/team.rs:615-623`) with a case naming `harness-config` in the team-level table; red on the old code because lookup fails with "unknown confinement grant", not with the team-level message.
5. Real bubblewrap, same safety rules as spec 3bs test 5 (fixture home under `CARGO_TARGET_TMPDIR`, direct bubblewrap, no daemon): with the grant, a write to a regular `~/.claude/settings.json` persists on the host and a temp-file-plus-rename of `.credentials.json` succeeds and is atomic (the host file is the renamed one); without the grant, the `settings.json` write fails (read-only, a regular-file fixture). Fails, not skips, when `A2AMX_REQUIRE_BWRAP=1`.

## 6. Out of scope

The grants `claude` and `omp` of spec 3bq (a session running another agent) and its draft's assumption about the table; `~/.claude.json` and `/login` (F59); persistent writes to other harnesses' directories; Codex; a team-level form; pre-creating directories.

## 7. Trust boundaries

The grant is operator input in `[session.confine]` or `--confine-grant`, session-only. Attack path: a client names the grant to widen a session it does not own. Disposition: constrained (the same path as every grant: the daemon applies the request's own grants, the team table cannot grant it). What it exposes is the point and is stated in the table and the docs: the agent can plant a hook, setting or program that runs unconfined in running sessions at once, in the next start, and through the daemon's quota probe when an OMP `argv0` lies in the directory. Abuse-case tests: 3 (hide and unhide conflicts, Generic) and 4.

## 8. Placement

A grant row and a flag are `grant.rs`'s job; turning the flag into a mount is `confine.rs`'s job, next to `bind_harness_data`. No new module.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
rg -n 'own_config|harness-config' src tests docs
```

Mutants for the tester: (1) `own_config` ignored (the grant does nothing); (2) the grant accepted for Generic; (3) a hide conflict ignored; (3b) an unhide conflict ignored; (4) `own_config` true on another row; (5) `team_default_ok` true; (6) the overlay still planned for the granted harness; (7) the data files inside the granted directory still bound separately; (8) the grant added after `bind_operator_writes` (the 3bo read-only entries are lost); (9) `render_list` column not widened.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `Grant` has `name`, `exposes`, `team_default_ok`, `sockets`, `env`; the table has 7 rows | `src/grant.rs:7-17`, `:45` | read |
| `render_list` prints name, scope and exposes | `src/grant.rs:150-166` | read |
| `plan_with_path` validates the grant names and the daemon applies a request's own grants | `src/confine.rs:734`, `:746` | read |
| `bind_harness_data` is the place harness paths are mounted; the grants are validated at `:734`, `bind_harness_data` at `:742`, `bind_operator_writes` at `:743`, `apply_operator_hides` at `:748` | `src/confine.rs:905-925` and the lines named | read |
| An entry in `hidden` and an entry in `writable` with the same key: the writable one wins in `mounts()` (inserted after hidden) | `src/confine.rs:1190-1215` | read |
| Tests and docs that count or print the grant table: `tests/grant.rs:6-62`, `:97-116`, `:118-128`; the unit test `src/confine.rs:1696`; `tests/team.rs:2871-2875` (generic team-level refusal); `docs/confinement.md:60-72`, `docs/teams.md:116`; `docs/cli.md:66-71` and `docs/harnesses.md:66` only point to `confine list` | as listed | read (consultant `m_4893`) |
| A single-file bind on an overlay pins the inode (a host rename is not seen); under a writable directory bind a host rename detaches the file bind and is seen; with the file bind kept, a rename gets EBUSY and Claude falls back to an in-place write | scratch bwrap | probed (consultant) |
| `bind_operator_writes` re-protects `~/.a2amx`, `PATH` directories and the team file against `writable` as it is at that moment | `src/confine.rs:972-997` (before 3br), call at `:743` | read (consultant) |
| `readonly` is inserted after `writable` in `mounts()` and `finish()` drops only explicit writes from it | `src/confine.rs:1200-1205`, `:1121-1122` | read (consultant) |
| `validate_harness_data_paths` validates the roots `~/.claude` and `~/.omp` | `src/confine.rs:430-452` | read (consultant) |

## Amendment 1 (2026-11-01, developer `m_4911`, architect adjudication)

Section 3 asked for the descriptive line in both the notice and the dry run, but section 0 excludes `src/main.rs`, the only place that prints the dry run, and the dry run needs a new `Plan` accessor to know which directory the grant opened. The conflict is real and was the architect's omission. Ruling: **the dry run is unchanged.** It already prints `grants: harness-config` and `writable: <dir>` for the opened directory, which says the same thing; scope stays as in section 0 (no `src/main.rs`). The notice line of section 3 stays, and test 2 also asserts it in `confine::notice` for the granted Claude and OMP fixtures (and its absence without the grant). Also, for test 1 the `render_list` column rule is: the name column is 16 wide (`harness-config` plus 2) and the scope column stays 15 wide, so a `session only` row has three spaces before the description; pin that literal.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus (review `m_4893` before the lock; exact model id not re-derived in this session); developer and tester on the OMP harness, `openai-codex/gpt-5.6-luna` as they reported it (planning and advisor models not exposed). BLOCKED/amend cycles: 1 (Amendment 1).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`m_4893`) | The plan step had to precede `bind_operator_writes` (the 3bo read-only re-protection checks `writable` at that moment), operator hides arrive after the grant so the conflict checks belong in `finish()`, a granted directory must drop the data binds inside it, the team-level refusal test and the table tests needed listing, and the `render_list` column would fuse with the scope column | Folded into the locked spec: placement before `bind_operator_writes`, conflict checks in `finish()`, data binds dropped, tests 1 and 4 and the column widening |
| Implementation | Developer (`m_4911`) | Section 3 asked for the grant line in the dry run, but section 0 excluded `src/main.rs`, the only place that prints it; the architect's literal for the `render_list` row had the wrong spacing | Amendment 1 (`0dc8f44`): dry run unchanged (it already prints `grants:` and `writable:`), notice-only line tested in test 2, name column 16 and scope column 15 |
| Architect review | Architect (Claude Sonnet 5.5) | The real-bubblewrap test bailed when the overlay was unsupported, so it would fail on CI's bubblewrap 0.9.0; the grant predicate and the root match were written twice (`bind_harness_config` and `finish()`); a blank line was missing | Rework: the ungranted half uses `without_overlays` when the overlay is unsupported, `finish()` uses `self.harness_config`, the blank line was added; mutants 3 and 3b rerun and killed |
| Tester pass | Tester | No problems found; mutants 1-9 all killed, symlinked and missing config directories, nested hide and read-only entries probed under real bubblewrap | None needed |
