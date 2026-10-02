# Spec 2h: a live install separate from the dev build

## 0. Status

**LOCKED.** Decisions D1 to D5 in section 2 were settled with the owner. Builds on spec 2g.
The work is done directly on `develop` in the main checkout, no worktree.

**Scope.** May edit exactly these files and no others. New files are marked.

- `scripts/install.sh` (new)
- `src/codex.rs`, `src/daemon.rs`
- `tests/codex_exe.rs` (new)
- `README.md`, `AGENTS.md`

Does not touch: every other file under `src/`, every other file under `tests/` (their diff
must be empty; `tests/common` is not edited), `Cargo.toml`, `Cargo.lock`, `docs/`,
the locked spec documents. No new dependency. Do not commit and do not merge: leave the
diff uncommitted in the working tree.

## 1. Why

The daemon builds each Codex session's MCP configuration from `std::env::current_exe()`.
`cargo build` replaces `target/debug/a2amx`, so a daemon started before a rebuild reports its
own path as `/…/a2amx (deleted)` (Linux appends that text to `/proc/self/exe`). A Codex
session started after a rebuild got `mcp_servers.a2amx.command=".../a2amx (deleted)"`,
Codex could not start the server, `/mcp` showed `a2amx: failed (0 tools)`, and the session
received messages but could not reply. It looked healthy from outside. (Found live on
2026-10-03; the s3 app-server's command line showed the path.)

Two fixes: run the live daemon from an installed copy that development builds never touch
(the pattern used for SALTMDB), and make the daemon refuse loudly when its own executable
path is not a real file.

## 2. Decisions settled with the owner

- **D1.** The live install lives in `~/.a2amx` (binary at `~/.a2amx/bin/a2amx`). The state
  directory stays `~/.local/state/a2amx`; the two are unrelated.
- **D2.** `scripts/install.sh` builds a release binary and installs it atomically: copy to a
  temporary name in the same directory, then `mv -f` over the target. A running daemon keeps
  its old image and is not restarted by the script.
- **D3.** The install directory is overridable with `A2AMX_INSTALL_DIR` (the tests and the
  acceptance run use a temp dir).
- **D4.** The daemon resolves its executable path once, at start, and checks it before it
  builds a Codex configuration. A path that is not a regular file makes `create_session`
  answer `Response::Error` with a message telling the operator to restart the daemon from the
  installed binary. No other harness is affected (Claude and OMP take the path from the
  client process).
- **D5.** Moving `amx` and the daemon onto the installed binary (PATH, alias, service) is
  the operator's step, documented in the README, not scripted.

## 3. `scripts/install.sh` (new)

POSIX `sh`, `set -eu`, mode 0755. Behavior, in order:

1. Resolve the repository root from the script's own location and `cd` there.
2. `dest="${A2AMX_INSTALL_DIR:-$HOME/.a2amx}"`; `mkdir -p "$dest/bin"`.
3. `cargo build --release --locked`.
4. `tmp="$dest/bin/.a2amx.$$"`; `cp target/release/a2amx "$tmp"`; `chmod 755 "$tmp"`;
   `mv -f "$tmp" "$dest/bin/a2amx"`. Remove `$tmp` on failure (a `trap`).
5. Print `installed <version> to <path>` using `"$dest/bin/a2amx" --version`, then one line:
   `a running daemon keeps its old binary; restart it to use this one (sessions end).`

No `sudo`, no PATH or shell-profile edits, no daemon start or stop.

## 4. `src/codex.rs`

Add `pub fn executable_error(exe: &Path) -> Option<String>`: `None` when `exe` is a regular
file; otherwise `Some` of `the daemon's executable is gone (<exe>); restart the daemon from
the installed binary`. Display the path with `to_string_lossy`.

## 5. `src/daemon.rs`

- The runtime stores `exe: PathBuf`, set in `Daemon::start` from `std::env::current_exe()`
  (an error ends the start with that error, as other start-up errors do). If the string ends
  with ` (deleted)`, strip that suffix first.
- In `create_session`, the Codex branch uses `self.exe` instead of calling `current_exe()`
  and, before `codex::start`, returns `Response::Error { message }` when
  `codex::executable_error(&self.exe)` is `Some(message)`. The error occurs before any
  app-server or session is created.

## 6. `tests/codex_exe.rs` (new)

Through the public function only; expected values are literals.

- `executable_error` on a path to an existing regular file (a temp file) is `None`.
- On a path in a temp dir that does not exist: `Some` text equal to `the daemon's executable
  is gone (<that path>); restart the daemon from the installed binary`.
- On a directory: `Some`.

The daemon integration (the error response) is not tested: a test daemon runs inside the
test binary and its executable cannot be removed.

## 7. Docs

- `README.md`: a short "Dev build versus live install" section: build and test from the
  repository; run the daemon from `~/.a2amx/bin/a2amx` (install with `scripts/install.sh`);
  point `amx` at the installed binary; restarting the daemon ends all sessions, so install
  freely and restart deliberately; never run the daemon from `target/`.
- `AGENTS.md`: one bullet under Commands or a new short section: the daemon of a working
  session runs from the installed copy, not from `target/`; development builds must not
  replace it.

## 8. Out of scope

Autostart or a systemd unit; a `daemon --detach` flag; copying binaries to a per-session
location; version checks between the installed client and the running daemon; any change to
the Claude or OMP launch paths.

## 9. Acceptance

```sh
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
d=$(mktemp -d); A2AMX_INSTALL_DIR=$d sh scripts/install.sh && A2AMX_INSTALL_DIR=$d sh scripts/install.sh && "$d/bin/a2amx" --version; ls -A "$d/bin"
```

The install run twice succeeds, prints a version, and leaves exactly one file, `a2amx`, in
`$d/bin` (no temporary file). Then `git status` shows only the files listed in section 0,
uncommitted.
