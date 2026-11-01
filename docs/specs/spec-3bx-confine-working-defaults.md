# Spec 3bx: confinement defaults under which agents work as usual (F62 steps b and c)

## 0. Status

**LOCKED** (2026-11-02; one consultant review `m_5035` applied, no second pass). Context
`a2amx-confine-defaults-2026-11-02`. Location: main checkout, branch `develop`.
Public test seams: `suggest::default_write_paths`, `confine::plan_with_path` / `confine::plan` with a
`ConfineRequest` built directly (`Plan::hidden_paths`, `Plan::wrap`, `Plan::notice`), `grant::all`,
`team::parse`, the `a2amx confine list` binary test in tests/grant.rs, the `team up --dry-run` binary test in
tests/team.rs.

Scope (may edit): `src/suggest.rs`, `src/confine.rs`, `src/grant.rs`, `src/main.rs` (only the dry-run label of 2.6),
`tests/suggest.rs`, `tests/confine.rs`, `tests/grant.rs`, `tests/team.rs`, `docs/confinement.md`, `docs/teams.md`.
Does not touch: `src/cli.rs`, `src/daemon.rs`, `src/wire.rs`, `src/team.rs` (the team-level check already reads
`team_default_ok`), `a2amx.toml`, `CHANGELOG.md`, `docs/backlog.md`, other specs.

## 1. Why

Owner requirement (2026-11-01, 2026-11-02): confinement saves the system from a catastrophic mistake and lets
the agents work as usual; grants are good but setup must be easy. The architect and the owner could not set up a
working team file: the architect's session lacked `~/.cargo/advisory-dbs` (cargo-deny), `SONAR_TOKEN`, a push
path, and the harness login needed an opt-in grant. The model stays an allow-list of writes (a harness directory
must not become an unconfined persistence path), so the fix is a larger default list of directories where a loss
does not matter, not a new model. Credentials stay opt-in, as the owner confirmed: "it is okay if they are not on
by default, since the ssh grant is all or nothing".

## 2. Behaviour

All of this follows the existing `build_paths` request flag (clients set it `true`; `--confine-no-build-paths` and
`build_paths = false` turn it off; the wire default stays `false`). Docs call the flag "tool directories and the
harness's own directory"; renaming it is out of scope.

1. **Common tool directories.** `suggest.rs` gets `COMMON_WRITE_PATHS: [&str; 1] = ["~/.cache"]` and
   `pub fn default_write_paths(found) -> Vec<&'static str>` returning the common paths followed by
   `write_paths(found)`. The Rust ecosystem's path changes from `~/.cargo/registry` to `~/.cargo` (registry, git
   checkouts, `advisory-dbs` for cargo-deny). `PlanBuilder::bind_build_writes` and `ensure_build_paths`
   (src/confine.rs:610, :1301) iterate `default_write_paths` instead of `write_paths`. `confine suggest` (`render`)
   keeps `write_paths`; its Rust line changes with the path and no other output changes.
2. **Directories that contain a hide.** `~/.cargo` holds `credentials.toml` and `~/.cache` holds
   `huggingface/token`, both default hides. The skip rule in `bind_build_path` (`paths_overlap(&path, hidden)`,
   src/confine.rs:1354) becomes `is_within(&path, hidden)`: a path on or inside a hidden path is skipped, an
   ancestor of a hidden path is allowed. The state directory, the runtime directory and the mandatory roots keep
   their own non-explicit overlap checks (`check_write_allowed`, `check_write_runtime_allowed`, src/confine.rs:
   262-306), so a state directory under `~/.cache` still makes `~/.cache` an `unresolved` build path. The nested hide
   survives because `mounts()` sorts mounts by `path_depth` first (src/confine.rs:1733), so a descendant mounts
   after its ancestor (probed 2026-11-02: hide after the bind reads 0 bytes and cannot be written, hide before the
   bind leaks the content). The 3bv abuse case stays covered (see 5).
3. **PATH directories stay read-only.** The rule that makes PATH directories and `~/.a2amx` read-only inside a
   writable directory lives in `bind_operator_writes` (src/confine.rs:1278-1297) and runs before build writes, so
   `~/.cargo/bin` would become writable. Move those two blocks (the `~/.a2amx` block and the PATH loop, not the
   team-file block) into one private method `protect_system_paths(&mut self, path_var: Option<&OsStr>)`, call it
   from `bind_operator_writes` where the blocks are today, and call it again right after `bind_build_writes` in
   `plan_with_path` (src/confine.rs:985-987). `~/.cargo/bin` then stays read-only when it is on `PATH`;
   `cargo install` into it fails and docs say to set `CARGO_INSTALL_ROOT`. Docs also say that `~/.cargo/env` and
   `~/.cargo/config.toml` are writable and later builds run what is in them.
4. **Harness directory by default.** In `plan_with_path`, compute the effective grant list once: the request's
   grants plus `harness-config` when `request.build_paths` is true, the harness is Claude or OMP,
   `harness-config` is not already listed, and no `request.hide` or `request.unhide` entry resolves (`user_path`,
   then canonical form) to exactly the harness root. Pass the effective list to `bind_harness_config`; never add
   it to `Plan.grants`, so the dry run's `grants:` line lists only named grants. An operator hide or unhide of
   exactly the root skips the implicit grant and records
   `Unresolved { option: "harness-config", path: root, reason: "operator hide or unhide of the harness root" }`
   (the dry run prints it; the notice has no unresolved lines and says nothing). Generic sessions never compute the
   implicit grant, so there is no error for them. An explicit `harness-config` grant keeps every current error.
   The notice line at src/confine.rs:2152-2156 prints `(grant harness-config)` only when `"harness-config"` is in
   `plan.grants`, and `(default for this harness)` otherwise. `no_default_hides` makes `~/.claude/ide` readable and
   writable, as with the explicit grant; docs say so.
5. **Team-level credential grants.** `team_default_ok` becomes `true` for `gh`, `ssh` and `git-credentials` in
   src/grant.rs; the container grants and `harness-config` stay session-only. `confine list` prints "team or
   session" for those three. A team's `[confine] grants = ["gh", "ssh"]` then applies to every session.
6. **Label.** The notice (src/confine.rs:2120-2124) and the dry run (src/main.rs ~1173-1178) print
   `writable: PATH (tool directory)` instead of `(from build files)` for each `build_writes` entry, because
   `~/.cache` is not from build files.
7. **Docs.** `docs/confinement.md` (lines 36, 56-58, 74, 79-80, 85, 126, 132-143, 401): what is writable by default
   and how to opt out; the grant table's third column ("Yes; ..." for the three credential grants); the registry
   recipe becomes the `~/.cargo` default; the limits: `CARGO_HOME` and `XDG_CACHE_HOME` set elsewhere are not
   honoured, `cargo install` into a PATH `~/.cargo/bin` fails, `mv` or `rm` of `~/.cache/huggingface` acts on the
   host (the token file itself stays hidden), a state directory under `~/.cache` makes `~/.cache` unresolved.
   `docs/teams.md` lines 114-121: team-level `gh`, `ssh` and `git-credentials` are allowed, `keep_env` and the other
   grants are not; a team-level credential grant gives every session in the team that access.

## 3. Out of scope

Renaming `build_paths`, a launch error that names the missing `write` line (F62 step d), the inversion to a
writable home, `SSH_AUTH_SOCK` pass-through (F63), npm or pip directories beyond the existing detection, Go paths,
alias hardening (F61), read-only `~/.cargo/env` and `config.toml`, `a2amx.toml` (the owner's team file), changelog
and backlog edits.

## 4. Attack paths (trust-boundaries)

| Path | Disposition |
| --- | --- |
| A symlinked or aliased default directory opens a hidden one | Constrained: `symlink_component` and the on-or-inside-hidden skip stay; contract test |
| A writable `~/.cargo` or `~/.cache` exposes the hidden credential file inside | Removed: the descendant mounts after its parent (depth sort); test asserts argv order and a real-bubblewrap read of an empty file |
| A writable `~/.cargo` lets an agent replace `~/.cargo/bin` tools that the user runs later | Removed for PATH directories: `protect_system_paths` runs after build writes; test |
| The harness directory is writable by default, so an agent can plant a hook that later runs unconfined | Accepted by the owner ("the agent's config dir"); documented, no tool-output warning |
| A team-level credential grant reaches sessions that do not need it | Accepted by the owner (explicit opt-in line); documented |
| `~/.cargo/config.toml` or `env` altered to run code in later builds | Accepted, documented (3) |

## 5. Acceptance

Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` (772 passed, 0 failed, 3 ignored at `9d3ad87`),
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Run before and after.

Existing tests that must change:
- tests/grant.rs: `team_table_rejects_ssh_grant` (:180) becomes
  `team_table_accepts_credential_grants_and_rejects_the_rest`; `grant_table_has_expected_rows` (:6) and the
  `assert!(!row.team_default_ok)` (:108) take a per-row flag; the loop at :201-203 (`.find("session only")` for
  every row) and the rows for `gh`, `git-credentials`, `ssh` (:216-224) read "team or session".
- tests/team.rs: `confine_table_rejects_old_keys_and_team_grants` (:2927-2935) expects a team-level `gh` to be
  accepted now (the `docker` and `harness-config` halves at :2914-2926 stay); the dry-run test at :1713-1719 expects
  `writable: <home>/.cargo (tool directory)` and `~/.cache`.
- tests/confine.rs:3812-3910 (3bv): the cargo-cache tests expect `~/.cargo` and `~/.cache`; the
  registry-symlink-to-`~/.ssh` test (:3845-3884) is rewritten: `~/.cargo/registry` is no longer a candidate, so the
  assertions become "`~/.ssh` stays in `hidden_paths()` and has no writable or bind window in `plan.wrap()`"; in
  the `~/.cargo`-symlink variant (:3887-3910) replace the vacuous `!target.join("registry")` assert with "`~/.cargo`
  is not created under the symlink target".
- tests/suggest.rs:41 and :60 (`write_paths` order and the rendered `write = ["~/.cargo/registry", "~/.npm"]`).
- tests/confine.rs:4453 and :4482 (explicit `harness-config` notice text) pass unchanged; do not edit them.

New tests (each fails on the old code unless marked contract):
- `suggest::default_write_paths` returns `~/.cache` first, then `~/.cargo` for a `Cargo.toml` marker.
- `plan_with_path` with `build_paths: true` and a fixture home with `.cargo/credentials.toml`, `.cache`,
  `.cache/huggingface/token`: both directories are writable and both files are hidden; in `plan.wrap()` each
  `--ro-bind <empty> <file>` window comes after the `--bind <dir> <dir>` window of its parent.
- With `.cargo/bin` on the `path_var` passed to the planner, `.cargo/bin` is read-only (in the protected list) while
  `.cargo` is writable; without it on `PATH` it is not.
- A state directory fixture under `.cache` makes `~/.cache` an `unresolved` build path (contract).
- Real bubblewrap (Host-fixture pattern near tests/confine.rs:4405, `Harness::Generic`, skip rules of the 3bw test):
  `cat <home>/.cargo/credentials.toml` prints nothing and `echo x > <home>/.cargo/new` succeeds.
- Claude and OMP plans with `build_paths: true` list the harness directory writable without any grant, with
  `plan.grants` empty and the notice saying `(default for this harness)`; a Generic plan does not fail;
  `build_paths: false` with no grant leaves it read-only (contract); an `unhide` of the harness root skips the
  implicit grant and the dry run shows the `harness-config` unresolved line; an explicit grant plus `unhide` of the
  root still fails the launch (contract).
- `team::parse` accepts `gh`, `ssh`, `git-credentials` in the team table and still refuses `docker` and
  `harness-config` with the existing message (every `[[session]]` in the fixture needs a name or role).

Mutation check on a scratch copy: restore `paths_overlap`; drop `~/.cache` from the common list; drop the
implicit harness grant; skip the second `protect_system_paths` call; flip `team_default_ok` back for `ssh`; reverse
the depth sort; name the failing test for each; the checkout matches its own diff afterward.

Live check by the owner after the install: a session started with only `[confine] enabled = true` builds, runs
`cargo deny`, and renews its login.

## 6. Claims verified (architect and consultant, 2026-11-02)

| claim | where | verified |
| --- | --- | --- |
| hide after a writable ancestor works; before it, the content leaks | `bwrap --bind D D --ro-bind EMPTY D/f` vs the reverse | probed by the architect |
| mounts are ordered by depth, so a descendant mounts after its ancestor | `path_depth`, src/confine.rs:227, :1733 | read by the consultant |
| an explicit write of an ancestor does not release a nested default hide | `release_operator_writes` matches exact paths, src/confine.rs:1531 | read |
| the skip rule that blocks `~/.cargo` today | `paths_overlap(&path, hidden)`, src/confine.rs:1354 | read by the consultant |
| state dir, runtime dir and mandatory roots are refused separately for ancestors | src/confine.rs:262-306 | read by the consultant |
| PATH and `~/.a2amx` protection runs before build writes | `bind_operator_writes` 1278-1297 at plan step 978, build writes at 985-986 | read by the consultant |
| default hides include `~/.cargo/credentials*` and `~/.cache/huggingface/token` | src/confine.rs:70-105 | read |
| `bind_harness_config` runs before operator hides; `finish()` bails on an exact root hide or unhide | src/confine.rs:977, 1575, 1581 | read by the consultant |
| a Generic session with the grant is an error today | src/confine.rs:1201 | read by the consultant |
| the notice prints every `build_writes` entry as "(from build files)" | src/confine.rs:2120-2124, src/main.rs 1173-1178 | read by the consultant |
| the team-level check reads `team_default_ok` | src/team.rs:619-626 | read by the architect |

## Review record

(Appended at acceptance.)
