# Spec 3cf: auto mode asks before it opens grants the user never named

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context
`a2amx-auto-consent-2026-11-03`. Public seams: the `a2amx` binary (`team up`, `team up --dry-run`,
`confine list`), the pure functions of `a2amx::grant`, the accepted-grants file in the state
directory.

Scope (may be edited): `src/grant.rs`, `src/cli.rs`, `src/main.rs`, `src/omp.rs` (only the visibility of `write_if_changed`), `tests/grant.rs`,
`tests/auto_confine.rs`, `tests/confine_show.rs`, `tests/team.rs`, `docs/confinement.md`, `docs/teams.md`, `docs/cli.md`.
Does not touch: `src/confine.rs`, `src/daemon.rs`, `src/wire.rs`, `src/suggest.rs`, `src/team.rs`,
`docs/backlog.md`, `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md`, `docs/specs/*` (the architect edits
those).

## 1. Why

Auto mode adds grants the user never wrote or read (`ssh`, `cache`, `cargo`, and so on). The owner's
rule for confinement is that choosing a grant by name after reading its text is the consent. Auto
removes that step, so it must put it back for the grants it adds: show what each one opens and why,
and wait for a yes. Owner decision (2026-11-03): the prompt applies to auto mode only; explicit
grants get a one-line warning and no prompt; how often to ask was settled with the consultant:
remember accepted grant names.

Rejected while grilling: ask on every interactive `team up` (nobody reads a prompt that always
appears, and it ends in `--yes` everywhere); remember the whole grant set per team file (the set
changes with each project's working directory, so it asks again for every project); skip the prompt
when stdin is not a terminal (that turns it off for exactly the callers that cannot read it); ask
in the daemon (it never runs detection and only receives the merged request).

## 2. The accepted-grants file

- Path: `<state dir>/auto-grants`, the same directory as `team.toml` (`home` in `run_team_up`).
  Plain text, one grant name per line, no comments. Written with the existing
  `omp::write_if_changed` (private today at `src/omp.rs:40`; make it `pub`, no other change to it),
  which writes a `0600` temporary file in the same directory and renames it over, run under
  `spawn_blocking`. It is a generic private-file writer that lives in `omp.rs` for historical reasons;
  moving it to a shared module is a separate refactor (section 6). The state directory exists
  because the global `team.toml` (the only place `auto = true` is valid) lives in it.
- Read: at most 4096 bytes; a larger file is an error that names the path. Lines are trimmed, empty
  lines and names that are not in `grant::all()` are ignored. A missing file is the empty set.
- It records names only, never reasons, paths or projects. Delete the file to be asked again.
- Confined sessions cannot reach it (the state directory is hidden from them).

## 3. Pure functions (`src/grant.rs`)

```rust
pub fn parse_accepted(text: &str) -> BTreeSet<String>;
pub fn render_accepted(names: &BTreeSet<String>) -> String;   // one name per line, sorted, trailing newline, empty set = ""
pub const CONSENT_QUESTION: &str = "Accept these for auto mode on all projects? [y/N] ";
pub fn consent_lines(new: &[(String, String)], rows: &[Grant]) -> String;
pub fn escape_notice(session: &str, grants: &[String], rows: &[Grant]) -> Vec<String>;
```

- `consent_lines(new, rows)`: `new` is `(grant, reason)` pairs in the order they were found. For each
  pair one line `<grant>: <reason> (<exposes>)` where `<exposes>` is the grant's `exposes` text; if
  the row has `escape: Some(text)`, a second line `  can escape: <text>`. Every line ends with a
  newline. A pair whose grant is not in `rows` prints `<grant>: <reason>` only. The prompt shown to
  the user is `consent_lines(...)` followed by `CONSENT_QUESTION`.
- `escape_notice(session, grants, rows)`: for each grant in `grants` whose row has an escape text,
  `session <session>: grant <grant> can escape: <text>`. Rows without a text give nothing.
- `Grant` gains `pub escape: Option<&'static str>`, `None` in every row for now (the escape audit,
  backlog F71, fills them). `confine list` (`render_list`) prints `  can escape: <text>` under a grant
  whose `escape` is `Some`, and its output is unchanged while every row is `None`.

## 4. The flow (`src/main.rs`, `src/cli.rs`)

`team up` gains `--yes` (a bool; allowed with `--dry-run`, which never prompts).

In `run_team_up`, after the auto loop and after the existing `validate_team_session` loop, before the
dry-run branch:

1. Collect, over all `Start` sessions with `auto`, the pairs in `confine.reasons` (the grants auto
   added, `confine.reasons` already holds only those). Keep the first reason per grant and the
   order of first appearance. Subtract the accepted set (section 2): the result is the new grants.
2. Always (also when the new grants are empty, and in a dry run): for each `Start` session, print the
   lines of `escape_notice` for the grants in its `confine.grants` that are not in its `reasons`, to
   stderr, once per session. No prompt.
3. Not dry-run, new grants not empty:
   - `--yes`: print `consent_lines(...)` to stderr, go on, record nothing.
   - otherwise call the existing `confirm(prompt, refusal)` with `consent_lines(...)` followed by
     `CONSENT_QUESTION` as the prompt. The refusal
     text is `auto confinement would open: <names>; run a2amx team up on a terminal to accept them, or pass --yes for this run`.
     A yes adds the new names to the accepted set and writes the file (section 2) before any session
     starts. Anything else (no, empty line, end of input) returns the error
     `auto grants not accepted; nothing started` and starts nothing.
4. Dry run (`--dry-run`): never prompt, never write. For each session that has new, unaccepted
   grants, print `  not yet accepted: <names>` after its reason lines (names in found order, comma
   separated); print nothing for a session whose grants are all accepted.

5. The existing stderr line `auto confinement for session <name>: grants <a, b>` moves from the auto
   loop to after the consent step: it is printed for each session that is about to start (after a
   yes, with `--yes`, when nothing was new, and also in a dry run as today), and never for a run
   that was refused or declined. Its wording is unchanged.

Nothing else changes: the stderr line's wording, the reasons in the summaries, `new --confine`,
`Reset` (it respawns the stored request and never re-runs detection).

## 5. Docs

- `docs/confinement.md`: "Automatic grant detection" explains the prompt, that acceptance is by
  grant name and covers all projects, where the file is and how to reset it, `--yes`, and that a
  non-terminal run refuses; "What stays open" says the prompt is consent and not a limit.
- `docs/teams.md`: the `team up` paragraph and the dry-run line `not yet accepted`.
- `docs/cli.md`: `team up --yes`.

## 6. Out of scope

- Moving `write_if_changed` out of `omp.rs`; notices for the implicit `harness-config` grant (it is not
  in `confine.grants`; the escape audit F71 decides); filling the `escape` texts (backlog F71); a prompt for explicit grants; per-project acceptance;
  a command to list or edit the accepted names (delete the file); changes to which grants auto
  picks; the daemon and the wire protocol; version and changelog (the architect).

## 7. Trust boundaries

Attack paths: (a) an agent runs `team up --yes` to skip the prompt: constrained, because `--yes` is
visible in its transcript, accepts for one run only and records nothing, and a session token cannot start a team (`session_token_permitted` in `src/daemon.rs` does not allow a spawn); (b) an agent plants the accepted file: constrained,
the state directory is hidden from confined sessions and the file is `0600`; an unconfined session
under the same home can still write it, which is the operator's own trust level; (c) a crafted accepted
file: constrained, bounded to 4096 bytes and only names found in the grant table are used.
Abuse-case tests: Acceptance 4 and 6.

## 8. Acceptance

Baseline (existing): `env -u A2AMX_BIN -u NO_COLOR cargo test` passes. Final: the three commands of
`AGENTS.md`, plus `scripts/prepush-fast.sh`.

1. `tests/grant.rs`: `parse_accepted` and `render_accepted` round-trip with sorted output; unknown
   names, blank lines and padding are dropped; the empty set renders `""`.
2. `tests/grant.rs`: `consent_lines` for two pairs prints the two lines with reasons and `exposes`
   text, and `CONSENT_QUESTION` is the exact question; with a fabricated `Grant` row whose `escape` is `Some`, the
   `  can escape:` line follows its grant line; `escape_notice` prints one line per grant that has
   a text and none otherwise; `render_list` output is unchanged for the real table (a snapshot of
   the existing `confine list` test already passes on old code: a contract test).
3. `tests/auto_confine.rs`, dry run: with a global `team.toml` with `auto = true` and a Cargo
   project with no git remote, `team up --dry-run` prints `  not yet accepted: cargo, cache` (grant
   table order: `cargo` is before `cache` in `grant::all()`, and `explain` puts `cache` last) and
   exits 0 without writing the file; after seeding `auto-grants` with `cache` and `cargo` the same
   run prints no `not yet accepted` line.
4. Refusal (abuse case): a real `team up` with the same setup, stdin not a terminal and no seeded
   file, exits non-zero with the refusal text, starts no session (`list` is empty), writes no file
   and prints no `auto confinement for session` line; the same with `--yes` starts the session, still
   writes no file and prints the line.
5. With the file seeded for all new names, a real `team up --detach` starts the session with stdin
   not a terminal and prints no prompt.
6. File handling (abuse case): a seeded file over 4096 bytes makes `team up` fail with an error
   naming the path; a seeded file with an unknown name starts a session and the unknown name has no
   effect.
7. The yes path at a terminal: a test through `tests/common/pty.rs` types `y`, the session starts
   and the file then holds the new names with mode `0600`; typing `n` (and separately end of input)
   exits non-zero, starts nothing and writes nothing. The pty helper runs the binary with arguments, so `team up --detach` runs on it.
8. The existing real-`team up` tests listed in section 9 keep their current assertions; they gain
   `--yes` (or a seeded file where the test is about the file). No assertion is weakened.
9. Mutation: the developer mutates the consent step in `run_team_up` so that it never prompts, always prompts, and writes the
   file on `--yes`, and names the test that fails on each.

Known flaky tests: none new; a test in the flake family `F37` of `docs/backlog.md` that fails once is
rerun once and recorded as pre-existing if it passes.

## 9. Claims (draft; verified how)

| Claim | Where | Verified |
| --- | --- | --- |
| `confirm(prompt, refusal)` returns the refusal as an error when stdin is not a terminal, writes the prompt to stderr, and treats an empty line or end of input as no | `src/main.rs:349-363` | read |
| The auto loop runs before `validate_team_session` and before the `dry_run` branch, and sets `confine.reasons` to the grants it added | `src/main.rs` (`run_team_up`) | read |
| The state directory exists because the global `team.toml` lives in it | `src/main.rs` (`resolve_team_file`) | read (consultant) |
| `omp::write_if_changed` writes a `0600` temp file in the target's directory and renames it; private at `src/omp.rs:40`; called only inside `omp.rs` | `src/omp.rs:35-75` | read, `rg write_if_changed` |
| The auto loop prints `auto confinement for session <name>: grants ...` before any consent and before validation | `src/main.rs:1483-1497` | read (consultant) |
| `explain` walks `grant::all()` and appends `cache` last; `ssh` precedes `cargo` in the table | `src/suggest.rs:82-100`, `src/grant.rs` | read (consultant) |
| `Grant` has `exposes`, no `escape` | `src/grant.rs:7-27` | read |
| Detection runs only in the client: no `suggest::` or `auto_grants` use in the daemon or session | `src/daemon.rs`, `src/session.rs` | grep, no hit |
| `team up` has no `--yes` today | `src/cli.rs:225-237` | read |
| A pty helper exists and runs the binary with arguments | `tests/common/pty.rs:46-70` | read |
| Every `Grant {` literal is in `src/grant.rs` (30 rows, each gets `escape: None`); none in `tests/` | `src/grant.rs:92-` | `rg -c '^    Grant \{' src/grant.rs` = 30; `rg 'Grant \{' tests` no hit |
| `render_list` is pinned by `render_list_has_every_grant_and_instruction` and the binary test | `tests/grant.rs:604`, `:656` | read |
| Existing tests that run a real `team up` with `auto = true` and now need `--yes` or a seeded file: `tests/auto_confine.rs` (lines 48, 102, 174, 408, 428, 504), `tests/confine_show.rs:189`, `tests/team.rs` (1697, 3054, 3061, 3081; the dry-run ones are unaffected) | grep `auto *= *true` | grep; the developer re-checks each by running it |
| `PtyHarness::spawn(args, home)` runs the `a2amx` binary on a pty with `A2AMX_HOME` set, `send` types bytes, `wait_exit` returns the exit code | `tests/common/pty.rs:46-200` | read |
| Session tokens cannot spawn | `src/daemon.rs:2124`, `:2153` | read |
