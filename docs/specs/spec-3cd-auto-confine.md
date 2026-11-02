# Spec 3cd: auto confinement and the default set (confinement redesign step 3)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context
`a2amx-auto-confine-2026-11-02`. Public seams: the `a2amx` binary (`confine suggest`, `team up
--dry-run`), `a2amx::suggest`, `a2amx::grant`, `a2amx::team::parse_with`, `a2amx::confine::plan_*`.

Scope (may be edited): `src/grant.rs`, `src/suggest.rs`, `src/confine.rs`, `src/team.rs`,
`src/main.rs`, `src/cli.rs`, `src/daemon.rs`, `tests/suggest.rs`, `tests/grant.rs`,
`tests/confine.rs`, `tests/team.rs`, `tests/wire.rs`, plus a new `tests/auto_confine.rs`;
`docs/teams.md`, `docs/confinement.md`, `docs/cli.md`, `a2amx.toml.example` (its `# build_paths = true`
line). Does not touch: `docs/backlog.md`, `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md` (the architect edits
them, including the `suggest` row of the module table), `docs/specs/*`, any harness, delivery or
messaging module.

## 1. Why

Confinement is a safety net against accidents, not a sandbox. Users who will never write grants
need one switch that makes it useful. Today the nearest thing is `build_paths`: it detects build
files and opens `~/.cache`, `~/.cargo`, `~/.npm` and others for every confined session, and it
also gates the implicit `harness-config` grant. The owner decided to drop it (nothing but the
working directory and the harness's own config is open by default) and to give non-technical
users `auto = true` in the global team file instead: for every grant that can be detected, auto
enables it. Technical users keep named grants, `write`, `hide` and `unhide`. Auto is judged against
an unconfined agent: a flow auto leaves broken (git push, a first build) makes users switch
confinement off.

Decisions (owner): auto is an explicit opt-in (default false) because it is weaker than explicit
grants; it applies only to the global team-file fallback; a local file or `--file` that sets it is
an error; it always adds `cache`; it enables every grant that has a detector and is allowed at
team level; the git remote is a detector; no `a2amx init`; confinement never shipped, so nothing
here is a breaking change for anyone.

Rejected: a separate auto list apart from the grant table (each grant carries its detector);
excluding credential grants from auto (detectability is the only filter); scanning subdirectories,
parents or globs; session-level auto; per-grant auto opt-out.

## 2. `src/grant.rs`: the table owns detection and directory creation

Add two fields to `Grant` and fill them in the table:

```rust
/// Names in the session's working directory whose presence shows this grant is needed; empty means not detectable.
pub detect: &'static [&'static str],
/// Tool directories created when missing (a subset of `unhides`); never a credential location.
pub create: &'static [&'static str],
```

Values (every other grant: `NO_PATHS` for both):

| grant | detect | create |
| --- | --- | --- |
| cargo | `Cargo.toml` | `~/.cargo` |
| go | `go.mod` | `~/go`, `~/.cache/go-build` |
| python | `pyproject.toml`, `requirements.txt` | `~/.cache/pip`, `~/.cache/uv` |
| node | `package.json` | `~/.npm` |
| java | `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` | `~/.m2/repository`, `~/.gradle` |
| c-cpp | `CMakeLists.txt`, `conanfile.txt`, `conanfile.py` | `~/.cache/ccache` |
| semgrep | `.semgrep.yml`, `.semgrep.yaml`, `.semgrep` | `~/.semgrep` |
| sonar | `sonar-project.properties` | `~/.sonar` |
| cache | none (always added by auto, see §3) | `~/.cache` |

Why `create`: today `ensure_build_paths(create = true)` makes a fresh user's tool directories
exist so the first build can write them; a grant path that does not exist is skipped by
`apply_grant_unhides` (it never creates). Without `create` a beginner on a fresh home gets a
read-only home and `npm install` fails. Credential grants have no `create`. Creating the empty
`~/.semgrep` or `~/.sonar` is not creating a credential: a token appears there only after a login.
A table test in `tests/grant.rs` pins three invariants: a grant with a non-empty `detect` has
`team_default_ok: true`; every `create` entry is one of that grant's `unhides`; `create` is empty for
every grant with a non-empty `env`, for `gh`, `git-credentials`, `ssh`, `aws`, `gcloud`, `azure`, `kube`,
`terraform`, `npm-token`, `pypi-token`, `cargo-token`, `netrc` and `huggingface-token`.

## 3. `src/suggest.rs`: detection, pure

Remove `Ecosystem`, `EcosystemInfo`, `ECOSYSTEMS`, `detect`'s old shape, `write_paths`,
`default_write_paths`, `COMMON_WRITE_PATHS`, `info_for` and the per-ecosystem notes. New API:

```rust
/// Grants whose marker exists, in grant-table order; only grants a team table may name.
pub fn detect(has_file: impl Fn(&str) -> bool) -> Vec<&'static str>;
/// Grants a git config's remotes call for, deduplicated, in grant-table order: an ssh-form remote -> ssh; an `https://`
/// remote -> gh and/or git-credentials, each only if `has_host_path` says its location exists
/// (`~/.config/gh`, `~/.git-credentials`, `~/.config/git/credentials`). The forms are pinned below.
pub fn git_remote_grants(config: &str, has_host_path: impl Fn(&str) -> bool) -> Vec<&'static str>;
/// What auto adds and `confine suggest` prints: `cache` plus detect plus git_remote_grants, deduplicated, in grant-table order.
pub fn grants(has_file: impl Fn(&str) -> bool, git_config: Option<&str>, has_host_path: impl Fn(&str) -> bool) -> Vec<&'static str>;
/// The `confine suggest` text: one `grants = [...]` line with a short header; no per-ecosystem notes.
pub fn render(found: &[&'static str]) -> String;
```

`git_remote_grants`: scan the lines of the config text, trimming each; track the current section header;
a key counts only under a header that starts with `[remote `. The key is `url` or `pushurl`, matched
case-insensitively, with or without spaces around `=`, and the value has surrounding double quotes
stripped. Other sections, `insteadOf` and `include` are ignored; every remote counts. A value is
`ssh` when it starts with `ssh://` or `git+ssh://`, or has no `://` and contains `:` (the scp form,
with or without a user); it is `https` when it starts with `https://`; anything else (`http://`,
`file://`, a local path) adds nothing. `ssh` adds `ssh`; `https` adds `gh` if
`has_host_path("~/.config/gh")` and `git-credentials` if `has_host_path("~/.git-credentials")` or
`has_host_path("~/.config/git/credentials")`. Detection uses grant-name constants; every name used
must pass `grant::lookup`. Known limit, stated in the docs: when `.git` is a file (a worktree or a
submodule) no git grant is detected, so a push needs an explicit grant there. `has_host_path` reads
the user's own home and is trusted input.

## 4. `src/confine.rs`: drop build paths, harness-config becomes the default

- Remove `ConfineRequest.build_paths` (line ~117), `ensure_build_paths` and its build half,
  `PlanBuilder::bind_build_writes`, `bind_build_path`, `allow_missing_build_writes`, `build_writes`
  and every use that exists only for them. `~/.cache` and tool directories open only through
  grants.
- Add `pub fn ensure_grant_dirs(request: &ConfineRequest, harness: Harness, host: &Host, create: bool) -> Vec<Unresolved>`
  replacing `ensure_build_paths` at its three callers (the harness is needed for the default hide
  sources, `default_hide_sources_for(&home, harness)`, as in the old function): for each granted grant and each path in its
  `create`, if the path does not exist and `create` is true, create it with `create_dir_all`;
  keep the checks the old function made for a missing path (inside the home, no symlink component,
  an ancestor that is writable, not hidden or under the state or runtime directory), recording a
  skipped path as `Unresolved` with option `grant`.
- The implicit `harness-config` grant (lines ~978 to ~1009) no longer depends on a request field:
  it applies to every Claude and OMP session, keeping the operator hide/unhide exception and the
  `Unresolved` entry unchanged.
- The `protect_system_paths` call that spec 3by moved so it runs when `build_paths` is false now
  always runs once; `bind_build_writes` was inside the other block and is gone.

## 5. `src/team.rs`: the `auto` key

- `ConfineTable`: remove `build_paths`; add `auto: Option<bool>`.
- `merge_confine` gains a parameter `global: bool` (true when `parse_with` has a `default_team`).
- `auto` in `[session.confine]`: error `session {name}: auto belongs in the team's [confine] table, not [session.confine]`.
- `auto = true` when `global` is false: error `[confine] auto is only read from the global team file; run a2amx confine suggest and paste the grants line into your team file`.
- `auto = true` without `enabled = true` at team level: error `[confine] auto needs enabled = true`.
- `auto = false` is accepted everywhere and means nothing.
- `TeamSession` gains `#[serde(skip)] pub auto: bool`, set from the team table when `global`. The
  team-level `enabled` and session-level `enabled = false` interplay is unchanged: a session whose
  merged confinement is disabled gets `confine = None` and auto does nothing for it.
- `own_has_options` no longer mentions `build_paths`; remove that key from `merge_confine`.

## 6. `src/main.rs`, `src/cli.rs`, `src/daemon.rs`

- `cli.rs`: remove `--confine-no-build-paths`. `main.rs` lines ~429, 454, 506, 666: remove its plumbing.
- `team up` builds a session's confinement request in three places (`validate_team_session`
  main.rs ~1232, `dry_run_start` ~1103 and `start_planned_session` ~1324, which builds the request
  sent to the daemon), and `new --confine` builds one at ~511. `resolved_confine_request` keeps its
  signature and none of its callers changes. Auto therefore runs once, as a pre-pass in
  `run_team_up` (main.rs ~1356): after `team::plan` returns the actions and before the validate loop
  (~1393), for each `Action::Start` session (the boxed session inside the action; `plan` clones it,
  team.rs ~901) with `auto` true and `confine` `Some`, resolve its directory (the session's `cwd`
  joined to the team file's directory, else the invoking directory, the rule at ~1095), call one new
  helper `auto_grants_for(dir, home)` in `main.rs`, and append each returned name not already in
  `confine.grants`. Run it in `spawn_blocking`. Validation, the dry run, the real start and the
  daemon's stored request then all see the same grants. `team::plan` compares only the session name,
  whether it exited and the team scope, never a confinement request, so a session that is already
  running is not scanned, prints no line and keeps the grants it started with; a changed directory
  takes effect after `team down` then `team up`. `team up <items>` (flag sessions, no team file) never
  has `auto`, and `team down`, `team status` and `team reset` read only session names, so none of
  them needs the pre-pass.
- `auto_grants_for(dir: &Path, home: &Path) -> Vec<&'static str>` is the only code that touches
  the filesystem for detection: `has_file = |name| dir.join(name).exists()`; the git config text is
  read only if `dir/.git` is a directory and `dir/.git/config` is a regular file, by opening it and
  reading at most 65537 bytes (more than 65536, or any read error, means no git detection and is
  not an error); `has_host_path` resolves `~/` against `home`. It calls `suggest::grants`. Both the
  pre-pass and `Command::Confine { Suggest }` (main.rs ~86 to ~103) call it; `confine suggest`
  prints `suggest::render` of the result.
- When the pre-pass added grants to a session, print one line to stderr once per session:
  `auto confinement for session {name}: grants {a}, {b}`. `team up --dry-run` lists the grants in its
  normal `grants:` line with no extra code, because they are in the request.
- A Codex session in a team whose `[confine]` has `enabled = true` is refused by existing code
  (`team.rs` ~1030); auto changes nothing about that, and the docs example for a global file puts
  `[session.confine] enabled = false` on a Codex session.
- A fresh-home dry run lists a tool directory the real start will create as skipped, because the
  dry run plans with `create = false` and the daemon with `create = true`; this already held for
  build paths and is not changed.
- The three `ensure_build_paths` callers (main.rs ~1111 and ~1241, `daemon.rs` ~395) call
  `ensure_grant_dirs` (the daemon with `true`, the others `false`).
- Reset and respawn do not rescan: the grants are in the stored request.

## 7. Docs

`docs/confinement.md` (lines 36 to 60, 160 to 185, and the recipes that say build paths cover Rust,
Node, Maven or Gradle): rewrite around the default set (working directory, harness config,
private `/tmp`; the rest read-only; credentials hidden), say the recipes now name grants, and add
a section "Auto mode" that states: `auto = true` in the global team file only; it is weaker than
explicit grants; what it adds (`cache`, detected toolchains and analysers, `ssh` or `gh` or
`git-credentials` from the git remote); a marker file or `.git/config` in a directory you run
`team up` in opens those grants, and its contents run in your own builds outside the session; a
git push works under auto only when the remote is detected (a worktree or submodule, where `.git`
is a file, and a local team file need an explicit grant), and an https push through the `gh` grant also needs
`gh` set as the git credential helper (`gh auth setup-git`); for real isolation use a container or VM. `docs/teams.md`
(lines 110 to 128) and `docs/cli.md` (line ~80, `confine suggest`) drop `build_paths` and the
flag, and describe `auto` and `confine suggest` output. The `confine list` text is unchanged.

## 8. Out of scope

`a2amx init`; new grants (gpg, dotnet, ruby, php, glab, version managers); detectors for cloud,
Kubernetes, registry token and Hugging Face grants; scanning subdirectories, parents or globs;
session-level or per-grant auto; the stored-token follow-up (backlog U9); the backlog row, the
changelog line and the version (architect). The old `build_paths` behavior is removed, not
migrated.

## 9. Acceptance

Baseline command (run before and after): `env -u A2AMX_BIN -u NO_COLOR cargo test`, then
`cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`; all must pass with no warnings.
Also `scripts/prepush-fast.sh`.

Tests, through public seams, literal expected values (new unless marked contract):

1. `suggest::detect`: Cargo.toml gives `["cargo"]`; `go.mod` and `package.json` give `["go", "node"]`
   in table order; each java marker gives `["java"]`; `.semgrep` and `sonar-project.properties`
   give semgrep and sonar; nothing gives `[]`. Table invariants (section 2) are tests in `tests/grant.rs`.
2. `suggest::git_remote_grants`: `git@host-a:team/repo.git` and `gitea@host-a:x.git` and
   `host-a:repo` give `["ssh"]`; `ssh://` and `git+ssh://` the same;
   `https://example.invalid/a.git` with both host paths present gives `["gh", "git-credentials"]`,
   with neither gives `[]`; `https://` with only `~/.config/gh` gives `["gh"]` and with only
   `~/.config/git/credentials` gives `["git-credentials"]`; `pushurl` counts; `url=git@host-a:x.git`
   without spaces counts; a quoted value counts; an ssh remote plus an https remote with `~/.config/gh`
   present gives `["gh", "ssh"]` (table order); `insteadOf` and an unrelated `[url]` section add nothing;
   `http://`, `file://` and a local path add nothing.
3. `suggest::grants` contains `cache`, has no duplicates and is in grant-table order; `suggest::render`
   on `["cargo", "cache"]` (table order) is a fixed literal; empty input is a fixed literal.
4. `team::parse_with`: with a default team, `[confine] enabled = true, auto = true` yields sessions
   with `auto == true`; without a default team the same file errors with the exact text in section 5;
   `[session.confine] auto` errors; `auto = true` without `enabled` errors; `auto = false` parses
   everywhere and leaves `auto == false`.
5. `confine::plan_*` (new tests, not contracts: the old code opened `~/.cache` whenever `build_paths`
   was true): no `build_paths` field exists; a Claude and an OMP plan list the harness config
   directory writable by default; a plan with no grants leaves `~/.cargo` and `~/.cache` read-only or
   hidden; `cache` opens `~/.cache`.
6. `ensure_grant_dirs(request, harness, host, create)`: with `create = true`, a fresh temp home and the `node` grant make `~/.npm`
   exist; with the `aws` grant `~/.aws` is not created; with `create = false` nothing is created; a
   symlinked `~/.npm` is recorded `Unresolved`, not created through.
7. Integration `tests/auto_confine.rs` (real daemon, temp state home, port 0, temp directory with a
   `Cargo.toml`, no local team file, global `team.toml` with `[confine] enabled = true, auto = true`):
   `team up --dry-run` lists `cache` and `cargo` for the session and prints the stderr line; without
   the marker it lists `cache` only; with `auto` absent it lists neither; a local `a2amx.toml` with
   `auto = true` and `--file` pointing at the same global file with `auto = true` each fail with the
   section 5 error. A `.git/config` larger than 65536 bytes adds no git grant and does not fail. A real
   `team up` (not a dry run) of the same setup stores a request whose grants equal those the dry run
   listed (read through an admin list request or the session's confinement listing), the stderr line
   prints once on a dry run and once on a real up and names the team-prefixed session name, and a second
   `team up` with the session running prints no auto line.
   A directory with `.git` as a file adds no git grant.
8. `a2amx confine suggest <dir>` prints the render of the same grants.
9. Abuse cases (section 10): tests 4 and 7 (local file, `--file`, session-level) fail on the old code
   only because the key does not exist; they are new contract tests of the refusal.

Test changes: every one of the 12 functions in `tests/suggest.rs` changes (four are binary contract
tests of `confine suggest` output, lines ~140 to ~203, and are rewritten for the grants line);
`tests/wire.rs:1006` has a `ConfineRequest` literal that stops compiling and drops the field; about 43
`build_paths` lines in `tests/confine.rs` and 10 in `tests/team.rs` (including tests named for build,
cache or harness-config) are removed or rewritten, and tests that set `build_paths: false` only to
avoid it drop that field. The developer lists removed and rewritten tests in the report; the architect
reads the list at review.

Mutation check (developer, in a scratch copy of the final tree; name the failing test per mutant):
change a detect marker; drop the `cache` addition; allow auto when `global` is false; skip the
64 KiB cap; create a credential directory; drop the harness-config default.

## 10. Trust boundaries

Untrusted input: the repository's file names and `.git/config` text in the directory where `team up`
runs through the global fallback. Paths: (a) a planted marker opens a toolchain grant: accepted,
explicit opt-in, documented in section 7; (b) a planted remote opens `ssh`, `gh` or
`git-credentials`: accepted for the same reason; (c) a huge or special `.git/config`: constrained,
read only if a regular file, through a read capped at 65537 bytes (a larger, growing, swapped or
unreadable file means no detection), so a symlink to a device is rejected as not regular; (d) a
repo-shipped local team file enabling auto for itself: removed, the key is an error outside the global
file.

## 11. Claims (draft, to verify before lock)

| claim | where | verified |
| --- | --- | --- |
| `suggest` maps markers to raw paths, `default_write_paths` adds `~/.cache` | `src/suggest.rs:19-107` | read |
| `bind_build_writes` and `ensure_build_paths` run `detect` on every session's cwd | `src/confine.rs:650, 1341-1346` | read |
| the `harness-config` default is gated on `request.build_paths` | `src/confine.rs:978-981` | read |
| `apply_grant_unhides` skips a missing path and never creates it | `src/confine.rs:1496-1498` | read |
| `ConfineTable` serves `[confine]` and `[session.confine]` | `src/team.rs:455-461` | read by the consultant |
| `parse_with` receives `default_team` only for the global file | `src/team.rs:430`, `src/main.rs:886` | read |
| `team reset` does not re-plan; the daemon plans once at spawn | `src/main.rs:1508-1515`, `src/daemon.rs:395` | read by the consultant |
| no other caller of `suggest::detect` or `write_paths` | `rg "suggest::" src tests` (confine.rs, main.rs, tests/suggest.rs only) | probed |
| `team up` builds a session's request in three places and `new --confine` in a fourth | `src/main.rs:511, 1103, 1232, 1324` | read |
| `run_team_up` reads the team, then `team::plan`, then validates | `src/main.rs:1356-1395` | read |
| Codex confinement is refused | `src/team.rs:1030-1037` | read |
| the dry run prints harness-config and a `grants:` line from the plan | `src/confine.rs:2240-2245`, `src/main.rs:1158` | read by the consultant |
| `ConfineRequest` fields are all `serde(default)`, so dropping one is wire-tolerant | `src/confine.rs:106-124` | read |
| `.semgrep.yml`, `.semgrep.yaml`, `.semgrep` are semgrep's markers; first-run writes under `~/.semgrep` and `~/.sonar` | recall | UNVERIFIED |
