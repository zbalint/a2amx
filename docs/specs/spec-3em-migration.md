# Spec 3em: migrate the old state directory to `~/.a2amx`

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from commit `ab85485` (spec 3el accepted).
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. Spec 6 of 7 of the owner's configuration redesign.
Nothing of the redesign is released before this spec is accepted. A first start after the upgrade moves data;
that is the one new behavior, it is automatic by the owner's decision, and it refuses rather than overwrites.

Public test seams: `a2amx::migrate` (`old_dir`, `migrate`) in `tests/migrate.rs` (new); the `a2amx` binary
(`daemon start --foreground`, `gateway token`) in `tests/migrate_cli.rs` (new) with a temporary `HOME`.

**Scope** (files that may be edited or created):

- `src/migrate.rs` (new), `src/lib.rs` (one `pub mod`), `src/main.rs` (the call sites of section 4; anchors in this spec are
  function names, because line numbers move with the specs before it).
- `tests/migrate.rs` (new), `tests/migrate_cli.rs` (new).
- `docs/install.md` (a new upgrade paragraph: stop the daemon and the gateway **before** installing, because the installer
  replaces the old binary; what the first start moves; delete the old directory), `docs/configuration.md` (one sentence
  pointing at it). No doc names the old directory today (`rg 'local/state|XDG_STATE_HOME' docs README.md` outside
  `docs/specs` and `docs/backlog.md` matches nothing).

Does **not** touch: `src/layout.rs`, `src/config.rs`, `src/daemon.rs`, `src/relay_files.rs`, `src/team.rs`,
`AGENTS.md`, `CHANGELOG.md`, `docs/architecture.md`, `docs/backlog.md`, earlier specs, the installer scripts.

## 1. Why

Spec 3eg moved everything from `~/.local/state/a2amx` (or `$XDG_STATE_HOME/a2amx`) to `~/.a2amx/{config,state,run,log}`
and spec 3ei and 3ek renamed keys. A user upgrading would start with an empty new directory and lose the message
database, the Discord pairing and token, the global team file and the gateway settings. The owner decided
(grilling, 2026-11-13/14): migrate automatically on the first start when the old directory exists and no home
is given explicitly; refuse while a daemon runs; move and convert the files; leave the old directory and tell the
user to delete it; when both directories hold data, refuse and name both. Old `state/auto-grants` is dropped
(each project is asked once again, spec 3el). Local `a2amx.toml` files are not migrated (the daemon does not know
them); their old keys fail with the rename messages of spec 3ek.

Rejected: an explicit `a2amx migrate` command (the owner chose automatic); merging two populated directories
(mixes two states); copying instead of moving (two live copies of the database and the token); leaving a
partial move after a failure (a next start would then refuse forever).

## 2. What moves

`old` is the old directory. Files absent from `old` are skipped. The table is complete: nothing else is moved.

| Old (under `old`) | New | Conversion |
| --- | --- | --- |
| `messages.db` and whichever of `messages.db-wal`, `messages.db-shm`, `messages.db-journal` exist | `state/` same names | none; they move together (a `-wal`, `-shm` or `-journal` without the db is not moved and does not make migration needed) |
| `gateway/pairing.json`, `gateway/seen.json` | `state/gateway/` | none |
| `team.toml` | `config/team.toml` | key rename (2.1) |
| `gateway/config.toml` | `config/gateway.toml` | key rename (2.2) |
| `gateway/discord.token` | `config/secrets/discord.token` | none |
| `daemon.log` | `log/daemon.log` | none |
| `gateway/gateway.log` | `log/gateway.log` | none |

Not moved (regenerated or dropped): `addr`, `admin.token`, `gateway.token`, `daemon.lock`, `gateway.lock`,
`gateway.pid`, `link/`, `codex/`, `omp/`, `confine-empty`, `auto-grants`.
A file is moved with `rename(2)`, keeping its mode and owner; a table file that is a symlink, or an `old/gateway` that is a
symlink, is refused (`old file {path} is a symlink`). The move stops with a clear error when `rename` fails (including a different
filesystem; `shortcut:` comment: copy-and-remove across filesystems if a real user hits it).

### 2.1 `team.toml` conversion

A lexical edit that keeps comments and every other byte, **verified structurally**. The scanner is a small lexer with
state: bracket depth outside strings, basic (`"`) and literal (`'`) strings, multi-line `"""` and `'''` strings with
their escapes, and comments. It recognizes table headers and key lines only at bracket depth 0 outside strings and
multi-line strings, so a nested-array line such as `["a"],` inside `command = [` or `reset = [`, and a `[[session]]` inside
a string, change nothing. Headers: `[[session]]` (also `[[ session ]]`, `[["session"]]`, with a trailing comment) set the
session context; any other header sets the other context; before any header is the top level. Keys: `allow` bare or
quoted, with or without spaces around `=`, indented with spaces or tabs, CRLF kept as it is. In the top level, `allow`
becomes `allow_teams`; in the session context it becomes `allow_agents`; elsewhere it is left alone. Only the key token changes.

Verification: parse the old text with the `toml` crate; if it does not parse, the file is moved unconverted. Otherwise apply the
two renames to the parsed value, parse the converted text, and require the two values to be equal. If they differ, move the
file unconverted. An unconverted file still moves, and the stderr report adds
`{new}: could not convert keys automatically; rename allow to allow_teams (file) and allow_agents (session) by hand`.
(Spec 3ek's rename errors then name the file at the first `team up`.)

### 2.2 `gateway.toml` conversion

Key lines `sync_ms`, `idle_ms`, `member_check_ms`, `reconnect_ms` at the top level (the gateway file has no tables) whose value
is a decimal integer (digits with optional `_` separators or a leading `+`; hex, octal and binary forms are left alone), with
optional spaces around `=`, indentation, and an optional trailing `#` comment with or without a space before it, become
`sync = "Nms"`, `idle = "Nms"`, `member_check = "Nms"`, `reconnect = "Nms"` (underscores and `+` dropped; the strings 3ei
accepts). Comment text and every other line stay byte for byte, CRLF kept. The same structural verification applies (old and
converted texts parse; the converted value equals the old one with the four keys renamed and the integers turned into the
duration strings); on failure the file moves unconverted and the report names the keys to rename by hand. The converted file
is valid for `load_gateway` (3ei); a zero value stays invalid, as before.

The conversion functions are pure (`fn convert_team(text: &str) -> String`, `fn convert_gateway(text: &str) -> String`).

## 3. `src/migrate.rs`

```rust
pub fn old_dir(xdg_state_home: Option<&OsStr>, user_home: Option<&Path>) -> Option<PathBuf>;
pub struct Report { pub moved: Vec<(PathBuf, PathBuf)> }   // old, new
pub enum Outcome { NotNeeded, Migrated(Report) }
pub fn migrate(layout: &Layout, old: &Path) -> anyhow::Result<Outcome>;
```

1. `old_dir`: `$XDG_STATE_HOME/a2amx` when the variable is set, non-empty and an absolute path, else
   `$HOME/.local/state/a2amx`, else `None`. (Where the previous version kept its state.)
2. `migrate` runs these steps in order and returns an error at the first failed precondition, before any change:
   1. **Resume.** If `state/.migrating` (the journal, step 5) exists in the new root, finish that migration: go to step 5
      and perform the journal's moves that are not yet done (a move is done when its old path is gone and its new path
      exists), skipping steps 2 to 3's refusals about new data (rule 2.2 ignores the journal file itself). A journal that
      cannot be parsed is an error naming it.
   2. `old` does not exist, is not a directory, or holds none of the section 2 files (a `-wal`, `-shm` or `-journal` alone
      does not count): `Ok(NotNeeded)`. After a successful move none of them is left, so a later start finds nothing to do.
   3. The new root holds any file or symlink other than under `bin/` (the installer's directory) and other than the journal,
      at any depth: error `both {old} and {root} hold data; move or delete one of them and start again`.
   4. A daemon or gateway runs from `old`: open `old/daemon.lock` and `old/gateway.lock` with
      `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC`, require a regular file, and try `flock(LOCK_EX | LOCK_NB)`
      (the lock the old daemon holds for its life and the old gateway holds, the same call as `src/gateway_proc.rs`; a
      read-only descriptor is enough for `flock`). A missing lock file counts as free. The acquired locks are held until
      the migration ends. A held lock is the error
      `a2amx is still running from {old} (pid {pid}); stop it with: kill -TERM {pid}, then start again`, where `{pid}` is
      found in `/proc/locks` by the lock file's `major:minor:inode` (`stat` device and inode; the format is
      `... FLOCK ADVISORY WRITE <pid> <maj>:<min>:<ino> ...`); when no line matches (for example the process runs in
      another pid namespace) the text is `a2amx is still running from {old}; stop the process that holds {path}, then start
      again`. The remedy never says to run the old binary: the installer replaced it.
   5. `layout.ensure()`, remembering which directories did not exist before. Then write and `fsync` the journal
      `state/.migrating` (the list of old to new pairs of this run, one line each), `fsync` the new `state/`.
      Move each file of the table in table order: plain files by `rename`; `team.toml` and `gateway.toml` by writing the
      converted text to the new path (`create_new`, then `fchmod` to the old file's mode so the umask does not matter)
      and leaving the old file in place. After **every** move succeeded, remove the old files of converted entries,
      then remove the journal and `fsync` the directories.
   6. On any error in step 5 before the journal is removed, undo this run: rename back every renamed file, delete every
      converted new file, remove the journal, remove the directories `ensure` created if they are empty, and return the
      original error plus, if an undo itself failed, the paths left in the wrong place. A crash (kill, power loss)
      during step 5 leaves the journal; the next start resumes forward (step 1) and never refuses forever.
   7. Release the locks and return `Migrated(Report)`.
3. The caller prints (stderr) one line per moved file `moved {old} -> {new}` and then
   `a2amx now uses {root}; {old} is no longer used and can be deleted: rm -r {shell-quoted old}` (the path single-quoted, with embedded
   `'` written `'\\''`). The old directory is not hidden from confined sessions, so the delete also removes its stale tokens.

## 4. `src/main.rs`

`dispatch` knows whether the home came from `--home`/`A2AMX_HOME` (`home_arg.is_some()`). A helper
`fn migrate_old_state(explicit_home: bool, layout: &Layout) -> anyhow::Result<()>` returns immediately when
`explicit_home` (except for the warning below), resolves `old_dir(env XDG_STATE_HOME, env HOME)`, runs `migrate` on a blocking
thread and prints the lines of 3.3. It is called, as the first statement and before any `ensure`, from: `run_daemon` and
`run_daemon_background` (after its `Client::connect` check, before its `ensure` and log open; the child it spawns carries
`--home` and never migrates again), `run_gateway_token` and `run_gateway_up` (before it calls into `gateway_launch::up`,
which calls `ensure` and `ensure_gateway_file`). No other command migrates. Only these call sites write anything under the
root before migrating; `dispatch`'s client settings load (3ej) only reads. A failure is a command error (exit 1) and
the command does not continue.

**Explicit home with old-layout files.** With `--home` or `A2AMX_HOME` the previous version used that directory as the flat
state directory. When `daemon start` runs with an explicit home and `<root>/messages.db`, `<root>/team.toml` or
`<root>/gateway/` exists, it prints one stderr line `{root} holds files in the old layout ({names}); a2amx now reads
{root}/state, {root}/config and {root}/log; see docs/install.md` and continues. It moves nothing.

## 5. Tests

Library tests build a fake old directory in a temp dir with all table files and assert exact bytes and modes (modes set
with an explicit `chmod`, not the umask).

1. All table files present: after `migrate` every new path holds the old bytes (converted ones as in 2 and 3), the old
   paths are gone, `auto-grants` and `addr` remain, the report lists every move, modes are unchanged (a 0600 token stays 0600).
2. `convert_team`: `allow = ["a"]` at top level becomes `allow_teams = ["a"]`; under `[[session]]` (also `[[ session ]] # c`)
   becomes `allow_agents`; under `[confine]` unchanged; `# allow = x` unchanged; a nested-array line `["a"],` inside
   `command = [` does not change the context (an `allow` after it keeps its context); a `[[session]]` inside a
   multi-line string changes nothing; `"allow" =`, `allow=`, tab indentation and CRLF files convert and keep every other byte.
3. `convert_gateway`: `sync_ms = 5000 # note` becomes `sync = "5000ms" # note`; `idle_ms=5_000#c` becomes `idle="5000ms"#c`
   (key spacing as written); `idle_ms = "x"` unchanged; unrelated lines unchanged.
3b. Structural verification: a team text where the lexical edit would differ from the value-level rename (for example an
   `allow` key inside an inline table in a session) and a text that does not parse are moved **unconverted**, and the report
   contains the `could not convert keys automatically` line for that file.
4. `old` missing, empty, or holding only `addr`, or only `messages.db-wal`: `NotNeeded` and the new root is not created.
5. New root holds `config/team.toml`: error naming both; nothing in the old directory changed.
6. New root holds only `bin/a2amx`: migrates.
7. `old/daemon.lock` locked by another process (the test holds `flock` on it, and the test's own pid is the expected pid):
   the error contains `still running`, the pid and `kill -TERM`, nothing moved; the same for `gateway.lock`; a lock file that
   is a FIFO or a symlink does not hang or get followed (the open flags).
8. `messages.db` with `-wal` and `-shm` move together; `-journal` alone does not trigger migration.
9. A symlink in the table, or `old/gateway` as a symlink: error naming it; nothing moved.
10. Rollback: only `messages.db` and `gateway/pairing.json` in `old`, and `state/gateway/pairing.json` already an **empty
    directory** in the new root (so rule 3 passes and `rename` fails): error, `messages.db` is back at its old path with its
    old bytes and mode, no converted file or journal remains.
11. After a successful `migrate`, a second `migrate` returns `NotNeeded` and does not refuse although `old` still exists.
12. Crash resume: a journal `state/.migrating` listing three moves, with the first done (old gone, new present) and the second
    not: `migrate` completes the moves, removes the journal, and does not return the both-hold-data error.
13. `old_dir`: `XDG_STATE_HOME` absolute `/x` gives `/x/a2amx`; relative or empty is ignored (documented drift from the old
    resolver); no home gives `None`; with `HOME=/h` gives `/h/.local/state/a2amx`.
14. CLI: `HOME=<tmp>` with `A2AMX_HOME` and `XDG_STATE_HOME` removed from the environment and an old directory in
    `<tmp>/.local/state/a2amx`: `daemon start --foreground` migrates (the new `state/messages.db` has the old bytes),
    prints the `moved` lines and the quoted delete hint on stderr, and starts. With `--home <tmp>/x` the old directory is
    ignored and left untouched. `gateway token` migrates as well. With `--home <tmp>/x` holding `messages.db` and `gateway/` in
    the old layout, `daemon start` prints the old-layout line and starts.

Mutants: skip the lock check (7); skip the new-root check (5); move auto-grants (1); convert in the wrong context
(2); drop rollback (10); migrate when `explicit_home` (14); skip journal resume (12); skip structural verification (3b);
`open` without `O_NONBLOCK` (7, FIFO).

## 6. Out of scope

Migrating local `a2amx.toml` files; converting `daemon.toml` or `client.toml` (they did not exist; the daemon writes the
commented templates); copying across filesystems; a `migrate` command; deleting the old directory (the user does it);
moving the old per-run files; migrating `auto-grants`; any change to the installer.

## 7. Acceptance

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test migrate
cargo test --test migrate_cli
rg -n "local/state" src docs README.md --glob '!docs/specs/**' --glob '!docs/backlog.md'
```

The `rg` command matches only `src/migrate.rs`, `src/main.rs` (the XDG/HOME lookup) and the `docs/install.md` upgrade paragraph.
In a confined session, `tests/migrate_cli.rs` uses a temp `HOME` under `target/scratch/3em/`.

## 8. Amendment 1 (journal states, commit point, recovery)

Raised by the developer (BLOCKED, verified by reading 3.2.1, 3.2.5 and 3.2.6 against each other): the "done" test of 3.2.1 does not fit a converted entry, because its old file stays until the end, and rollback cannot restore a converted original that was already deleted. This amendment replaces the "done" definition of 3.2.1 and the text of 3.2.5 and 3.2.6; nothing else changes.

**Entry states.** A *plain* entry is done when its old path is gone and its new path exists. A *converted* entry is written when its new path exists (the new file appears only by an atomic `rename` of a finished temporary file, so an existing new file is complete); the old file is deleted only in the commit phase. Converted new files are made by writing the converted text to `{new}.migrating` in the same directory (`create_new`, `fchmod` to the old file's mode, `fsync`), then `rename` to `{new}`. A leftover `{new}.migrating` is removed and rewritten on resume.

**Phases of step 5.** (a) *Prepare*: write and `fsync` the journal; for each entry in table order do its move (plain: `rename`; converted: the temporary file and its `rename`). Old converted files are untouched. (b) *Commit*: begins when every entry is moved or written; it deletes the old files of converted entries one by one (an already missing one is fine), then removes the journal and `fsync`s the directories. The commit phase is the point of no return.

**Errors and crashes.** A returned error in phase (a) undoes the run exactly as 3.2.6 says (rename plain files back, delete the written converted new files and any `.migrating` file, remove the journal and the directories `ensure` created if empty). Because the old converted files still exist in phase (a), nothing needs restoring. An error in phase (b) is not rolled back: the command fails with the error plus `the migration is nearly finished; start again to complete it`, and leaves the journal. A crash in either phase leaves the journal. **Resume** (3.2.1) always goes forward: for each journal entry it does the entry's move if not done (a converted entry whose new file is missing is rewritten from its old file; if both its old and new files are missing, the error names the entry), and then runs the commit phase. Resume never needs rollback and never refuses with the both-hold-data error.

**Tests added.** 12b: a journal with a converted entry whose old file exists and whose new file is written (crash before the deletes): `migrate` removes the old file and the journal and does not fail on the existing new file. 12c: the same with a leftover `{new}.migrating` and no new file: the new file is written from the old one, the temporary file is gone. 12d: failure during phase (a) (test 10) leaves the old `team.toml` in place when it was a converted entry earlier in the table than the failing move (order the fixture so `team.toml` is converted before the failing rename). Mutant added: deleting converted old files before every entry is moved (12d).
