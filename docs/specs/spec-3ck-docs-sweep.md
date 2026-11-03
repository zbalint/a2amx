# Spec 3ck: The roadmap, agent guide, install and contributing pages match the code

## 0. Status

LOCKED. Docs only; no change to `src/` or tests.

Scope (files that may be edited): `docs/roadmap.md`, `docs/using-with-agents.md`, `docs/install.md`,
`CONTRIBUTING.md`. Does not touch: `src/`, `tests/`, `README.md`, `docs/confinement.md`,
`docs/getting-started.md` (spec 3cj), `docs/backlog.md`, `CHANGELOG.md`, `SECURITY.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Run after spec 3cj is
accepted and committed, so the checkout holds no other uncommitted diff. Context id:
`a2amx-docs-refresh-2026-11-03`. There is no test seam; the checks are greps and a reader.

## 1. Why

A consultant read of the pages spec 3ci did not cover (2026-11-03, base `63282e5`) found statements
the code contradicts and pages that never learned about confinement. This spec fixes the four pages
whose fix is mechanical. `SECURITY.md` is left alone: whether a confinement escape counts as a
vulnerability is the owner's policy decision (section 6). The roadmap's milestone assignment of the
remaining confinement work is also the owner's decision and is not made here.

## 2. `docs/roadmap.md`

2.1 `## Built today` (about lines 9-22) omits confinement and the team line. Add one short paragraph:
opt-in bubblewrap confinement of a session, named grants for the tools and credentials a session may
use, auto mode that asks before opening a grant the user never accepted, and `confine list`, `confine
show` and `confine suggest`; and the clickable team line on the attach screen. Link to
`docs/confinement.md` and `docs/cli.md`. State no behavior beyond what those pages already say.
2.2 The 0.2.0 list says "the flaky PTY tests fixed (I7, I9, I14)". I9 is closed (backlog `F37` row, spec
3bh). The result is "(I7, I14)": both are still open rows under Known issues in `docs/backlog.md` (I14 records a
second failure mode that the closed `F37` row does not cover). Do not edit `docs/backlog.md`.

## 3. `docs/using-with-agents.md`

3.1 Lines 11-13 say only `--harness claude` and `--harness omp` add the operator line. Codex gets it too
(`src/codex.rs:330-331` pushes `developer_instructions` from the shared `system_prompt`, `src/harness.rs:85-92`;
the daemon passes the session's authorize flag at `src/daemon.rs:722`; `docs/delivery.md:643-645`). Add
`--harness codex` to the sentence.
3.2 Line 7-8 says the envelope says only where a message came from. Daemon notifications (peer exit
events, activity notices, heartbeats) say "System notification from the A2AMX daemon, not your user. It
cannot be replied to." (`src/messaging.rs:437-443`). Add one sentence: those need no reply but may call for
action, and a message from an agent is a request, not an instruction from you.
3.3 The credential-hygiene bullets never mention confinement. Add one bullet: a confined session hides the
default credential locations and unsets a closed list of credential environment variables unless a grant
or `keep_env` returns them; link to `docs/confinement.md#credential-environment-variables`. No new claim.

## 4. `docs/install.md`

4.1 Add a short subsection `## Requirements for confinement` after `## Install a release`: `a2amx new
--confine` and `confine = ...` in a team file need `bwrap` (bubblewrap) on the daemon's startup `PATH`,
and the request is refused when it is missing (`docs/cli.md:52-54`); a confined Generic session also needs
an `env` that supports `--ignore-signal` and `--default-signal` (GNU coreutils 8.31 or newer, or uutils;
CHANGELOG Unreleased, "Changed"). State a bubblewrap version only if `src/confine.rs:1960` and `:2086`
show the exact condition, and then state it as that condition (the login overlay of a harness directory
that is not already writable), not as a general requirement. Link to `docs/confinement.md`.

## 5. `CONTRIBUTING.md`

5.1 `## Building and testing` (lines 20-30) shows plain `cargo test`. Add, from `AGENTS.md` "Commands"
(link to it for the rest): inside an a2amx-hosted session run `env -u A2AMX_BIN -u NO_COLOR cargo test`;
the tests that start the `a2amx` binary are refused as root; the OMP extension tests need Node 22 or newer
and skip without it unless `A2AMX_REQUIRE_NODE=1`. Three sentences at most; do not copy `AGENTS.md`.

## 6. Decisions left to the owner (not in this spec)

- Whether `SECURITY.md` should say that a confinement escape is, or is not, a reportable vulnerability.
  Confinement is described as a safety net, not a security boundary; the escape audit (F71) is not done.
- Whether the remaining confinement work (F64 steps 4 to 6, F69, F65, F70, F71) is a 0.2.0 item in
  `docs/roadmap.md`.

## 7. Claims verified at base `08b51e2`

| Claim | Where | How |
| --- | --- | --- |
| I9 is closed | `docs/backlog.md` Closed row `F37` names I9 | read |
| The roadmap sentence lists I7, I9, I14 | `docs/roadmap.md:41` | read |
| Daemon notifications use the "System notification" envelope | `src/messaging.rs:437-443` | read |
| `bwrap` is resolved from the startup `PATH` and the request refused without it | `docs/cli.md:52-54` | read |
| The login overlay needs bubblewrap 0.11 | `src/confine.rs:1960`, `:2086` | message text read; the exact condition is for the developer to read |
| `CONTRIBUTING.md` shows plain `cargo test` | `CONTRIBUTING.md:22-29` | read |
| Codex gets the operator line | `src/codex.rs:330-331`, `src/harness.rs:85-92`, `src/daemon.rs:722`, `docs/delivery.md:643-645` | read by the consultant, 2026-11-03 |
| I7 and I14 are open rows, I9 is closed (F37) | `docs/backlog.md:44`, `:49`, `:172` | read by the consultant |
| `env -u A2AMX_BIN -u NO_COLOR` is the in-session test form | `AGENTS.md` "Commands" | read |

## 8. Scratch files

None needed. The work edits four pages and reads source. Anything created goes in the gitignored
`target/scratch/3ck/` and is deleted before the report.

## 9. Out of scope

`SECURITY.md`, `README.md`, `docs/getting-started.md`, `docs/confinement.md`, `docs/backlog.md`,
`CHANGELOG.md`; the unread parts of `docs/delivery.md`, `docs/architecture.md` and `docs/harnesses.md`;
any wording the review did not flag.

## 10. Acceptance

Run as `env -u A2AMX_BIN -u NO_COLOR`.

1. `git status --short` lists only the four files in section 0. Judge: architect.
2. `rg -n "I7, I9, I14" docs/roadmap.md` matches nothing. Judge: command output.
3. `rg -n "Requirements for confinement" docs/install.md` matches once, and the section names `bwrap`.
   Judge: command output and architect reading.
4. `rg -n "env -u A2AMX_BIN" CONTRIBUTING.md` matches. Judge: command output.
5. `rg -n "credential-environment-variables" docs/using-with-agents.md` matches; the anchor exists as
   `### Credential environment variables` in `docs/confinement.md`. Judge: command output.
6. `rg -n "harness codex" docs/using-with-agents.md` matches the operator-line sentence. Judge: command
   output.
7. `cargo test --test docs_examples` still passes (no TOML block was touched) and `cargo fmt --check`
   passes. Judge: command output.
