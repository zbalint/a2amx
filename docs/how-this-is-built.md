# How A2AMX is built

A2AMX is an experiment: can AI agents, working under human direction, produce and maintain a
real project as carefully as a human team would? This page says how the work is organised, so
that you can judge the code for yourself.

## What is and is not human

- **All code, tests and documentation are written by AI agents.** No human writes or
  hand-edits them. The exceptions are the GitHub-generated initial commit (`.gitignore`,
  license, a two-line README) and the OpenSSF Scorecard workflow, which the owner added
  through GitHub from the OpenSSF template. Nearly every commit carries a `Co-Authored-By`
  trailer naming the committing agent's model. Every commit's author is the owner's git
  identity, because the agents commit through it, and the trailer names the model that
  committed. The developer's model, which wrote the code, is not recorded in the commit.
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
| Tester | Independently checks the diff against the specification and runs fresh checks | Change production code, commit or push |
| Consultant | Answers one bounded question from the architect, such as a pre-lock specification review | Write code, specifications or files |

The roles run on different models, from more than one provider, so the implementer and its
checker are not the same model. As of 2026-10-16:

| Role | Model |
| --- | --- |
| Architect | Claude Sonnet 5.5 |
| Consultant | Claude Opus 5.5 |
| Developer | GPT-6.1 (plans), GPT-5.6 (works and is watched by its advisor) |
| Tester | GPT-6.1 (plans), GLM-5.3 (works and is watched by its advisor) |

The OMP-based roles plan with one model, do the work with a smaller one, and have an advisor
model watch each step.

## How a change moves

1. **Specification.** The architect writes a numbered specification in
   [`docs/specs/`](specs/): why, exact changes per file, what is out of scope, and acceptance
   commands. Since spec 3ak it is not locked until a checklist has been run against the current code
   and the consultant has reviewed it.
2. **Implementation.** The developer plans from the repository, implements exactly the locked
   scope, and leaves the work uncommitted. If the specification contradicts the code or
   itself, the developer stops and reports it. The architect fixes the specification with a
   numbered amendment before work resumes.
3. **Independent check.** The tester inspects the diff against the specification and runs
   fresh checks. The architect then reads the real diff, runs the checks again, and only then
   commits.
4. **Checks.** Nothing blocks a push to `develop` by itself, and the project has no pull-request
   flow. The architect runs `scripts/prepush-sonar.sh` by hand before pushing: the full test
   suite under coverage and a SonarQube scan that blocks on a failing quality gate. After the
   push, CI runs `cargo fmt`, `cargo clippy -D warnings` and `cargo test` on every push, a
   build on the declared minimum Rust version, secret, dependency and workflow checks, and a
   SonarQube analysis for `develop` and `master`. The Sonar result in CI is reported on
   SonarCloud and does not fail the CI run. [OpenSSF Scorecard](https://securityscorecards.dev)
   tracks the repository's supply-chain hygiene.
5. **Record.** Specifications, amendments, backlog items and commits stay in the repository.
   The reviews themselves (the consultant's and the tester's reports) are in the agents'
   message logs and are not published.

## An example

Spec [3al](specs/spec-3al-sonar-open-issues.md) cleared the open Sonar findings. Before it was
locked, the consultant's review found six errors in the draft, mostly wrong line numbers
and checks, among them a check that could never pass and a wrong test reference. After it
was locked, the developer stopped on one more: the specification told it to call a function
that is not in scope where it was to be used. The architect and the consultant, both Claude
models, had missed it; the developer, on a different model, caught it. The architect
confirmed it, added Amendment 1 (in the spec), and the work resumed. The tester then re-ran
the full suite, `clippy`, `fmt` and the extension tests five times without a finding. The
Sonar gate on `develop` read OK with no open issues after the push. The consultant's and
tester's reports are not in the repository.

## What this does not give you

- It does not show that agent-written code is as reliable as human-written code. That is the
  question the experiment asks, and one project cannot answer it.
- Agents share blind spots, and models from the same family share them most. A specification
  can pass both the architect and the consultant and still be wrong, as the example shows.
- The checks find what they are written to find. Tests, static analysis and agent review are
  not a proof of correctness or a security audit.

Report problems you find through [CONTRIBUTING.md](../CONTRIBUTING.md), or privately through
[SECURITY.md](../SECURITY.md) if they affect security.
