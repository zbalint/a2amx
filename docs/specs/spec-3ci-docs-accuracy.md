# Spec 3ci: Confinement docs match the code, and a test keeps the grant table in sync

## 0. Status

LOCKED. Docs plus one test; no change to `src/`.

Scope (files that may be edited): `docs/confinement.md`, `docs/teams.md`, `docs/harnesses.md`,
`docs/architecture.md`, `tests/docs_examples.rs`. Does not touch: `src/`, `README.md`,
`docs/cli.md`, `docs/backlog.md`, `CHANGELOG.md`, `a2amx.toml.example`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `92913bb`.
Context id: `a2amx-docs-refresh-2026-11-03`. Public test seam: `a2amx::grant::all()` and the
text of `docs/confinement.md`, read with `include_str!`.

## 1. Why

A consultant review of the docs on 2026-11-03 found text that predates grants and auto mode.
The recipes (Rust, Node, Python, Go, Gradle) tell users to hand-write `write` paths the named
grants now cover, and two of them say the opposite of what a grant does. `confine suggest` is
described wrongly. A removed key name is still documented. Two other pages describe harness
confinement as it was before the harness config root became read-write by default.

Nothing stops the grant table from drifting from `src/grant.rs`: `tests/docs_examples.rs`
parses only complete TOML blocks. The table is correct today, so the new test passes on the
current code (a contract test); its value is catching the next change that edits one side only.

Rejected: generating the table from `grant::all()` at build time (a docs generator is a new
mechanism for one table); a test for `# fragment` blocks (they are not valid team files on
their own, and wrapping them is a design question left for a later spec).

## 2. `docs/confinement.md`

Edits are by anchor; line numbers are at base `92913bb`.

2.1 Line 20 (`The quickest way to get the write line...`): `a2amx confine suggest [DIR]` prints a
`grants = [...]` line, checks both build-file markers and git remotes, and uses `HOME` when it can
(`docs/cli.md`, `confine suggest` entry). Reword to say that.

2.2 Lines 22-31 (`### One command`): add `--confine-grant NAME`, `--confine-keep-env NAME` and
`--confine-no-default-hides` to the example flag list and say in one sentence what each does. The
flag names are the ones in `src/cli.rs` (`confine-grant`, `confine-keep-env`,
`confine-no-default-hides`). One short clause per flag.

2.3 Lines 38-43: replace "Auto detection recognizes Go and Python markers..." with a statement
that detection covers the marker-based grants listed in the detection table and, from git remotes,
`ssh`, `gh` and `git-credentials`, and always adds `cache` (see the line "no cache grant is implicit"
above, which concerns explicit configuration); link to that table instead of restating it. Keep the sentence
that environment-relocated paths still need an explicit `write` entry.

2.4 Lines 45-51 (`### Team file`): the list of keys the table accepts gains `keep_env`
(per-session only; see the existing `keep_env` section near line 458).

2.5 Line 62: delete the sentence "The complete team file at the end of this page parses as a team
file." It describes a test, not behavior a reader needs.

2.6 Lines 203-208: move the paragraph "Auto-grant consent is permission for the named grant..." out
from between "It leaves these residuals:" and the numbered list. Put it directly after the list.

2.7 Lines 219-237 (`## Harness login and state files`): the paragraphs starting "Claude and OMP
sessions bind their real harness configuration root..." and "Claude and OMP sessions bind their
harness configuration root..." say the same thing. Keep one; keep the `/login` and
`~/.claude.json` limitation and the `harness-config` sentence.

2.8 Recipes (`## Recipes by ecosystem`, lines 239-361): each of Rust, Node, Python, Go, Maven and
Gradle leads with the grant, as `grants = ["cargo"]` (and so on) plus its `a2amx new --confine
--confine-grant cargo -- ...` form, and keeps the narrower `write` path as a labelled fallback for a
reader who does not want the whole grant. Specific corrections:

| Recipe | Today | Required |
| --- | --- | --- |
| Rust | calls `~/.cargo` "the default for a Rust marker"; line 257 "The default protects `~/.cargo/bin`" | no such default exists; use `cargo` grant (it also opens `~/.rustup`); say the grant keeps `~/.cargo/bin` read-only when it is on `PATH` |
| Node | `write = ["~/.npm"]` only | `node` grant first |
| Python | "No recipe is given here for uv, poetry, conda, or pipx" | the `python` grant covers pip, uv, poetry, virtualenv and pipx data; say so; keep "conda" as not covered. The fallback is the existing note that a workspace virtual environment needs no `write` entry; do not invent a `write` path (none is tested) |
| Go | "These paths were not tested", `go env` placeholders; "Do not open `~/go/bin`" (line 297) | `go` grant first; keep the `go env` form as the fallback for relocated paths. Replace the `~/go/bin` sentence with: the grant opens all of `~/go`, including `bin`, which stays read-only only when it is on `PATH` (`confinement.md` line 10); do not reuse the phrase "Do not open" |
| Maven | `write = ["~/.m2/repository"]` | `java` grant first; the "hide settings.xml" advice stays only for the fallback form |
| Gradle | "Do not open all of `~/.gradle`" | the `java` grant opens all of `~/.gradle`; say what that exposes (gradle.properties credentials, init.d scripts) and offer the narrower `write` fallback |

2.9 Line 485 (the sentence that opens "With" and names the old key): the team-file setting is
`default_hides = false`; on a command line it is `--confine-no-default-hides`. Do not name the old
team-file key in the new sentence (the session-level key `confine_no_default_hides` is rejected with a message naming `default_hides =
false`, `src/team.rs:595-596`, and an old `no_default_hides` inside `[session.confine]` is an
unknown field).

2.10 Troubleshooting row 2 (`A tool says it is not logged in`): the fix names the matching grant
first (`gh`, `ssh`, `git-credentials`, `sonar`, `aws`, `gcloud`, `azure`, `kube`, `terraform`,
`npm-token`, `pypi-token`, `cargo-token`, `netrc`, `huggingface-token`), and `unhide` only for a credential no grant covers.

2.11 `## A complete team file` (line 502): replace `write = ["~/.cargo", "~/.npm"]` on the
developer session with `grants = ["cargo", "node"]`. The existing `toml_examples_in_the_confinement_guide_parse` still parses the block (it is the only
non-fragment block in the page); the new test in section 5 does not parse TOML.

2.12 `Under auto, git push works only when a remote is detected...` (line 154) stays here. Section 3
turns the copy in `teams.md` into a link.

## 3. `docs/teams.md`

3.1 Lines 135-137: the paragraph that repeats the `confinement.md` line 154 paragraph word for word
becomes one sentence plus a link to `confinement.md`. No behavior is stated only in the copy.

## 4. `docs/harnesses.md` and `docs/architecture.md`

4.1 `docs/harnesses.md` lines 61-62 say harness data directories are "read-only except for the
documented data allow-list". Claude and OMP sessions get their harness config root read-write and
persistent by default. Reword to match `confinement.md` (Harness login and state files). Line 71
still says "ephemeral harness writes"; reword the link text to "harness login and state files".

4.2 `docs/architecture.md` `### PTY confinement` (lines 353-362): add a short paragraph naming the
`grant` module (the table of named grants and consent text) and the `suggest` module (detection from
project markers and git remotes), and saying that `team up` under global `auto` asks before it opens a
grant the user has not accepted, keeping the accepted names in the `auto-grants` file in the state
directory. Three to six sentences; no new diagram.

## 5. `tests/docs_examples.rs`

One new test, `grant_table_matches_the_code`, through the public seam only.

- Read `docs/confinement.md` with `include_str!`. Take the lines after the `### Grants` heading
  up to the next `### ` heading. Each row of the form `| `NAME` | EXPOSES | DEFAULT |` is one grant.
- Remove backticks from both the doc cell and `Grant::exposes` before comparing (the doc wraps
  paths in backticks for `gh` and `ssh`; the code does not).
- Assert: the doc names are exactly `grant::all()` names, in the same order (collect both name lists
  and compare them whole; do not zip, which silently stops after a deleted row); each doc EXPOSES text
  equals the code text after the backtick rule; a DEFAULT cell starts with `Yes` exactly when
  `team_default_ok` is true and with `No` when it is false.
- Failure messages name the grant and which side differs.
- Expected values are the doc and code text, never recomputed with the logic under test.

Mutation check (required): on a scratch copy or in the checkout restored afterwards, (a) delete one
grant row from the table, (b) change one word in one EXPOSES cell, (c) flip one `Yes` to `No`, (d)
swap two rows. Each must fail the new test. The developer names the failure for each and restores
the tree.

## 6. Claims verified at base `92913bb`

| Claim | Where | How |
| --- | --- | --- |
| The grant table has one row per `grant::all()` entry and the texts match | `docs/confinement.md:79-112`, `src/grant.rs:95-492` | consultant script; to be re-run by the developer as the first red/green step |
| `confine suggest` prints `grants = [...]`, reads git remotes | `docs/cli.md` `confine suggest` entry | read by consultant, architect to re-read before lock |
| `keep_env` is a per-session key; a team-level `keep_env` is rejected | `src/team.rs:82`, `src/team.rs:688-690` | read |
| The old session key is rejected naming `default_hides = false` | `src/team.rs:595-596` (inside `reject_old_confine_format`, starting at 569) | read by consultant and architect |
| The `--confine-grant`, `--confine-keep-env`, `--confine-no-default-hides` flags exist | `src/cli.rs:72-75` and nearby | grep |
| The fragment blocks are skipped by the TOML test | `tests/docs_examples.rs:29-31` | read by consultant |
| No test checks the grant table | `grep -rn '### Grants\|Choosing grants' tests/ src/` finds nothing | probed, 2026-11-03 |
| `confine suggest` prints a `grants = [...]` line from markers and git remotes | `docs/cli.md:70-74` | read |
| The old team-file key is rejected with a message naming `default_hides = false` | `src/team.rs:595-596` | read |
| The three flags exist | `src/cli.rs:69-70` (`confine-no-default-hides`), `72-73` (`confine-grant`), `75-76` (`confine-keep-env`) | read by consultant |
| The `git push ... remote is detected` paragraph occurs once in each of the two pages | `grep -rc` over `docs/` | probed, 2026-11-03 (the spec file adds two more, hence the file-scoped acceptance greps) |

## 7. Out of scope

- Onboarding material (a global `auto = true` example, real command output, a first-team
  walkthrough, a cloud and token grant section): spec 3cj.
- Validating `# fragment` TOML blocks.
- `CHANGELOG.md` (the architect adds the line), `README.md`, `docs/cli.md`, `docs/backlog.md`.
- Any change to `src/` or to grant text in `src/grant.rs`.
- Rewording sections the review did not flag.

## 8. Acceptance

Run as `env -u A2AMX_BIN -u NO_COLOR` (see `AGENTS.md`).

1. `cargo test --test docs_examples` passes, including `grant_table_matches_the_code`. The new
   test passes on the unmodified base (a contract test); the mutation check in section 5 is how
   it is shown to bite. Judge: the command output and the developer's list of four failures.
2. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with no
   warnings. Judge: command output.
3. `git status --short` lists only the files in section 0's scope. Judge: architect.
4. `rg -n "no_default_hides|Go and Python markers|get the .write. line|parses as a team file" docs/confinement.md`
   matches nothing. Judge: command output.
5. `rg -n "Do not open .~/go/bin|Do not open all of .~/.gradle|default for a Rust marker|The default protects" docs/confinement.md`
   matches nothing. Judge: command output.
6. Every recipe in 2.8 shows a `grants = [...]` form; each except Python also keeps its narrower
   fallback, and Python keeps the workspace virtual environment note. Judge: architect at final
   review, reading the six recipes.
7. The paragraph in `teams.md` 3.1 is a link, not a copy: `rg -c "works only when a remote is detected" docs/confinement.md docs/teams.md` prints
   `docs/confinement.md:1` and no line for `docs/teams.md` (the link sentence must not repeat the
   phrase). Judge: command output.

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant a2amx-consultant (Claude Code, model not
exposed to the architect); developer OMP with `openai-codex/gpt-5.6-luna` for planning and working, advisor
model not exposed; tester OMP with `openai-codex/gpt-5.6-luna`, advisor model not exposed.

BLOCKED/amend cycles: 1 before the lock (consultant review of the draft, amended in the draft), 0 after.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft, architect probe | architect | Acceptance greps over `docs/` matched this spec's own quoted text | Greps scoped to single files |
| Draft review | consultant | Go row (2.8) and acceptance 5 contradicted: a "keep the fallback" rule preserved the forbidden `~/go/bin` sentence | Go row names the replacement sentence |
| Draft review | consultant | Python had no narrower fallback to keep, yet acceptance 6 required one | Python exempt; its workspace-venv note is the fallback |
| Draft review | consultant | Wrong anchors (`src/team.rs` key name, `src/cli.rs` flag lines, `tests/docs_examples.rs` lines); 2.11 said the new test parses TOML; acceptance 4 grep too loose; 2.3 omitted `cache`; 2.10 omitted `ssh`, `git-credentials`, `sonar`; Rust line 257 kept "default" framing | All fixed in the draft before the lock |
| Implementation | developer's advisor | The first edit of the "One command" block deleted the existing `--confine-write`, `--confine-unhide` and `--confine-hide` examples while adding the three new flags | Developer restored all six forms and re-read the block before reporting |
| Tester pass | tester | No blank line after list item 7, so the moved consent paragraph rendered inside the list (spec 2.6 had no acceptance line for it) | Developer inserted the blank line (`confinement.md` line 216); architect verified |
| Tester pass | tester | Mutation pass could not run: the assignment named no scratch location, and the private `/tmp` (1 GB tmpfs) was too small; the tester built once in `~/.cache` | Architect ran seven mutants in place and restored the file by checksum; the owner set the rule that scratch files go in the gitignored `target/` and are deleted by their creator |
| Architect review | architect | Four `tests/auto_confine.rs` PTY tests fail when the suite runs inside a confined session (the daemon takes `HOME` from its inherited read-only environment); not caused by this diff | Disposition: environmental, follow-up tests-only hardening; owner or CI runs the full suite |
