# Spec 3w: a global team file as the fallback when the current directory has no a2amx.toml

## 0. Status

**LOCKED** (2026-10-14; pre-lock gate run against develop at `3af6dbf`; consultant review `m_1598`, amended A1-A6, C; B1 kept as a documented limit, B2 documented as a limitation). Owner-directed design, context `a2amx-global-team-2026-10-14`. Implementation goes to
`a2amx-developer` only.

**Baseline:** develop at `3af6dbf`; `cargo test --test team` passes 44/44.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the pure functions in `src/team.rs` (`team_name_for`, `check_global_allowed`,
`parse_with`) and the `a2amx` binary (`tests/team.rs::run_binary` style, isolated `--home` and a separate
working directory).

**Scope.** May edit exactly these files and no others.

- `src/team.rs`
- `src/main.rs` (`read_team`, `run_team_status`, `run_team_up`, `run_team_down`, `run_team_reset`, and one new
  resolver function)
- `src/cli.rs` (the four `--file` help strings only)
- `tests/team.rs`
- `README.md`, `docs/architecture.md`

Does not touch: `src/wire.rs`, `src/daemon.rs`, `src/messaging.rs`, `src/mcp.rs`, every other test file, every spec
document, `a2amx.toml.example`. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

The owner starts teams from `a2amx.toml` files that are near copies across repositories (the a2amx, SALTMDB and
`~/.agents` files differ mainly in the `team` name). They want one shared file used as a fallback, with the team
named after the directory. The risk is that a bare `a2amx team up` run in the wrong directory (`/`, `~`, a system
directory) now starts a whole team with agents that have shell access, where today it fails for want of a file.
So the fallback is guarded: it works only in a directory that looks like a project directory.

## 2. Decisions

- **D1. Lookup.** For `team status`, `team up` (file form), `team down` and `team reset` without explicit names, when
  `--file` is not given: use `a2amx.toml` in the invoking directory if it exists; otherwise use the global file
  `<state home>/team.toml` (the daemon state directory, `--home` / `A2AMX_HOME`, already passed to each `run_team_*`
  as `home`) if it exists and the guard (D4) passes; otherwise fail. "Exists" means `std::fs::metadata` succeeds; only a `NotFound` error
  falls through to the next candidate, any other error (permission denied, ...) is reported with the path. `--file` always uses exactly that file and never
  falls back or checks the guard. The flag form of `team up` (`NAME=EXE`) and explicit session names are unchanged.
  There is no merging: a local file is used alone.
- **D2. Team name.** A global file gets its team name from the directory: `team_name_for(cwd)` takes the last
  component of the given directory (the caller canonicalizes first, as for D4; the function itself never touches the
  filesystem), converts it with `to_string_lossy`, lowercases it (ASCII), replaces each run of characters outside
  `[a-z0-9]` with one `-`, trims leading and trailing `-`, truncates to 40 bytes (so `{team}-{role}` stays within the 63-byte session-name limit of `validate_name`)
  and trims a trailing `-` again.
  The result must pass `messaging::validate_name`; an empty or invalid result (for example `s12`, or only symbols)
  fails with the reason, the directory, and the advice to create a local `a2amx.toml` or pass `--file`.
  A `team` key in a global file is an error (`the global team file must not set team; the team is named after the
  directory`), because one fixed name would make every project share session names. `private` and `allow` work as in a
  local file and apply to the derived team (so a global file without `private` gives a private team).
- **D3. Relative paths.** For a global file, a session `cwd` that is relative, and an absent `cwd`, resolve against the
  invoking directory (not the state directory), so `cwd = "."` means "this project".
- **D4. Guard.** `check_global_allowed(cwd, user_home, state_home) -> anyhow::Result<()>` rejects the invoking
  directory (already canonicalized by the caller, so a symlink to `/` is caught) when any rule holds. Rules are
  tested in the order 1 to 6 and the first match names the reason. The guard runs before name derivation (D2).
  1. it has no parent (the filesystem root);
  2. it equals `user_home` or is an ancestor of it (covers `/`, `/home`, `/Users`); `user_home` is `$HOME`; when
     `$HOME` is unset or not absolute, rules 2 and 3 are skipped and rules 1, 4, 5 and 6 still apply;
  3. its first component below `user_home` starts with `.` (anything in or under `~/.ssh`, `~/.config`, `~/.a2amx`,
     `~/.cache`, `~/.agents`, ...), or it *is* (not merely lies beneath) one of `~/Desktop`, `Documents`, `Downloads`,
     `Music`, `Pictures`, `Videos`, `Public`, `Templates`, `Library`, `Applications`; a project at
     `~/Documents/x` is allowed;
  4. it equals or lies beneath `state_home` or one of the system trees `/bin /boot /dev /etc /lib /lib32 /lib64
     /libx32 /proc /run /sbin /sys /usr /snap /nix /lost+found /System /Library /Applications /private/etc /var/lib
     /var/log /var/cache /var/spool /var/mail /mnt/wsl /mnt/wslg`, or (WSL, compared ASCII case-insensitively)
     `/mnt/<letter>/Windows`, `/mnt/<letter>/Program Files`, `/mnt/<letter>/Program Files (x86)`,
     `/mnt/<letter>/ProgramData`; a project under `/usr/local/src` is therefore refused (accepted cost);
  5. it equals one of the container directories `/home /Users /root /tmp /var /opt /srv /mnt /media /Volumes
     /private /private/tmp /private/var`, or is a direct child of `/home` or `/Users` (another user's home), or is
     `/mnt/<single ASCII letter>` (a Windows drive root under WSL);
  6. (WSL, case-insensitive) it is `/mnt/<letter>/Users` or a direct child of it (a Windows home).
  `<letter>` is a single ASCII letter. The caller canonicalizes `user_home` and `state_home` too when they exist (a
  failed canonicalization keeps the path as given), so a symlinked home still matches.
  The error is `refusing the global team file in {cwd}: {reason}; create an a2amx.toml here, or pass --file, to
  start a team in this directory`. `{reason}` names the matched rule in words: "the filesystem root", "your home
  directory", "a hidden directory under your home directory", "a user-data folder", "the daemon state directory or a
  system directory", "a top-level container directory", "another user's home directory", "a Windows home
  directory". Tests assert the reason text only for the first-match cases listed in section 8.
  There is no environment variable or flag that overrides the guard; a local file or `--file` is the override.
  Hidden directories under home (for example `~/.agents`) therefore keep using a local `a2amx.toml`.
- **D5. Visibility.** When the global file is used, print one line to stderr before any other output:
  `using global team file {path} as team {name}`. Local-file and `--file` runs print nothing new.
- **D6. Missing files.** Neither local nor global file present: the error names both
  (`no a2amx.toml in {cwd} and no global team file at {path}`). It must still contain the substring `a2amx.toml`
  and not contain `start it` (existing test `file_and_argument_errors_precede_daemon_connection`). All file and
  guard errors occur before the daemon connection.

## 3. `src/team.rs`

Add three public functions; no existing signature changes.

- `pub fn team_name_for(cwd: &Path) -> anyhow::Result<String>` (D2).
- `pub fn check_global_allowed(cwd: &Path, user_home: Option<&Path>, state_home: &Path) -> anyhow::Result<()>` (D4).
- `pub fn parse_with(text: &str, default_team: Option<&str>) -> anyhow::Result<Vec<TeamSession>>`: the current
  `parse` body with `team` replaced by `team.or(default_team)`, preceded by the D2 check that a file parsed with a
  `default_team` must not itself contain `team`. `parse(text)` becomes `parse_with(text, None)` so its 39 test call
  sites stay as they are. Reuse the existing team validation path; do not duplicate it.

## 4. `src/main.rs`

- One new function `resolve_team_file(home: &Path, file: Option<PathBuf>) -> anyhow::Result<TeamSource>` where
  `TeamSource { path: PathBuf, dir: PathBuf, default_team: Option<String> }`. It implements D1, D2 (calls
  `team_name_for`), D4 (calls `check_global_allowed` with `std::env::var_os("HOME")`), D5 and D6. It replaces the four
  copies of `cwd.join(file.unwrap_or_else(|| PathBuf::from("a2amx.toml")))`, at the `run_team_status`, `run_team_up`,
  `run_team_down` and `run_team_reset` sites. Existence is checked with `std::fs::metadata` on a blocking thread.
- `read_team` takes the `TeamSource` (or its `path` and `default_team`) and calls `team::parse_with`.
- `run_team_up` uses `TeamSource::dir` as `file_dir` (D3: the invoking directory for a global file, the file's
  directory otherwise, as today).
- Helper naming and exact signatures are the developer's choice within these behaviors.

## 5. `src/cli.rs`

The four `/// Team file (default: a2amx.toml)` strings say
`Team file (default: a2amx.toml in the current directory, else the global team.toml in the state directory)`.
The existing suffixes ("Cannot be combined with ...") stay; the wrong "NAME=EXECUTABLE" suffix on `Down` is left
as is. No other change. The help text change affects no test (verified by `rg "default: a2amx.toml" tests` having no hits).

## 6. Docs

- `README.md` Teams section (line ~195): describe the fallback, the derived name, the `team`-key error, relative
  paths, the guard list in plain words, and that a local file or `--file` overrides the guard.
- `docs/architecture.md`: the team lookup rule and the guard, where the team file is already described.

## 7. Out of scope

- Merging or overriding fields between a local and the global file.
- A flag or environment variable to bypass the guard.
- A `team init` command that writes a global file.
- Any change to `a2amx.toml.example`, to the daemon, or to `prefix` (already renamed to `team`).
- Guarding the local file or `--file`.
- Two directories with the same name (`~/a/api`, `~/b/api`) derive the same team, so `team up` in the second sees
  the first one's sessions as running, and `team down` or `team reset` there acts on them (`inspect` matches on name and
  team scope only, `src/team.rs:153-166`). This is a documented limitation (README), not fixed here; the same holds
  for two local files with the same `team`. A cwd check is a possible follow-up.

## 8. Acceptance

Baseline (runs now, passes): `env -u A2AMX_BIN -u NO_COLOR cargo test --test team` is 44/44.
Post-implementation (needs the new code):

- `env -u A2AMX_BIN -u NO_COLOR cargo test` all green; `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` clean.
- New tests in `tests/team.rs`, written first (red), through the seams above:
  - `team_name_for`: `/work/My_Project.v2` gives `my-project-v2`; `/work/ABC` gives `abc`; `/work/s12` fails;
    `/work/---` fails; a 70-character name gives at most 40 bytes with no trailing `-`, and `parse_with` of a file with
    sessions named `architect` and `consultant` and that team name accepts them.
  - `check_global_allowed` with `user_home = /home/u`, `state_home = /home/u/.a2amx`: rejects `/`, `/home`, `/home/u`,
    `/home/other`, `/home/u/.ssh`, `/home/u/.a2amx/x`, `/home/u/Documents`, `/etc`, `/etc/x`, `/usr/local`, `/tmp`,
    `/var`, `/var/log/x`, `/mnt/c`, `/mnt/c/Windows`, `/mnt/C/users/x`, `/mnt/wsl`; accepts `/home/u/work/p`,
    `/home/u/Documents/p`, `/tmp/p`, `/var/www/p`, `/mnt/c/p`, `/mnt/c/Users/x/p`, `/srv/p`. With `user_home = None` it
    still rejects `/` and `/etc` and accepts `/work/p`. Reason text is asserted for `/` (root), `/home/u` (home: rule 2
    wins over rule 5) and `/home/u/.a2amx/x` (hidden directory: rule 3 wins over rule 4).
  - `parse_with`: a file without `team` and `default_team = Some("proj")` gives sessions named `proj-architect`,
    private; a file with `team` and a `default_team` fails with the D2 message; `parse(text)` behaves as before.
  - binary tests (`--home` a temp dir; current directory `<second temp dir>/proj`, so the derived team and the
    expected names are the literals `proj`, `proj-architect`, ...; `HOME` set to a third temp dir):
    global file used when no local file (stderr has the D5 line, `team status` lists the derived names); local file
    wins and prints no D5 line; `--file` never falls back; the guard error appears when the invoking directory is
    `HOME`; neither file gives the D6 error naming both; a global session with `cwd = "."` is spawned in the invoking
    directory (reuse the pattern of `file_cwd_is_relative_to_its_directory_not_the_invoking_directory`).
- `git status --short` lists exactly the Scope files.
- Known flake: none recorded for `tests/team.rs`; a failure elsewhere is rerun once, then recorded as pre-existing.
- Hazard: several existing tests write `team.toml` into the state directory and pass `--file team.toml`
  (`tests/team.rs` around lines 798, 1062, 1108, 1152, 1339); a new test there without `--file` would pick it up as
  the global file. New tests keep the global file only in a dedicated `--home`.
- Note: the existing `run_binary` helper uses the state directory as the working directory; the new binary tests
  must use a separate directory, because rule 4 of D4 rejects a working directory inside the state directory.
