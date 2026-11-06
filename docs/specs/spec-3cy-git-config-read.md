# Spec 3cy: The confinement plan reads the repository's git config without hanging and without ignoring it (F77)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/confine.rs` (`parse_core_hooks_path`, `protect_git_metadata`, the
`bind_workspace` call site, one new constant) and `tests/confine.rs`. Does not touch: `src/main.rs` (its
`auto_grants_for` guard stays as it is), `src/harness.rs`, `docs/` (the architect updates the backlog and adds
the changelog line), every other test file, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seam: `a2amx::confine::plan` (the tests call it the way
`git_hooks_and_core_hooks_path_are_protected_inside_writable_repo` at `tests/confine.rs:3372` does).

## 1. Why

Backlog F77, found by the developer of spec 3cs. `plan` reaches `protect_git_metadata`
(`src/confine.rs:1322`) for every session started in a git repository, and that calls `parse_core_hooks_path`
(`src/confine.rs:834`), which reads `<git dir>/config` with `fs::read_to_string`. If `config` is a FIFO, the
open blocks until a writer appears, so `a2amx team up` and `a2amx new --confine` hang with no output. A
repository cannot ship a FIFO through git, but a hand-made or archived tree can. The auto detector in
`src/main.rs::auto_grants_for` already refuses non-regular files and caps the read; the planner does not.

Reading the config is a safety function, not a convenience: the planner binds a repository's `core.hooksPath`
target read-only so a confined agent cannot rewrite the hook that git runs outside the session. The current
code is fail-open in two more ways, so this spec fixes them in the same function:
- A config that is not valid UTF-8 (a Latin-1 name in `user.name` is enough) makes `read_to_string` fail,
  `.ok()?` returns `None`, and the `hooksPath` target stays writable.
- Capping the read the way `auto_grants_for` does (64 KiB, over the cap means "no config") would be fail-open
  too: a config padded past the cap would hide its `hooksPath`.

Decision: a config that is not a regular file is treated as absent: git would block on a FIFO too and nothing
in the tree can supply its content (the `config` path itself is still bound read-only by name); a regular file over the cap fails the plan with a clear error;
bytes that are not UTF-8 are decoded lossily so the `[core]` section is still found.

Rejected: reusing `read_pointer_text` (a 4 KiB cap, and `None` over the cap, fail-open); the cap of
`auto_grants_for` (same reason); `spawn_blocking` or a timeout around the read (the hang is a FIFO open, which
a non-blocking open and a type check remove, and a timeout would leave the thread blocked); a shared helper
with `auto_grants_for` now (its callers want "no config" on oversize, the planner wants an error, so a shared
helper would need a mode flag; revisit if a third reader appears).

## 2. Change in `src/confine.rs`

1. A constant for the plan's limit, one MiB (`1024 * 1024`), next to `POINTER_LIMIT`
   (line 861). Name it so it cannot be confused with `MAX_GIT_CONFIG_BYTES` in `src/main.rs:750` (for
   example `GIT_CONFIG_PLAN_LIMIT`), with a `//` comment saying why it differs.
2. `parse_core_hooks_path(common, worktree)` becomes `-> Result<Option<PathBuf>>`. It opens `<common>/config`
   with `OpenOptions::new().read(true).custom_flags((OFlags::NONBLOCK | OFlags::NOCTTY).bits() as i32)` using
   `rustix::fs::OFlags` (add `use rustix::fs::OFlags;`; `NOCTTY` so that opening a tty device through a
   planted symlink cannot make it the controlling terminal) (`rustix` is a dependency with the `fs` feature; `OFlags` is a `c_uint` bitflags type
   and `custom_flags` takes an `i32`, so the cast is needed; opening a FIFO read-only with `O_NONBLOCK`
   returns at once, and the flag has no effect on regular files), takes `File::metadata()` of the opened
   file, and returns `Ok(None)` when it is not a regular file (a FIFO, a directory, a device) or when the open
   fails (as today: a missing or unreadable config is "no config"). It reads at most `limit + 1` bytes with
   `Read::take`; more than the limit is `bail!("git config {} is larger than {} bytes; the confinement plan
   cannot check its hooksPath", path.display(), limit)`, and a read error after a successful open is also a
   plan error (`with_context`, fail-closed, like oversize). It decodes with `String::from_utf8_lossy`.
3. The parsing loop keeps its shape with one fix: `let (key, value) = line.split_once('=')?;` (line 847)
   cannot stay, because `?` on an `Option` does not compile in a `Result` function, and it is fail-open
   today (a blank line, a `#` or `;` comment, or a bare boolean key such as `bare` inside `[core]` makes the
   whole function return `None` and hides a later `hooksPath`). Replace it with `let Some((key, value)) =
   line.split_once('=') else { continue };`. The loop returns `Ok(Some(path))` on a hit and `Ok(None)` at the
   end.
4. `protect_git_metadata` returns `Result<()>` and propagates the error with `?`; the `if let Some(path) =`
   becomes a `parse_core_hooks_path(..)?` followed by the existing canonicalize and `add_unique`. The call in
   `bind_workspace` (line 1317) gets a `?`.

## 3. Out of scope

`auto_grants_for` and `MAX_GIT_CONFIG_BYTES` in `src/main.rs`, `config.worktree` content (never read, only
bound), the `--settings` file and the team file reads (named by the operator, not shipped in a repository),
parsing improvements (`include.path`, `[core]` with a trailing comment, `hooksPath` in `config.worktree`),
and the confinement docs.

## 4. Tests in `tests/confine.rs`

Each uses the `Fixture` and the repo layout of `tests/confine.rs:3372` and calls `confine::plan` on a thread
with a 5 second `recv_timeout`, like `git_common_dir_with_timeout` (`tests/confine.rs:3942`).
1. `plan_does_not_hang_on_fifo_git_config`: `.git/hooks` created as in the layout of line 3372, `.git/config` made with `mkfifo` instead of the config write (as at line 3930). Expect the
   plan returns `Ok` and `plan.protected` contains the canonical path of `.git/config` and of `.git/hooks`.
   This is red on the old code (the thread never answers; the test fails by timeout).
2. `plan_rejects_oversized_git_config`: a regular `.git/config` of limit + 1 bytes. Expect an `Err` whose text
   contains `larger than` and `hooksPath`. Red on the old code (it returned `Ok`). A second case at exactly the
   limit: `[user]\n\tpad = <N letters a>\n[core]\n\thooksPath = .husky\n` with N = the limit minus the byte length of the rest of that literal, so the total length
   equals the limit exactly and the `.husky` directory created: expect `Ok` and the `.husky` path protected
   (the padding sits outside `[core]` so the parser, not the cap, decides).
3. `plan_protects_hooks_path_in_config_with_invalid_utf8`: `.git/config` containing `[user]\n\tname =
   \xe9\n[core]\n\thooksPath = .husky\n` (raw bytes). Expect `plan.protected` contains the canonical `.husky`.
   Red on the old code.

4. `plan_ignores_directory_git_config`: `.git/config` is a directory. Expect `Ok` and the canonical `.git/config`
   in `plan.protected`. It kills the mutant that drops the regular-file check (reading a directory fails with
   `EISDIR`, which would turn into a plan error). Red-phase note: on the old code this case also returns an
   `Ok` (the read error becomes `None`); it is a contract test for the new code.
5. `plan_finds_hooks_path_after_lines_without_equals`: `.git/config` = `[core]\n\n\tbare\n# note\n\thooksPath =
   .husky\n`, `.husky` created. Expect `.husky` in `plan.protected`. Red on the old code.

## 5. Acceptance

1. `cargo test --test confine` shows the five new tests passing; before the change the developer shows
   tests 1, 2, 3 and 5 failing (test 4 is a contract test; the output goes in the report).
2. Mutation check by restore-by-checksum edits of `src/confine.rs` in the checkout (the tree must equal the
   developer's own diff afterward): (a) drop the `NONBLOCK` flag, test 1 times out; (b) drop the regular-file
   check, test 4 fails (the FIFO read returns EOF at once with no writer, so test 1 alone would survive); (c) change the cap comparison from `>` to `>=`, the
   exactly-at-limit case fails; (d) replace `from_utf8_lossy` by `String::from_utf8(..).ok()`-style
   rejection, test 3 fails; (e) turn the oversize `bail!` into `Ok(None)`, test 2 fails; (f) put back `?` semantics by returning
   `Ok(None)` at a line without `=`, test 5 fails. The developer names
   the failing test for each.
3. `rg -n 'read_to_string' src/confine.rs` prints nothing.
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient
   environment of a hosted session. Known flakes I7, I14 and I18 (`docs/backlog.md`): rerun once and record as
   pre-existing.
5. Scratch files stay under `target/scratch/3cy/` and are removed before the report.
