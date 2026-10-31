# Spec 3bl: Loud confinement path errors and a notice that always names the working directory

## 0. Status

**LOCKED** (2026-10-31; one consultant review `m_4735`, all findings applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F47 and F55). Architect-directed, before 0.2.0.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `605a5f6` (after spec 3bk); full suite 708 passed, 0 failed, 3 ignored, run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::plan` and `confine::notice` (`tests/confine.rs`), `team::parse` and
`a2amx team up --dry-run` (`tests/team.rs`).

**Scope.** May edit exactly these files and no others: `src/confine.rs`, `src/main.rs`, `src/team.rs`,
`tests/confine.rs`, `tests/team.rs`, `docs/confinement.md`, `docs/teams.md`. Does not touch: `a2amx.toml`,
`CHANGELOG.md` (the architect edits it), `src/daemon.rs`, `src/wire.rs`, `src/grant.rs`, `src/cli.rs`, every other file.
No new dependency. Leave the diff uncommitted.

## 1. Why

Three ways a confinement setting fails quietly today:

1. **Relative and shell-style entries.** `resolve_confine_paths` (`src/main.rs:600`) joins every entry that is not
   absolute and not `~` / `~/...` to the session's working directory. `write = ["$HOME/.cache"]` becomes
   `<cwd>/$HOME/.cache`, a path that does not exist and is silently skipped; `~alice/x` likewise. Nothing says the entry
   was never understood. At the daemon, `plan()` hands the expanded entry to `fs::canonicalize` unchanged
   (`add_canonical_existing`, `src/confine.rs:552`), so a relative entry sent by a wire client is resolved against the
   daemon's own directory.
2. **A missing operator path looks like a missing default.** The dry run prints every skipped path as
   `  hidden: <path> (skipped missing)` (`src/main.rs:1121`), including a `write` entry the user typed, and a path that
   failed for a reason other than "not found" (permission denied, not a directory) is skipped with only a debug log.
   The user cannot tell a harmless absent default hide from their own typo.
3. **The agent notice can omit the working directory.** `notice()` lists the sorted writable paths capped at 20
   (`capped_paths`, `src/confine.rs:1280`); with many writable paths the working directory falls into the
   "and N more" remainder, and a reader (a developer, in spec 3bk) concluded the checkout was read-only (F55).

## 2. Entry syntax check (`src/confine.rs`, new)

One pure function, the single definition of what an entry may look like, used by the team-file parse, the CLI resolver and
(for the absolute requirement) `plan()`:

```rust
/// Reject a confinement entry that no later stage would understand.
pub fn check_entry_syntax(option: &str, entry: &str) -> Result<()>
```

Accepts: an absolute path; `~`; `~/...`; any other path that starts with neither `$` nor `~` (a relative path, which the
CLI and the team file join to the working directory as today). Rejects, with the entry quoted:

- entry starting with `$`: `{option} entry {entry:?}: environment variables are not expanded; write the path out, or use ~/ for your home`
- entry starting with `~` but not `~` or `~/`: `{option} entry {entry:?}: only ~ and ~/... are understood, not another user's home`
- empty entry: `{option} entry is empty`

`option` is `write`, `hide` or `unhide`.

## 3. Callers (`src/team.rs`, `src/main.rs`, `src/confine.rs`)

- `src/team.rs` `merge_confine`: before the `enabled` branch (so a typo fails even when no session is confined), call
  `confine::check_entry_syntax(label, value)` for each value of the team table's `write`, `hide`, `unhide` (error
  prefixed `[confine] `) and of the session's own table (prefixed `session {session_name}: `); not `grants`. A bad entry is
  therefore refused at parse time. `validate_confine_list` keeps only the count and byte limits.
- `src/main.rs` `resolve_confine_paths` and `resolved_confine_request` return `anyhow::Result`: each entry goes through
  `check_entry_syntax` first (the `new --confine-write '$HOME/x'` path, which does not pass through the team parse). The
  four call sites (`new` at `:480`, dry-run planning at `:1064`, `validate_team_session` at `:1149`) propagate the error with
  `?`; the team start site (`:1235`) is inside a closure, so it becomes `.map(...).transpose()?`.
- `src/confine.rs` `check_request_limits` (the existing gate that runs before any filesystem work in `plan()`, `:682`): each
  `write`, `hide` and `unhide` entry, and `team_file` when present, must be `~`, start with `~/`, or be absolute; otherwise
  `{option} entry {entry:?} must be an absolute path or start with ~/` (for the team file, option `team_file`). A wire client
  therefore cannot make the daemon resolve a path against its own directory. `canonical_existing` is not reused: it names
  `label`, not the entry. The CLI already canonicalizes the team file (`src/main.rs:1278-1280`), so no client path breaks.

## 4. Operator paths that did not resolve (`src/confine.rs`, `src/main.rs`)

New public type and `Plan` field:

```rust
pub struct Unresolved { pub option: &'static str, pub path: PathBuf, pub reason: String }
// Plan: pub unresolved: Vec<Unresolved>   // sorted by (option, path); only operator write/hide/unhide entries
```

- Mechanism: `add_canonical_existing` (`:552-569`) discards the I/O error, so it is shared by callers that must not record.
  Give `PlanBuilder` one method, `resolve_operator_path(&mut self, option: &'static str, source: &Path) -> Option<PathBuf>`,
  that canonicalizes, and on failure adds the source to `skipped` (as today) and pushes an `Unresolved`; `reason` is
  `"not found"` for `ErrorKind::NotFound`, otherwise `error.kind().to_string()` (`"permission denied"`,
  `"not a directory"`). Only the operator `write` loop in `bind_operator_writes`, the operator `hide` loop in
  `apply_operator_hides` and `unhide` call it. The workspace and git common directory (`bind_workspace`), the team file,
  harness data, default hides, container hides and the mandatory roots never produce an `Unresolved`. Keep `skipped`
  as it is so the agent notice and every `skipped_paths()` consumer are unchanged. The hand-built `Plan` in
  `src/confine.rs` (`mod tests`, `:1387-1407`) gets `unresolved: Vec::new()`.
- Dry run (`src/main.rs`, after the `hidden:` lines): for each `Unresolved` print
  `  unresolved: <path> ({option}: {reason})`, and print the existing `  hidden: <path> (skipped missing)` line only for
  skipped paths that are not also an `Unresolved` path. The line `  hidden: ... (skipped missing)` stays for the missing
  default hides.
- `docs/confinement.md` (line 46 and the troubleshooting row at line 238) and `docs/teams.md` (line 175) describe the new
  line and the entry rules, including one sentence that a relative directory literally named `$x` is reached as `./$x`.

## 5. The notice names the working directory (`src/confine.rs`)

In `notice()`, when `plan.writable` contains `plan.cwd`, move it to the front of the list before capping at 20; when it does
not, leave the list alone (the notice never claims a path is writable that is not). The label stays `Writable locations: `.
In every real plan the canonical working directory is in `writable` (`bind_workspace`, `:739-740`), so the checkout is always
named. The existing cap test (`notice_caps_each_category_at_twenty_entries`) builds a plan whose `writable` lacks the
working directory and keeps passing unchanged.

## 6. Tests

`tests/confine.rs`:
1. `plan_refuses_relative_operator_entries`: a request with `write: ["relative/dir"]`, then `hide`, then `unhide`, each
   returns an error containing the entry and `must be an absolute path or start with ~/`; a relative `team_file` likewise (contract test: the old code
   resolved it against the process directory, so this one fails on the old code).
2. `plan_reports_unresolved_operator_paths`: a missing `write` entry gives one `Unresolved` with option `write`, reason
   `not found`; a `hide` entry inside a directory with mode `0o000` (created in the fixture, mode restored afterwards)
   gives reason `permission denied`; a missing default hide, a missing `team_file` and a missing working directory
   entry give none. The permission case restores mode `0o755` before any assertion (or uses a drop guard that drops before
   the `TempDir`) and returns early when `rustix::process::geteuid().is_root()` (a root user can search a `0o000`
   directory, and `TempDir` cleanup cannot list one).
3. `notice_always_names_the_working_directory`: take a real plan, replace `writable` with `plan.cwd` plus 25 paths named
   `/a-00` to `/a-24` (they sort before the working directory, so the old code puts it into `and N more`); the notice lists
   `plan.cwd` before the first `and N more`. A second case: `writable` without `plan.cwd` does not name it as writable.
4. `check_entry_syntax_accepts_and_rejects`: table of accepted (`/a`, `~`, `~/x`, `rel/x`) and rejected (`$HOME/x`, `~bob/x`,
   `~x`, ``) entries with the quoted messages.
5. Two existing tests pass a relative missing entry to `plan()` and now fail on the absolute check; change their entry to
   an absolute missing path under the fixture (or `~/missing-path`) and keep their assertions: `plan_skips_missing_and_unhides_existing_home_secrets_read_only`
   (`tests/confine.rs:165-200`, unhide `"missing-path"`, assert at `:192`) and `notice_describes_restrictions_and_keeps_closing_sentence`
   (`:1679-1690`, hide `"missing"`). The test that uses a missing relative `team_file` (`tests/confine.rs:174`) gets an absolute one.

`tests/team.rs`:
6. `confine_entry_syntax_is_checked_at_parse_time`: a team file with `write = ["$HOME/.cache"]` in `[session.confine]` fails
   `team::parse` with `session <name>: write entry "$HOME/.cache": environment variables are not expanded...`; the same entry
   under the team `[confine]` table fails with the `[confine] ` prefix even when no session is confined (the team table without
   `enabled`).
7. Update the dry-run test that asserts the `(skipped missing)` line for `~/.missing-tool` (`tests/team.rs:1601`): that
   entry is a `write` entry, so it now appears as `  unresolved: <path> (write: not found)`, and the default hides still
   print `(skipped missing)`.

## 7. Out of scope

`~user` expansion, environment-variable expansion, `grants`, new default hides, the WSL hides, changes to the
bubblewrap arguments, the `Plan` fields other than the new one, wire changes (`Plan` is not a wire type), the
notice's other categories, the `new` command's behavior when `bwrap` is missing.

## 8. Trust boundaries

Attack path: a wire client sends a relative entry so the daemon resolves it against its own working directory. Disposition:
constrained in `check_request_limits`, the gate inside `plan()` that every entry point passes through before any filesystem work (section 3); abuse-case test is test 1 (it fails on
the old code). The parse-time and CLI checks are early diagnostics only; `plan()` is the enforcement point.

## 9. Placement

`check_entry_syntax` and `Unresolved` belong to `confine.rs`, which owns what a confinement entry means; `team.rs` and
`main.rs` call it rather than each owning a rule. `main.rs` only prints.

## 10. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
rg -n 'check_entry_syntax' src   # defined once in confine.rs, called in team.rs and main.rs
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent` (`screen not ready`):
rerun once and record as pre-existing. The tester applies mutants to a scratch copy and names the killing test: (1) remove
the `$` rule; (2) remove the `~user` rule; (3) drop the absolute check in `plan()` for `hide` only; (4) record
`Unresolved` for default hides too; (5) classify every error as `not found`; (6) notice lists the working directory last;
(7) the dry run prints `(skipped missing)` for an unresolved operator path as well.

## 11. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `resolve_confine_paths` joins non-absolute, non-tilde entries to the base | `src/main.rs:600-616` | read |
| `add_canonical_existing` canonicalizes the (possibly relative) source, so a relative wire entry resolves against the daemon's cwd | `src/confine.rs:552-569` | read |
| Dry run prints all skipped paths as `hidden: ... (skipped missing)` | `src/main.rs:1121-1123` | read |
| A non-NotFound canonicalize error is skipped with a debug log only | `src/confine.rs:561-566` | read |
| `notice()` lists `plan.writable` capped at 20, label `Writable locations: ` | `src/confine.rs:1280-1294`, `:1314` | read |
| Four call sites of `resolved_confine_request` | `src/main.rs:480`, `:1064`, `:1149`, `:1235` | grep |
| `validate_confine_list` is called for write, hide, unhide, grants | `src/team.rs` (spec 3bk) | read |
| Tests that pin the `(skipped missing)` line: one | `tests/team.rs:1601` | grep |
| `tests/confine.rs` uses `skipped_paths()` at 192, 197, 250, 251, 268, 292, 312 and `plan.skipped` at 382; `skipped` is unchanged, but the relative entry at `:192` and `:1679-1690` is rejected by the new check (test 5) | `tests/confine.rs` | grep |
| `docs/confinement.md:46` and `:238`, `docs/teams.md:175` mention skipped missing paths | docs | grep |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer and tester on the
OMP harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed to either).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | Two existing tests passed a relative missing entry to `plan()` and would fail on the new check; the cap test cannot keep "exactly 20" if the working directory is always listed (and the notice would claim a non-writable path); the recording mechanism for `Unresolved` was unnamed while `add_canonical_existing` is shared; the `0o000` fixture needs a root skip and restore-before-assert; the team-level check ran only for enabled sessions; one call site is in a closure; `team_file` was not covered by the absolute check; the check belongs in `check_request_limits` | All fixed in the spec before the lock |
| Tester pass | Tester (`gpt-5.6-luna`) | Appending an `Unresolved` for missing harness-data paths survived all tests | Developer added `plan_keeps_missing_harness_data_skipped_without_unresolved`; mutant killed |
