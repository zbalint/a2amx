# Spec 3el: auto grants in any team file, accepted per project

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from commit `49f21a9` (spec 3ek accepted). Anchors below are at that commit.
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. Spec 5b of 7 of the owner's configuration redesign.
**Breaking**: the accepted-grants file changes name and shape, and old acceptances are dropped (section 1).

Public test seams: `a2amx::grant` (`parse_accepted`, `render_accepted`, `consent_*`) in `tests/grant.rs`;
`a2amx::team::parse` in `tests/team.rs`; the `a2amx` binary (`team up`, `team up --dry-run`) in
`tests/auto_confine.rs`; `Layout::auto_grants` in `tests/layout.rs`.

**Scope** (files that may be edited):

- `src/grant.rs`, `src/team.rs`, `src/layout.rs`, `src/main.rs`.
- `tests/grant.rs`, `tests/team.rs`, `tests/auto_confine.rs`, `tests/layout.rs`, `tests/confine.rs` (only if it
  pins a changed message), `tests/docs_examples.rs` (only if a doc block changes).
- `docs/confinement.md`, `docs/teams.md`, `docs/getting-started.md`, `docs/cli.md`, `docs/philosophy.md` (lines 51-54 only), `docs/roadmap.md` (line 28 only), `docs/architecture.md` (lines 388 and 422
  only: the `auto_grants()` row and the paragraph on accepted names), `a2amx.toml.example`.

Does **not** touch: `src/confine.rs`, `src/suggest.rs`, `src/daemon.rs`, `src/config.rs`, migration (spec 6),
`AGENTS.md`, `CHANGELOG.md`, the rest of `docs/architecture.md`, `docs/backlog.md`, earlier specs.

## 1. Why

`auto = true` detects grants from project markers and git remotes at each `team up`. Today it is valid only in the
global team file, and an accepted grant is remembered by name for **every** project that uses that file
(`state/auto-grants`, one grant per line). A grant accepted for one project silently applies to the next. The owner
decided (grilling, 2026-11-14): `auto` is allowed in any team file; acceptance is recorded per canonical project
directory in one file, `state/auto-grants.toml`, mapping a project path to its accepted grants; the owner reviews
or revokes by editing the file (no new command); the old `state/auto-grants` is dropped and each project is asked
once again.

Rejected: one file per project under `state/` (unreadable hashes, many files); a new `confine grants` command
(a new public interface for a rare action); copying old acceptances to every project (the old file has no project
list, so it cannot say who accepted what).

Trust boundary: a repository can ship an `a2amx.toml` with `[confine] auto = true`. The paths: (a) the file
asks for grants the project's own markers justify. Disposition: **constrained**. The detected grants are the same
table as today, shown with reason and exposure, nothing starts before the owner answers, a non-terminal run
without `--yes` refuses, and `--yes` records nothing; (b) a file that already lists grants or writable paths
explicitly has no prompt today: unchanged, running `team up` trusts the file; (c) acceptance for project A applies
to project B: **removed** by the per-project key.

## 2. Rules

1. `auto = true` is accepted in `[confine]` of any team file (local, `--file`, global). It needs confinement on (spec 3ek already turned
   the `enabled != Some(true)` check into `enabled == Some(false)` with the text
   `[confine] auto needs confinement; remove enabled = false`; this spec leaves that check as it is). Session-level `auto` stays an error. `auto = false`
   means nothing, as today.
2. The project key is `TeamOrigin.dir`, which spec 3ek already makes canonical for all three file kinds (the file's
   directory for a local or `--file` file, the invoking directory for the global file). It is not canonicalized
   again. A directory that is not valid UTF-8 is an error naming the directory (no lossy conversion: two names
   could collide), in `team up` and `--dry-run` alike. Detection still scans each session's own `cwd`; a session
   whose `cwd` leaves the project directory has its detected grants recorded under the project, and the doc says so.
3. `state/auto-grants.toml` is serialized with the `toml` crate (never string-built, so a directory name with a
   quote, backslash or newline cannot inject a key or a table) and holds one table per project:
   ```toml
   [projects."/home/u/proj"]
   grants = ["cache", "cargo"]
   ```
   Keys are canonical absolute paths, as printed in the consent question; a hand-written key that is not absolute
   (does not start with `/`) is an error under rule 4. A key never matches a non-canonical spelling (a trailing
   `/`, a symlink): the project is then asked again.
   Names are grant names (`a2amx confine list`). The file is created owner-only (0600) when the first
   acceptance is written, by the existing write path (`omp::write_if_changed`), and only when the set changes.
4. Reading: first `std::fs::metadata(path)?.is_file()` (follows symlinks, so a FIFO is refused before `open`
   would block; a symlinked file is allowed, the directory is owner-only), then at most 65,536 bytes (a larger
   file is the error `accepted auto-grants.toml file {path} is larger than 65536 bytes`), UTF-8, valid TOML of exactly this shape (unknown top-level or
   project keys, a non-list `grants`, or a non-string entry are errors naming the file). Grant names that are not
   in the grant table are ignored, so a grant removed in a later release does not break the file. A missing file
   is an empty acceptance. `write_if_changed` replaces a symlinked file by a regular one on the first write; this
   is accepted and documented. A hand edit that removes a project's table revokes it. An invalid file stops `team up` (and `--dry-run`) with an error naming the file.
5. Consent text: the question becomes `Accept these for auto mode in this project ({dir})? [y/N] `. The new-grant
   detection (`new_auto_grants`) subtracts only the grants accepted for **this** project.
6. `team up` answering yes records the grants under the project key (merged with the existing ones, sorted).
   `--yes` accepts for the run and records nothing. A non-terminal run without `--yes` refuses, as today.
   `--dry-run` never prompts or writes and prints `  not yet accepted: NAME, ...` per project, as today.
7. The old `state/auto-grants` is never read or written. The docs say it can be deleted.

## 3. Code

1. `src/layout.rs`: `Layout::auto_grants()` returns `state/auto-grants.toml`; doc comment updated.
2. `src/grant.rs`: `parse_accepted(text) -> BTreeSet<String>` and `render_accepted(&BTreeSet<String>)` (647-667)
   become a per-project pair. `pub type Accepted = BTreeMap<String, BTreeSet<String>>;`
   `pub fn parse_accepted(text: &str) -> anyhow::Result<Accepted>` (rule 4, pure, the `toml` crate already in
   the tree) and `pub fn render_accepted(accepted: &Accepted) -> String` (through `toml::Table`, sorted, projects with no grants omitted,
   empty input renders the empty string).
   `// shortcut:` comment at the save site: two concurrent `team up` runs can lose one project's new acceptance
   (last rename wins, the project is asked again); a file lock if users report repeated prompts. `CONSENT_QUESTION` becomes `pub fn consent_question(project: &Path) -> String`.
3. `src/main.rs`: `MAX_AUTO_GRANTS_BYTES` becomes 65,536; `read_accepted_grants` keeps its bounded reader
   (regular file check added: today it only bounds the read) and returns `Accepted`; `load_accepted_grants` and
   `save_accepted_grants` take the project key; `confirm_new_grants` takes the project directory and merges into
   that project's entry only. The project directory is `TeamOrigin.dir` (rule 2). The dry-run report and the `team up` path (1876-1885) use the same
   project accepted set. The two error
   messages at `main.rs:70,76` are reworded to say `accepted auto-grants.toml`, so the acceptance `rg` stays clean.
4. `src/team.rs`: `merge_confine` loses the `!global` error (636-640) and its `global` parameter (callers in `parse`
   updated); the line `session.auto = global && ...` (523-526) becomes `session.auto = confine table auto == Some(true)`;
   the `auto needs confinement` check (3ek) is unchanged. `new_auto_grants` keeps its signature
   (`&BTreeSet<String>` is that project's set).

## 4. Docs

`docs/confinement.md` (every sentence about auto being global-only or acceptance being by name for all projects: lines 55, 89-139, 155-164, 219-225, 247-262, 274-284, 343, 560, 717; also: unknown grant names are dropped when the file is rewritten, so an older binary forgets a newer grant's acceptance and asks again; the same global file run from two directories asks twice; a session `cwd` outside the project directory is recorded under the project): auto in any file, acceptance per project in
`state/auto-grants.toml`, the format, how to revoke (delete the project's table), the old file is ignored, the
per-project consent question. `docs/getting-started.md:59-60,174-178` and `docs/teams.md:134-143,231`, `a2amx.toml.example:46`,
`docs/cli.md:160`: the same facts; `docs/philosophy.md:51-54` (principle 4: the tool-picked grant is no longer "only global auto mode"; the yes is remembered per project, not by grant name) and `docs/roadmap.md:28` ("global auto mode"): the same, in their own words; the example error text at `getting-started.md:178` is removed (the error no
longer exists).

## 5. Tests

Literals only. The existing tests that seed or assert `state/auto-grants` move to `state/auto-grants.toml` **and to
its format**; the developer lists each in the report: `tests/auto_confine.rs:372` (4097 bytes of `x`, now the exact
size error), `:455` (`"unknown\n"` becomes a valid project table with an unknown grant), `:478`, `:658`, `:720`
(file contents asserted in the new format), `:677` (`"cache\n"` becomes a table for the test's project directory),
the seeds at `:354`; `tests/grant.rs:130-136` and `:181-182` (parse, render and the consent question);
`tests/team.rs:3635` `confine_auto_is_global_only_and_team_false_is_accepted` (the removed global-only error becomes a positive parse test) and
`tests/auto_confine.rs:1420-1466` (`local_auto_and_file_auto_are_rejected_before_daemon_use`: delete the rejection
asserts and turn it into a `--dry-run` test that needs the running daemon of its fixture, listing the detected grants).

1. `parse_accepted` of the section 2.3 example gives `{"/home/u/proj": {"cache","cargo"}}`; unknown grant name
   `nope` is dropped; an empty string gives an empty map; `[projects."/p"]` with `grants = "x"` fails; an
   unknown top-level key fails; invalid TOML fails; a key `rel/dir` (not absolute) fails.
2. `render_accepted` of `{"/b": {"ssh","cache"}, "/a": {"cargo"}}` is exactly
   `[projects."/a"]\ngrants = ["cargo"]\n\n[projects."/b"]\ngrants = ["cache", "ssh"]\n`
   (if the crate's real output differs from this literal, the developer reports BLOCKED so the architect fixes
   the literal; it is not recomputed); an empty map renders `""`.
2b. Round trip: a map with the keys `/p "q\ x` and `/p\n[projects."/o"]` (a newline in the key) renders and parses
   back to the same map and to exactly two projects.
3. A local `a2amx.toml` with `[confine] auto = true` parses (`session.auto` is true); with `enabled = false` it fails
   with the exact 3ek text `[confine] auto needs confinement; remove enabled = false`.
4. CLI: `team up --dry-run` in project A with `auto = true` and a seeded `auto-grants.toml` accepting
   `cache, cargo` for A only: no `not yet accepted`; the same file in a second project directory B: the line appears.
5. CLI, PTY: answering `y` in project A writes exactly the project table, mode 0600; project B is still asked;
   answering `y` in B afterwards keeps A's table.
6. CLI: an invalid `auto-grants.toml` stops `team up` and `team up --dry-run` with exit 1 and the file path in
   stderr; a 65,537-byte file gives the exact `larger than 65536 bytes` text; a valid 65,536-byte file (one table
   plus comment padding) is accepted; a FIFO at that path is refused without hanging; the old `state/auto-grants`
   seeded alone is ignored (the grants are asked for).
7. CLI: `--yes` records nothing (no file); a non-terminal run without `--yes` refuses with the existing text.
8. Local file and `--file` file with `auto = true` work end to end in `--dry-run` (detected grants listed).
9. `Layout::auto_grants` in `tests/layout.rs` row: `/r/state/auto-grants.toml`.
10. `grant::consent_question(Path::new("/w/p"))` is exactly `Accept these for auto mode in this project (/w/p)? [y/N] `.

Mutants: key by the wrong directory (the invoking one instead of the project one, test 4 with `--file`); merge
replaces other projects' entries (5); unknown names kept (1); `--yes` writes (7); size cap off by one (6);
string-built keys (2b); FIFO check removed (6).

## 6. Out of scope

A command to list or revoke grants; migration of old acceptances (spec 6 drops the old file); the grant table and
detection (`src/suggest.rs`, `src/confine.rs`); a UI for the file; removing the global-file directory guard.

## 7. Acceptance

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test grant
cargo test --test auto_confine
cargo test --test team
cargo test --test layout
rg -n "all projects|auto-grants\b[^.]|only read from the global|only global auto|global auto mode" src a2amx.toml.example README.md docs/*.md
```

The `rg` command (no `docs/specs`, which are history) matches only lines that say the old file is ignored or dropped (the architect reads them at review). Test 8 and the existing auto tests with renamed file names fail first; the contract tests are
those that do not touch the file (`dry_run` without seeds). In a confined session rerun the known read-only-HOME tests
with `HOME` under `target/scratch/3el/`.

## 8. Amendment 1

The developer reported (BLOCKED) that `docs/philosophy.md:51-54` and `docs/roadmap.md:28` state the old rule (only global auto; acceptance by grant name). Both are added to the scope and to section 4, and the acceptance `rg` gains `only global auto|global auto mode`. Nothing else changes.

### Amendment 2

Section 2.4's size error text conflicted with section 3.3 and the acceptance `rg` (`accepted auto-grants file` matches `auto-grants\b[^.]`). Ruling: every diagnostic about the accepted file says `accepted auto-grants.toml file ...`, so the size error is exactly `accepted auto-grants.toml file {path} is larger than 65536 bytes` and test 6 asserts the text `larger than 65536 bytes` as before. The developer's wording is correct. Nothing else changes.

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (model not recorded); developer OMP, openai-codex/gpt-5.6-luna; tester OMP, opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 2 (amendments 1 and 2).

| Found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | Findings m_7203 (anchors, trust-boundary dispositions, round-trip test, size cap, FIFO) | Spec revised before the lock |
| Implementation | developer, gpt-5.6-luna | `docs/philosophy.md:51-54` and `docs/roadmap.md:28` state the old rule and were outside scope; the gate's old-text grep missed them | Amendment 1 |
| Implementation | developer | Size-error literal in 2.4 conflicted with 3.3 and the acceptance `rg` | Amendment 2 |
| Tester pass | tester, deepseek-v4.1-flash | `docs_examples` failed (marker line removed from a doc block); developer had claimed only the daemon test fails | Fixed in fix round 1 |
| Tester pass | tester | Consent prompt order reversed; pty assertions dropped or weakened; test 4 second half, test 5 first-creation and exactness, test 6 old-file seed absent or weak | Fixed in fix round 1 |
| Tester pass | tester | Non-UTF-8 key and symlinked accepted file untested (two mutants survived); stale docs line 723, detection-cwd text, missing symlink and two-directory notes | Tests and docs added in fix round 1 |
| Architect review | architect, Sonnet 5.5 | UTF-8 key conversion duplicated; a doc comment deleted | Fix round 2 |
| Architect review | architect | Unknown project key check survived a mutant | Test added in fix round 3 |
