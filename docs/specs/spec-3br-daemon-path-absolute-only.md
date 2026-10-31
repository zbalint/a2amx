# Spec 3br: the daemon's executable lookups ignore relative and empty PATH entries

## 0. Status

**LOCKED** (2026-10-31, one consultant review `m_4861` applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F56, found by the tester in spec 3bo, `m_4855`, verified by the architect).
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `9a7b88f`; full suite 731 passed, 0 failed, 3 ignored; run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR HOME=<writable> XDG_RUNTIME_DIR=<existing 0700 dir> CARGO_TARGET_DIR=<one disk-backed dir under ~/.cache> cargo test`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::find_executable` (new, public), `confine::resolve_bwrap` (public, `tests/daemon.rs:178`), the quota probe through `DaemonConfig::bwrap_path` (section 3).

**Scope.** May edit exactly: `src/confine.rs`, `src/daemon.rs`, `tests/confine.rs`, `tests/quota.rs`, `docs/confinement.md`.
Does not touch: `CHANGELOG.md` (the architect edits it), every other file. No new dependency. Leave the diff uncommitted.
Re-anchor every line number against the tree; the anchors below are from `9a7b88f`.

## 1. Why

The daemon runs two programs outside the sandbox, found through its `PATH`: `bwrap` at every confined start
(`src/daemon.rs:366` calls `confine::resolve_bwrap`), and the `omp` that polls quota (`src/daemon.rs:1513`, `Command::new(argv0)`).
Both lookups honour relative and empty `PATH` entries. `resolve_bwrap` maps an empty entry to `.` and joins a relative
entry onto `bwrap` (`src/confine.rs:1415-1420`), resolved against the daemon's working directory. `Command::new("omp")` leaves the
lookup to the operating system, which resolves a relative or empty entry against the child's working directory, and the daemon sets that to
the session's own directory (`command.current_dir(cwd)`, `:1523-1525`), which the confined agent can write.

So if the daemon's `PATH` contains an empty or relative entry (for example `PATH=:/usr/bin` or `PATH=.:/usr/bin`), a confined agent can
create `bwrap` in the daemon's working directory (when that lies inside a writable area, which `write = ["~"]` now makes likely) or `omp`
in its own workspace, and the daemon runs it outside the sandbox. Spec 3bo protects absolute `PATH` directories inside the home; the relative
entries cannot be protected as mounts because they move with the working directory. The fix is to stop honouring them: a relative or empty `PATH` entry no longer
selects a program the daemon runs unconfined. Section 6 lists what stays open.

## 2. One lookup (`src/confine.rs`)

Add `pub fn find_executable(name: &str, path_var: &OsStr) -> Option<PathBuf>`: for each entry of `std::env::split_paths(path_var)`, skip it unless it
is absolute (an empty entry is not absolute); join `name`; the first candidate that is a file with an execute bit is returned, canonicalized.

`resolve_bwrap` keeps its signature and error texts. Two changes: the branch `path.components().count() > 1 && path.is_file()` (`:1402-1404`, a `PATH` value that is itself
a file) gains `path.is_absolute()`, so a relative file path such as `scripts/bwrap` falls through and ends in `bubblewrap was not found in PATH`; and the entry loop (`:1413-1427`) is replaced by a call to
`find_executable("bwrap", &path)`. The empty-value check (`:1399-1401`) runs first and is unchanged. The branch at `:1405-1409` (a single-component file name, which
joins `bwrap/bwrap` and is effectively dead) is left alone: the replacement changes nothing observable there.

## 3. The quota probe (`src/daemon.rs`)

`run_omp_usage` receives `argv0` from `omp_usage_command` (`:264-281`), the first word of the session's argv, and `Command::new(argv0)` searches the process `PATH` in the child, after
`current_dir(session cwd)` (`:1523-1525`). When `argv0` contains no `/`, resolve it before spawning with `confine::find_executable(argv0, &self.bwrap_path)` (the daemon's captured `PATH`: it is the
process environment's `PATH` unless a config injects it, `:1644-1648`; the lookup runs in `poll_omp_usage` or inside `run_omp_usage`, the developer's choice) and spawn the returned path. If it is not found,
return the existing error string for a spawn failure (`"spawn"`). An unset `PATH` becomes an empty value (`.unwrap_or_default()`), so nothing is found and the probe reports `"spawn"`: the old call would
have used libc's default search path. An `argv0` with a `/` is used as it is. Add a one-line doc comment on `DaemonConfig::bwrap_path` (`:60`) saying it is the daemon's `PATH` value, used for `bwrap` and `omp`.

## 4. Docs (`docs/confinement.md`)

One sentence in "What stays open" (`:80-89`, residual 6 or a new item): the daemon finds `bwrap` and the `omp` quota probe only in absolute `PATH` directories, and relative and empty entries are ignored; an `omp`
started from a path the session can write (a relative path with a slash, or an absolute one outside the `PATH` directories that `write` opens) is still run by the quota probe outside the sandbox, so keep the
harness binary in a read-only directory.

## 5. Tests

The tests run with the crate root as the process working directory (cargo's rule), so a relative `PATH` entry can be pointed at a tracked file without changing process state:
`scripts/prepush-fast.sh` is a tracked executable (mode 100755). The absolute form uses `env!("CARGO_MANIFEST_DIR")`.

1. `tests/confine.rs::find_executable_ignores_relative_and_empty_entries`: `find_executable("prepush-fast.sh", "scripts")` returns `None`; `find_executable("scripts/prepush-fast.sh", "")` and `(.., ":")`
   (empty entries, which the old loop turned into `.`) return `None`; `find_executable("prepush-fast.sh", "<CARGO_MANIFEST_DIR>/scripts")` returns the canonical path of that file (contract
   test: absolute entries still work). Red at compile time on the old code (the function does not exist yet); say so in the report. Pins mutants 1 and 2.
2. `tests/confine.rs::resolve_bwrap_refuses_a_relative_file_path`: `resolve_bwrap(Some("scripts/prepush-fast.sh"))` fails with `bubblewrap was not found in PATH` (abuse case; red on the old code, where the
   `components().count() > 1 && is_file()` branch returned the canonical path). A second assertion `resolve_bwrap(Some("scripts"))` fails with the same error is a contract test (green on old code, pins nothing
   about relativity). The absolute case stays covered by the existing `bwrap_path_dir` tests (`tests/confine.rs:63`). Pins mutant 4.
3. Contract test in `tests/quota.rs` next to `failed_omp_usage_poll_is_not_retried_every_check` (`:996`): an omp session started with an absolute fake `omp` as argv (as `direct_omp_session`, `:393`) is still
   polled after the change. **The probe's bare-name lookup has no red test:** the session itself starts through the process `PATH`, a daemon test cannot reach a bare name without changing process
   state (`set_var` is unsafe and races), and the old probe never reads `bwrap_path`. Mutant 3 is therefore not killable by a test; the architect checks the wiring by reading at final review and the tester reports mutant 3
   as "reviewed by reading", not as a survivor.

## 6. Out of scope

Protecting the daemon's working directory with a mount; refusing to start on a relative `PATH`; `A2AMX_INSTALL_DIR`; the hook binary and the `a2amx` executable (launched confined or by absolute path); sanitizing the `PATH`
that a confined session inherits; the Codex spawn (`src/codex.rs:375` resolves a bare name against the session cwd, but confined Codex sessions are refused, `src/daemon.rs:473-476`, so it never runs for a confined agent).
Accepted residuals, documented in section 4: the quota probe runs the first running omp session's `argv0` outside the sandbox whether or not that session is confined (`omp_usage_command` does not check), so a relative
`argv0` with a slash, or an absolute one inside an area the session can write, is still a way to run a planted program; skipping confined sessions or requiring a read-only `argv0` is a separate spec if the owner wants it.

## 7. Trust boundaries

The daemon's `PATH` is operator input; the working directories it resolves relative entries against include ones a confined agent writes. Attack path A: plant `bwrap` or `omp` where a relative or empty `PATH`
entry resolves. Disposition: removed for `PATH` entries (they are no longer honoured), tested by test 1 (the lookup) and test 2 (the file branch of `resolve_bwrap`; red on the old code). Attack path B: plant the
program an omp session's `argv0` names when it has a slash or lies in a writable area. Disposition: monitored, not removed (section 6), because closing it needs a decision on the quota probe's design; it is
documented in section 4. Overall the spec reduces the exposure, it does not remove it.

## 8. Placement

Executable lookup is `confine.rs`'s job already (`resolve_bwrap`); the quota probe lives in `daemon.rs` and calls the one function. One helper replaces an inline loop; it is
used by two callers.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
rg -n 'find_executable|resolve_bwrap' src tests
```

Mutants for the tester: (1) `find_executable` honours relative entries again; (2) honours empty entries as `.`; (3) the quota probe spawns `argv0` without resolving it (not killable by a test, section 5 test 3: reported as reviewed by reading); (4) the `is_absolute()` guard on the `resolve_bwrap` file branch removed.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `resolve_bwrap` maps an empty entry to `.` and joins relative entries onto `bwrap` | `src/confine.rs:1413-1427` (empty to `.` at `:1414-1418`) | read |
| The daemon resolves `bwrap` per confined request from the PATH it captured at startup | `src/daemon.rs:365-366`, `:1644-1648` | read |
| The quota probe spawns `Command::new(argv0)` with `current_dir(session cwd)` | `src/daemon.rs:1513`, `:1523-1525` | read |
| `resolve_bwrap(None)` is the only direct test caller; every test injects an absolute `bwrap_path` directory | `tests/daemon.rs:178`; `tests/confine.rs:63`, `:130` and others | rg (consultant `m_4861`) |
| `resolve_bwrap` canonicalizes a relative file path with more than one component | `src/confine.rs:1402-1404` | read (consultant) |
| The probe never reads `bwrap_path` today and the session spawns through the process `PATH` | `src/daemon.rs:1513`, `tests/common/mod.rs:43-50` | read (consultant) |
| `scripts/prepush-fast.sh` is tracked with mode 100755 and cargo runs tests with the crate root as cwd | `git ls-files -s` | probed (consultant) |
| The default `env` path is the constant `confine::ENV_PATH`, not a `PATH` lookup | `src/daemon.rs:1649-1652` | read |

## Review record

Models: architect Claude Sonnet 5.5; developer and tester on the OMP harness, `openai-codex/gpt-5.6-luna` as they reported it (planning and
advisor models not exposed); the consultant review `m_4861` before the lock, its model not re-derived in this session. BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`m_4861`) | Test 3 could not be red on the old code (the probe never reads `bwrap_path`, and the session starts through the process `PATH`); the `resolve_bwrap` relative file branch was kept although it honours a relative path; the open `argv0` cases and the Codex spawn were not listed; line anchors were off | Section 5 says test 3 is a contract test and mutant 3 is reviewed by reading; the file branch got an `is_absolute()` guard and a red test; residuals and the Codex exclusion moved to section 6 and the disposition changed to "reduced"; anchors fixed |
| Developer report | Developer | The report gave 733 passed; its own suite figures (71 confine, 37 quota) add to 734 | The architect reran the full suite: 734 passed, 0 failed, 3 ignored |
| Architect review | Architect (Claude Sonnet 5.5) | The docs residual was appended to item 6, which is about grants; a blank line was missing before the first new test | Architect moved it to a new item 7 and added the line; fmt and clippy rechecked |
| Tester pass | Tester | None in the implementation. It noted that the new quota test would not fail if the probe ignored the resolved executable (its `argv0` has a slash) | Accepted: section 5 test 3 states it; mutant 3 stays reviewed by reading |
| Tester pass | Tester | A scratch backup file was created in the main checkout by a working-directory mistake | Compared, removed, `git status` confirmed no out-of-scope file |

