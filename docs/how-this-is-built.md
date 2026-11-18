# How A2AMX is built

A2AMX is an experiment: can AI agents, working under human direction, produce and maintain a
real project as carefully as a human team would? This page says how the work is organised, so
that you can judge the code for yourself.

The idea behind it is that quality comes from the engineering around the code, not from who
types it. People write bad code, skip tests, cut corners and let static-analysis findings pile
up too, and teams defend against that with practices. A2AMX asks its agents to follow the same
practices a good human team does:

| Practice in a human team | How it is done here | Where to look |
| --- | --- | --- |
| Design review before coding | A consultant reviews each specification before it is locked | [`docs/specs/`](specs/) |
| Independent QA | A tester, on a different model than the developer, checks non-trivial diffs; for behavior changes it also breaks the code on purpose to see whether a test fails (a mutation pass), and a pure code move is checked by equivalence checks instead | review records in [`docs/specs/`](specs/), CI |
| Speaking up when the plan is wrong | A developer who finds the specification contradicts the code must stop and report it; any agent may refuse a request that breaks the owner's rules | spec amendments, e.g. Amendment 1 of [3al](specs/spec-3al-sonar-open-issues.md) |
| Hiring the right person for each role | Each role has its own model; the reasoning is in [the roles section](#the-roles) | the roles section |
| Test-first development | The convention in `AGENTS.md` | [`AGENTS.md`](../AGENTS.md), `tests/` |
| Static-analysis gate | `clippy`, `rustfmt`, SonarQube, repository Semgrep rules, an advisory CodeScene code-health report and a blocking pre-push scan | CI, `scripts/prepush-sonar.sh` |
| Institutional memory | Specs, `AGENTS.md`, the backlog, the review record in each spec and a private persistent memory | the repository |
| Measuring the process | Every accepted spec records the problems each review stage found | [What the reviews found](#what-the-reviews-found) |
| Someone accountable for direction | The human owner | [What is and is not human](#what-is-and-is-not-human) |

The one real difference is memory. A person keeps years of context in their head, and an agent
session starts empty, so the project writes the context down instead.

## What is and is not human

- **All code, tests and documentation are written by AI agents.** No human writes or
  hand-edits them. The exceptions are the GitHub-generated initial commit (`.gitignore`,
  license, a two-line README), the OpenSSF Scorecard workflow, and Dependabot update commits
  merged by maintainers. Nearly every commit carries a `Co-Authored-By` trailer naming the
  committing agent's model. Agents commit through the owner's git identity, with two exceptions:
  one Scorecard-workflow commit under another author name and the Dependabot commits. The
  developer's model, which wrote the code, is not in the commit trailer; the review record in the accepted spec names it.
- **A human sets the direction.** The owner decides what A2AMX should do, how it should work,
  what it should support, and what is released. The rules require the owner's explicit
  approval for any change to `master`; this is a project rule, not branch protection.
- **A human does not read every change line by line.** Changes are reviewed by agents in
  separate roles, not by the owner.
- **There has been no independent security audit.** The areas the project cares most about
  are listed in [SECURITY.md](../SECURITY.md).

## The roles

Each role is a separate agent session with its own address, and the agent that implements a
change never reviews it. A non-trivial change goes through a specification, a developer, an
independent tester and the architect's review, and the consultant reviews specifications
before they are locked. The architect makes trivial changes (a typo, a badge, a config value)
directly, without a specification.

| Role | Does | May not |
| --- | --- | --- |
| Architect | Writes and locks the specification, assigns the work, reviews the result, commits and pushes accepted work to `develop` | Implement non-trivial changes itself, or change `master` without the owner's explicit approval |
| Developer | Implements one locked specification | Commit, merge or push |
| Tester | Independently checks the diff against the specification and runs fresh checks | Change production code permanently, commit or push |
| Consultant | Answers one bounded question from the architect, such as a pre-lock specification review or a design read of a refactor | Write code, specifications or files (it has no file-editing tools) |

Three working rules about the sessions themselves (a description of practice, not a rule
stated elsewhere in the repository; see [the lessons](#failure-modes-and-lessons)):

- **A fresh session per assignment.** The architect resets the developer, tester and
  consultant before each assignment, so one specification's context cannot leak into the next. A
  reset can be refused (a busy session, or a draft left in its input), and then the assignment waits.
- **One developer or tester assignment at a time.** Several agents running test suites at once
  were found to overload the machine. Consultants only read, so they are exempt.
- **Developer tiers.** The team can have a senior and a junior developer, both in the developer
  role. The harder specs (risk and open design, not size) go to the senior one. When one
  developer's quota is used up, the other takes any spec.

The roles run on different models, from more than one provider, so the implementer and its
checker are not the same model. As configured on 2026-11-18 (the agents' own reports of their
models have not always matched the configuration, which the review records note; the table
changed since 2026-10-16, when the tester ran GLM-5.3 and there was one developer):

| Role | Harness | Model |
| --- | --- | --- |
| Architect | Claude Code | Claude Sonnet 5.5 |
| Consultant | Claude Code (read-only: it may not edit files) | Claude Opus 5.5 |
| Senior developer | Antigravity CLI (`agy`) | Gemini 4 Hydrogen (medium) |
| Junior developer | OMP | GPT-6.1 (plans), GPT-5.6 (works and is watched by its advisor) |
| Tester | OMP | GPT-6.1 (plans), DeepSeek V4.1 Flash (works and is its own advisor) |

The OMP-based roles plan with one model, do the work with a smaller one, and have an advisor
model watch each step.

The choices follow two assumptions of the owner's, which the project has not measured:

- **Different models tend to make different mistakes**, so a checker on another model is more
  likely to catch what the implementer's model would miss.
- **Planning, architecture and specification review need the stronger models.** Following a
  strict, locked specification step by step is work a smaller model does about as well, so the
  developer and tester execute on smaller ones.

## How a change moves

1. **Specification.** The architect writes a numbered specification in
   [`docs/specs/`](specs/): why, exact changes per file, what is out of scope, and acceptance
   commands. Since spec 3ak it is not locked until a checklist has been run against the current code
   and the consultant has reviewed it. A refactor spec states the target shape (each new
   function or module, its one responsibility), and the consultant attacks that shape before the
   lock. Specs for mechanical refactors are partly generated by scripts that also check the
   result, which is how the line ranges and counts are proved; see
   [the lessons](#failure-modes-and-lessons) for where such scripts fell short.
2. **Implementation.** The developer plans from the repository, implements exactly the locked
   scope, and leaves the work uncommitted. If the specification contradicts the code or
   itself, the developer stops and reports it. The architect fixes the specification with a
   numbered amendment before work resumes.
3. **Independent check.** The tester inspects the diff against the specification, rebuilds the
   change independently where the spec allows it, runs fresh checks and, for behavior changes, a
   mutation pass. The architect then reads the real diff, runs the checks again, and only then
   commits. For a refactor the architect usually also asks the consultant for a design read of
   the diff; the review record says when it did not and why.
4. **Checks.** Nothing blocks a push to `develop` by itself. The project's own pull-request flow
   is limited to Dependabot update pull requests, which maintainers merge. The architect runs
   `scripts/prepush-fast.sh` (secrets, dependency policy, workflow audit, the repository's Semgrep
   rules and an advisory CodeScene report) and then `scripts/prepush-sonar.sh` by hand before
   pushing; the second runs the full test suite under coverage and a
   SonarQube scan that blocks on a failing quality gate. After the push, CI runs `cargo fmt`,
   `cargo clippy -D warnings` and `cargo test` on every push, a build on the declared minimum Rust
   version, secret, dependency and workflow checks, and the SonarQube scan only on `master`,
   `develop`, and pull requests from the same repository. A failing SonarQube quality gate fails
   those CI runs. CI runs after the push, so it reports a problem; it does not stop a push.
   [OpenSSF Scorecard](https://securityscorecards.dev) tracks the repository's supply-chain hygiene.
5. **Record.** Specifications, amendments, backlog items and commits stay in the repository.
   Each accepted spec ends with a `## Review record`: the models used, the number of
   BLOCKED and amend cycles, and one row per problem found after the first draft (where, by whom,
   what, how it was resolved). The consultant's and the tester's full reports are in the agents'
   message logs and are not published.

## How the agents keep context and quality

An agent session starts with no memory of the last one, so the project writes things down.

- **Written context.** [`AGENTS.md`](../AGENTS.md) holds the conventions every session reads. The
  specifications in [`docs/specs/`](specs/) record why each change was made, and
  The pages under `docs/` describe the architecture and behaviour. The backlog records what is known to
  be missing or wrong.
- **Persistent memory.** The agents also keep a memory of decisions, resolved bugs and
  constraints across sessions, so a lesson learned in one session reaches the next. That memory
  is private working state and is not part of this repository.
- **Test first.** The convention in `AGENTS.md` is to write the failing test first and then the
  minimum code to pass it, through public interfaces. Refactors get tests that pin the current
  behaviour before the code changes. The convention is stated and the history is open to
  inspection; this page does not claim every commit followed it.
- **Documentation.** The agents write the documentation alongside the code. The README
  rewrite (spec 3ak) was checked statement by statement against the code, and the checks found
  and fixed three false statements carried over from the old README.
- **Static analysis.** `clippy` with warnings denied, `rustfmt`, SonarQube, `cargo-deny`,
  `gitleaks`, workflow linting and the repository's own Semgrep rules run against the code. CodeScene
  reports code-health regressions as advice only; it does not block a push.

## An example

Spec [3al](specs/spec-3al-sonar-open-issues.md) cleared the open Sonar findings. Before it was
locked, the consultant's review found seven problems in the draft, among them a check that could
never pass, a wrong test reference, a wrong caller count and a mutation check that proved nothing. After it
was locked, the developer stopped on one more: the specification told it to call a function
that is not in scope where it was to be used. The architect and the consultant, both Claude
models, had missed it; the developer, on a different model, caught it. The architect
confirmed it, added Amendment 1 (in the spec), and the work resumed. The tester then re-ran
the full suite, `clippy`, `fmt` and the extension tests five times without a finding. The
Sonar gate on `develop` read OK with no open issues after the push. The consultant's and
tester's full reports are not in the repository; the spec's review record summarizes them.

## What the reviews found

Every accepted spec ends with a review record (see [How a change moves](#how-a-change-moves)),
so the process can be measured and not only described. Counted by script on 2026-11-18 from the 122
specs whose record is filled in (the tables changed layout over time, so read the numbers as approximate):

- **No problem found, or fixed before it mattered.** Of the 112 records whose cycle count the script could read, 70 needed no
  BLOCKED or amend cycle, 29 needed one and 13 needed two or more (at most four); a second count
  of 120 records gave 75, 32 and 13, so about 60%, 26% and 12%.
- **Where the problems were found.** About 760 problem rows are recorded, meaning a change to the spec or
  the diff, or a claim in the spec that turned out false. Roughly 41% were found in the consultant's pre-lock
  review of the draft, 23% by the tester, 16% by the architect's own review, 13% during
  implementation and 2% only after the push (CI or Sonar). The rest came from design reads, owner
  questions and similar one-off stages.
- **What that does and does not show.** Most problems were caught before any code existed, which is the
  cheapest place. It does not show that the later stages could be dropped: the process says
  explicitly not to drop a check without a measured run of specs in which it found nothing, and
  nobody has run that experiment. These numbers count stages, not models, so they do not test the
  assumption that different models make different mistakes.

## Failure modes and lessons

These are the ways the process failed, taken from the review records, the backlog and the
specs themselves, each with the change it caused.

| Failure mode | What happened | What changed |
| --- | --- | --- |
| A draft states facts nobody checked | Wrong line numbers, wrong counts, a test name that does not exist, an acceptance command that could never pass: the draft review is where the most problems were found. Spec [3al](specs/spec-3al-sonar-open-issues.md) had six such errors before it was locked | The pre-lock checklist, run against the current code, and the consultant's review of every spec |
| A script proves the wrong thing | A script that listed the tests of a file counted a `{` inside a string literal as a brace and missed the last test ([3fk](specs/spec-3fk-discord-tests-split.md)). A "same lines, sorted" comparison cannot see a line in the wrong file or position, a limit that [3fp](specs/spec-3fp-session-src-split.md) states and designs around | Scripts assert their own counts (attributes equal items); a split is checked by an ordered rebuild and by structural counts as well as by the sorted comparison |
| The architect and consultant share a blind spot | Both are Claude models. A specification told the developer to call a function that was not in scope; the developer, on another model, caught it ([3al](specs/spec-3al-sonar-open-issues.md), Amendment 1) | A developer and a tester on other models and harnesses; no reviewer is the author's own role |
| Metrics counted by hand | A hand count of cognitive complexity was 30 where Sonar said 31 ([3ay](specs/spec-3ay-confine-plan-complexity.md)), and later complexity specs again passed functions that nobody had counted | Nothing yet: no local complexity tool exists for Rust (backlog F25, open), so a hand count keeps a margin and Sonar is the judge after the push |
| A test that depends on the machine | A test hard-coded WSL-only paths and passed locally and failed in CI ([3ec](specs/spec-3ec-codex-confine.md)); full runs also exposed timing-sensitive tests that fail under load (backlog I18, I27 and I28) | Flaky tests go in the backlog with what to do on a failure (rerun once, record it as pre-existing); a spec lists the known flakes by checking the backlog, because one spec said there were none when the backlog had two |
| A tool's number depends on something unexpected | `cs check` scored identical bytes 7.24 under one path and 6.41 under a directory named like "test" ([3fk](specs/spec-3fk-discord-tests-split.md)), which explained a gap between the developer's and the tester's scores for the same old file | Later specs tell the tester to save the old copy under a path without "test" in its name, and a byte move is judged by its equivalence checks, not by a score |
| A spec makes the developer delete the evidence | The spec told the developer to delete its scratch directory before reporting, and the saved test list the tester needed to rerun the check was in it ([3fh](specs/spec-3fh-presence-tests-split.md)) | Later specs say to keep the scratch directory until the architect accepts |
| Too much context, or too many sessions at once | A long-lived session carries every earlier spec's context into the next; several agents running test suites at once overloaded the machine | A reset before every assignment, and one developer or tester assignment at a time |
| An agent's report of itself is wrong | Agents have reported models that did not match the configuration, and a report that says "no uncertainties" is not evidence | The review records state the configured models; the architect reruns every check itself and treats a report as a pointer |

Lessons that held up:

- **Give every claim a judge outside the author.** A criterion is a command, a test or a named
  reader, and a criterion that cannot fail is not a check.
- **Count with tools, not by reading.** Line numbers, test counts and complexity are cheap to get
  wrong and cheap to measure.
- **Make the check able to see the failure it is for.** A sorted comparison proves that no line was
  lost; it cannot prove that the lines are in the right file.
- **Different model, different harness, different session** each remove a different shared blind
  spot, and none removes all of them.
- **Write it down.** A session starts empty, so a decision that is not in a spec, the backlog, a
  review record or `AGENTS.md` will be rediscovered.
- **Stop on contradiction.** The cheapest fix is an amendment to the spec before work resumes, and
  in the architect's judgment each BLOCKED cycle in the records cost less than the wrong build it prevented.
- **Record the failure where the next agent reads it.** A repeated miss becomes a change to the
  process, not a note.

Since 2026-11-16 the same process was also used for a refactoring campaign with the owner's goal that no source or
test file exceeds 1,000 lines. Specs 3ev to 3fl turned 17 test files into directories of modules that are still one test binary each, and
specs 3fm and later split the source files. Each split moves the code unchanged, proves it with a
list of tests before and after, and is reviewed like any other change. The campaign is the
process's densest sample: its review records are the best place to read the failure modes above.

## What each analyzer found

Each analyzer was added at a known date and found things the others did not. Dates come from the
git history; counts come from the specs and a fresh run on 2026-11-18.

| Analyzer | Added | What it found | What it missed |
| --- | --- | --- | --- |
| Clippy and `rustfmt` | from the start | Style and common mistakes; they gate every change | Function size, cohesion, file size |
| SonarQube Cloud | project analysis from 2026-10-14 (`aa3d4f9`); its issue history reaches back to code written on 2026-09-30 | Read from its issue history on 2026-11-18: 47 issues closed after a fix, 6 marked false positive or accepted, 1 open (minor, in a test file). The 47 were 26 closure sites (`rust:S1612`), 11 cognitive-complexity findings (`rust:S3776`, the worst at 78 and 72 against the limit of 30, in `daemon.rs` and `picker.rs`), 7 in the OMP extension (TypeScript), and 3 others. Specs 3x to 3ae and 3al cleared them. The git history adds about a dozen later commits that split a function that had grown past the limit and four that fixed blocking calls in async tests (`S7487`, `S7488`); this page did not check how those appear in the Sonar history | File size, cohesion and files full of long functions: with the gate green and one minor issue open, it saw none of what CodeScene reported below |
| Semgrep | evaluated 2026-10-16, repository rules and a blocking gate from 2026-10-26 (`7eec717`) | A scan with the community rules found only the missing Dependabot cooldown (fixed in `c531933`); the four repository rules (no shell-string commands, no Ctrl-C literal, terminal types and PTY spawning confined to one file each) guard invariants no generic rule knows | Anything that is not one of those four invariants |
| CodeScene (`cs`) | 2026-11-15, advisory (`prepush-fast.sh`) | On 2026-11-15, with the Sonar gate green, every one of 24 files over 1,000 lines scored between 2.49 and 7.80 out of 10 with warnings such as low cohesion, too many functions in one module and long functions: `src/main.rs` 2.59, `src/daemon.rs` 2.49, `src/confine.rs` 2.65 (rerun on 2026-11-18, the same) and the 7,086-line `tests/confine.rs` 3.48. This is where the file-size goal and the refactoring campaign below came from. Specs 3ev onward raised the scores of the split files (spec 3ev: 5.90 to 9.60) | It does not fail a build; it is advisory because the existing large files would fail every change |

The pattern is the point: SonarQube's quality gate on `develop` was green, with no open issue
worth a ticket, while CodeScene rated the biggest files as hard to maintain. A gate is a set of
questions, and a codebase can pass every one of them and still be hard to change. Each added
analyzer asked a new question.

## Technical debt

Technical debt grows in every project, whoever writes it, and an agent-written project can
accumulate it faster if nobody looks. A2AMX tracks it in the open:

- **Deliberate corners** are marked in the code. The convention in `AGENTS.md` is a
  `// shortcut:` comment naming the ceiling and the trigger to upgrade it. On 2026-11-18 there were 78 of them in 28 files under `src/`,
  `tests/`, `extension/` and `scripts/` (62 in 20 files on 2026-10-16).
- **Known gaps and issues** are in [`docs/backlog.md`](backlog.md): 86 open items and 89 closed
  ones on 2026-11-18 (25 and 44 on 2026-10-16), each with the evidence for how its state was learned.
- **Static analysis findings** are worked down, not left to pile up. On 2026-10-16 the SonarQube
  quality gate on `develop` was OK with no open issues, after a pass that fixed 12 findings and
  dismissed 6 as false positives. On 2026-11-18 the gate was still OK, with one open minor finding
  (in a test file).

These numbers are snapshots, not promises. They show that debt is visible and managed, not
that it is lower than in a comparable human-written project; that comparison has not been made.

## What this does not give you

- It does not show that agent-written code is as reliable as human-written code. That is the
  question the experiment asks, and one project cannot answer it.
- Reviewers share blind spots, human or agent, and models from the same family share them most. A specification
  can pass both the architect and the consultant and still be wrong, as the example shows.
- The checks find what they are written to find. Tests, static analysis and agent review are
  not a proof of correctness or a security audit.

Report problems you find through [CONTRIBUTING.md](../CONTRIBUTING.md), or privately through
[SECURITY.md](../SECURITY.md) if they affect security.
