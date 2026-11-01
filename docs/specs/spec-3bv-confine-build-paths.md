# Spec 3bv: confinement opens the build-tool paths it finds, by default (F62 step b)

## 0. Status

**LOCKED** (2026-11-01; one consultant review `m_4993` applied, no second pass). Context `a2amx-confine-grants-2026-10-31`. Location: main
checkout, branch `develop`.
Public test seams: `suggest::write_paths`, `team::parse` (merge of confine tables), `confine::plan_with_path`
(with a `ConfineRequest` built directly), the `a2amx team up --dry-run` binary tests in tests/team.rs.

Scope (may edit): `src/suggest.rs`, `src/confine.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/daemon.rs`
(only the directory creation of 2.4), `tests/suggest.rs`, `tests/confine.rs`, `tests/team.rs`, `tests/wire.rs`,
`docs/confinement.md`, `docs/teams.md`, `docs/cli.md`, `a2amx.toml.example`.
Does not touch: the grant table (`src/grant.rs`, `tests/grant.rs`), alias checks, `CHANGELOG.md`,
`docs/backlog.md`, other specs.

## 1. Why

Owner test (F62): a new user turns confinement on with defaults and needs no workaround for a Rust, npm or
Python build. Spec 3bu prints the `write` line, but the user still copies it. This spec makes the copy
unnecessary: the daemon adds the paths itself. `harness-config` as a team default (F62 step c) is left out: a
team-level grant would fail every Generic session (the harness is inferred at launch), and it needs a design of
its own.

## 2. Behaviour

1. `ConfineRequest` gets `build_paths: bool`. Both `Default` and serde default are `false` (tests/wire.rs
   asserts `{}` equals `ConfineRequest::default()`); an older client behaves as before. `a2amx new --confine`
   (src/main.rs ~499) and `team::merge_confine` (src/team.rs ~670) set it to `true` unless opted out with
   `--confine-no-build-paths` or `build_paths = false` (a `ConfineTable` key, session overrides team, like
   `default_hides`). `own_has_options` and `has_confine_options` count the new option, so it needs confinement
   enabled, as `default_hides` does.
2. `confine::plan_with_path`, when `request.build_paths` is true, runs `suggest::detect` on the session's working
   directory (marker files in that directory only) and takes `suggest::write_paths(found)` (one new function; the
   existing loop in `render`, `src/suggest.rs` lines 105-116, calls it too).
3. Detected paths are NOT appended to `request.write`. Entries of `request.write` become explicit operator writes
   that release default hides (`bind_operator_writes`, `release_operator_writes`), so a symlink such as
   `~/.cargo/registry` -> `~/.ssh` would open a hidden directory by default. Instead a separate step binds each
   detected path writable with `explicit = false`, outside `explicit_writes`, and skips (never errors, never
   propagates `?` from `check_write_allowed`) a path that is a symlink, whose canonical form is on, inside or
   above a hidden path, or that `check_write_allowed` or `check_write_runtime_allowed` refuses. A skip is pushed to
   `Plan.unresolved` with option `"build"` and the reason, shown by the dry run as the existing `unresolved:` line.
4. A detected path that does not exist is created (`create_dir_all`) when its nearest existing ancestor passes the
   same skip rule and lies inside the home directory; the first `cargo build` on a fresh machine would otherwise
   still fail because the parent is read-only. Creation runs where the daemon already creates the harness data
   directories (`src/daemon.rs` near `ensure_data_dirs`, ~line 398), before `plan`; a failure becomes an
   `unresolved` note, not a launch failure.
5. `Plan` gains `build_writes: Vec<PathBuf>`; the dry run and the agent notice print them as
   `writable: PATH (from build files)`.

## 3. Out of scope

`harness-config` as a team default, presets by name, a launch error naming the missing `write` line, `gh` push
without flags, Go and Python detection beyond today's table, any new grant, `flag_sessions` (the `team up` flag
form) unless it already passes through `merge_confine`.

## 4. Acceptance

Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` (762 passed, 0 failed, 3 ignored at `9b6db8b`),
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.

Tests that must change: exhaustive `ConfineRequest` literals in tests/team.rs (~2817) and tests/wire.rs (~972) get
`build_paths`. New tests:
- `plan_with_path` with `build_paths: true` and a `Cargo.toml` cwd lists `~/.cargo/registry` in `build_writes`
  (new behaviour, fails on the old code).
- Abuse case: `~/.cargo/registry` a symlink to a hidden `~/.ssh` is skipped, `~/.ssh` stays hidden, an
  `unresolved` note with option `build` appears (fails if the path is appended to `write`).
- A missing detected path under a tempdir home is created; one whose ancestor is a symlink is not.
- `team::parse`: `build_paths` is true by default, false with `build_paths = false` at team and at session
  level; with `enabled` unset the option is an error. `team up --dry-run` (as at tests/team.rs ~1547) shows
  `writable: ... (from build files)` for a team whose cwd has a `Cargo.toml`.
- wire: `{}` still equals `ConfineRequest::default()`.
- `a2amx new --confine` default is not testable without a daemon; say so in the report.

Mutation check on a scratch copy of the tree: append to `write` instead of the separate step, drop the symlink
skip, flip the team default, drop `create_dir_all`; name the failing test for each; the checkout matches its own
diff afterward.

Docs: say the recipe is no longer copied by hand, name the opt-out, and keep the 3bu text for ecosystems with no
default path.

## 5. Claims verified (consultant review, 2026-11-01)

| claim | where | verified |
| --- | --- | --- |
| `plan_with_path` takes request and cwd | src/confine.rs:835 | read |
| explicit writes release default hides | src/confine.rs:1103-1118, :1306-1311 | read by consultant |
| an unresolved path never refuses a launch | src/daemon.rs:391-399, docs/confinement.md:34,48,362 | read |
| `{}` equals `ConfineRequest::default()` | tests/wire.rs:1026-1028 | read |
| old clients and daemons ignore the field | src/wire.rs:148-151 | read |
