# Spec 3cs: Auto detection follows a `.git` file, and the docs say how to push without an upstream

## 0. Status

LOCKED.

Scope (files that may be edited): `src/confine.rs` (`git_common_dir`: one `pub` and two bounded
reads), `src/main.rs` (`auto_grants_for`), `tests/auto_confine.rs`, `tests/confine.rs`,
`docs/confinement.md`, `docs/teams.md`, `docs/cli.md`.
Does not touch: every other file under `src/` and `tests/`, `AGENTS.md`, `CHANGELOG.md`,
`docs/backlog.md` (the architect closes F65 and F69's docs part), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `3723fad`. Context id:
`a2amx-auto-gitdir-2026-11-05`. Public test seams: the `a2amx` binary (`confine suggest` and
`team up --dry-run`, as `tests/auto_confine.rs` already runs them through `run_binary`) and
`a2amx::confine::git_common_dir`.

## 1. Why

Backlog F65. Under global `auto`, and for `a2amx confine suggest`, the git-remote grants (`ssh`,
`gh`, `git-credentials`) come from `.git/config`, but `auto_grants_for` (`src/main.rs:751`) reads
it only when `<dir>/.git` is a directory. In a git worktree or a submodule `.git` is a file with
a `gitdir:` pointer, and a session started from a subdirectory of a repository has no `.git`
beside it, so a project that pushes over ssh or https gets only `cache` and the first `git push`
fails with a permission error. The owner decided (2026-11-05) to follow the pointer in 0.2.0, as
detection only. Worktrees are the headline of 0.3.0, and the philosophy's rule 7 (a beginner's
first run must work) says detection should cover what a freshly cloned project needs.

The confinement plan already resolves the same layout: `git_common_dir` (`src/confine.rs:823`)
walks up from the working directory to the nearest `.git`, follows a `gitdir:` pointer and a
`commondir` file, and returns the common git directory. This spec reuses it, so detection and the
writable git directory in the plan name the same directory and cannot disagree.

Also (owner decision, 2026-11-05, F69): the git-config protection stays and nothing new is added
for 0.2.0; the docs say what to do instead. The user-facing blocker is backlog F76.

Trust boundary. The content of a project's `.git` file and of the config it points to comes from the
repository, or from an archive that shipped its own `.git`. The paths this spec creates, and their
disposition: (1) the pointer names any directory, so `<target>/config` is read when it is a regular
file of at most 65,536 bytes; only grant names leave it, and a tool-picked grant still needs the
user's yes unless that grant name was accepted before (philosophy rule 4), so the path is
constrained and accepted; (2) a `config` that is a FIFO, device, directory or symlink to one gives
no grants; (3) a `.git` file or `commondir` that is huge or not a regular file could hang or fill
memory a new entry point (`confine suggest`, `team up`): removed by bounding both reads (section
2); (4) a relative or `..` project path could make the walk find an unrelated repository and skip
the home stop: removed by canonicalizing first (section 3); (5) walking up into an enclosing
repository is intended.

Rejected: a second pointer parser in `main.rs` (the logic exists and has tests); following only
`<dir>/.git` files and not parents (a session whose directory is a subdirectory of a repository is
the same user in the same position, and the plan's bind already walks up); reading `config.worktree`
(remotes live in the common `config`).

## 2. `src/confine.rs`

Make `git_common_dir` `pub` (line 823), same signature, and add a one-line doc comment: returns the
canonical common git directory and the worktree root, or `None` (no repository, a repository whose
directory is the home directory, or a pointer file that is not usable); `cwd` must be canonical.
Two reads in its body become bounded (Coding Standards rule 26), because it now runs on two more
entry points:

- The `.git` marker file (line 836) is read through `File::open(..).take(POINTER_LIMIT + 1)`; more
  than `POINTER_LIMIT` bytes (a new `const` of 4096) or invalid UTF-8 returns `None`.
- `commondir` (line 848) is checked before it is opened: `fs::metadata(path)` first (it follows
  symlinks), then `is_file()`, then `File::open(..).take(POINTER_LIMIT + 1)`. The order matters:
  `File::open` with `O_RDONLY` on a FIFO blocks until a writer appears, so checking the opened file
  is too late. A `NotFound` from `metadata` keeps today's meaning (no `commondir`: the common
  directory is the `gitdir` itself). Any other error, a non-regular file, more than `POINTER_LIMIT`
  bytes or invalid UTF-8 returns `None`.

The marker file needs no extra check: `marker.is_file()` (line 835) already runs before its read.
This also changes the confinement plan, not only for hostile input: a `commondir` that is
unreadable (permission denied), not UTF-8, longer than 4096 bytes, or not a regular file (a
directory included) used to fall back to the `gitdir` and now yields no common git directory, so the
plan binds none. No other behavior of the function changes. A `.git` that is a FIFO, socket or
device is neither a directory nor a regular file, so the walk skips it and climbs into an enclosing
repository; that is unchanged and not a regression.

## 3. `src/main.rs`

`auto_grants_for(dir, home)` (line 751) takes its git config from the common git directory:

- Canonicalize first: bind `let canonical_dir = std::fs::canonicalize(dir)`; a failure means no git
  grants but the marker detection still runs. Keep the raw `dir` for the markers (a new name for
  the canonical path, no shadowing). `home` is canonicalized when it is not empty (on failure the
  raw value is used). The confinement
  plan canonicalizes its working directory (`src/confine.rs:1027`) but `confine suggest [DIR]`
  (`src/main.rs:133-146`) and `team up` (`:803-809`) pass the path raw, and the walk in
  `git_common_dir` is lexical (`Path::parent`): a relative or `..` path reaches `""`, where
  `join(".git")` resolves against the process directory and finds an unrelated repository. A team
  file `cwd = "../x"` does the same: `origin.dir.join("../x")` has the team directory as a lexical
  ancestor that is not an ancestor of the real directory.
- `let git_dir = a2amx::confine::git_common_dir(&canonical_dir, &home).map(|(common, _)| common)`;
  `config_path = git_dir.join("config")`.
- Everything after that stays as it is: the config must be a regular file
  (`std::fs::metadata(..).is_file()`), is read through `take(MAX_GIT_CONFIG_BYTES + 1)`, is refused
  when larger than 65,536 bytes or not UTF-8, and any failure means no git grants.
- The `dir.join(".git").is_dir()` test is removed. `home` is the host home already passed in; an
  empty `home` (the `HOME`-unset case) stays as `git_common_dir` treats it: no special case.

The marker detection (`dir.join(name).exists()`) is unchanged and keeps using the path as given:
only the git-remote grants follow the repository upward.

## 4. Tests (`tests/auto_confine.rs`)

Written first, each through the binary with the fixtures the file already uses (`global_auto_file`,
`run_binary`, `tempfile` directories, a git remote written into the config):

1. Worktree: `<project>/.git` is a file `gitdir: <meta>`, `<meta>/commondir` holds a path to a
   common directory whose `config` has `url = git@host-a:team/demo.git`. `team up --dry-run`
   prints `  grants: cache, ssh` (the dry run lists `cache` first, as `tests/auto_confine.rs:89` and
   `:537` assert) and `confine suggest` prints `grants = ["ssh", "cache"]` (suggest lists in grant
   table order, as `:719` asserts). Every test states both literals for its own case.
2. Submodule: `.git` file points to a directory that has no `commondir`; its `config` has the ssh
   remote; same result.
3. Subdirectory: the repository root has `.git/config` with an ssh remote; the session directory is
   `<root>/sub`; same result.
4. HTTPS in a worktree with a `~/.config/gh` present in the fixture home: `gh` is added. The
   developer takes the two literals (dry run and suggest) from the first run and states them in
   the report, because the grant table's order decides where `gh` sits.
5. Abuse cases in `tests/auto_confine.rs`, each yielding `  grants: cache` and a successful dry
   run. Each fixture holds an ssh remote (`url = git@host-a:team/demo.git`) in the place the old or
   a broken rule would read, as the existing test at `:753` does, so that `cache` is not what a
   missing check gives anyway: a `.git` file whose `gitdir:` target does not exist; a target whose
   `config` is larger than 65,536 bytes and starts with the remote; a target whose `config` is a
   directory (with the remote in a sibling `config` of the pointer's parent); a target whose
   `config` is a symlink to a directory; and a `gitdir:` that names the fixture home directory,
   where `<home>/config` is a file with the ssh remote, so that dropping the `common != home` stop
   turns `cache` into `cache, ssh`.
6. Relative path (red after section 3 without the canonicalization): inside a fixture repository
   with an ssh remote, `a2amx confine suggest ../sibling` where `sibling` is a directory outside
   any repository prints `grants = ["cache"]`.
7. Bounded reads in `tests/confine.rs`, through `a2amx::confine::git_common_dir`, each run on a
   thread with a 5 s `recv_timeout` (std `mpsc`; a thread that hangs on the old code stays blocked
   until the test binary exits and is never joined) so a hang fails the test and does not stall
   the suite. Each call passes a canonical tempdir as `cwd` and a `home` that is not an ancestor, and
   each red case has a valid `gitdir:` line to an existing directory (the oversized `.git` file holds
   it first, padded after it), or the old code would also return `None`. The cases: a
   `commondir` that is a FIFO (`mkfifo` through `std::process::Command`) returns `None`; a `.git`
   file larger than 4096 bytes returns `None`; a `commondir` that is a directory returns `None`.
   The FIFO is made with `rustix::fs::mknodat` (rustix with `fs` is already a dependency) or
   `mkfifo` through `std::process::Command`. These do not compile on the old code (the function is
   private): they are red after the `pub` change alone, before the bounded reads, by hang or wrong
   result (a directory `commondir` gives `Some(gitdir)` there).
8. The existing `tests/confine.rs::git_worktree_and_submodule_common_dirs_are_bound` and
   `git_common_directory_and_exact_writes_override_protection` pass unchanged (contract tests).
9. The existing `global_auto_ignores_worktree_git_files_and_oversized_configs` keeps passing
   unchanged (its worktree pointer `/other/repo/.git` does not exist): this is a contract test.
   Tests 1 to 4 fail on the old code (red phase); the abuse cases of 5 pass on the old code and are
   contract tests. The fixtures create their directories with `tempfile::tempdir()`, under `TMPDIR`;
   if `TMPDIR` pointed inside a repository the new walk would find it, so the tests set none.

## 5. Docs

- `docs/confinement.md` (lines ~221-229): replace "worktree `.git` files and unreadable or invalid
  configs add no git grants" and "A `.git` file for a worktree or submodule ... require an explicit
  grant" with the new rule: the repository is found from the project directory upward, through a
  `.git` file's pointer; the common directory's `config` must be a regular file of at most 65,536
  bytes; anything else adds no git grants. Keep "a local team file requires an explicit grant" and
  the `gh auth setup-git` sentence.
- `docs/teams.md` (line ~132) and `docs/cli.md` (lines 101-102, which today says only "when `.git/config` is a regular file of at
  most 65,536 bytes"): same change in one sentence each.
- `docs/confinement.md` Troubleshooting table: add one row. Symptom: `git push -u`, `git branch
  --set-upstream-to` or `git remote add` fails with `could not write config file .git/config:
  Device or resource busy`. Cause: `.git/config` is read-only in a confined session on purpose
  (an agent could set `core.hooksPath`, `fsmonitor` or an alias that runs later outside the
  sandbox). Fix: push with `git push origin <branch>`, which needs no upstream, and set the
  upstream or remote from an unconfined shell; a supported path is tracked as a known gap.
  Do not state that `git config` of other keys works.

## 6. Out of scope

The pre-existing fact that the confinement plan binds the common directory a `.git` file names as
writable (`bind_workspace`, `src/confine.rs:1230-1243`), including one in a cloned repository: the
architect carries it to the escape audit (F71); following a pointer anywhere else; `config.worktree`, `includeIf` or `include.path` (an included
file is not read); `.git` handling in the plan (`protect_git_metadata` is unchanged); changing the
git-config protection or adding a grant for it (F76); the `explain` text for a grant (it already
says "git remote uses ssh"); the `CHANGELOG` and backlog rows.

## 7. Acceptance

Baseline at lock time: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --check`. After implementation: the same three, plus `cargo test --test auto_confine`
and `cargo test --test confine`. The tester runs mutants and names the test that kills each: on `auto_grants_for` (use
`dir.join(".git")` instead of `git_common_dir`; drop the regular-file check; drop the size cap; use
`config.worktree`; drop the canonicalization; drop the `common != home` stop) and on
`git_common_dir` (drop each `take(POINTER_LIMIT + 1)` bound; drop the `is_file()` check of
`commondir`; open before `metadata`). The tests create their directories under the system temp
directory; a `TMPDIR` inside the checkout (where scratch lives) would make the walk find this
repository's own ssh remote and turn every `cache` expectation into `cache, ssh`, so the tester
runs them with `TMPDIR` unset. The architect reads the diff, removed
lines included.

## 8. Claims about the current tree

| Claim | Where | Verified how |
| --- | --- | --- |
| `auto_grants_for` reads `.git/config` only if `.git` is a directory | `src/main.rs:751-786` | read |
| `git_common_dir` follows `gitdir:` and `commondir` and walks upward | `src/confine.rs:823-868` | read |
| `git_common_dir` has one caller, in the plan | `src/confine.rs:1232` (rg over src and tests) | grepped |
| Existing test uses a missing pointer target and expects `grants: cache` | `tests/auto_confine.rs:753-775` | read |
| Docs say worktree files are not followed | `docs/confinement.md:221-229`, `docs/teams.md:131-133` | read |
| `docs/cli.md:101-102` states only the regular-file and size rule, no worktree sentence | `docs/cli.md` | read |
| The plan canonicalizes its working directory | `src/confine.rs:1027` | read |
| `confine suggest [DIR]` and `team up` pass the path raw | `src/main.rs:133-146`, `:803-809` | read |
| `git_common_dir` reads the `.git` file and `commondir` without a size bound | `src/confine.rs:836, 848` | read |
| An existing dry-run test asserts the order `cache, cargo, ssh` and an existing suggest test `["ssh", "cache"]` | `tests/auto_confine.rs:89, 537, 719` | read |
| `File::open(O_RDONLY)` on a FIFO blocks until a writer opens it | POSIX behaviour | recalled; test 7 turns a hang into a failure, and the tester shows the `commondir` FIFO case is red after the `pub` change alone |
| `Path::parent` of `../x` is `..`, then `""` | Rust standard library behaviour | probed with a one-file rustc program (2026-11-05): `../x`, `..`, `""`, and `"".join(".git")` is `.git` |
| The plan test for worktree and submodule layouts exists | `tests/confine.rs:3407` | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5; developer and tester OMP
`openai-codex/gpt-5.6-luna` (planning, working and advisor slot names are not exposed in the OMP
sessions). BLOCKED/amend cycles: 1 (Amendment 1).

| Where found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Implementation | developer, gpt-5.6-luna | The FIFO-config abuse case asked of the developer could not use `team up --dry-run`: the confinement plan's `parse_core_hooks_path` reads `<common>/config` with `read_to_string` and blocks on a FIFO (pre-existing) | Amendment 1: the case runs `confine suggest`; the planner hang is backlog F77 |
| Developer mutants | developer, gpt-5.6-luna | Dropping the regular-file check of the config survived: the spec's abuse cases covered a directory and a symlink to one, not a FIFO | FIFO-config test added (spec gap, missed by the draft review of architect and consultant) |
| Tester pass | tester, gpt-5.6-luna | Dropping the home canonicalization and resolving a relative `gitdir:` from the initial directory both survived the suite | Two `confine suggest` tests added |
| Tester pass | tester, gpt-5.6-luna | The troubleshooting row lacked the spec's "a supported way is a known gap" wording | Architect added the sentence |
| Architect review | architect, Sonnet 5.5 | No problems beyond the above; own mutants (a directory `commondir` accepted, the config cap raised by one byte) were killed | none needed |

Accepted survivors: the two `take(POINTER_LIMIT + 1)` read bounds (the post-read length check gives
the same result; only memory use differs, which no test at the public seam can see) and a lossy
UTF-8 decode of the marker (no observable difference).
