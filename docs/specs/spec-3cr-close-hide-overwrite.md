# Spec 3cr: A writable path the tool picked cannot open a hidden path

## 0. Status

LOCKED.

Scope (files that may be edited): `src/confine.rs` (three changes below), `tests/confine.rs`,
`tests/auto_confine.rs` (fixtures of section 4 and the new tests of section 5 only), `docs/confinement.md`
(one troubleshooting row, one limits sentence). Does not touch: every other file under `src/` and `tests/`
(`tests/confine_show.rs` included), `CHANGELOG.md` and `docs/backlog.md` (the architect updates both),
`AGENTS.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-escape-audit-2026-11-06`. Public test seams: `a2amx::confine::plan`, `plan_with_path` and
`git_common_dir`.

## 1. Why

Backlog F71 (escape audit) and F61. A confined session can reach things the plan hides, with ordinary
tools, in two steps across a restart. Verified 2026-11-06 with an isolated daemon: a project whose `.git`
file reads `gitdir: <HOME>/.ssh` made a default confined session (no `ssh` grant) see `~/.ssh` and create a
file in it on the host. The agent can write that `.git` file itself in its working directory and restart.
(The probe ran on the code before this spec, with a plain `.ssh`. The shape check of section 4 closes the
plain case: the pointer is not followed and `~/.ssh` stays hidden. Section 2 closes the case where the
target looks like a git directory, for example a versioned `~/.ssh` or a `pass` store with a git
repository.)

Root cause (read in the tree): a path the tool picks, not the user (the working directory, the git common
directory that `bind_workspace` binds writable, and the harness data targets `add_harness_data_mounts`
binds writable), goes through `check_write_allowed` with `explicit = false`, which refuses only a path that
equals or contains `$HOME` (`src/confine.rs:296`). A path inside home, or inside a default hide, passes.
`mounts()` then lets a writable entry overwrite a hidden entry for the same path (the map insert at
`src/confine.rs:1797-1807`), so the hide loses. Harness data has the same gap through a planted symlink
(for example `ln -s ~/.ssh ~/.claude/todos`), and `bind_harness_config` keeps a data path whose canonical
target lies outside the harness root writable even in the default mode (`src/confine.rs:1319-1333`).
A target that is not a hide but runs later (`~/.bashrc`, `~/.config`, `~/.gitconfig`) is reached the same
way, so a hide check alone is not enough.

Owner position (2026-11-03, and 2026-11-06 for the answers below): close the cheap escapes, disclose the
rest; the line is decided from `docs/philosophy.md`. The line this spec implements: what one or two ordinary
commands in the session reach on the next start is closed; what needs a crafted binary or a kernel exploit
is out of scope; harness config that runs code outside, and project files the user runs unconfined later,
are disclosed, not restricted (a separate docs spec, F73).

Rejected: skipping instead of refusing a hidden implicit writable path (a skipped git directory leaves the
repository silently read-only and a skipped harness data directory breaks the harness in a way nobody can
trace; a refusal names the fix, philosophy rule 7); a second check in each entry point (one place, coding
rule 24); restricting harness config (high workflow cost, disclosed instead); path-by-path deny lists for
`~/.bashrc` and `~/.config` (containment of the harness root and the git directory shape closes the class);
applying the git shape check to the `.git` directory branch too (the session controls the content of its own
`.git` directory, so a failing shape check there would switch off `protect_git_metadata`, which runs only
when `git_common_dir` returns `Some`, `src/confine.rs:1258-1281`; hooks and config would lose read-only
protection and run unconfined later).

Trust boundary. Inputs from a less trusted source: the content of a `.git` file and `commondir` (the agent
writes them), and symlinks inside the harness data directories (the agent plants them). Paths and
dispositions: (1) a pointer or common directory that resolves inside a hidden path: removed by the refusal
(section 2); (2) a pointer to a directory that is not a git directory (`~/.config`, a plain `~/.ssh`):
removed by the shape check on the `.git` file branch (section 4); (3) a harness data symlink to a hidden
path or to a path outside its harness location: removed by section 3 (it runs first, in
`validate_harness_data_paths`); section 2 does not check harness data (Amendment 1: a link into a hide
inside its own harness root, such as `~/.claude/todos` linked to `~/.claude/ide`, passes section 3, and
`bind_harness_config` then drops that entry from `self.writable` in favour of the root, lines 1319-1333,
so the hide stays intact and no overwrite happens); (4) a harness data symlink to a
read-only-bound target that runs later: removed by section 3; (5) a working directory inside a hidden path:
removed by section 2 (the user started it there, so the refusal tells them the `write` line that opens it);
(6) harness config and project files that run later outside the sandbox: not removable without breaking the
harness and the work; monitored by disclosure.

## 2. `src/confine.rs`: one refusal in `PlanBuilder::finish`

Add a private method `check_implicit_writes_not_hidden(&self) -> Result<()>` and call it in `finish` right
after the existing `harness-config` and grant conflict checks (the loop ending at line 1677, before
`protected_paths`). It refuses the plan when a path that one of the two binders added to `self.writable`,
and that is not in `self.explicit_writes`, equals or lies inside (`is_within`, `src/confine.rs:257`) a path
in `self.default_hidden`, `self.mandatory_hidden` or `self.container_hidden`. The two sources are the
working directory and the git common directory. Harness data targets are covered by section 3 alone
(Amendment 1). Grant paths, harness data targets and the harness-config root are also in `self.writable`
and are excluded: grant paths are checked at lines 1508-1535. Operator `hide` lines
(`self.hidden`) are not in the protected set, so a deliberately hidden harness root keeps working.

One error for the whole plan: the method collects every offender and reports them together, one clause per
offender joined with `; `, so that a project inside a hide (the working directory and its `.git` directory
both offend) is told every `write` line in one round. Each clause follows the style at lines 1517-1521:
`working directory {p} resolves to the hidden path {h}; open it with write = ["{p}"] if that is intended`,
`git directory {p} resolves to the hidden path {h}; ...`. The developer decides how the method knows the
source of a path (for example the two binders record the source in a small private struct); the texts are
the contract.

The check runs after the hides are final (default hides, grant unhides and explicit writes have pruned
`default_hidden`, lines 1540, 1611, 1635), so an explicit `write` or `unhide` line, a grant that unhides a
path, and `--confine-default-hides=false` (empty `default_hidden`) all pass without a special case. Note
that `write = ["{p}"]` for a working directory prunes only an exact match of the hide (line 1611); the
developer verifies with a test that the printed line really opens the plan for a project inside a hide
(working directory inside `~/.password-store`, with the `.git` directory's line also given) and reports a
mismatch as BLOCKED.

## 3. `src/confine.rs`: harness data stays where it belongs

In `validate_harness_data_paths` (line 588): for each harness data directory and data file that exists,
compute the canonical target and require it to be at the location that data path expects:

- a path under a harness root (everything in `harness_data` today except the Claude home file): the
  canonical target must lie inside the canonical harness root it belongs to (`~/.claude` for Claude; for
  OMP the roots `~/.omp` and `~/.omp/agent` that the function already knows, line 596 and neighbours). The
  root is canonicalized first, so a `~/.claude` that is itself a symlink to a dotfiles repository works and
  every target is compared with the canonical root;
- the Claude home file `~/.claude.json` (`src/confine.rs:434-439`) is not under a root: its canonical target
  must be a file whose parent is the canonical home directory.

The developer lists every entry of `harness_data` with its expected location in the plan and reports any
entry this rule does not cover. A target outside its expected location is refused with `harness data path
{source} resolves to {target}, outside {root}; remove the link, keep the data inside {root}, or open it
with write = ["{target}"] if that is intended` naming the symlink source (`{root}` is the canonical home
for the home file). The refusal is lifted when the canonical target is in `self.explicit_writes`: the user
named it, so it is theirs (philosophy rules 2 and 4); the agent cannot edit the request, and the team file
is protected (lines 1383-1389). A data path that does not exist or whose target is at its expected location
is unchanged. Walking up for `$HOME` and the state directory stays as it is.

## 4. `src/confine.rs`: a git pointer must lead to a git directory

In `git_common_dir` (line 825), on the branch that follows a `.git` FILE (the `gitdir:` pointer, with its
`commondir` file), after the final `canonicalize` return `None` unless `<common>/HEAD` is a regular file
(`fs::metadata().is_file()`, as for `commondir`) and `<common>/objects` is a directory. The branch for a
`.git` DIRECTORY is unchanged: the session controls its content, and returning `None` there would switch
off `protect_git_metadata` (section 1, rejected alternatives). Because `git_common_dir` is the one resolver
used by the plan and by auto detection (spec 3cs), both follow.

Fixtures that build a fake git directory behind a pointer need a `HEAD` file and an `objects` directory
added. The consultant listed them; the developer re-verifies each by running the test and reports the final
list:
- Pointer targets: `tests/auto_confine.rs` `write_worktree_git_file` (about lines 56-66, used at about 69
  and 227), the submodule fixture (about 126), the relative `gitdir:` fixture (about 1221);
  `tests/confine.rs` about 3410-3420 and 3432-3437 (`git_worktree_and_submodule_common_dirs_are_bound`).
- Tests that would go vacuous (green but no longer exercising their guard) unless their target is
  git-shaped: `tests/auto_confine.rs` about 999, 1032, 1062, 1093 (oversized, directory, symlink-to-directory
  and FIFO `config`) and 1279. After the fixture change each of these must still reach the guard it
  names; the developer shows that with a mutant that removes the guard.
- Already `None` or the directory branch, unaffected: `tests/confine.rs` 2115, 3257, 3370, 3463, 3488,
  3500, 3517; `tests/auto_confine.rs` 180, 286, 733, 886, 915, 920, 977, 1146, 1184, 1255, 1285, 1305.

## 5. Tests (`tests/confine.rs`, through `plan`, `plan_with_path` and `git_common_dir`)

Written first, with a temp home and the fixture names the file already uses; each red test fails on the
current code because the plan is `Ok` there with the path writable (or, for section 4, because the
resolver returns `Some`).

Red:
1. A git-shaped directory inside a default hide: `<home>/.ssh` with a `HEAD` file and an `objects`
   directory (a versioned `~/.ssh`). A project `.git` file `gitdir: <home>/.ssh` gives `Err` containing
   `resolves to the hidden path`, and the same through a `commondir` file.
2. The Claude harness with `~/.claude/todos` a symlink to `<home>/.ssh` gives `Err` containing `harness
   data path` and `outside`; `~/.claude/history.jsonl` a symlink to a hidden file (`~/.netrc`, created)
   does too. Contract test (Amendment 1, green on old and new code): `~/.claude/todos` a symlink to
   `<home>/.claude/ide` (a directory, created) gives `Ok`, and the hide of `~/.claude/ide` is still in
   effect in the plan.
3. `cwd = <home>/.password-store/sub` (a default hide) gives `Err` containing `working directory`; with
   `cwd` inside the store and a `.git` directory there too, one error names both clauses.
4. `~/.claude/todos` a symlink to `<home>/.config` (not hidden) gives `Err` containing `outside`.
5. `git_common_dir` on a `.git` file naming a directory without `HEAD` returns `None`; one with `HEAD` but
   no `objects` returns `None`; a `.git` file naming `<home>/.config` returns `None`; a plain
   `<home>/.ssh` (no `HEAD`) returns `None` and the plan keeps `~/.ssh` hidden and unchanged.
6. A `HEAD` that is a directory, or a FIFO, returns `None` without blocking (thread with a 5 s timeout, as
   in the existing bounded-read tests).

Contract (green on old and new code, named as such): the git-shaped `.ssh` pointer plus `write =
["<home>/.ssh"]` is `Ok`; an operator hide of `~/.claude` with the data dirs present is `Ok`; a data
directory symlinked to a directory inside the harness root is `Ok`; a `~/.claude` that is a symlink to a
directory elsewhere (dotfiles) with real data inside is `Ok`; an existing `~/.claude.json` regular file is
`Ok`; a `~/.claude/todos` symlink to `<home>/.config` plus `write = ["<home>/.config"]` is `Ok`;
`no_default_hides = true` with the git-shaped pointer is `Ok`; a `.git` DIRECTORY without `HEAD` and
`objects` still gets `protect_git_metadata`; the existing tests
`git_worktree_and_submodule_common_dirs_are_bound` and `git_common_directory_and_exact_writes_override_protection`
keep passing once their pointer fixtures carry `HEAD` and `objects`.

## 6. Docs

`docs/confinement.md`: one troubleshooting row (symptom: the session is refused with `resolves to the hidden
path` or `harness data path`; cause: the working directory or git directory is inside a default hide, for
example a `pass` store or a versioned `~/.ssh`, or a harness data directory is a link that leaves its
harness folder; fix: add the `write` lines the message prints), and one sentence in the limits discussion
that the plan refuses paths it picks that sit inside a hide and keeps harness data and a git pointer inside
their expected places (no promise beyond the tests, philosophy rule 10).

## 7. Out of scope

Disclosure of the implicit `harness-config` grant and the docs line on project files (a separate docs and
escape-text spec, F73); the omp quota probe (F56); shell startup files; hides that exist only at session
start; the planner's read of a FIFO `config` (F77); the user-facing git-config blocker (F76); changing
`protect_git_metadata`; renaming anything; `tests/confine_show.rs`.

## 8. Acceptance

1. `cargo test --test confine` and `cargo test --test auto_confine`, then `cargo test`, `cargo clippy
   --all-targets -- -D warnings`, `cargo fmt --check`. `TMPDIR` unset.
2. Mutants (developer, restored by checksum; each killed by a named test of section 5): delete the call
   in `finish`; drop `self.default_hidden` from the protected set; make the check skip the writable entry
   that came from the git directory; make it skip the working directory; drop the root containment in
   section 3; drop the `~/.claude.json` expectation (the contract test with an existing `~/.claude.json`
   must fail); drop the `explicit_writes` lift in section 3; drop the `HEAD` requirement; drop the
   `objects` requirement; apply the shape check to the `.git` directory branch too (the contract test
   `.git` DIRECTORY without `HEAD` and `objects` must fail). The `mandatory_hidden` and `container_hidden`
   members of the protected set are defensive: every entry of the first is already refused by
   `check_write_allowed`, and every entry of the second is a socket file that neither source can
   name. They get no mutant; the developer states that in the report.
3. A hand probe by the tester with an isolated daemon (temp home, `target/debug` binary, scratch under
   `target/scratch`, deleted afterwards): (a) a `.git` pointer to a git-shaped `$HOME/.ssh` (with `HEAD`
   and `objects`) is refused with the message and `~/.ssh` is untouched; (b) a `.git` pointer to a plain
   `$HOME/.ssh` starts the session with `~/.ssh` hidden and no file can be created in it.
4. `git status --short` lists only the scope files.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5; developer and tester OMP with
`openai-codex/gpt-5.6-luna` (the advisor slot model was not exposed to either session). BLOCKED/amend
cycles: 1 (Amendment 1).

| Found at | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant, Opus 5.5 | The shape check made red test 1 and the hand probe impossible: a plain `~/.ssh` has no `HEAD`, so the pointer was ignored and the plan was `Ok` | Red test 1 uses a git-shaped `.ssh`; the probe covers the git-shaped and the plain case |
| Draft review | consultant, Opus 5.5 | Root containment would refuse `~/.claude.json`, which lies outside `~/.claude`, for every Claude user; the spec said all data paths are under the roots | Section 3 gives each data path its expected location and a contract test with an existing `~/.claude.json` |
| Draft review | consultant, Opus 5.5 | The shape check on the `.git` directory branch let the session switch off `protect_git_metadata` | The check applies to the `.git` file pointer only |
| Draft review | consultant, Opus 5.5 | Two required mutants (`mandatory_hidden`, `container_hidden`) cannot be killed | Labelled defensive, no mutant |
| Draft review | consultant, Opus 5.5 | Scope list missed `tests/auto_confine.rs`; the fixture list was incomplete; "no write-line escape" for harness data blocked legitimate links; a project inside a hide took two refusal rounds; grant paths and the harness-config root were unaddressed in the check | Scope and fixture list fixed; explicit `write` lifts the harness data refusal; one aggregated error; the excluded sources are named |
| Implementation | developer, gpt-5.6-luna (BLOCKED) | The spec said `~/.claude/ide` is a harness data path reachable by the section 2 clause; `harness_data` does not list it. The claim came from the consultant and the architect adopted it without reading the list | Amendment 1: section 2 has two sources, harness data is covered by section 3 alone, and a contract test pins the `todos` to `ide` link |
| Architect review | architect, Sonnet 5.5 | `git_common_dir` lost its final `canonicalize`, so a worktree `commondir` of `../..` came back non-canonical and the home check could be bypassed; 898 tests and ten mutants did not notice | Canonicalize restored, red test and mutant added |
| Architect review | architect, Sonnet 5.5 | The harness data error text was built three times; the symlinked-home case for `~/.claude.json` had no evidence | One closure; a symlinked-home test |
| Architect review | architect, Sonnet 5.5 (own mutant) | `objects` as a regular file was accepted by the shape check and no test failed | Test case added, mutant killed |
| Tester pass | tester, gpt-5.6-luna | No material finding; one own mutant survived as equivalent (skipping the `explicit_writes` test in the implicit check, because explicit writes already prune the hide) | Recorded, no change |
