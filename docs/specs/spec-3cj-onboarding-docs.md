# Spec 3cj: A new user can start a confined team from the docs alone

## 0. Status

LOCKED. Docs plus test changes; no change to `src/`.

Scope (files that may be edited or created): `docs/getting-started.md` (new), `README.md`,
`docs/confinement.md`, `tests/docs_examples.rs`. Does not touch: `src/`, `docs/cli.md`,
`docs/teams.md`, `CHANGELOG.md`, `docs/backlog.md`, `a2amx.toml.example`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `63282e5`.
Context id: `a2amx-docs-refresh-2026-11-03`. Public test seam: `a2amx::team::parse` and
`a2amx::team::parse_with` on the text of the docs, read with `include_str!`.

## 1. Why

The 2026-11-03 consultant review found five onboarding gaps: no example of the global team file
with `auto = true`, no real output for `team up --dry-run`, the consent prompt or `confine suggest`,
no start-to-finish walkthrough (the README quick start ends at `a2amx new` and never mentions teams),
no `--confine-grant` example outside Docker, and the cloud and token grants have table rows only.
The owner's goal (stored decision, 2026-11-02) is a confinement a user without an IT background can
set up; a probe the architect ran today showed how easy it is to get wrong: a global team file that
sets `team = "..."` is refused (`the global team file must not set team; the team is named after the
directory`), and `team up --dry-run` fails without a running daemon. Only real output shows those.

Rejected: putting the walkthrough in the README (it is a landing page, `project-docs`); rewriting the
recipes again (spec 3ci did); testing that every shown command exists (a new mechanism for one page;
the commands are checked by the tester instead).

## 2. `docs/getting-started.md` (new)

Title "Your first team". Sections, in this order, each short:

2.1 Install and start the daemon: three lines, link to `docs/install.md`.
2.2 Write the team file: copy `a2amx.toml.example` to `a2amx.toml` in the project; show a two-session
complete TOML block (`team = "demo"`, names `architect` and `developer`, commands `["claude"]` and `["omp"]`,
`cwd = "."`); say that `a2amx.toml` is gitignored in this repository and that a session name is unique
per daemon.
2.3 Let a2amx suggest grants: `a2amx confine suggest` with its real output (section 6 sample S1), then
a complete TOML block that adds `[confine] enabled = true` and the suggested `grants` line.
2.4 Preview: `a2amx team up --dry-run` on that local file with abridged real output (S2). Say what each shown line means
in one clause: `confinement:`, the detection reason, `grants:`, `writable:`, `protected:`, `hidden:`.
2.5 Start and look around: `a2amx team up`, `a2amx team status`, `a2amx list`, `a2amx confine show
SESSION` with real output (S4), detach with the prefix key, the session picker.
2.6 Send a message: one paragraph, what an agent calls (`send_message`), with a link to
`docs/using-with-agents.md`.
2.7 Stop: `a2amx team down`.
2.8 "If something fails": two failures with their real messages and the fix: `cannot reach the a2amx
daemon: start it with a2amx daemon start` (run `team up --dry-run` with no daemon), and a refusal
that a local file hits (S3 is not shown here, because it only occurs in auto mode and belongs to 4.1).

State at the top of the page the version printed by `a2amx --version`, that long paths are shortened, and,
for each sample, which command produced it: S1, S2, S3 and S5 start nothing and use the page's own file
with its real `claude` and `omp` commands; S4 needs running sessions and uses the same file with both
`command` values replaced by `["sleep", "600"]` so no agent starts (say so next to S4). The page uses the
local file and explicit grants; the global `auto = true` file is taught only in `docs/confinement.md` 4.1.

## 3. `README.md`

3.1 Under `## Quick start`, after the existing install lines, add one short paragraph that points to
`docs/getting-started.md` for starting a team, in at most three sentences.
3.2 Add a line for `docs/getting-started.md` to the `## Documentation` list, before the CLI reference.
3.3 `## Example`: the text before the first command says "Start the daemon in one terminal". The
daemon detaches by default (`docs/cli.md`: `a2amx daemon start [--foreground]`, "detached by default";
run on 2026-11-03: it printed `listening on 127.0.0.1:PORT` and returned). Reword so a reader does not
keep a terminal for it, and fix the sentence "use two more terminals" to match (the `new` commands
attach their own terminal).
3.4 The `a2amx list` sample has no `CONFINE` column. The real header is
`ID NAME HARNESS CONFINE STATE UPTIME ACTIVITY IN-STATE ATTACHED PENDING HELD QUOTA SIZE`
(`src/main.rs:2918-2921`). Add the column with the value `off` to both rows of the sample, and check
that the header line equals the header printed by a real `a2amx list` run with stand-in `sleep 600`
sessions (the developer captures one in the scratch of section 6). The planner and reviewer rows stay
illustrative; reword "This run used a temporary state directory and a freshly installed build" so it
states only what is true of the sample.
3.5 `## Compatibility and limits`: add one bullet: confinement is opt-in (`--confine`), a safety net
against accidents by a trusted agent and not a sandbox, and needs `bwrap` on the daemon's `PATH`; link
to `docs/confinement.md`. Add a one-line mention of confinement to `## What you get`. Nothing else in
the README changes.

## 4. `docs/confinement.md`

4.1 New subsection `### Auto mode in one file`, inside `## How to set it up`, after `### Preview`:
a complete global team file as a TOML block whose first line is the marker comment `# global team file`
(no `team` key; `[confine] enabled = true`, `auto = true`; one session), the path it goes in
(`<state home>/team.toml`, state home order as `docs/cli.md` gives), what happens on the first `team up`
(the prompt, S5; the accepted names are kept in `<state home>/auto-grants`), `team up --yes`, the
non-terminal refusal (S3) and `team up --dry-run` with `not yet accepted` (S2). Link to
[Automatic grant detection](#automatic-grant-detection) for the rules; do not restate them.
4.2 In `### One command`, add a second example without Docker:
`a2amx new --confine --confine-grant cargo --confine-grant cache -- omp`, one sentence.
4.3 New section `## Cloud CLIs and registry tokens`, placed after `## Credentials a tool needs` (with its
`### Credential environment variables` and `### ssh and signed commits` subsections) and before
`## Troubleshooting`: one `# fragment` TOML block with `grants = ["aws", "kube"]` for a session that uses
them, then a list of the ten credential grants not otherwise shown (`aws`, `gcloud`, `azure`, `kube`,
`terraform`, `netrc`, `huggingface-token`, `npm-token`, `pypi-token`, `cargo-token`) with the one-clause
exposure from the grants table (link to the table, do not copy the texts), and one sentence linking to
[Credential environment variables](#credential-environment-variables) for the AWS and GCP names that
stay set. No new claim about what a grant opens beyond the table.

## 5. `tests/docs_examples.rs`

5.1 The loop that parses the complete TOML blocks of `docs/confinement.md` moves into one helper
`parse_complete_blocks(markdown: &str, label: &str) -> usize` (extend the existing code; do not add a
second near-identical helper). A block whose first line is `# fragment` is skipped. A block whose first
line is `# global team file` is parsed with `a2amx::team::parse_with(block, Some("demo"))`; every other
block with `a2amx::team::parse`. The helper returns the number of parsed blocks.
5.2 `toml_examples_in_the_confinement_guide_parse` keeps its checks and calls the helper; it also
asserts that the markdown contains a line equal to `# global team file` (the helper keeps the `usize`
return; the marker is asserted on the text, not counted).
5.3 New test `toml_examples_in_getting_started_parse` calls the helper on `docs/getting-started.md` and
asserts at least two parsed blocks.
5.4 Mutation check (required): (a) remove the `# global team file` marker line from the confinement
block: the confinement test fails (the file sets `auto`, rejected outside the global file); (b) add a
`team = "x"` line at the top level of that block, after the marker and before `[confine]`: fails (`the global team file must not set team`); (c) break one TOML
block in `getting-started.md` (an unterminated string): the new test fails; (d) delete all complete
blocks from `getting-started.md`: the count assertion fails. Name the failure message for each;
restore each file and check its checksum.

## 6. Output samples (S1 to S5)

Every sample in the docs is pasted from a real run of the installed binary (`~/.a2amx/bin/a2amx`) and
nothing is typed by hand. Where it helps, long lists are shortened with `...` and the temporary path is
replaced with `/home/user/demo/proj`. The architect ran these on 2026-11-03 (a2amx v0.1.2-133-g91c802a,
Cargo project with an `https://github.com/example-org/demo.git` remote, a temporary home without
credentials) and the developer reruns them:

| Sample | Command | Observed on 2026-11-03 (abridged) |
| --- | --- | --- |
| S1 | `a2amx confine suggest .` | `# Suggested grants:` then `grants = ["cargo", "cache"]` |
| S2 | `a2amx team up --dry-run` on the page's local file (explicit grants, sessions `architect` and `developer`) in 3cj, and on the global file with `auto = true` for 4.1 (observed so, below) | `using global team file ... as team proj`; `auto confinement for session proj-builder: grants cargo, cache`; `would start proj-builder`; lines `harness: omp`, `confinement: auto`, `cargo: Cargo.toml found`, `cache: always added by auto`, `not yet accepted: cargo, cache`, `grants: cache, cargo`, `writable: .../proj/.git`, `protected: .../proj/.git/config`, then many `hidden:` lines |
| S3 | `a2amx team up </dev/null` (global auto file only; used in 4.1, not in the walkthrough) | `a2amx: auto confinement would open: cargo, cache; run a2amx team up on a terminal to accept them, or pass --yes for this run`; exit 1 |
| S4 | `a2amx list`, `a2amx team status`, `a2amx confine show demo-architect` after `team up` on the page's local file with both commands replaced by `["sleep", "600"]` | not yet observed; the developer captures them |
| S5 | `a2amx team up` on a terminal on the global auto file (4.1), answering `n` | not yet observed; contains the question `Accept these for auto mode on all projects?`; the developer captures it through a pseudo-terminal and answers `n`, so nothing starts |

Method for the developer (scratch rules apply, section 7): create `target/scratch/3cj/` with a project
directory `proj`, an empty home `home` and a state directory `state`. Because `proj` sits inside this
repository, run `git init` in it and `git remote add origin https://github.com/example-org/demo.git`,
and create a `Cargo.toml` (package `demo`), or git detection finds the parent repository's real remote.
`export HOME=<scratch>/home A2AMX_HOME=<scratch>/state` in the shell that runs every command below,
`daemon stop` included: the CLI resolves its daemon from the state home and never from `A2AMX_ADDR`
(`src/main.rs:221-233`), and the session's default state home is hidden, so a command that forgets it
fails with `cannot reach` and never touches the working daemon. Then `a2amx daemon start`, run the
commands, `a2amx daemon stop`, and verify `a2amx daemon status` reports it not running before removing
`target/scratch/3cj/`. Capture S5 with `script -qc` and a piped `n`. Never start a real agent, never use
`~/.cache`. Inside a confined session `pgrep` sees only that session's own processes, so it proves
nothing about the daemons; use `daemon status`.
Path shortening: the daemon takes `HOME` from its own environment, so the dry run names both the project
path and the scratch home. Replace the project path with `/home/user/demo/proj` and the scratch home with
`/home/user`; the state directory path with `/home/user/.local/state/a2amx`.

## 7. Scratch files

Scratch goes in the gitignored `target/scratch/3cj/` only and is deleted before the report (owner rule,
2026-11-03; `AGENTS.md`). No cargo build or second target directory is needed: the work uses the
installed binary and edits docs; `cargo test --test docs_examples` reuses the existing `target/`.

## 8. Claims verified at base `63282e5`

| Claim | Where | How |
| --- | --- | --- |
| A global team file that sets `team` is refused with the quoted message | `src/team.rs:497`; run | probed 2026-11-03 |
| `team up --dry-run` needs a running daemon | run: `cannot reach the a2amx daemon: start it with a2amx daemon start` | probed 2026-11-03 |
| `parse_with(text, Some(name))` is the global-file parse, `parse(text)` the local | `src/team.rs:480-521` | read |
| `auto` is read only in the global file | `src/team.rs:673-675` | read |
| The prompt text contains `Accept these for auto mode on all projects?` | `tests/auto_confine.rs` (the waits in the four PTY tests) | read |
| State home order is `--home`, `A2AMX_HOME`, `$XDG_STATE_HOME/a2amx`, then `$HOME/.local/state/a2amx` | `docs/cli.md:128` | read by consultant and architect |
| A local `a2amx.toml` wins over the global file, and `auto` in a local file is refused | `src/main.rs:965-975`, `src/team.rs:672-675` | read by consultant |
| The CLI never reads `A2AMX_ADDR`; only the MCP server does | `src/main.rs:221-233`, `src/mcp.rs` | read by consultant |
| Inside a confined session `pgrep` sees only that session's processes | the consultant's own session argv shows `bwrap --unshare-pid` | read by consultant |
| `confine suggest` prints no `gh` grant when the home has no `~/.config/gh` | run in a credential-free home | probed 2026-11-03; the developer must not claim `gh` appears |
| `a2amx.toml` is gitignored here | `.gitignore` line `/a2amx.toml` | read |
| The daemon detaches by default | `docs/cli.md:10`, `src/cli.rs:200-202`; run 2026-11-03 | read, probed |
| The `list` header has a `CONFINE` column after `HARNESS` | `src/main.rs:2918-2921` | read |
| `bwrap` is resolved from the daemon's startup `PATH` and the request is refused without it | `docs/cli.md:52-54` | read |
| The existing guide test parses non-fragment blocks of `docs/confinement.md` with `team::parse` | `tests/docs_examples.rs:25-45` | read |

## 9. Out of scope

- Output samples for commands not named above; screenshots; recorded terminal sessions (backlog F13).
- A link checker or a command-existence test.
- Any change to `src/`, `docs/cli.md`, `docs/teams.md` or `a2amx.toml.example`.
- `CHANGELOG.md` (the architect adds the line), `docs/backlog.md`.
- The other pages the consultant's second review flagged (`docs/roadmap.md`, `docs/using-with-agents.md`,
  `docs/install.md`, `CONTRIBUTING.md`): spec 3ck. `CHANGELOG.md` and `docs/backlog.md` were corrected by
  the architect (commit 08b51e2).

## 10. Acceptance

Run as `env -u A2AMX_BIN -u NO_COLOR` (see `AGENTS.md`).

1. `cargo test --test docs_examples` passes: the confinement test and `toml_examples_in_getting_started_parse`.
   The confinement test passes on the unmodified base; the new test fails before `docs/getting-started.md`
   exists. Judge: command output; the developer names the failure of the new test on the base.
2. The mutation check of 5.4: four mutants, each failing the named test. Judge: the developer's list; the
   architect reruns two.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass (the four
   `auto_confine` PTY tests may fail only when run inside a confined session: record them as the known
   environmental failure, not new). Judge: command output.
4. `git status --short` lists only the files in section 0. `ls target/scratch/3cj` fails (the directory is
   gone); the developer's report shows `a2amx daemon status` (with the scratch `A2AMX_HOME`) reporting not
   running before the removal. Judge: architect (the architect cannot see the developer's process
   namespace, so no `pgrep` check is possible).
5. `rg -n "getting-started" README.md` matches both the quick start and the documentation list.
6. Every sample in `docs/getting-started.md` and the new `docs/confinement.md` sections appears in the
   developer's captured output files (named in the report). Judge: the tester reruns S1 to S5 in its own
   scratch under `target/scratch/3cj-tester/` and compares; any difference other than the shortened paths
   is a finding.
7. `rg -n "/home/zbalint|scratchpad|claude-1000" docs/getting-started.md docs/confinement.md README.md`
   matches nothing (no personal path leaks into the public docs). Judge: command output.
