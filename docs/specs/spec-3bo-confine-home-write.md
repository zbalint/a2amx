# Spec 3bo: `write = ["~"]` is allowed, the launch inputs stay read-only inside it, the residuals are documented

## 0. Status

**LOCKED** (2026-10-31, one consultant review `m_4787` applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F49, second part). Architect-directed, before 0.2.0.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `2c43214` (after spec 3bn); full suite 727 passed, 0 failed, 3 ignored; run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::plan`, the new `confine::plan_with_path` (section 3), `confine::check_write_allowed` (`tests/confine.rs`), `Plan::wrap` argv (`tests/confine.rs`).

**Scope.** May edit exactly these files and no others: `src/confine.rs`, `tests/confine.rs`, `docs/confinement.md`.
Does not touch: `a2amx.toml`, `CHANGELOG.md` (the architect edits it), every other file. No new dependency. Leave the diff
uncommitted. Re-anchor every line number against the tree after spec 3bn; the anchors below are from before it.

## 1. Why

Confinement is a seatbelt, not a sandbox: the defaults are strict and the user may open anything on purpose except `/`.
Today `write = ["~"]` is refused (`writable path $HOME is not allowed`), so a user who wants an agent that works almost as if
unconfined must list every directory. The owner's decision: allow it. Two things must hold when it is allowed. The hides
(`~/.ssh`, the state directory, the default list) stay mounted on top of the writable home, because deeper mounts win. And
the programs that the daemon itself runs outside the sandbox stay read-only inside it unless the user names them exactly. The team
file and the session's git `hooks`/`config` already do. `~/.a2amx` (the installed `a2amx`) and every directory of the daemon's
`PATH` that lies inside the home join them: the daemon finds `bwrap` and the `omp` it polls through `PATH` (`src/daemon.rs:366`,
`:1508-1520`), and `~/.local/bin` comes first in the owner's `PATH`, so an agent that could create `~/.local/bin/bwrap` would be running
code outside the sandbox on the next confined start.

## 2. Allowing the home directory (`src/confine.rs`)

`check_write_allowed` (`:230-253`) is public and used by the harness data (`validate_harness_data_paths`, `:415`) and, through
`add_validated_write` (`:611`, shared by the workspace binding and the operator `write` entries), by the workspace. It gains one
parameter, `explicit: bool`, true only for operator `write` entries (`bind_operator_writes`); `add_validated_write` and
`check_write_runtime_allowed` (`:255`) take it too:

- `/` stays refused for every caller.
- `path == home` or a path that **contains** the home: refused unless `explicit`. A session started with the home directory (or
  `/home`) as its working directory still gets `writable path $HOME is not allowed` (the workspace binding and the harness data pass
  `explicit = false`): opening the whole home is a choice
  the user makes in `write`, not a side effect of where they launched.
- The state directory and the runtime directory: with `explicit`, a path that **contains** them (an ancestor, for example
  the home) is allowed because they stay hidden on top; a path equal to them or inside them is refused as today. Without
  `explicit` the current overlap refusal is unchanged.
- The mandatory roots (`/run/WSL`, `/mnt/wslg`): the same rule, an explicit ancestor is allowed, equal or inside is refused.
  `check_write_runtime_allowed` follows the same rule.

For an explicit ancestor the hide mounts of the state directory, the runtime directory, the mandatory roots, the default
hides and the container sockets stay in `hidden`, so `mounts()` (deeper mount wins, `:1060-1100`) places them over the
writable ancestor. `release_operator_writes` only lifts hides whose path **equals** an explicit write entry, so a write of `~` lifts none
of them. A test pins this (section 5).

## 3. Launch inputs stay read-only (`src/confine.rs`)

`plan()` becomes `plan(...) = plan_with_path(..., std::env::var_os("PATH"))`, and `pub fn plan_with_path(request, harness, cwd, host,
path_var: Option<OsString>) -> Result<Plan>` is the new seam, so a test passes its own `PATH` without touching the process
environment. In `bind_operator_writes`, add to `readonly`, exactly as the team file is added (only when the path lies inside a
writable area, so a plan without a writable home gets no extra mount): `~/.a2amx` when it exists, and every directory of `path_var` that
exists, is absolute, and whose canonical path lies inside the canonical home. The existing `readonly.retain(|path|
!explicit_writes.contains(path))` (`:995`) lifts any of them when a `write` entry names that exact directory. Nothing else is added: the git hooks
and config and the team file are already handled, the OMP extension lives under the hidden state directory. A custom
`A2AMX_INSTALL_DIR` is covered only when it is on the `PATH` (document it).

## 4. Docs (`docs/confinement.md`)

- The Read-only row (line 10) lists `~/.a2amx` with the team file and the git files.
- The advice at line 71 ("add the directory named in the error, not `$HOME`") keeps its point but no longer says home is refused:
  it says what `write = ["~"]` means (everything under home writable except the hides and the protected launch inputs) and that it
  is the user's choice.
- The Read-only row also lists the `PATH` directories inside the home. A new section, "What stays open", listing the residuals plainly: (1) the git history is writable inside the writable git
  directory, so an agent can rewrite commits there; (2) the harness login files it needs (for example `~/.claude/.credentials.json`
  for Claude) are writable and readable by the harness it runs, and so are `~/.claude/settings.json` hooks the user's next unconfined run executes;
  only this session's repository `hooks`/`config` and team file are read-only: every other repository's `.git/hooks` and `.git/config` and every other
  `a2amx.toml` under a writable home are writable, so are the Windows drives under `/mnt` if `/mnt` is opened; (3) the hides are decided when the session starts, a file
  created later at a hidden path is not hidden; (4) network access is not restricted; (5) a writable home lets the agent edit
  shell startup files and editor configs that the user's next unconfined run executes, so open it knowing that; (6) a grant opens
  what its row says and no more. The page still parses (`tests/docs_examples.rs`).

## 5. Tests (`tests/confine.rs`)

1. `write_home_is_allowed_and_keeps_hides_on_top`: the fixture's state, runtime, home and workspace directories are siblings under
   one root, so open the ancestor: a request with `write: [<fixture root>]` plans without error; the state directory, the runtime
   directory, the fixture `~/.ssh` (a directory, tmpfs) and the fixture `~/.netrc` (a file, empty mount; created so both are hidden)
   stay hidden, and in `Plan::wrap` argv the index of each hide mount is greater than the index of the `["--bind", root, root]`
   mount (canonical fixture paths). A second case with `write: ["~"]` asserts the home is writable and `~/.ssh` and `~/.netrc` are
   hidden after it.
2. `write_home_keeps_the_launch_inputs_read_only`: with `~/.a2amx` and `~/.local/bin` created in the fixture home and
   `plan_with_path(..., Some("<fixture home>/.local/bin:/usr/bin"))`, `write: ["~"]` leaves both in `protected`; `/usr/bin` is not added;
   with `write: ["~", "~/.a2amx"]` only `~/.a2amx` is lifted; with `write: ["~", "~/.local/bin"]` only the `PATH` directory is
   lifted; a relative or empty `PATH` entry is ignored.
3. `write_slash_and_inside_state_dir_stay_refused`: `write: ["/"]` fails; `write` of the state directory and of a directory
   inside it fail; `write` of the runtime directory fails; contract tests (the old code refuses all, so none is red), plus the
   existing assertions that stay.
4. `workspace_in_home_is_still_refused`: a session whose working directory is the fixture home fails with
   `writable path $HOME is not allowed` even though `write: ["~"]` would be accepted; so does a working directory that contains
   the home (a fixture whose state directory lies elsewhere, so the state-directory overlap does not fire first) (abuse case:
   where it is launched never widens the sandbox).
5. Existing tests the signature change touches: `tests/confine.rs:1216` (`check_write_allowed(state)` error) and `:1218-1224`
   (`check_write_allowed(state.parent())` error) pass `explicit = false` and keep their meaning; add the `explicit = true` pair
   (ancestor of the state directory allowed, the state directory itself and a path inside it refused); `:1225` (runtime
   `Ok`) and `:1628` (state symlink error) are unchanged; the plan-level refusals at `:1256` (`/`), `:1268` (state), `:1281`
   (runtime) stay. No existing test asserts the `$HOME` refusal or writes `~`, so test 4 is new, not an update.

## 6. Out of scope

New default hides, the WSL probe, env credentials (spec 3bn), grants, any change to `unhide`'s refusal of home
(`check_unhide_allowed` stays: unhiding the home would expose the state directory), `~/.ssh` handling, shell startup file
protection (documented residual, the owner chose the Linux philosophy).

## 7. Trust boundaries

The `write` list is client-controlled input to `plan()`. Allowing the home as an explicit entry opens only what the user wrote
in their own configuration; the hides and the launch inputs are re-applied on top inside `plan()`, the one place every
entry point passes through. Attack path: a wire client passes the working directory as home to widen the sandbox. Disposition:
constrained (`explicit = false` for the workspace binding); abuse-case test is test 4.

## 8. Placement

`check_write_allowed` and the mount plan are `confine.rs`'s job; the new rule is a parameter of the existing check, not a new
function. The protected-launch-inputs list belongs next to where the team file is added.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
rg -n 'check_write_allowed|add_validated_write|check_write_runtime_allowed' src tests   # every caller passes the new argument
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent`: rerun once. Mutants for
the tester: (1) `explicit` ignored (home always allowed, then test 4 must kill); (2) `explicit` always true; (3) ancestor rule
also allows a path inside the state directory; (4) `~/.a2amx` or the `PATH` directories not added to `readonly`; (5) the exact-entry lift removed; (6) a
write of `~` lifts the default hides; (7) `/` allowed when explicit.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `check_write_allowed` refuses `/`, home, and overlaps with the state directory and the WSL roots | `src/confine.rs:230-253` | read |
| `check_write_runtime_allowed` refuses overlap with the runtime directory | `src/confine.rs:255-262` | read |
| The workspace binding calls `add_validated_write` for the cwd and the git common directory | `src/confine.rs:775-781` | read |
| Hides are inserted first and writable paths after; the mount list sorts by depth so a deeper hide wins | `src/confine.rs:1060-1100` | read |
| `release_operator_writes` lifts only hides equal to an explicit write | `src/confine.rs:949-956` | read |
| The team file is added to `readonly` when inside a writable area and lifted by an exact write | `src/confine.rs` (`bind_operator_writes`, `:995`) | read |
| `~/.a2amx` is not referenced in `src` (the only `\.a2amx` hit is the unrelated `mcp_servers.a2amx` in `src/codex.rs`) | `rg -n '~/\.a2amx|"\.a2amx"' src` | grep |
| The daemon finds `bwrap` through its `PATH` at each confined start and runs the `omp` quota probe by `argv[0]` unconfined | `src/daemon.rs:366`, `:1508-1520`, `src/confine.rs:1254-1289` | read (consultant `m_4787`) |
| `git_common_dir` stops at the home directory, so only this session's repository is protected | `src/confine.rs:775-781` | read (consultant) |
| `docs/confinement.md` line 10 and line 71 describe the read-only row and the `$HOME` advice | `docs/confinement.md` | read |
