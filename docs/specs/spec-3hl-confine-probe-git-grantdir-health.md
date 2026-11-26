# Spec 3hl: function-level code health of `src/confine/bwrap.rs`, `git.rs` and `grant_dirs.rs`

## 0. Status

LOCKED after consultant review m_9555 (F1-F6 and the reachability and mutant findings applied). Context `a2amx-3hl-confine-probe-git-grantdir-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `1ceb9b0`. Behaviour-preserving refactor, test-first where a message or reason is not yet pinned. Public seams: `confine::preflight`, `confine::supports_tmp_overlay`, `confine::resolve_bwrap`, `confine::find_executable`, `confine::git_common_dir`, `confine::ensure_grant_dirs`, `confine::plan` (which reaches `parse_core_hooks_path`).

Scope (edit): `src/confine/bwrap.rs`, `src/confine/git.rs`, `src/confine/grant_dirs.rs` (production code; none has a `#[cfg(test)]` module), three new test files `tests/confine/bwrap_probe_messages.rs`, `tests/confine/git_config_case.rs` and `tests/confine/grant_dir_reasons.rs`, and one `mod` line for each in `tests/confine/main.rs` (alphabetical: `bwrap_probe_messages` before `git_dirs_and_roots`, `git_config_case` before it, `grant_dir_reasons` before `grant_writes_and_create`). Does not touch: any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same results, same error and reason text byte for byte, same limits, same probe order.

## 1. Why

`cs check` (2026-11-26, `1ceb9b0`) reports 14 findings in these files:

- `bwrap.rs`: `command_with_timeout` `:13` (Bumpy Road 2, Deep Nested Complexity depth 4); `resolve_bwrap` `:73` (cc 9) and its conditional `:82`.
- `git.rs`: `parse_core_hooks_path` `:9` (Bumpy Road 2, cc 10); `git_common_dir` `:89` (Bumpy Road 3, nesting 4, cc 13).
- `grant_dirs.rs`: `grant_dir_problem` `:55` (cc 16) with conditionals `:57` (3) and `:81` (5); `ensure_grant_dirs` `:100` (cc 9) with conditional `:122`.

Each function does two or three separable jobs in one body: running a child and turning its exit into an error message; reading a file under a limit and parsing it; following a `.git` pointer file and then its `commondir`; checking an existing path and a missing path against the same protected set; looping over grants and creating one directory. The refactor cuts along those jobs and adds nothing else.

Several error texts are unpinned: no test names the `grant_dir_problem` reasons beyond a `contains("protected")`, or the exit-with-stderr message of `command_with_timeout`. A "byte-identical" claim needs those pins first (Step A).

Rules that bind (earlier refactors): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function; split only along contiguous runs so order stays; no suppression. Alternatives rejected: a trait for the git layouts (a function per layout is the whole variation); moving these functions to other modules (placement is right: probes in `bwrap`, git layout in `git`, grant directories in `grant_dirs`).

## 2. Step A: pin the unpinned behaviour first (tests, green on the current code)

Contract tests, written and run against the unchanged source first (no red phase). The developer probes the current output once, reads it, and writes the literal into the test; only fixture paths are substituted.

`tests/confine/bwrap_probe_messages.rs` (fake bwrap and env scripts in a temp dir, the pattern of `notices_and_preflight.rs:372-413`; `preflight` canonicalizes the script, so the expected text uses the canonical path):

| Test | Setup | Expected |
| --- | --- | --- |
| `preflight_error_carries_the_exit_status_and_stderr` | script exits 3 and writes `boom` to stderr | `confine::preflight(script)` is `Err` with the whole text `{script} exited with exit status: 3: boom\n` (literal as probed) |
| `preflight_error_without_stderr_has_no_suffix` | script exits 3 silently | the text is `{script} exited with exit status: 3` |
| `overlay_probe_failure_text_is_pinned` | script fails the overlay probe with two stderr lines | `supports_tmp_overlay` is `Err` with the literal as probed. Expected shape `3: <first stderr line>`: it splits at the FIRST `": "`, the one after `exit status`, so the code number stays in the text. This is a pre-existing quirk, pinned as is and out of scope. |
| `resolve_bwrap_refuses_an_empty_path_value` | `resolve_bwrap(Some(OsStr::new("")))` | `Err` with `PATH is empty; cannot find bubblewrap` |

Not pinned, reported by the tester as not killed with the reason: the timeout branch (a 5 s child, too slow) and `PATH is not set` (needs the process `PATH` unset, unsafe in parallel tests).

`tests/confine/git_config_case.rs`: `hooks_path_accepts_a_capitalised_core_section` - a git config with `[Core]` and a `hooksPath` yields the same plan as `[core]` (the assertion style of `paths_and_git_config.rs:181`). It is the only test that kills a case-sensitive `[core]` mutant.

`tests/confine/grant_dir_reasons.rs` (through `confine::ensure_grant_dirs`, the `Fixture` and grants with `create` paths, `grant_writes_and_create.rs:234-315`): assert `Unresolved.reason` equals the whole literal for each reason a public request can reach:

| Test | Setup | Expected reason |
| --- | --- | --- |
| protected overlap | an existing create path inside the a2amx state root (`grant_writes_and_create.rs:254`) | `path overlaps protected location` |
| hidden existing | an existing create path under a hidden path | `path overlaps protected location` |
| symlink, existing and missing | the two sources, each with a symlink component | `symlink component: <path>` |
| hidden ancestor | a missing create path whose nearest existing ancestor is hidden (hide an existing `~/.cache`, grant `cache-pip`), or the state directory under home | `ancestor is not writable` |
| create failure | `~/.cache` is a regular file, grant pip or uv | the `create_dir_all` text as probed (`Not a directory (os error 20)`) |
| existing regular file is left alone | a regular file at `~/.npm`, node grant, `create` true | no `Unresolved` (kills a mutant that creates an existing source) |
| order | two grants (or one grant's two paths) both unresolved | `assert_eq!` on the whole `Vec<Unresolved>` in grant order then `create` order |

`no existing ancestor` and `ancestor cannot be canonicalized` are unreachable (`/` always exists and canonicalizes). `ancestor is not writable` through the write policy is unreachable too: an ancestor of a create path is under the canonical home, and `check_write_allowed` with `explicit = false` fails there only on a root overlap (already caught by `overlaps_reserved`) or on `MANDATORY_ROOTS` (`/run/WSL`, `/mnt/wslg`). The developer confirms by trying and lists each in its REPORT as unreachable with the attempt; no test and no hook is added for them. A reason that needs a private accessor is reported BLOCKED.

## 3. Step B: target shape (the design is the instruction; the `cs` result is the check)

Hand count (consultant, +1 per `if`, `if let`, `let else`, `&&`, `||`; `?` and closures do not count): the shapes below bring every function under the cc 9 threshold and every conditional under two operators.

`src/confine/bwrap.rs`:

- `command_with_timeout`: the loop keeps `try_wait`, the deadline check and the sleep. The exit branch becomes `fn exit_result(path: &Path, status: ExitStatus, child: &mut Child) -> Result<()>`: success is `Ok` and is checked BEFORE `stderr_suffix` is called; a failure builds `"{path} exited with {status}"` plus the suffix. `fn stderr_suffix(child: &mut Child) -> String` returns `""` or `": <text>"` (`take()` keeps the read to once). The timeout branch (kill, wait, bail) stays inline.
- `resolve_bwrap`: `fn search_path_value(path_var: Option<&OsStr>) -> Result<PathBuf>` takes the first block (argument or `PATH`, `PATH is not set`, `PATH is empty`). The `:82` condition becomes `path.is_absolute() && path.is_file()`: the `components().count() > 1` term is redundant (an absolute path that is a file is not `/`, so it has at least two components) and dropping it removes the conditional finding with no helper. The rest of the function is unchanged.

`src/confine/git.rs`:

- Absolute-or-relative joins become `base.join(path)` at the three sites (`:53-57`, `:108-112`, `:119-123`): `Path::join` with an absolute argument replaces the base, so the result is identical and a nesting level goes.
- `parse_core_hooks_path(common, worktree)`: `fn read_git_config(common: &Path) -> Result<Option<String>>` (open non-blocking, stat, bounded read, the size error; `Ok(None)` for an unreadable or non-regular config) and `fn hooks_path_in(text: &str, worktree: &Path) -> Option<PathBuf>` (the `[core]` scan, then `worktree.join(value)`). `parse_core_hooks_path` is the two calls.
- `git_common_dir`: `fn pointer_gitdir(marker: &Path, current: &Path) -> Option<PathBuf>` (read the `.git` file, find the `gitdir:` line, `current.join(target)`, canonicalize) and `fn common_dir_of(gitdir: PathBuf) -> Option<PathBuf>` (`commondir`: regular file means read and `gitdir.join`, missing means `gitdir` itself, any other metadata or error means `None`; canonicalize). The `.git` file case in the loop returns after one `?` chain (`pointer_gitdir`, `common_dir_of`, then the `home` and shape check): on any failure it RETURNS `None`, it never continues the parent walk (no test pins this, so the developer takes care). The loop keeps the `home` check, the `.git` directory case and the parent walk; no new branch is added to it (cc 8 by hand count).

`src/confine/grant_dirs.rs`:

- Methods on `GrantDirScope`: `overlaps_reserved(&self, path: &Path) -> bool` (overlaps the state root or the runtime directory), `hides(&self, path: &Path) -> bool` (inside any hidden path), and `ancestor_unwritable(&self, ancestor: &Path) -> bool` (early returns in today's order: outside home or `overlaps_reserved`; below home and not write-allowed; `hides`).
- `grant_dir_problem` is split, mandatory: `fn existing_source_problem(source, scope) -> Option<String>` (protected overlap as `fs::canonicalize(source).is_ok_and(|c| scope.overlaps_reserved(&c) || scope.hides(&c))`, then the symlink component) and `fn missing_source_problem(source, scope) -> Option<String>` (symlink component, nearest existing ancestor, canonicalize, `ancestor_unwritable`); `grant_dir_problem` picks by `source.exists()`. Order inside each is today's.
- `ensure_grant_dir(source: PathBuf, scope: &GrantDirScope, create: bool) -> Option<Unresolved>` with one guard and one construction: `let reason = match grant_dir_problem(&source, scope) { Some(reason) => reason, None if !create || source.exists() => return None, None => fs::create_dir_all(&source).err()?.to_string() }; Some(Unresolved { option: "grant", path: source, reason })`. `ensure_grant_dirs` maps the grants' `create` paths through it, in grant order then `create` order.

## 4. Out of scope

- Any change to a message, reason, limit or probe order; the timeout test; `supports_tmp_overlay`, `find_executable`, `preflight_env`, `ensure_empty_file`, `grant_dir_scope` (no findings).
- Other files with findings (`confine.rs`, `hides.rs`): the next spec.

## 5. Trust boundary

Creates none: no new input path. The git config and pointer files are read as today (bounded, non-blocking open, regular files only), the same checks in the same order; Step A pins the texts that report a refusal.

## 6. Acceptance

Baseline (before any edit, on `1ceb9b0`): `cargo test` passes (1500 passed, 3 ignored).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/confine/bwrap.rs
cs check src/confine/git.rs
cs check src/confine/grant_dirs.rs
cs check tests/confine/bwrap_probe_messages.rs
cs check tests/confine/git_config_case.rs
cs check tests/confine/grant_dir_reasons.rs
git status --short
```

- All pass; `cs check` lists none of the 14 findings of section 1 and no finding on the three new test files; the whole-tree cs count is below 894 (judge: the architect's `cs` run; owner rule: every commit lowers it).
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`, no unused item.
- Step A tests pass on the unchanged source (the developer reports it; the tester reruns them on `git show HEAD:` sources) and on the diff; the whole suite passes on both trees.
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hl/` (both deleted before its REPORT); per mutant run `cargo test --lib` and the test targets `confine`, `auto_confine`, `team`, `teams`, `daemon`, `gateway_up`, `gateway_background` (the tester lists the targets it used). Mutants, one change each: (a) `exit_result` treats a failed exit as success; (b) `stderr_suffix` returns `""`; (c) `resolve_bwrap` skips the `PATH is empty` error; (e) `hooks_path_in` matches `[core]` case-sensitively; (f) `hooks_path_in` uses a relative value unjoined; (g) `read_git_config` allows one byte over `GIT_CONFIG_PLAN_LIMIT`; (h) `pointer_gitdir` joins a relative `gitdir:` onto the wrong base; (i) `common_dir_of` treats a missing `commondir` as `None`; (j) `common_dir_of` accepts a non-file `commondir`; (k) `git_common_dir` skips the `home` check after a pointer; (k2) a failed pointer lets the walk continue (expected survivor, see below); (l) `overlaps_reserved` ignores the runtime directory; (m) `hides` is never true; (p) `ensure_grant_dir` creates when `create` is false; (q) `ensure_grant_dir` creates an existing source; (r) grant order reversed. Expected kills: (a) the `stub_path_dir` preflight tests; (b) Step A test 1; (c) Step A test 4; (e) the new `git_config_case` test; (f) `paths_and_git_config.rs:181`; (g) `:250` and `:266`; (h) `git_dirs_and_roots.rs:15,36`; (i) the submodule pointer fixture (`git_dirs_and_roots.rs:36`, to be confirmed); (j) `:666`; (k) `:567`; (m) the hidden-source Step A test; (p) `grant_writes_and_create.rs:299`; (q) the existing-file Step A test; (r) the order Step A test. Expected non-kills, reported with the reason: the timeout lines and `PATH is not set` (not pinned, section 2); (k2) (every pointer fixture sits in a temp directory with no repository above it, so walking on also ends in `None`: a survivor no public request can tell apart, dispositioned by the architect; the developer follows section 3 by reading); mutants on `no existing ancestor`, `ancestor cannot be canonicalized` and the write-policy part of `ancestor_unwritable` (unreachable, section 2). Any other survivor in moved code is a fix request to the developer (a new test in the files of section 0).

## 7. Claims (architect, `1ceb9b0`)

| Claim | Where | Verified how |
| --- | --- | --- |
| the 14 findings and their lines | `bwrap.rs:13,73,82`; `git.rs:9,89`; `grant_dirs.rs:55,57,81,100,122` | `cs check` run on the three files |
| none of the three files has a unit-test module | `rg "mod tests"` | grep: no hit |
| `grant_dir_problem` reason strings appear in no test except `contains("protected")` | `tests/confine/grant_writes_and_create.rs:273` | grep of each string over `tests/`: no hit; the `protected` assert read |
| `command_with_timeout` has no direct test; the exit and stderr texts have none | `bwrap.rs:13-50` | grep over `tests/`: only `tests/privilege.rs` mentions the phrases (a different message, to be confirmed by the consultant) |
| callers: `parse_core_hooks_path` `builder.rs`; `git_common_dir` `builder.rs`, `confine.rs`, `commands/grants.rs`; `ensure_grant_dirs` `confine.rs`, `commands/team.rs`, `commands/team/dry_run.rs`, `daemon/launch.rs`; `resolve_bwrap` `daemon/launch.rs`, `gateway_launch.rs` | grep over `src/` | grep |
| tests reaching these: `tests/confine/{paths_and_git_config,git_dirs_and_roots,notices_and_preflight,grant_writes_and_create,grants_runtime,real_bwrap,harness_config,limits_and_hides}.rs`, `tests/auto_confine/{git_detection,git_config_limits}.rs`, `tests/daemon/confine.rs`, `tests/gateway_*.rs` | grep | grep; the exhaustive list is the consultant's check |
