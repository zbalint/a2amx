# Spec 3ax: Opt-in confinement of PTY sessions with bubblewrap

## 0. Status

**LOCKED** (2026-10-29; pre-lock gate run against `develop` at `f24a345`; consultant reviews `m_4261`, `m_4281` and `m_4286`
and the owner's assistant's review adopted; owner confirmations of 2026-10-29 recorded in section 7). Architect-directed, owner
request, context `a2amx-confine-2026-10-29`. Implementation goes to `a2amx-developer` only, on `develop` in the main checkout
`/home/zbalint/workspace/a2amx`; anchors are at `f24a345`.

**Baseline:** `env -u A2AMX_BIN -u NO_COLOR cargo test` (637 passed, 0 failed, 3 ignored on `f24a345`; the developer records
its own count before the first edit), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Known flaky tests:
`tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (backlog I14) and
`tests/daemon.rs::exited_claude_session_never_reports_needs_input` (timing); rerun once and record as pre-existing.
Mutant and scratch builds go on the home volume with their own `CARGO_TARGET_DIR`, never `/tmp` (a 3.9 GB tmpfs that
mutant builds have filled).
**Public test seams:** `confine::plan`, `confine::launch_argv` and `confine::notice` (`tests/confine.rs`, over a temp-dir
fixture); `harness::append_system_prompt`; `wire` serde round trips (`tests/wire.rs`); the team file parser
(`tests/team.rs`); the daemon's session creation with a real `bwrap` (`tests/confine.rs`, skipped when bubblewrap cannot
start; section 6).

**Scope.** New: `src/confine.rs`, `tests/confine.rs`. Edit: `src/lib.rs`, `src/wire.rs`, `src/cli.rs`, `src/main.rs`,
`src/team.rs`, `src/harness.rs`, `src/daemon.rs`, `src/session.rs`, `src/status.rs`, `tests/status.rs`, `tests/wire.rs`,
`tests/team.rs`, `tests/attach_cli.rs`, `tests/daemon.rs`, `tests/broker.rs`, `tests/common/mod.rs`, `docs/cli.md`, `docs/teams.md`, `docs/architecture.md`, `docs/harnesses.md`,
`.github/workflows/ci.yml` (4.9). Does not touch: `CHANGELOG.md` and `AGENTS.md` (architect), `Cargo.toml`, `Cargo.lock`,
`docs/backlog.md`, `docs/specs/`. No new dependency. Leave the diff uncommitted. (The test files beyond `tests/confine.rs` are there for the new
`DaemonConfig.bwrap_path` field: seven struct literals in `src/main.rs`, `tests/daemon.rs` (4), `tests/broker.rs`, `tests/attach_cli.rs`
and `tests/common/mod.rs`; write the field explicitly in each.)

## 1. Why

Owner request, 2026-10-28. An agent that is trusted but fallible can still delete, overwrite or leak what it can reach.
The harnesses' own sandboxes are narrower than an operator wants: Claude Code's covers shell commands only (its file
tools, MCP servers and hooks run outside it, and it is off by default), Codex's covers model shell commands only, and
OMP has none. a2amx confines the whole session process, the same way for every harness, and hides common secret
locations from reads.

**Threat model (state it in `docs/harnesses.md`).** Confinement is a safety net against a trusted but fallible agent's
accidents and casual overreach, not a security boundary like a container or a VM. The session shares the kernel, the
user ID, the network and the process list with everything else. It closes every escape the project knows of and makes no
claim that none exists. Anyone who needs a real boundary uses a container or a VM.

Two spikes (unmerged branches `spike/landlock`, `spike/bwrap`) decided the mechanism. The lessons are stated here:

- **Landlock is dropped.** A Landlock domain forbids the mount and pivot_root calls a harness's own bubblewrap sandbox
  needs, and it does not mediate chmod, chgrp, utimes or xattr.
- **bubblewrap works outside the harness sandbox.** Nested bubblewrap works, Codex's own sandbox starts inside an outer
  bwrap and stacks with it, `--die-with-parent` cleans up, resize and exit codes propagate, and chmod, chgrp, utimes
  and creation outside the binds are denied. Do not use `--new-session` (it detaches the controlling terminal).
- **Confinement is opt-in, bubblewrap-only.** If a compatible bwrap is missing the operator relies on the harness's own
  sandbox; a2amx never starts a session unconfined when confinement was requested, and never loosens a harness's own
  protection.
- **A read-only root is not confinement by itself.** Connecting to a Unix socket works through a read-only mount.
  Probed 2026-10-29 on the dev machine (WSL2, bwrap 0.11.1, user in the `docker` group): inside `--ro-bind / /`,
  `systemd-run --user` over the session bus started a command **outside the sandbox** where `$HOME` was writable;
  `curl --unix-socket /var/run/docker.sock` answered (Docker is root on the host); and a `connect()` to
  `/run/WSL/*_interop` succeeded (the WSL interop that starts Windows programs; binfmt `WSLInterop` carries flag `F`, so
  it works across mount namespaces). Sockets of same-user or root services are an escape and are hidden (2.3 items 4-4a).
  Hiding `$XDG_RUNTIME_DIR`, `/run/docker.sock` and `/run/WSL` closed each in the probe.
- **Writable configuration is a way to persist into the next unconfined run.** Hooks, plugin and extension directories,
  the scripts a settings file points to, `.git/hooks` and `.git/config`, and the team file are code the operator will
  run later without a sandbox. A deny-list of such paths always misses one (the consultant found `~/.claude/statusline.sh`
  and `~/.claude/dev-mods` on this machine), so a harness's directory is read-only and only a short allow-list of data
  paths inside it is writable (2.3 item 3).
- **Codex is not covered here.** In `--remote` mode its tools run in the session's private app-server, started outside
  `Session::spawn` (`src/daemon.rs:601`), and all Codex sessions share one socket directory, so a confined app-server
  could replace another session's socket. A per-session socket directory is the follow-up (backlog F31). This spec
  refuses `--confine` for Codex.

## 2. Design (owner decisions in bold)

### 2.1 Enablement

- **Per session, off by default.** `a2amx new --confine`, with the repeatable `--confine-write DIR`,
  `--confine-hide PATH` and `--confine-unhide PATH`, and the switches `--confine-no-default-hides` and
  `--confine-allow-containers`. Team file session keys: `confine = true`, `confine_write = [...]`, `confine_hide = [...]`,
  `confine_unhide = [...]`, `confine_no_default_hides = true` (default false; drops the default hides of 2.3 item 5, never
  the mandatory ones) and `confine_allow_containers = true` (default false; turns off the container restriction of 2.3
  item 4a). A top-level `confine` (bool) in the team file is a default for its sessions; a session key overrides it. Local
  and global team files take the same keys. A confine key on an unconfined session (`confine` false), and any `--confine-*` flag given without `--confine`, is an
  error, so a typo fails loudly instead of enabling confinement silently.
- Relative `confine_write`, `confine_hide` and `confine_unhide` entries in a team file resolve like `cwd` (spec 3w D3);
  flag values resolve against the invoking directory.
- **Wire: a wrapper request, safe defaults.** `Request::Confine { confine: ConfineRequest, request: Box<Request> }` with
  `ConfineRequest { write: Vec<String>, hide: Vec<String>, unhide: Vec<String>, no_default_hides: bool,
  allow_containers: bool, team_file: Option<String> }`, every field `#[serde(default)]`, so that an absent field is the safe value. The daemon
  accepts only a `NewSession` inside it (a nested `Confine` or any other request is an error) and passes `Some(confine)`
  to `create_session`. A `Confine` whose `NewSession` has `cwd: None` is refused. An older daemon answers an unknown
  variant with `invalid control request` (`src/daemon.rs:1889`), so it fails closed; the client maps that answer, for a
  `Confine` request only, to `the running daemon does not support --confine; restart it`. `session_request_denied`
  (`src/daemon.rs:1896`) runs before dispatch, so a session token cannot send `Confine`.
- `team up` creates sessions one by one and stops at the first error, leaving earlier ones running
  (`src/main.rs:1121-1130`); a refused confined session behaves the same. Static errors (Codex, bad paths, the guards of
  2.4) are found by team validation before any session starts; the preflight result is daemon-side and appears at creation.

### 2.2 What is confined

- **Harnesses.** Claude, Omp and Generic. **Codex is refused** (`confinement is not supported for Codex sessions yet: its
  tools run outside the session process`) by `confine::check_harness(harness)`, called by `create_session`
  (authoritative) and by team validation (so `team up --dry-run` shows it).
- **Fail closed.** `confine::preflight(bwrap: &Path) -> anyhow::Result<()>` runs `<bwrap> --version` and, bounded to 5
  seconds, a probe with the real base options of 2.3 item 1 and `true` as the command. A missing binary, a failed probe
  or a timeout refuses the session with the reason. The bwrap path is resolved **per request** from `DaemonConfig.bwrap_path` (a new `Option<OsString>` holding a
  PATH-style search list; `None` means the process `PATH` read at `Daemon::start`) to an absolute path, and that path is
  what is executed; the client-supplied environment never selects the binary. Tests give each in-process test daemon its
  own `bwrap_path` (a directory with a stub), because the test binary's own `PATH` is shared by parallel tests and
  `set_var` is unsafe in edition 2024.
  `create_session` runs the preflight on a blocking thread. Argument vectors only. **No fallback:** once a `Confine`
  request exists, no code path starts the unwrapped command. A failed plan, preflight or spawn ends in an error and no
  session; a bwrap that exits at once after a good preflight is an exited session, never a retry without it. The only
  `tty::Shell::new` call in `Session::spawn` takes the result of `confine::launch_argv` when a plan is present.
- **Inherited descriptors.** Hiding a path does not revoke a descriptor the process already holds (probed: a descriptor
  opened before `bwrap` stays readable after the path is hidden, and `bwrap` does not close it). The confined child must
  therefore hold no descriptor above 2. The daemon's own descriptors are created close-on-exec; the real-bwrap test lists
  `/proc/self/fd` in the child and requires only 0, 1, 2 and the listing descriptor. A failure is a daemon descriptor leak
  to fix at its source; the developer reports it as BLOCKED if the fix lies outside the scope.
- The wrapped process is the PTY child; everything it starts (file tools, hooks, MCP servers, shell commands) inherits the
  mount, PID and IPC namespaces. The PID namespace (`--unshare-pid`) means the agent sees only its own processes, so a
  stray `pkill` or `killall` cannot hit the daemon, a peer or the operator's editor (a same-user host process otherwise
  receives signals, and `kill(2)` of the daemon would end every session; ptrace and `/proc/<pid>/mem` are already denied
  across the user namespace, probed). Probed: host PIDs are invisible, the exit code propagates and killing the outer bwrap
  ends the child. Terminal behaviour (resize, Ctrl-C to the foreground job) under `--unshare-pid` is checked by the
  real-bwrap test and the interactive tester pass; a problem there is a BLOCKED, not a silent removal of the flag. The
  abstract Unix socket namespace belongs to the network namespace and is not isolated.

### 2.3 Mount plan

`confine::plan(request, harness, cwd, host) -> anyhow::Result<Plan>` and `Plan::wrap(&self, argv) -> Vec<String>` produce
`bwrap <options> -- <argv>`. `Host { home, state_dir, runtime_dir, empty_file }` is plain data; tests pass a temp dir.
**Every path (cwd, write, hide, unhide, defaults, the state directory) is canonicalised before it is compared, ordered or
mounted**, because mounts follow symlinks (probed: `/var/run` is a link to `/run`, and a bind on `/var/run/docker.sock`
fails with `Can't create file`). A path that does not exist is skipped (bwrap cannot create a mount point on the read-only
root; probed) and listed as `skipped (missing)` in `--dry-run`. The whole bwrap argument vector must stay under 256 KiB
or the session is refused (a single argument is limited to 128 KiB).
`// shortcut:` the daemon canonicalises and bwrap resolves again, so a path swapped in between is not caught; the ceiling
is a same-user attacker racing a session start, the trigger is a stronger threat model.

1. **Base:** `--die-with-parent --unshare-user --unshare-ipc --unshare-pid --ro-bind / / --dev /dev --proc /proc --size
   1073741824 --tmpfs /tmp` (`--unshare-user` explicit so that a setuid bwrap, which would not create a user namespace,
   still gets one or fails the preflight; the 1 GiB cap keeps a runaway write from filling half the memory), and always
   `--unsetenv DBUS_SESSION_BUS_ADDRESS --unsetenv SSH_AUTH_SOCK --unsetenv GPG_AGENT_INFO --unsetenv WSL_INTEROP
   --unsetenv DISPLAY --unsetenv WAYLAND_DISPLAY`. While the container restriction of item 4a is active it also unsets the
   container-engine variables listed there. bwrap also leaves `NoNewPrivs` set (probed), so `sudo` and setuid programs gain nothing. `/tmp` is private to the session and discarded when it ends (peers share
   files through `/tmp` today; the notice and the docs say a confined agent cannot see a peer's `/tmp` paths).
2. **Writable (`--bind`):**
   - the workspace: the session's canonical `cwd`, which goes through `check_write_allowed` (so `cd ~ && a2amx new
     --confine claude`, `/` and a directory containing the state directory are refused);
   - the git common directory: walk up from `cwd` to the first `.git`, stopping before `$HOME` (a dotfiles repo in
     `$HOME` is not bound). A `.git` directory is itself the common directory. A `.git` file holds `gitdir: X` (relative to
     the file's directory when not absolute); a `commondir` file inside X names the common directory (relative to X); with
     no `commondir` (a submodule) X is the common directory. `GIT_DIR` and `GIT_COMMON_DIR` in the environment are ignored.
     The result goes through `check_write_allowed`. A `cwd` below the repository root makes only that subdirectory
     writable (intended);
   - the harness's data paths (item 3);
   - the operator's `confine_write` entries.
3. **Harness directories are read-only except a data allow-list** (a named constant, existing paths only; the harness
   directory itself is not bound, so everything else in it, including settings, plugins, agents, skills, hooks, mods,
   scripts and native binaries, stays read-only):
   - Claude, under `~/.claude`: `projects`, `todos`, `shell-snapshots`, `session-env`, `sessions`, `statsig`,
     `file-history`, `paste-cache`, `debug`, `cache`, `telemetry`, `plans`, `backups`, and the files `history.jsonl`,
     `stats-cache.json` and `.credentials.json`; plus the file `~/.claude.json` (bound as a file, never `$HOME`).
   - Omp, under `~/.omp`: `cache`, `logs`, `run`, `agent/sessions`, `agent/projects`, `agent/cache`, `agent/blobs`, the
     existing files `agent/*.db`, `agent/*.db-wal`, `agent/*.db-shm`, `stats.db`, `stats.db-wal` and `stats.db-shm`. (`// shortcut:` the database files are
     listed once at plan time; a database created later is not writable. Upgrade trigger: an OMP version that creates
     databases after start.)
   - Generic: none.
   `confine::ensure_data_dirs` creates missing allow-listed **directories** (mode 0700) before launch, only when the
   harness directory already exists, because a read-only parent would stop the harness creating them. Files are never
   created, with one exception: for each listed OMP database, `ensure_data_files` creates the empty `-wal` and `-shm`
   sidecar files (0600) when they are missing, because SQLite deletes them when the last connection closes and cannot create
   them in a read-only directory on a cold start. The exact allow-lists are corrected by the tester pass (O2): a missing entry shows as a visible `Read-only
   file system` error, never as a silent hole.
   **Protected inside a writable bind** (`--ro-bind P P`, existing paths only, applied after the binds): `<common>/hooks`,
   `<common>/config`, `<common>/config.worktree`; the directory named by `core.hooksPath` in `<common>/config` when it lies
   inside a writable path (for example `.husky/`); the team file the session was started from when it lies inside a
   writable path. An exact path named in `confine_write` is writable again. `~/.claude.json` holds user-scope MCP server
   commands and cannot be protected because Claude replaces it by atomic rename; it is a documented residual risk, as is
   everything inside the workspace (2.7).
4. **Mandatory hides, never overridable** (a named constant; tmpfs + remount-ro for directories, the empty file for
   files): `$XDG_RUNTIME_DIR` (session bus, systemd, gpg and ssh agent sockets; rootless Docker, Podman and containerd
   sockets live here and so stay unreachable even with containers allowed); `/run/WSL` (WSL interop) and `/mnt/wslg`
   (WSLg display sockets); **the whole state directory**, except that for the Omp harness `<state>/omp` is exposed
   read-only on top (the OMP extension and overlay are written there, `src/omp.rs:26-36`). Nothing else in a session reads
   the state directory (`hook`, `mcp`, `bridge` and `channel` use the environment). Only the session bus (and the agent sockets) is hidden by force; the system bus
   directory `/run/dbus` is an overridable default (item 5), so desktop integrations that use the system bus can be kept.
   Principle: any reachable Unix socket of a same-user or root service is an escape, and `admin.token` is the only credential that can send `NewSession`
   (`src/daemon.rs:2027`).
4a. **Container restriction, its own switch** (owner; on by default for a confined session): hide these sockets with the
   empty file: `/run/docker.sock`, `/run/podman/podman.sock`, `/run/containerd/containerd.sock` and
   `containerd.sock.ttrpc`, `/run/buildkit/buildkitd.sock`, `/run/crio/crio.sock`, `/run/k3s/containerd/containerd.sock`,
   `/run/libvirt/libvirt-sock`, `/run/snapd.socket` and `/run/snapd-snap.socket` (a world-writable package-manager socket on this
   machine), `/var/snap/lxd/common/lxd/unix.socket`, `/var/lib/lxd/unix.socket`,
   `/var/lib/incus/unix.socket`, `~/.docker/desktop/docker.sock` and `~/.docker/run/docker.sock`; and unset `DOCKER_HOST`,
   `DOCKER_CONTEXT`, `CONTAINER_HOST`, `CONTAINER_CONNECTION`, `CONTAINERD_ADDRESS`, `BUILDKIT_HOST` and
   `LIBVIRT_DEFAULT_URI`. These do not depend on the overridable `~/.docker` hide. `confine_allow_containers = true` turns
   the whole item off: the sockets stay visible and the variables pass through. The docs state that this gives the agent
   root-equivalent power over the host (a member of the `docker` group can mount the host's root), and that an engine on
   another machine is out of reach of any local hide.
5. **Default hides, overridable** (a named constant; **secret locations only, no user folders** (owner): an operator who
   wants `~/Documents` or similar hidden adds it with `confine_hide`): `~/.ssh`, `~/.gnupg`, `~/.aws`,
   `~/.config/gcloud`, `~/.kube`, `~/.docker`, `~/.netrc`, `~/.local/share/keyrings`, the Chromium and Firefox profile
   directories, `~/.config/gh`, `~/.git-credentials`, `~/.config/git/credentials`, `~/.npmrc`, `~/.pypirc`,
   `~/.cargo/credentials` and `~/.cargo/credentials.toml`, `~/.azure`, `~/.terraform.d`, `~/.vault-token`, `~/.pgpass`,
   `~/.password-store`, `~/.config/op`, `~/.cache/huggingface/token`, `~/.config/rclone`, `~/.s3cfg`, `~/.claude/ide` (IDE
   lock files hold a loopback port and token; hiding it may break `/ide`) and the system bus directory `/run/dbus`. A
   token a tool needs in the session (for example `gh` for pushes) is exposed with `confine_unhide`. The network is open,
   so a readable token is a way to act as the operator on a remote service. `confine_no_default_hides = true` drops all of them; `confine_unhide` exposes a path read-only
   (a path equal to a hide removes it; a path inside a hidden directory, such as one key file under `~/.ssh`, is bound
   read-only on top of it). **The other harnesses' state directories stay readable and read-only** (owner), **but their
   credential files are hidden** (a default hide, so the secrets-only principle holds): `~/.codex/auth.json` for every
   session that is not Codex, and `~/.claude/.credentials.json` for every session that is not Claude; an operator hides more
   with `confine_hide`. **`.env` files are not hidden** (owner; revisit later). Hidden files use
   `--ro-bind <empty_file> P`; a `/dev/null` bind fails with permission denied (probed). `empty_file` is
   `<state>/confine-empty`, an empty 0444 file the daemon creates at start; the state directory is hidden by tmpfs but a
   bind source resolves against the host, so it still works (probed).
6. **Order.** Operations apply by path depth, shallowest first; at equal depth hides come before binds; at the same path a
   bind (writable or read-only) replaces the hide. Every `--remount-ro` comes last, then `--chdir <cwd>`. [probed: tmpfs,
   then workspace bind, then remount-ro leaves the workspace writable and the rest of the hidden directory read-only; the
   reverse order hides the workspace; remount-ro before the bind fails; a hide nested in a writable bind works; read-only
   binds over `.git/hooks` and `.git/config` block edits while `index.lock` can still be created.]

### 2.4 Guards on operator paths (trust boundary)

All guards act on canonical paths. `check_write_allowed(path, home, state_dir)` rejects `/`, `$HOME`, any path that equals,
contains or lies inside the state directory or `$XDG_RUNTIME_DIR`, and a path that is not absolute after resolution. It is
applied to `cwd`, `confine_write` entries and the git common directory. `check_unhide_allowed(path, state_dir,
runtime_dir)` rejects an unhide that equals, contains or lies inside a mandatory hide (the built-in `<state>/omp`
exposure is the only exception) and `$HOME`. Lists are capped at 64 entries of 4096 bytes. The state directory is `--home`,
`$A2AMX_HOME`, `$XDG_STATE_HOME/a2amx` or `~/.local/state/a2amx`, so it is not always under `$HOME`; both guards name
`$HOME` explicitly.

### 2.5 Visibility

`wire::StatusInfo` gains `confined: bool` (`#[serde(default, skip_serializing_if = "is_default")]`); the attach status
bar shows `[sandbox]` in the dim attribute (`\x1b[2m` ... `\x1b[22m`) directly after the session name and before the dot,
whatever the activity state, and it is the last item the width-degradation loop drops (`src/status.rs:246-253`). `team up
--dry-run` prints per session `confined` or `unconfined`, the extra writable directories, the protected paths that exist,
and the final hidden list with `skipped (missing)` marks. `a2amx list` keeps showing the command the client sent
(`SessionSpec.argv`), not the wrapped or notice-bearing argv.

### 2.6 Telling the agent (owner)

A confined session's agent is told what it can and cannot touch, so it does not discover the limits as `Read-only file
system` errors. `confine::notice(plan: &Plan) -> String` renders the text **from the same `Plan` that builds the mounts**.
Content, in plain words, never naming bubblewrap: the session is confined; the writable locations; the read-only protected
paths; the hidden locations (a hidden directory looks empty, a hidden file looks empty); everything else is read-only;
`/tmp` is private to this session and discarded when it ends, and paths under a peer's `/tmp` cannot be read; whether
container engines are `allowed` or `restricted`; and that a `Read-only file system` error is authoritative where the notice
and the system disagree. It lists paths only, never reads or quotes file content, skips missing paths, prints at most 20
entries per category then `and N more`, and the whole notice is capped at 12 KiB. It ends with this fixed constant (owner;
wording reviewed by the consultant), always present and never trimmed by the caps:

> Do not try to get around these limits, whether through other programs, services or sockets, through other agents, or by
> editing configuration to loosen them. If your task needs access they do not allow, stop that part and tell the person
> running this session which path or access you need and why. Other errors, such as a missing file or a failing build or
> test, are ordinary: handle them as usual.

It is advice to a cooperative agent, not enforcement (2.7).

**Delivery.** The Claude and Omp wiring runs in the client (`src/main.rs:518-550`; `src/harness.rs:330-333`, `:454-457`)
before the daemon has the plan, so the daemon adds the notice: `Session::spawn` builds the launch argv with
`confine::launch_argv(plan, notice, argv, harness)`, which calls `harness::append_system_prompt(argv, notice)`. Both
harnesses keep only the last `--append-system-prompt` value (OMP: source string `e.appendSystemPrompt = t`; Claude:
commander single-value option, inference). The rules: (a) target the **last** occurrence before a `--` argument, and never
look past `--`; (b) handle the `--append-system-prompt=VALUE` form; (c) append the notice after a blank line, or insert
the flag with `insert_extras` when absent; (d) for Omp, whose option accepts text **or a file path**, refuse confinement
with a clear error when the existing value names an existing file (never read the file into argv); (e) for Claude, refuse
confinement when `--append-system-prompt-file` is present. Generic sessions have no system-prompt channel and get no
notice (the docs say so); Codex is refused; an unconfined session's argv is never changed.

### 2.7 What confinement does not cover (state in `docs/harnesses.md` and `docs/cli.md`)

Network access, including any service listening on localhost (an IDE's MCP server with a readable token, Jupyter, dev
servers, databases, and on WSL2 the Windows host's forwarded localhost ports): a process can ask such a service to act for
it, and only a network namespace would stop that (it would also stop the agent's own API access); abstract Unix
sockets (an X server's abstract socket); requests to peer sessions through `send_message`; kernel keyrings (`keyctl`,
inference); CPU, memory outside `/tmp` and disk exhaustion (no cgroups); instructions rather than code planted in the data
the next unconfined run reads (a harness's per-project memory, todos and plans, the workspace's `CLAUDE.md` and
`AGENTS.md`, OMP's agent databases); Windows drives
under `/mnt` on WSL, which stay readable; everything the agent may write in its writable set. The writable set includes
files that run later outside the sandbox: the workspace's `package.json` scripts, `Makefile`, `.pre-commit-config.yaml`,
`lefthook.yml`, `.husky/`, `.vscode/tasks.json`, `.envrc`, and the repository's `.claude/settings*.json` and `.mcp.json`
(hooks and MCP commands for the next Claude run in that repository). **Review the workspace diff before you build, test
or commit it outside a sandbox.** Confinement protects files and the listed code-carrying paths; it does not make the
writable set trustworthy.

## 3. Out of scope

Codex (backlog F31); a `confine check` diagnostic command that prints the effective restrictions of a configuration and
runs representative blocked-operation probes (backlog F32; `team up --dry-run` already prints the effective restrictions);
network restriction; a read allow-list; `.env` hiding; cgroup limits; hiding by glob;
Landlock; protecting the repository's own `.claude/settings*.json` and `.mcp.json` (an operator-listed
protected path is a possible follow-up); a user's own `--append-system-prompt` being silently replaced by a2amx's peer
prompt (pre-existing; backlog); changing any harness's own sandbox settings; the picker.

## 4. Changes

Line anchors are for `f24a345`.

- **4.1 `src/confine.rs` (new).** `Host`, `Plan`, `plan`, `Plan::wrap`, `launch_argv`, `notice`, `ensure_data_dirs`, `ensure_data_files`,
  `check_harness`, `check_write_allowed`, `check_unhide_allowed`, `preflight`, `resolve_bwrap(path_var)`,
  `ensure_empty_file(state_dir)`, the named constants of 2.3 items 3-5 and 4a, and the notice closing sentence. The only
  module that names `bwrap` or runs it. The daemon creates `<state>/confine-empty` at start (0444).
- **4.2 `src/wire.rs`.** `ConfineRequest`, `Request::Confine`, `StatusInfo.confined`. `src/lib.rs`: `pub mod confine`.
- **4.3 `src/session.rs`.** `SessionSpec` gains `confine: Option<ConfineLaunch { plan, notice }>`; `Session::spawn`
  (`:289-311`) builds `tty::Shell::new` from `confine::launch_argv(...)` when set; `Session.argv` keeps the client's command;
  `Session::confined()` for the status.
- **4.4 `src/daemon.rs`.** Dispatch of `Request::Confine` next to `NewSession` (`:1914`); `create_session` (`:322`) takes the
  optional request, runs `check_harness`, `preflight` (blocking), `plan`, `ensure_data_dirs`, `ensure_data_files`, `notice`, and fills
  `SessionSpec.confine` (`:425`); creates `confine-empty` at start; `attachment_status` fills `StatusInfo.confined`.
  `DaemonConfig` gains `bwrap_path: Option<OsString>` (2.2; `tests/common/mod.rs` sets it per test daemon).
  `Daemon::start` also marks every inherited descriptor above 2 close-on-exec (`rustix::io::close_range` with
  `CLOSE_RANGE_CLOEXEC`) so the in-process test daemons and a daemon started from a wrapper do not hand a harness
  descriptor to a child; use it only if the pinned `rustix` features allow it without touching `Cargo.toml`, otherwise
  report it as a note. `session_token_permitted` (`:2027`) is unchanged and must stay so.
- **4.5 `src/harness.rs`.** `append_system_prompt` (2.6 rules a-e).
- **4.6 `src/cli.rs`, `src/main.rs`.** The new `new` flags and a path-resolution helper shared with the team code; the client
  wraps `NewSession` in `Confine` when `--confine` is given (any other `--confine-*` flag without it is an error) and maps
  the old-daemon answer; `--dry-run` output. The new dry-run lines come after the existing `would start ... harness: ...
  cwd: ...` block (`src/main.rs:973`), whose tests use `contains()` (`tests/team.rs:1426-1463`).
- **4.7 `src/team.rs`.** `TeamFile.confine`, the `TeamSession` keys (`deny_unknown_fields` stays), validation (Codex, guards,
  list caps, confine keys without `confine`, and the `--append-system-prompt` refusals of 2.6 rules d-e, resolved against
  the session `cwd`, so `--dry-run` reports them). New fields break struct literals: `TeamSession` literals in `tests/team.rs`
  (4) and `src/team.rs` (2); `StatusInfo` literals in `tests/attach_cli.rs` (2), `tests/status.rs` (2), `tests/wire.rs` (2)
  and `src` (2). Write the new field explicitly in each; do not add `Default`.
- **4.8 `src/status.rs`.** The `[sandbox]` tag.
- **4.9 `.github/workflows/ci.yml`.** If the sysctl probe of section 6 works, one step before the tests; otherwise untouched.
- **4.10 Docs.** `docs/cli.md` (flags), `docs/teams.md` (keys), `docs/harnesses.md` (threat model, what is covered, 2.7, the
  notice), `docs/architecture.md` (the module and the mount plan).

## 5. Acceptance

- `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `tests/confine.rs`, `plan`/`wrap` with literal expected argv: Generic, Claude and Omp; workspace under an operator-hidden
  directory such as `~/Documents` (order); a hide inside the workspace; writable path equal to a hidden path; missing paths
  skipped; `confine_no_default_hides = true`; `confine_unhide` of a default (`~/.aws`) and of one file inside a hidden
  directory (`~/.ssh/config`), read-only; the mandatory hides present even with defaults off (`$XDG_RUNTIME_DIR`,
  `/run/WSL`, `/mnt/wslg`, the state directory); the Omp `<state>/omp` exposure only for Omp; harness directories
  read-only with only the data allow-list writable; protected paths over a writable bind including a `core.hooksPath`
  directory; git common directory for a plain repo, a repo subdirectory, a worktree with relative `gitdir`/`commondir`, a
  submodule, and `$HOME` as a repository; symlinked `cwd` and `/var/run`-style link targets; the guards of 2.4 (each case,
  plus `$HOME`, `cwd = $HOME`, a path inside the state directory, an unhide of `admin.token`, of the state directory and of
  `$XDG_RUNTIME_DIR`); the Codex refusal; the always-unset environment variables; the container switch (default: sockets
  hidden and variables unset; allowed: sockets and variables untouched; an unconfined session's environment never changed).
- Notice and delivery tests: `notice` has literal expected text including the closing sentence (writable, protected,
  hidden, `/tmp` line, containers restricted and allowed, the 20-entry cap, the 12 KiB cap, missing paths skipped);
  `append_system_prompt` handles the last occurrence, the `=` form, a flag after `--`, an absent flag, and the Omp
  file-path and Claude `--append-system-prompt-file` refusals; a confined Claude session's launched argv carries the notice
  while `a2amx list` still shows the client's command; an unconfined session's argv is byte-identical to today; a Generic
  session gets no notice.
- Fail-closed tests (each test daemon gets its own `bwrap_path` directory; no `set_var`): with a stub `bwrap` that fails its probe, with one that passes the probe and
  then exits non-zero, and with none, `Confine` creates no session (or an exited one) and never runs the command
  unwrapped; `rg -n "tty::Shell::new" src/session.rs` finds exactly one call and it takes `launch_argv`'s result.
- Filesystem edge cases (pure `plan` tests): a symlinked workspace, a symlinked hide target, a hide that is a symlink to the
  workspace, overlapping and nested rules in every order, a missing path in each list, a path given twice, a trailing slash,
  and `..` segments.
- Real-bwrap tests (skip when `preflight` fails; local run with `A2AMX_REQUIRE_BWRAP=1`): a confined Generic session cannot
  create a file outside its binds, can create one in the workspace, `chmod` outside is denied, `admin.token` is not
  readable, a fixture directory named in `confine_hide` is empty, the child's `/proc/self/fd` lists only 0, 1, 2 and the listing descriptor, `NoNewPrivs` is 1, a connect to a fixture unix socket under a fake runtime
  directory fails and so does one to a fake interop socket under a fake `/run/WSL`, the status shows `confined`, a
  `Confine` with `cwd: None` is refused, and `PATH` without bwrap refuses the session and creates none.
- Wire, team and CLI tests: round trips; absent means the safe default; old shapes stay green; an old-daemon answer maps to
  the restart message; a nested `Confine` is refused.
- `rg -n "bwrap" src` finds only `src/confine.rs` and user-visible strings in `src/cli.rs`, `src/daemon.rs` and
  `src/main.rs`; every `Command::new` in `src/confine.rs` runs the resolved bwrap path and none runs a shell.
- Abuse cases (each fails on a build without the guard): a confined session cannot reach a protected file through the
  `/proc` entry of an unconfined process (a test helper holds a fixture file open and the child cannot read
  `/proc/<pid>/fd/N`, `root`, `cwd`, `environ` or `maps`, and with `--unshare-pid` cannot see the process at all; the
  `--unshare-user` flag is what makes the denial hold, so the test also fails if it is dropped from the base options); a
  confined session cannot send `NewSession` (no admin token);
  `confine_write = ["/"]`, `$HOME`, `cwd = $HOME` and a state-directory path are refused; a hide symlinked into the
  workspace stays hidden; the runtime-directory, container-socket and interop-socket escapes are closed.
- Mutation check (developer, in a scratch copy on the home volume): drop the `--remount-ro`; swap the depth order; drop the
  state-directory hide; drop the runtime-directory hide; drop the `/run/WSL` hide; let `confine_unhide` expose a mandatory
  hide; drop the Codex refusal; let `preflight` return Ok on a failing probe; drop the protected-path overlay; bind a whole
  harness directory writable; drop the container-socket hide; unset the container variables even when containers are
  allowed; build the notice from a list other than the plan's; drop the 20-entry cap; drop the closing sentence; let
  `cwd = $HOME` through; fall back to the unwrapped command when the plan fails. Name the failing test for each.
- Tester pass (O2): a real interactive Claude and a real OMP session start, work, write their data and resume under
  confinement, **starting OMP cold (no OMP process running, so no `-wal`/`-shm` exists yet)**, forcing a Claude
  credential refresh or checking its fallback for `.credentials.json` (a refresh that cannot be saved could invalidate a
  rotated refresh token and log out every session), checking resize and Ctrl-C under `--unshare-pid`, and looking for
  sockets in `~/.claude` and `~/.omp/run` while the harnesses run; the allow-lists of 2.3 item 3 are corrected from what fails; Claude's `~/.claude.json` and
  `.credentials.json` updates succeed or fail as documented; OMP's extension loads (Bun transpile cache, if any).
- `git status --short` shows only scope paths.

## 6. CI

GitHub's Ubuntu runners restrict unprivileged user namespaces by default; the developer tries the documented
`kernel.apparmor_restrict_unprivileged_userns=0` sysctl step in CI (inference, unverified). If it works the real-bwrap tests
run there with `A2AMX_REQUIRE_BWRAP=1`; if not they skip and the pure `plan`/`notice` tests plus the PATH-stripped refusal
test (which needs no bwrap) carry the coverage. The Sonar new-code gate is read after the push.

## 7. Items settled at the tester pass, and owner confirmations

- **O1. Claude's `~/.claude.json`.** Written through `~/.claude.json.tmp.<pid>.<hash>` in `$HOME` (seen on disk). Decision:
  bind the single file and do not make `$HOME` writable (a writable `$HOME` allows new `~/.bashrc`, `~/.gitconfig`,
  autostart entries). A rename onto a bound file fails with `Device or resource busy` and an in-place write works (probed);
  Claude's installed binary appears to have a non-atomic fallback (consultant, inference from minified source), so the tester
  confirms with a real run. `CLAUDE_CONFIG_DIR` is rejected (it moves the file and loses user-scope MCP servers). If the
  fallback fails, Claude's confinement is documented unsupported and the spec is amended before acceptance.
- **O2.** Real interactive Claude and OMP under bwrap (section 5, tester pass), including `.credentials.json` refresh by
  rename inside a read-only `~/.claude` (a refresh that fails to save may log out every session if refresh tokens rotate,
  inference; the docs warn about it until the tester shows the fallback works), whether Claude needs `~/.cache` or `~/.local/share/claude` writable (auto-update is
  expected to fail inside the sandbox), and the allow-lists.
- **Owner confirmations (2026-10-29):** (1) with harness directories read-only except data paths, a confined Claude cannot
  edit its own `settings.json`, `CLAUDE.md`, plugins, agents, skills or hooks, nor write the repository's protected hooks
  and config: accepted. (2) The repository's own `.claude/settings*.json` and `.mcp.json` stay writable and are named in 2.7:
  accepted. (3) The other harnesses' credential files are hidden by default while their directories stay readable: accepted.

## 8. Claims

| # | Claim | Evidence | Verified how |
| --- | --- | --- | --- |
| C1 | `/dev/null` bind over a file cannot be opened; an empty regular file reads 0 bytes and rejects writes | scratchpad probe 2026-10-29, bwrap 0.11.1 | probed |
| C2 | tmpfs, workspace bind, remount-ro: workspace writable, rest read-only; reverse order hides the workspace; remount-ro before the bind fails | same | probed |
| C3 | tmpfs `/tmp` is writable; default `/tmp` is read-only | same | probed |
| C4 | Hiding a missing path fails | same | probed |
| C5 | Sessions get `A2AMX_TOKEN` and `A2AMX_ADDR` by environment; only the human CLI reads `admin.token`; `hook`, `mcp`, `bridge`, `channel` never read the state directory | `src/daemon.rs:406`, `src/client.rs:59`; consultant grep | read |
| C6 | `Session::spawn` is the only PTY-child creation; Reset never respawns; Codex's app-server starts outside it | `src/daemon.rs:425`, `:601`; `src/session.rs:289-311` | read |
| C7 | `Request::NewSession {` literals: 45 in tests across 12 files, 3 in `src` (48); untouched by the wrapper design | consultant `git grep` at `f24a345` | grepped |
| C8 | bwrap 0.11.1 at `/usr/bin/bwrap` | `bwrap --version` | ran |
| C9 | An unparseable request gets `invalid control request` and the connection continues | `src/daemon.rs:1889` | read |
| C10 | A session token may send only `SendMessage`, `Reset`, `ListAgents`, `MessageStatus`, `ReportPrompt`, `BridgeAttach`; `NewSession` needs the admin token | `src/daemon.rs:2027` | read |
| C11 | `~/.claude`, `~/.claude.json`, `~/.codex`, `~/.omp` exist; atomic temp files sit in `$HOME` | `ls ~` | ran |
| C12 | Inside the base mounts `systemd-run --user` runs a command outside the sandbox; the docker socket answers; hiding `$XDG_RUNTIME_DIR` and `/run/docker.sock` closes both; the system bus stays reachable | architect probe 2026-10-29 | probed |
| C13 | `team up` stops at the first creation error and leaves earlier sessions running | `src/main.rs:1121-1130` | read |
| C14 | OMP sessions load their extension from `<state>/omp`; the state directory holds `addr`, `admin.token`, `daemon.log`, `messages.db*`, `codex/`, `omp/`, `team.toml` | `src/omp.rs:26-36`; `ls ~/.local/state/a2amx` | read, ran |
| C15 | Claude's own sandbox inside the outer bwrap | inconclusive in the spike | unverified; no claim made |
| C16 | GitHub runners allow bwrap after the sysctl step | inference | unverified; developer tries it |
| C17 | A tmpfs hide nested in a writable bind is empty and read-only while the workspace stays writable; read-only binds over `.git/hooks` and `.git/config` block edits while `.git/index.lock` can still be created | architect probe 2026-10-29 | probed |
| C18 | A rename onto a single bound file fails with `Device or resource busy`; an in-place write succeeds | same | probed |
| C19 | Inside the base mounts a `connect()` to `/run/WSL/*_interop` succeeds; hiding `/run/WSL` makes the directory empty; binfmt `WSLInterop` is enabled with flags `PF`; running a Windows program from the sandbox was not executed | architect probe 2026-10-29; consultant probe | probed (connect), inference (exec) |
| C20 | A bind source inside a directory hidden by tmpfs still resolves against the host (`<state>/confine-empty` works) | architect probe 2026-10-29 | probed |
| C21 | Both Claude 2.1.312 and OMP 18.13.2 keep the last `--append-system-prompt` value; OMP accepts text or a file path; Claude has `--append-system-prompt-file` | consultant reading of the binaries' option strings | read (OMP), inference (Claude last-wins) |
| C22 | Claude/Omp wiring is client-side: `wire_claude_argv`, `wire_omp_argv`, flag sites `src/harness.rs:330-333`, `:454-457`, `insert_extras` `:461` | consultant read; `src/main.rs:518-550` | read |
| C24 | Inside the sandbox `NoNewPrivs` is 1 and `sudo -n true` is refused for it; `/usr/bin/crontab` is setgid but the spool is read-only | architect probe 2026-10-29 | probed |
| C25 | A descriptor opened before `bwrap` stays open and readable in the sandbox after its directory is hidden | architect probe 2026-10-29 | probed |
| C26 | `/run/snapd.socket` is mode 0666 on this machine; loopback listeners exist on several ports | `ls -l`, `ss -ltn` | ran |
| C27 | The daemon's descriptors above 2 are close-on-exec (pid 951 fdinfo), alacritty_terminal 0.26.0 `tty/unix.rs:195-265` only closes slave/master in `pre_exec`, rustix-openpty 0.2.0 opens with CLOEXEC (`lib.rs:124,130`), `try_clone` uses `F_DUPFD_CLOEXEC`; another session's PTY cannot leak | consultant probe and read | read, probed |
| C28 | From inside the base mounts, `PTRACE_ATTACH` to a same-uid host process gives EPERM and `/proc/<pid>/mem`, `cwd`, `root`, `environ` are denied; `kill(pid, 0)` and `SIGSTOP` are delivered (before `--unshare-pid`) | consultant probe | probed |
| C29 | With `--unshare-pid` host PIDs are invisible, the exit code propagates and killing the outer bwrap ends the child; `--size` caps a tmpfs; terminal resize and Ctrl-C under `--unshare-pid` were NOT confirmed by the architect's PTY probe (test harness inconclusive) | architect probe 2026-10-29 | probed (partly) |
| C30 | `~/.config/gh/hosts.yml` and `~/.codex/auth.json` exist on this machine and stay readable without the added hides | consultant `ls` | ran |
| C31 | The attach emulator drops terminal title, clipboard (OSC 52) and hyperlink sequences | `src/emulator.rs:152-153`, `:297` | read |
| C32 | `DaemonConfig` is built in tests through `Daemon::start(DaemonConfig { state_dir, listen, host_name, limits })`, in-process | `tests/common/mod.rs:36-60` | read (consultant) |
| C33 | From inside the sandbox, an unconfined same-uid process holding a secret file open is unreachable through `/proc`: `fd/N`, `root`, `cwd`, `exe`, `maps`, `environ` give `Permission denied` (only the fd numbers and `cmdline` are readable) without a PID namespace, and the process is invisible with `--unshare-pid`; outside the sandbox every route works | architect probe 2026-10-29, `ptrace_scope=1` | probed |
| C23 | `~/.claude` and `~/.omp` hold code and scripts outside the settings files the first draft listed (`statusline.sh`, `dev-mods`, OMP `natives`, `puppeteer`) | consultant listing (names only) | read |

## Amendment 1 (docs and one test; no rule changes)

Found by the owner's assistant's review of the locked spec; the architect confirmed the first point by probe.

1. **Late-created paths (residual).** A hide is planned only for a path that exists at launch (2.3). A path created on the
   host afterwards, for example a container daemon started after the session, is visible and usable inside the sandbox:
   the read-only bind blocks writes, not `connect()`. bwrap cannot mount over a path that does not exist on a read-only
   root, and hiding all of `/run` is too broad, so the spec accepts this. Add it to the residuals list in
   `docs/architecture.md` and `docs/cli.md` with the advice to start container daemons before confined sessions.
   The notice (2.7) does not mention it, because it lists only the plan.
2. **Pinning test.** `tests/confine.rs` gains one real-bwrap test: with a confined shell running, the test binds a Unix
   socket in a host tmpfs directory outside every hide and creates a file beside it; inside, `connect()` to the socket
   succeeds and the file is readable. It documents the residual, so a future change that fixes it changes the test on
   purpose. It is a contract test (it passes on the first implementation).
3. **Scope of `--confine-allow-containers`.** `docs/cli.md` and `docs/teams.md` say it lifts only the system socket and
   environment restriction of 2.3 group 4a. Rootless sockets under `$XDG_RUNTIME_DIR` stay hidden because that directory
   is a mandatory hide. One test asserts this: with the flag set, `$XDG_RUNTIME_DIR` is still hidden in the plan.
4. **Notice wording.** The container line of the notice says `restricted`, not `blocked` (2.6), because late-created
   sockets are not covered. The notice tests assert the word `restricted`.
5. Claim C34: a file created on the host's `/run` tmpfs after the sandbox started is visible inside it, and a write there
   fails with `Read-only file system` (architect probe 2026-10-29, probed).

Scope: the docs already listed in 0 and `tests/confine.rs`. Backlog F32 (`confine check`) may warn about a container
daemon that is not running yet.

## Amendment 2 (wire shape: the team file to protect)

Raised by the developer (BLOCKED, 2026-10-30) and confirmed by the architect: 2.3 requires the team file the session was
started from to be read-only when it lies inside a writable path, but `NewSession` carries no source-file metadata and the
daemon cannot infer a custom `team up --file` path.

1. `ConfineRequest` gains `team_file: Option<String>` (`#[serde(default)]`, absent = `None` = protect nothing extra, which
   is the safe value for a field that only adds a read-only mount). Section 2.2's wire shape is read with this field.
2. `team up` sets it to the canonical path of the team file in use (local, `--file`, or global); `a2amx new` and any other
   caller leave it `None`. There is no CLI flag and no team-file key for it.
3. The daemon treats it like any other path: at most 4096 bytes, canonicalised, skipped (and listed under `skipped`) when
   missing, and applied as a protected read-only bind only when the canonical path lies inside a writable path, exactly as
   2.3 says. A value outside every writable path changes nothing. It can only add a read-only mount, never widen access,
   so a client-supplied value is not a trust-boundary risk beyond the existing `Confine` client.
4. Tests: a plan with `team_file` inside the cwd lists it under protected and gives it `--ro-bind` after the writable
   bind; one outside the writable set is not in the plan; a `ConfineRequest` JSON without the field decodes to `None`.
   `tests/wire.rs` gets the round trip. Scope: the files already listed in 0.
