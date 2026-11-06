# Changelog

All notable user-visible changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The full history is in `git log`.

## [Unreleased]

### Changed

- The container-runtime grants (`docker`, `podman`, `containerd`, `crio`, `lxd`) now carry an escape text: `a2amx confine list` prints a warning line under each, and `team up` shows `grant NAME can escape: ...` for an explicitly granted one, because a privileged container started through a root daemon is root on the host.

### Fixed

- The confinement plan no longer hangs when a repository's `.git/config` is a FIFO or another non-regular file (it is treated as absent), reads a config that is not valid UTF-8, finds `core.hooksPath` after lines without `=`, and fails the plan with an error when the config is larger than 1 MiB instead of ignoring its `hooksPath`.

## [0.2.0] - 2026-11-06

### Added

- Auto detection follows a `.git` file. Under global `auto` and in `a2amx confine suggest`, the git-remote grants
  (`ssh`, `gh`, `git-credentials`) are read from the common git directory, so a worktree, a submodule and a
  session started in a subdirectory of a repository get them too. Detection only; the `.git/config` protection
  is unchanged, and `docs/confinement.md` says that `git push origin <branch>` works without an upstream. A
  `.git` file or `commondir` that is larger than 4096 bytes, not UTF-8 or not a regular file now makes the
  confinement plan treat the directory as not a repository (`docs/confinement.md`, `docs/teams.md`, `docs/cli.md`).
- A start-to-finish guide, `docs/getting-started.md` ("Your first team"): install, a team file, grant suggestion, the dry-run preview, inspecting the running team and stopping it, with output from the installed binary, and a section on auto mode with a global team file in `docs/confinement.md`. The README quick start points to it, its `list` sample shows the `CONFINE` column, and `docs_examples` now parses the complete TOML examples of both pages.
- Auto confinement asks first. `a2amx team up` lists each grant that global `auto` would add and has not
  yet been accepted, with the reason and what the grant opens, and waits for a yes; the accepted names are
  kept in `auto-grants` in the state directory, so a grant is asked about once for all projects (delete the
  file to be asked again). `team up --yes` accepts for one run and records nothing; a run without a terminal
  refuses; `--dry-run` prints `not yet accepted: <names>` and never asks. Grants gain an optional `escape`
  text that the prompt and `confine list` show once the escape audit fills it. New flag `--yes` on `team up`
  (`docs/confinement.md`, `docs/teams.md`, `docs/cli.md`).
- Confinement is visible. The status line shows `[confined auto]` or `[confined]`; `list` and `team status` gain a
  `CONFINE` column (`off`, `manual`, `auto`); `team up --dry-run` prints the mode and, for auto, the reason for
  each grant it added; the new `a2amx confine show [SESSION]` prints a running session's mode, grants and
  operator `write` entries; and the MCP `list_agents` result carries each agent's `confine`, so a peer can tell
  what another agent can reach. Additive protocol fields (`confine` on session and agent summaries,
  `confine_auto` on the status frame, `auto` and `reasons` on the confinement request); an older client or
  daemon ignores them (`docs/confinement.md`, `docs/cli.md`).
- Confinement grants `cache` (the whole `~/.cache`, read-write; the default hides under it stay) and
  `huggingface-token` (the Hugging Face login file and its two environment variables), so a team no longer needs a
  raw `write` of `~/.cache` (`docs/confinement.md`).
- Team line: a session in a team shows a row at the top of the attach screen, in the status bar's
  style, listing the team's members with an activity dot (grey for exited members). A click on a
  member, or the prefix then `1`-`9`, `n` or `p`, switches to it; a click on the team label opens the
  picker; the prefix then `s` hides it together with the status bar. It needs a terminal of at least
  five rows. Additive protocol fields: the attach request's `top_rows`, the `AttachedTop` reply and
  the status frame's `members`; an older daemon simply shows no line (`docs/cli.md`).
- A dim rule under the team line, like the one above the status bar, on terminals of at least six rows
  (at five rows the line shows without it). A click on the rule does nothing.
- Opt-in confinement of a session with bubblewrap: `a2amx new --confine` (and a `[confine]` table with
  `enabled = true` in a team file, or `[session.confine]` per session) runs a Claude, OMP or Generic session
  with a read-only root, a private `/tmp`, its own process list, and writes limited to the workspace, the git directory and the harness's data paths. Secret locations
  (`~/.ssh`, `~/.aws`, the state directory, the session bus, container sockets and others) are hidden;
  `--confine-write`, `--confine-hide`, `--confine-unhide`, `--confine-no-default-hides` and
  `--confine-grant NAME` adjust it, with the keys `write`, `hide`, `unhide`, `default_hides` and `grants` in the
  confine table (session lists add to the team's lists). Grants are one per container runtime (`docker`, `podman`, `containerd`, `crio`,
  `libvirt`, `lxd`, `snap`), each opening only its own sockets and environment names, and `a2amx confine list` prints
  them with what each exposes; a team-level table cannot grant one. The session-only grant `harness-config` makes the session's own harness directory (`~/.claude` or `~/.omp`) writable and persistent; the agent can then change settings, hooks and programs that your other sessions run at once (`docs/confinement.md`). The grants `gh` and `git-credentials` open your GitHub CLI login (`~/.config/gh`, read-write, and its three token environment variables) or the plain-text git credential files; both can be set team-wide. `a2amx confine suggest [DIR]` looks for build files (`Cargo.toml`, `package.json`, `go.mod`, `pom.xml`, Gradle and Python markers) and prints a `grants` line for them (see "Confinement default set and auto mode" below), so a project does not need the recipe copied by hand. Build-file cache paths are opened through grants and not by default (see "Confinement default set and auto mode" below). The old flat team-file keys (`confine = true`, `confine_write` and the rest) are
  refused with a message naming the replacement. A `write`, `hide` or `unhide` entry must be absolute, `~/...` or
  relative to the working directory; `$VAR` and `~user` forms are refused instead of silently skipped, a listed path that
  is missing or unreadable gets its own `unresolved:` line in the dry run, and the agent notice always names the working
  directory. Credential environment variables (a fixed list of 29 names such as `GITHUB_TOKEN`, `NPM_TOKEN`, `VAULT_TOKEN`; never LLM provider keys, `A2AMX_*` or the Bedrock and Vertex logins) are unset in a confined session; `--confine-keep-env NAME` or `keep_env` in `[session.confine]` returns one, and a daemon upgrade alone changes the environment of every confined session. `write = ["~"]` is allowed: the hides stay on top of the writable home, `~/.a2amx` and the `PATH` directories inside the home stay read-only unless a `write` entry names them, and a session launched from the home directory is still refused; `docs/confinement.md` lists what stays open. The daemon finds `bwrap` and the omp quota probe only in absolute `PATH` directories: relative and empty `PATH` entries are ignored, because they resolved against a directory a confined agent can write. A confined Claude or OMP session can renew its login: its own harness directory (`~/.claude` or `~/.omp`) is read-write and persistent by default (see "Confinement default set and auto mode" below), and `/login` and saves to `~/.claude.json` from inside a session still need a normal terminal (`docs/confinement.md`, "Harness login and state files"). The agent is told its limits in its system prompt, the status bar shows `[confined]` (`[confined auto]` under auto mode), and `team up --dry-run` prints the effective
  restrictions. A session never starts unconfined when confinement was requested; Codex sessions are
  refused. It is a safety net for a trusted but fallible agent, not a security boundary. A Ctrl-C in a
  confined Generic session reaches the command, not the sandbox (see Changed). Additive protocol: the
  `Confine` request (its `allow_containers` field became `grants`; an older client's field is ignored) and
  `confined` on the status; restart an older daemon to use `--confine`
  (`docs/cli.md`, `docs/teams.md`, `docs/harnesses.md`).
- Confinement grant `ssh` (team or session): makes `~/.ssh` read-write and shows the root-owned `/etc/ssh/ssh_config`
  as an empty file, so `ssh` and `git push` over ssh work in a confined session. No ssh agent is passed through, so a passphrase-protected key cannot be used (`docs/confinement.md`).
- Confinement default set and auto mode: a confined session opens only its working directory, the git
  directory, a private `/tmp` and, for Claude and OMP, the harness's own config directory (persistent). There is
  no `~/.cache` or tool-directory default, no `build_paths` key and no `--confine-no-build-paths` flag; tool
  directories open through grants (`cargo`, `node`, `cache` and the others), which now create their missing tool
  directories (`~/.cargo`, `~/.npm`) when a real session starts. The `gh`, `ssh` and `git-credentials` grants may
  be set team-wide. `a2amx confine suggest` prints a `grants` line. New `[confine] auto = true` in the global team
  file (`<state home>/team.toml`, needs `enabled = true`) detects the project's markers and its git remote at each
  start and adds the matching grants plus `cache`; it is weaker than explicit grants, and a local team file,
  `--file` or `[session.confine]` that sets it is an error (`docs/confinement.md`, `docs/teams.md`).
- `a2amx explain SESSION` says why a session is in the state it is in: its activity and what that activity
  came from (a channel hold, a reset, a screen that is not ready, or how long the screen has been still),
  readiness (`no (starting)` for a session that has not reached a ready screen yet), whether it needs input,
  the hold and its explanation, the number of queued messages, and whether `reset` would be allowed and, if
  not, with which error code and reason. It changes nothing and is admin only. New request and response pair
  `explain` on the control protocol (`docs/cli.md`).

### Changed

- **Breaking:** the `a2amx new` flag `--confine-no-default-hides` is replaced by `--confine-default-hides=false`,
  the same setting under the name the team file already uses (`default_hides = false`). The old flag is now an
  unknown argument, there is no alias, and the new flag without `--confine` is refused like the other
  `--confine-*` flags (`docs/cli.md`, `docs/confinement.md`).
- `a2amx team status` marks a running session whose harness has not yet been seen at a ready screen: the NOTE is `<id> (not ready: starting)` (for example a Claude Code session stopped at a startup dialog). STATE stays `running` and the exit code is unchanged. Generic sessions never show it, and a session that was ready earlier and later shows a dialog is not marked. Additive protocol field: `starting` on the session summaries of the list response; an older client ignores it (`docs/teams.md`).
- A message body may not contain the text `<a2amx-message` (an opening envelope tag, compared without regard to ASCII case), as it already may not contain the closing tag: `send_message` fails with `invalid_content` and the text "message contains an envelope opening tag (write it without the leading <)". A body that quoted another open message's header made the prompt look like two envelopes, so the hook blocked it and charged a rejection to both messages (`docs/architecture.md`). `SECURITY.md` now says a confinement escape is welcome as an ordinary bug report and is not handled as a vulnerability.
- The `docker` grant is detected by `a2amx confine suggest` and by global `auto` mode when the project has a `Dockerfile`, a compose file (`compose.yaml`, `compose.yml`, `docker-compose.yaml`, `docker-compose.yml`) or a `.devcontainer`, with the usual consent prompt for an auto-picked grant; a team file may now name `docker` (it was refused before; `podman`, `containerd`, `crio`, `libvirt`, `lxd` and `snap` stay session-only). The container grant texts no longer say "effectively root on the host": they say what the grant adds and that a deliberate privileged container is root on a rootful daemon, and that `userns-remap` or a sandboxed runtime does not stop the holder of the socket. `a2amx confine --help` describes `suggest` as printing a grants line (`docs/confinement.md`, `docs/teams.md`, `docs/philosophy.md`).
- Breaking: messages from the daemon itself (peer exit events, activity notices, heartbeat digests) no longer
  say "From another agent" and no longer tell the recipient to reply to `a2amx-daemon@<host>`, which cannot
  receive one. Their envelope says "System notification from the A2AMX daemon, not your user. It cannot be replied
  to."; the Claude channel instructions and the operator line say they need no reply but may call for action.
  Messages between agents are unchanged. Sessions started earlier keep the old operator line until restarted.
- Breaking: the `quota` object in `list_agents` output and in the control protocol's session entries now names
  its fields `five_hour_left` and `weekly_left` (they were `five_hour` and `weekly`); the values are unchanged,
  percent left, and the `list_agents` tool description says so. An older client or daemon shows no quota.
- In a confined Generic session, Ctrl-C now reaches the command inside the sandbox instead of ending the
  session, so a shell, a Python REPL or a running command behaves as it does unconfined. Confining a Generic
  session now needs an `env` with `--ignore-signal` and `--default-signal` (GNU coreutils 8.31 or newer, or
  uutils); an older `env` is refused with a message and the session does not start. Claude and OMP sessions
  are unchanged (`docs/harnesses.md`, `docs/cli.md`).
- **Breaking:** a2amx and its daemon refuse to run when the effective uid is 0 (as root, under `sudo`, or as a
  root user in a container). The command prints `a2amx: refusing to run as root (effective uid 0): run a2amx
  as an ordinary user` and exits 1. There is no override. `--help` and `--version` still work. Start it as an
  ordinary user, and use a non-root user in containers (`docs/install.md`, `SECURITY.md`).
- The activity dot follows one colour table, in the status bar and on the team line: idle green, working
  blue (it was yellow), starting yellow (a Claude, Codex or OMP session that has not been ready yet),
  resetting orange, held red (a team member waiting on a delivery hold or for input; the attached
  session's own hold still shows as the red `HELD` alert instead of a dot), and exited grey. The status bar's
  words are idle, working, starting and resetting. Additive protocol fields: `held` and `starting` on team
  members and `starting` on the status; an older daemon sends neither, so a newer client draws members by
  the old rules, and an older client ignores them (`docs/architecture.md`).
- The `gh`, `git-credentials` and `ssh` grants now open their paths read-write instead of read-only, so
  `ssh` can record a new host key, `ssh-keygen` can write keys, and `gh` can rewrite its `hosts.yml`. A
  grant path is checked like a `write` entry: a symlink to your home directory (or above), into `~/.a2amx`,
  or into the session's own harness directory without the harness-config access is refused when the plan is
  built. The `git-credentials` files can be written in place but not replaced or deleted
  (`a2amx confine list`, `docs/confinement.md`).
- New grants, all read-write and settable at team or session level: toolchain caches `cargo`, `go`,
  `python`, `node`, `java`, `c-cpp`; analyser data `semgrep` and `sonar` (with `SONAR_TOKEN`); and the credential
  stores that are hidden by default, `aws`, `gcloud`, `azure`, `kube`, `terraform`, `npm-token`, `pypi-token`,
  `cargo-token` and `netrc`. A grant path that does not exist is not opened, and a hidden credential file inside a
  granted directory (`~/.cargo/credentials.toml`) stays hidden unless its own grant is given. **Breaking:** a grant
  path that passes through a symlink is now refused when the plan is built (it used to open the symlink's target);
  name the real path with a `write` entry (`a2amx confine list`, `docs/confinement.md`).

### Fixed

- A confined session could reach a hidden path through a path the tool picked itself. A `.git` file whose
  `gitdir:` points at `~/.ssh` made the session see and write `~/.ssh`, and a link planted in the harness
  data folder (for example `~/.claude/todos` to `~/.ssh`) did the same. The plan now refuses a working
  directory or git directory that lies inside a default hide, and names the `write` line that opens it. A
  `.git` file must point at a real git directory (`HEAD` and `objects`), and harness data links must stay
  inside the harness folder unless you name the target in `write`. An existing setup that had a project
  inside a hide, such as a `pass` store with a repository, now needs that one `write` line
  (`docs/confinement.md`, troubleshooting).

- A team-level `[confine] grants` list was accepted but never applied to the team's sessions; it is now merged
  before each session's own grants (`docs/teams.md`).

- A mouse drag that starts in the session and ends on the team line is forwarded to the session at the
  grid's first row instead of a row lower.

## [0.1.2] - 2026-10-26

### Added

- Activity notices: a session started with `activity` (team file `activity = { working_after =
  "2m", idle_after = "30s" }`, or `a2amx new --activity-working-after 2m --activity-idle-after
  30s`) is told when a watched peer starts or stops working. Each direction is debounced
  separately, the first settled state is silent, and the notice is one ordinary daemon message
  per watcher at a time. `team up` rejects teams whose watchers would notify each other in a
  loop. New optional field on the `new` request and the team file; older files and clients keep
  working (`docs/teams.md`, `docs/cli.md`).

### Fixed

- The quota shown for an OMP session now follows the provider of the model it is using, instead
  of the smallest value across all providers on the account. The OMP extension reports the
  current provider on each state frame (additive field `provider`), so a switch between
  providers changes the quota shown on the next list. A session whose provider is unknown shows
  `-` instead of another provider's quota (`docs/cli.md`).
- Updated `tokio` to 1.53.3 for RUSTSEC-2026-0386 (the safe `AsyncFd` constructors). A2AMX does
  not call `AsyncFd` directly.

## [0.1.1] - 2026-10-16

### Added

- Signed releases: pushing a version tag on `master` builds a Linux x86_64 archive in CI, signs it
  and its checksums with Sigstore, attests its build provenance and publishes a GitHub release
  with notes from this changelog (`docs/releasing.md`, `docs/install.md`).

### Changed

- The README is now a short landing page with an example and a quick start; the command,
  team-file, harness and installation reference moved to `docs/` (`cli.md`, `teams.md`,
  `harnesses.md`, `install.md`, `using-with-agents.md`, `roadmap.md`).

## [0.1.0] - 2026-10-15

### Added

- A local daemon that owns PTY sessions and keeps their screen state while detached, with
  attach, detach, switch, and scroll from a client that redraws from the terminal model.
- Durable SQLite messages, per-session tokens, and a stdio MCP server with `list_agents`,
  `send_message`, `message_status`, and `reset_session`.
- Message delivery into a session's composer that protects an unfinished human draft, with
  submission receipts.
- Native delivery channels for Claude Code (a channel), OMP (an extension), and Codex (a
  private app-server).
- Session names, a status line with quota, a session picker with team sections, and
  `a2amx team` to start and stop a team of sessions.

### Fixed

- An idle OMP session that holds queued next-turn messages of its own (for example after a failed `todo` call) no longer blocks message delivery or `reset_session`.
