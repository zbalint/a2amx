# Spec 3ak: README as a landing page, reference moved to docs/

## 0. Status

LOCKED.

**Scope.** Edit only: `README.md` (rewritten in full; the one place a full rewrite is intended),
`CONTRIBUTING.md`, and new files `docs/cli.md`, `docs/teams.md`, `docs/harnesses.md`,
`docs/install.md`, `docs/using-with-agents.md`, `docs/roadmap.md`. Edit `docs/architecture.md`
only for the additions §4 names, and `docs/delivery.md`, `SECURITY.md`, `docs/backlog.md` only where
the truth check (§8) proves a sentence false; list each such edit in the completion report. Does not touch: any file under
`src/`, `tests/`, `extension/`, `scripts/`, `.github/`, `Cargo.toml`, `Cargo.lock`,
`CHANGELOG.md`, `AGENTS.md`, `a2amx.toml.example`, `docs/specs/`, `docs/validation-plan.md`,
`.cargo/`, `*.lcov`.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Public test seams: none
(docs only; the checks are §9). Shared `context_id`: `a2amx-readme-refactor`.
Governing skill: `project-docs` (read it before planning).

## 1. Why

`README.md` is 404 lines and reads as a reference manual. A visitor meets a status
paragraph, a design essay and a command list before any example; the team file format, `list`
column semantics, quota polling and reset timing sit in the middle of it. It has two Contributing
sections, and its status paragraph is partly stale. The `project-docs` skill defines the
target: a landing page that answers what it is, why, and how to try it within 30 seconds, with
every other fact one link away under `docs/`.

## 2. README.md (new shape, about 90 lines)

The title is the only `#` heading. Then six `##` sections in this order, no others (§2.1's block
sits under the title, before the first `##`):

1. **Title and badges.** `# A2AMX`, then the badge block (§3), then one sentence saying what it
   does for the reader (hosts AI agent sessions, Claude Code, OMP and Codex, under PTYs and lets
   them message each other), then a one-line status (alpha; what works; what is not built:
   cross-host support and agent-initiated launch). Keep the "Not the A2A protocol" note, shortened
   to two sentences, with its link to the Agent2Agent project.
2. **The problem.** (`## The problem`; the headings below follow the same pattern.) Ordinary developer language: agents of one harness cannot message agents of
   another, a message typed into a terminal can destroy the human's half-written draft, and
   sessions die with the terminal. Then how A2AMX removes each: it hosts the sessions under
   PTYs, delivers through each harness's native channel, and protects the draft.
3. **Example.** A two-session round trip with Claude Code and OMP (owner decision): start the daemon,
   start both sessions, an agent sends a message, `a2amx list` shows the result. Commands and
   output come from a real run of the current build (§9, provenance rule).
4. **What you get.** Four to six items, each phrased as what the reader gains.
5. **Quick start.** The shortest verified route from a clone to the §2.3 example working:
   `scripts/install.sh`, then running the installed binary, then the example.
6. **Compatibility and limits.** Linux only; Rust 1.88 or newer to build; Claude Code, OMP, Codex
   have native delivery, other commands are held or opt in to blind delivery; no cross-host support;
   no agent-initiated launch; teams are not OS-level isolation.
7. **Documentation.** Links to all of: `docs/cli.md`, `docs/teams.md`, `docs/harnesses.md`,
   `docs/install.md`, `docs/using-with-agents.md`, `docs/roadmap.md`, `docs/architecture.md`,
   `docs/delivery.md`, `docs/validation-plan.md`, `docs/backlog.md`, `CONTRIBUTING.md`,
   `SECURITY.md`, `CHANGELOG.md`, `LICENSE`. Each link has a few words saying when to read it
   (this replaces the "Design documents" table at README lines 82 to 89).

The README holds no flag tables, no config reference and no per-version behavior. The opening
sentence and the example are the developer's to write from the code and a real run; do not copy the
current README's "Agent-to-Agent Message eXchange" tagline paragraph.

## 3. Badges

Exactly this set, in this order, each on its own line (GitHub renders them inline), each with an
`alt` text and a link target that resolves (§8):

1. CI, master and develop: `https://github.com/zbalint/a2amx/actions/workflows/ci.yml/badge.svg?branch=<branch>`,
   linked to `https://github.com/zbalint/a2amx/actions/workflows/ci.yml`.
2. Sonar for `develop`: quality gate (`alert_status`), coverage (`coverage`), security rating
   (`security_rating`), as `https://sonarcloud.io/api/project_badges/measure?project=zbalint_a2amx&branch=develop&metric=<metric>`,
   linked to `https://sonarcloud.io/summary/new_code?id=zbalint_a2amx&branch=develop`.
3. Rust MSRV `https://img.shields.io/badge/Rust-1.88%2B-orange?logo=rust` linked to the
   `Cargo.toml` on develop; platform `https://img.shields.io/badge/platform-Linux-blue?logo=linux`;
   license `https://img.shields.io/badge/license-AGPL--3.0--or--later-blue` linked to
   `https://spdx.org/licenses/AGPL-3.0-or-later.html`.
4. OpenSSF Scorecard, only if `https://api.scorecard.dev/projects/github.com/zbalint/a2amx/badge`
   returns an image when fetched; otherwise omit it and say so in the report.

The CI badge for master must not be a quality-gate badge. Drop the current master quality-gate badge.
No HTML comments between badges are required; a short comment per group is allowed.

## 4. New docs pages

Each page starts with a one-line statement of its kind (reference, how-to, explanation) and its reader.
The README's text moves, it is not rewritten: wording may be tightened, meaning may not change.

| Page | Kind | Receives (current README sections / line ranges) |
| --- | --- | --- |
| `docs/cli.md` | reference | the command list (lines 102 to 126); kill/exited semantics (128 to 137); `list` ACTIVITY/UPTIME/IN-STATE/HELD/QUOTA/TEAM columns and quota polling (170 to 192); prefix key, picker, scroll, mouse, status line (158 to 170); `reset` (194 to 199); `team` command semantics that are about commands, not the file (289 to 342) |
| `docs/teams.md` | how-to plus reference | `### Teams` heading through the team file format, visibility and allow rules, `reset`, `control_from`, `watch`, `heartbeat`, `role`, fallback global file (201 to 288); `team status`, `--dry-run`, `team reset` details (289 to 342) |
| `docs/harnesses.md` | explanation | `--harness` behavior for claude, omp, codex, generic; channel versus terminal delivery; hook; receipts (139 to 156) and the idea section's per-harness delivery text (34 to 57) |
| `docs/install.md` | how-to | "Dev build versus live install" (363 to 380) |
| `docs/using-with-agents.md` | how-to | "Using it with agents" (344 to 361) |
| `docs/roadmap.md` | explanation | the stale "Status" and "Design documents" milestone prose (15 to 32, 91 to 98), rewritten to be true today: what is built, what is not (G1, G3 in `docs/backlog.md`), next milestones from the backlog |

Where a block could go to either `cli.md` or `teams.md`, the developer picks one, states the choice
in the report, and links the other page. The ranges are for the current README (identical at
`418a5f3` and `10b1b71`); the README is the source of truth, not the ranges. Known overlaps the
developer resolves once, naming the choice: lines 100 to 104 (`cargo build`/`cargo test`) duplicate
CONTRIBUTING.md and are deleted and listed; lines 158 to 159 (state directory order) go to `cli.md`;
the `team` command semantics at 289 to 342 go to `teams.md` and `cli.md` links there; lines 183 to
192 and 250 to 252 repeat the visibility rule, so keep one copy in `teams.md`.

**Catch-all (move rule).** Every removed README line has exactly one fate, recorded in the report's
move table: moved to a named page and heading, deleted as a duplicate (name the existing text),
deleted as false or stale (name the evidence), or superseded by §2 (name the new sentence). The
ranges above name the main destinations; the rest:

| README lines | Fate |
| --- | --- |
| 1 to 14 (title, quality-gate badges, tagline, A2A notice) | superseded by §2.1 and §3 |
| 34 to 44 (the idea, per-harness delivery) | `docs/harnesses.md` |
| 46 to 49 (`host-a`/`host-b` example) | `docs/roadmap.md` (cross-host is proposed, not built), keeping it marked as proposed |
| 51 to 53 (cross-harness gap) | the claim becomes the README problem statement (§2.2) only if true today; the same fact is kept in `docs/harnesses.md` |
| 55 to 57 (PTY bytes are not proof of submission; a receipt is not proof of processing) | `docs/harnesses.md`, unchanged in meaning |
| 59 to 80 (Scope, plus the not-a-planner paragraph) | per the next paragraph |
| 82 to 89 (Design documents table) | §2.7 |
| 91 to 98 (milestones) | `docs/roadmap.md` |
| 382 to 394 | `CONTRIBUTING.md` (§5) |
| 396 to 404 | §2.7 |

"Scope" (59 to 80): compare each bullet and the closing paragraph (78 to 80) with the **whole** of
`docs/architecture.md`, not only its Requirements and Scope sections. A fact already stated there is
deleted from the README and named in the report as "duplicate of architecture.md, heading X". A fact
not stated there moves into `docs/architecture.md`, under its Requirements or Scope section, only if
the truth check shows it is true today (these additions are the one permitted edit to that file);
otherwise it is deleted and listed under "false or stale".

`docs/roadmap.md` lists every sentence it drops from README lines 15 to 32 and 91 to 98, with the
reason. It states that backlog G2 was dropped by the owner (see `docs/backlog.md`), so it is not carried
forward as planned, and that the prompt-submit hook safety net for Codex and OMP (README line 27) is
checked against the code and backlog before it is listed as planned.

**One permitted addition.** `docs/teams.md` gets one new sentence explaining that the old `prefix`
team-file key was renamed to `team` (the error text is `src/team.rs:406`, asserted in
`tests/team.rs:273`); its README link is in §2.7.

## 5. CONTRIBUTING.md

The README's "Contributing" section (382 to 394) moves here. Add its public-repository rules
(fictional names and hosts, no credentials or private conversations, runtime databases outside the
checkout, review of staged content) and the line asking to identify whether a suggestion changes a
requirement, a proposal or an open question. Do not duplicate content already in CONTRIBUTING.md.
The README must contain exactly one contributing pointer, under §2.7.

## 6. SECURITY.md

Verify, do not rewrite: reporting channel (§8 item 7), supported-versions sentence against
`CHANGELOG.md` and the tags (`git tag`), scope list against `docs/architecture.md` "Identity and authority".

## 7. Out of scope

Rewriting `docs/architecture.md` or `docs/delivery.md`; changing the `CHANGELOG.md`; touching
`docs/specs/`; renaming existing docs; any behavior change in `src/`; adding recordings
(asciinema or `vhs`); the `.cargo/` directory; creating or editing GitHub settings. The
`src/team.rs:406` message "see the README" stays as is (it is in `src/`, out of scope); the rename is
explained in `docs/teams.md` (§4).

## 8. Truth check (project-docs), run on every page touched plus the README quick start

Report each item as checked (command or file), failing, or not run (reason):

1. Every command in README and the new pages runs as written. Use `A2AMX_HOME` set to a temp
   directory and the freshly built binary for runs; never use the working daemon or `~/.a2amx`.
   Install commands: `A2AMX_INSTALL_DIR=<temp dir> scripts/install.sh`.
2. Every flag and default against `a2amx <cmd> --help` of that binary.
3. Every relative link and anchor in the touched pages resolves (`rg`/script, name the command).
4. Every listed capability exists on `develop` (cite file or test).
5. Every number, timeout and default stated (30 s per reset step, 1.5 s settle, 10 s `input?`,
   seven-day retention, 24 h heartbeat bound, quota poll interval) against the code (`rg`).
6. Badge URLs: fetch each image URL and each link target (`curl -sI`), record the status.
7. SECURITY's private reporting channel: not run (needs repository settings); report as such.
8. Each statement the check proves false is corrected or deleted and listed in the report under
   "false or stale"; never moved unchanged.

**Provenance (§2.3):** the example's commands and output come from a real run of the freshly built
binary in a temp `A2AMX_HOME`, with real `claude` and `omp` sessions (they use the machine's
existing logins). You run inside an a2amx session, so run every such command as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR A2AMX_HOME=<temp dir> ...`; never touch
the working daemon, `~/.a2amx` or the real state directory. There is no CLI command that types into a
session, so drive `a2amx attach` from a script under a pty (Python's `pty` module is fine; the script
stays in the scratchpad, not in the repo). Send one short prompt per session, only as many as the
example needs. If a login prompt, trust dialog or any dialog appears, stop and report it to the
architect (a2amx-architect@desktop-aavo022); do not answer it blindly and do not invent output.
Kill the temp daemon and sessions and delete the temp directory afterwards. The recorded real output
(`a2amx list` and the message as received) goes into the README, with fictional session names
(`planner`, `reviewer`) and no paths, tokens or host names from this machine.

## 9. Acceptance

The post-implementation checks (these need the new files, so they are run only after the change):

```sh
wc -l README.md                                  # at most 130
rg -n '^#{1,2} ' README.md                       # the title, then the six §2 headings in order
rg -c '^\s*(\$ )?a2amx ' README.md               # at most 12 command lines, all inside §2.3 and §2.5
env -u A2AMX_BIN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short                               # only §0 files, plus the untracked .cargo/ already present
```

Move rule check (reviewer): for each README line removed at `418a5f3`, its destination page and
heading is named in the report's move table, one row per block. Pre-existing failures: none
expected; `cargo test` must pass unchanged (docs only).
