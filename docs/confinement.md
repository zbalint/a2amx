# Confinement

Confinement is a safety net for a trusted but fallible agent, not a security boundary; see [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries).

## Defaults at a glance

| Area | Default |
| --- | --- |
| Writable | The workspace and its Git directory, a private `/tmp`, and the tool or harness directories named by explicit grants. Claude and OMP sessions always open their own harness config root read-write and persistent by default. |
| Read-only | The rest of the filesystem by default; writable defaults still keep `~/.a2amx`, existing `PATH` directories inside home, harness control files and directories, and this session's Git `hooks`, `config`, `config.worktree`, `core.hooksPath`, and team file read-only. |
| Hidden | Secret locations such as `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.npmrc`, and `~/.docker`; credential files under the writable tool directories, the state and runtime directories, `/run/WSL`, `/mnt/wslg`, and container sockets. |


Hide and write are strict by default; you open what you name.

## How to set it up

You can configure confinement on one command, in a team file, or by previewing a team file before starting it.

The quickest way to get the grants line for a project is `a2amx confine suggest [DIR]`. It checks the project's known build-file markers and supported git remotes, prints a copyable `grants = [...]` line, and uses `HOME` when it is available; it does not require a daemon.

### One command

Use repeatable path flags with `a2amx new`:

```sh
a2amx new --confine \
  --confine-write PATH \
  --confine-unhide PATH \
  --confine-hide PATH \
  --confine-grant NAME \
  --confine-keep-env NAME \
  --confine-no-default-hides \
  -- <command>
```

`--confine-grant NAME` opens one named capability, `--confine-keep-env NAME` returns one listed credential environment variable to this session, and `--confine-no-default-hides` disables optional home hides while keeping mandatory hides.

`~` and `~/...` expand to your home directory. Absolute paths are accepted; other relative paths are relative to the session's working directory. Environment variables are not expanded, and `~user` forms are rejected. To reach a relative directory literally named `$x`, write `./$x`. A path that does not exist is retained in the dry run as an unresolved operator entry with its option and reason; missing defaults remain skipped. Each of the three lists accepts at most 64 entries.

When confinement is enabled by `a2amx new --confine` or a team file, no cache grant is
implicit. `cache` opens `~/.cache`; marker-specific toolchain grants open their literal
tool directories only when listed. Claude and OMP sessions always open their own harness
config root read-write and persistent by default. Use `a2amx confine suggest` to print a
copyable grants fragment, or add grants explicitly to the team/session table.
Auto detection covers the marker-based grants listed in [the detection table](#automatic-grant-detection), adds `ssh`, `gh`, or `git-credentials` from supported git remotes, and always adds `cache`; explicit confinement still has no implicit cache grant. Paths relocated by environment variables still need an explicit `write` entry.

### Team file

Use one optional `[confine]` table for team defaults and `[session.confine]` for per-session
settings. Set `enabled = true` in either table; lists add and scalars override. `write`,
`hide`, `unhide`, and `keep_env` are arrays, with `keep_env` per-session only;
`default_hides` and `auto` are booleans, and `grants` names a capability for that
session. In the complete example below, `cwd = "."` is relative to the team file's
directory; other relative paths are relative to the resolved session working directory.

### Preview

Run this before starting the team:

```sh
a2amx team up --dry-run
```

The dry run shows the effective restrictions. An operator `write`, `hide`, or `unhide` entry that cannot be resolved is printed as `unresolved: PATH (write|hide|unhide: REASON)`; missing default hides and other optional paths remain `hidden: PATH (skipped missing)`.

### How the options combine

The effective session setting uses the session table when a scalar is present, otherwise
the team table, otherwise the default (`enabled = false`, `default_hides = true`).
`auto = true` is valid only in the global `<state home>/team.toml` `[confine]` table and
requires `enabled = true`. It detects project markers and bounded regular git configs,
then adds the matching grants plus `cache`; local and explicit team files must not set it.
`auto = false` is accepted in any team table and means nothing. A session-level `auto`
key is always an error. `write`, `hide`, and `unhide` lists add in team-then-session order,
with exact duplicates kept once. `grants` is per session by default; every grant may be set
in the team table except container grants and `harness-config`, which remain session-only.
`enabled = false` needs no other key. A team table with options but no `enabled` anywhere
confines nothing, while team `enabled = false` and session `enabled = true` confines that
session and still applies the team's lists.

### Grants

| Name | Exposes | Team default |
| --- | --- | --- |
| `docker` | docker container daemon sockets: effectively root on the host | No; grant it per session. |
| `podman` | podman container daemon sockets: effectively root on the host | No; grant it per session. |
| `containerd` | containerd, buildkit and k3s container daemon sockets: effectively root on the host | No; grant it per session. |
| `crio` | crio container daemon sockets: effectively root on the host | No; grant it per session. |
| `libvirt` | libvirt virtual-machine control: controls virtual machines | No; grant it per session. |
| `lxd` | lxd and incus container daemon sockets: effectively root on the host | No; grant it per session. |
| `snap` | snapd package management: installs and removes packages | No; grant it per session. |
| `harness-config` | the session's own harness config directory, read-write and persistent: settings, hooks, instructions and programs under it; running sessions of that harness pick up changes immediately | No; grant it per session. |
| `gh` | your GitHub CLI login (`~/.config/gh`, read-write) and its tokens in the environment: the agent can push, open and merge pull requests, read private repositories as you, and change or delete the login | Yes; team or session. |
| `git-credentials` | the plain-text git credential files (read-write in place): every host and password stored there, which the agent can also overwrite | Yes; team or session. |
| `ssh` | your ssh keys and config (`~/.ssh`, read-write): the agent can log in to every host those keys open as you, push with them, and change or delete them | Yes; team or session. |
| `cargo` | the Rust toolchain, read-write: ~/.cargo (registry and git caches, installed crates; its bin directory stays read-only when it is on PATH) and ~/.rustup (installed toolchains), whose programs and settings (config.toml, env) your own builds outside the session run; your registry login stays hidden while it exists unless you also give cargo-token | Yes; team or session. |
| `go` | the Go toolchain, read-write: ~/go (module cache, downloaded toolchains and installed binaries, which your own builds outside the session may run) and ~/.cache/go-build; ~/.netrc stays hidden unless you also give netrc | Yes; team or session. |
| `python` | the Python tool caches, read-write: pip, uv, poetry, virtualenv and pipx data under ~/.cache and ~/.local/share, including tool environments whose programs your own shell may run; ~/.pypirc stays hidden unless you also give pypi-token | Yes; team or session. |
| `node` | the JavaScript package caches, read-write: ~/.npm, ~/.yarn/berry, ~/.local/share/pnpm and ~/.bun/install/cache; ~/.npmrc stays hidden unless you also give npm-token | Yes; team or session. |
| `java` | the Java build caches, read-write: ~/.m2/repository and the whole ~/.gradle, where a gradle.properties you wrote may hold repository passwords the agent can then read and an init.d script is run by every gradle build you start outside the session | Yes; team or session. |
| `c-cpp` | the C and C++ build caches, read-write: ccache, conan and cmake data in ~/.cache/ccache, ~/.ccache, ~/.conan2 and ~/.cmake, which your own builds outside the session use | Yes; team or session. |
| `semgrep` | semgrep's data directory ~/.semgrep, read-write: its settings file holds the login token if you ran semgrep login | Yes; team or session. |
| `sonar` | sonar-scanner's data directory ~/.sonar, read-write, and the SONAR_TOKEN environment variable: the agent can run analyses and publish results to your SonarQube or SonarCloud project as you | Yes; team or session. |
| `aws` | your AWS CLI configuration and credentials (~/.aws, read-write, including cached SSO tokens): the agent acts in your AWS accounts as you and can change or delete the files | Yes; team or session. |
| `gcloud` | your Google Cloud CLI login (~/.config/gcloud, read-write): the agent acts in your Google Cloud projects as you and can change or delete the login | Yes; team or session. |
| `azure` | your Azure CLI login (~/.azure, read-write) and the AZURE_CLIENT_SECRET and ARM_CLIENT_SECRET environment variables: the agent acts in your Azure subscriptions as you and can change or delete the login | Yes; team or session. |
| `kube` | your kubectl configuration (~/.kube, read-write): the cluster credentials in it let the agent act in your clusters as you, and it can change or delete them | Yes; team or session. |
| `terraform` | terraform's user directory (~/.terraform.d, read-write): the saved Terraform Cloud or registry tokens in it let the agent act as you, and it can change or delete them | Yes; team or session. |
| `npm-token` | your ~/.npmrc (read-write in place) and the NPM_TOKEN and NODE_AUTH_TOKEN environment variables: the agent can publish packages as you and change the file | Yes; team or session. |
| `pypi-token` | your ~/.pypirc (read-write in place) and the PYPI_TOKEN and TWINE_PASSWORD environment variables: the agent can upload packages as you and change the file | Yes; team or session. |
| `cargo-token` | your cargo registry login (~/.cargo/credentials and ~/.cargo/credentials.toml, read-write in place) and the CARGO_REGISTRY_TOKEN and CARGO_REGISTRIES_CRATES_IO_TOKEN environment variables: the agent can publish crates as you and change the file | Yes; team or session. |
| `netrc` | your ~/.netrc (read-write in place): every host login and password stored there, which the agent can also overwrite | Yes; team or session. |
| `cache` | your whole ~/.cache, read-write: every tool's cache and scratch data, including tool environments and hooks whose programs your own shell and builds outside the session run; the default hides under it (for example the Hugging Face token) stay hidden only while they exist when the session starts, and only until something moves them; a credential file created or renamed later is open to the agent, and a state directory under ~/.cache makes the grant fail | Yes; team or session. |
| `huggingface-token` | your Hugging Face login (~/.cache/huggingface/token, read-write in place) and the HF_TOKEN and HUGGING_FACE_HUB_TOKEN environment variables: the agent can act on your Hugging Face account and change the file | Yes; team or session. |

### Choosing grants

A grant is all or nothing and read-write. Toolchain grants open caches, never credentials.
A credential grant lifts its own hide only. A path that does not exist is not opened; a
real session may create only the grant's declared tool directories, never credential files.
An environment-relocated directory (`CARGO_HOME`, `GOPATH`, `GRADLE_USER_HOME`,
`XDG_CACHE_HOME` and the like) is not followed; use `write` for the real path. A grant
lifts only the hide of its own path: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`,
`AWS_SESSION_TOKEN`, `GOOGLE_APPLICATION_CREDENTIALS`, `KUBECONFIG` and `TF_TOKEN_*`
are not in the closed credential list and reach every session either way. A grant path
that is a symlink is refused or reported unresolved; it is never created through.

Grant names are closed and validated before launch; each grant is separate and explicit
unless added by global auto detection. `harness-config` remains available as a
session-only explicit grant, while Claude and OMP sessions receive equivalent persistent
access to their harness root by default.

### Automatic grant detection

`a2amx confine suggest` and global `[confine] auto = true` use the same detector. Marker
files are checked only in the session's project directory:

| Grant | Marker(s) |
| --- | --- |
| `cargo` | `Cargo.toml` |
| `go` | `go.mod` |
| `python` | `pyproject.toml`, `requirements.txt` |
| `node` | `package.json` |
| `java` | `pom.xml`, `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` |
| `c-cpp` | `CMakeLists.txt`, `conanfile.txt`, `conanfile.py` |
| `semgrep` | `.semgrep.yml`, `.semgrep.yaml`, `.semgrep` |
| `sonar` | `sonar-project.properties` |

Git remotes add `ssh` for `ssh://`, `git+ssh://`, and scp-style URLs. HTTPS remotes
add `gh` when `~/.config/gh` exists and `git-credentials` when either supported
credential file exists. `.git` must be a directory and its `config` must be a regular
file no larger than 65,536 bytes; worktree `.git` files and unreadable or invalid
configs add no git grants. `cache` is always included. Results are deduplicated in
grant-table order.

Under `auto`, `git push` works only when a remote is detected. A `.git` file for a
worktree or submodule, and a local team file, require an explicit grant. HTTPS pushes
through `gh` also require `gh auth setup-git`.

When global auto detection adds a grant that is not already accepted, `a2amx team up`
prints the detector reason and the grant's exposure, then asks for confirmation before
starting any session. Acceptance is by grant name, not by project or path: accepted
names apply to every project started from the global team file. The names are stored
one per line in `<state home>/auto-grants`; delete that file to ask again. Unknown,
blank, and padded lines are ignored.

`a2amx team up --yes` prints the same consent details but accepts the detected grants
for that run only and never writes the accepted-grants file. A non-terminal run without
`--yes` refuses rather than silently accepting grants. `--dry-run` never prompts or
writes; it reports each new grant as `  not yet accepted: NAME, ...`.

`harness-config` is deliberately broad: the agent can change settings, hooks, instructions and
programs under `~/.claude` or `~/.omp`, and a running session of that harness—including your own
unconfined sessions—picks changes up immediately. A hook it adds therefore runs with your full
rights at once. The directory also holds programs such as `~/.claude/local/claude` and anything
under `~/.omp`; a launched `omp` whose `argv0` lies there is run by the daemon's quota probe
outside the sandbox within minutes. Symlinked entries such as `hooks`, `agents` and `CLAUDE.md`
can be replaced for good, which breaks a setup managed elsewhere. If `~/.claude` is itself a
symlink, the grant opens its canonical target. Open it only for an agent you trust with that.

## Inspecting a session's effective confinement

The attach status line shows a dim `[confined]` tag for manually configured confinement
and `[confined auto]` when global auto detection added grants.

`a2amx list` and `a2amx list --details` add a `CONFINE` column immediately after
`HARNESS`. It is `off` for an unconfined session, `manual` for explicit confinement,
and `auto` when global auto detection was enabled. The value remains available for
exited sessions.

Use `a2amx confine show [SESSION]` to read the daemon's recorded confinement plan;
omit `SESSION` to show every session. The output lists the session's mode, effective
grants, their source (`set` or `auto: REASON`), and explicit writable paths. For an
automatic session, the `auto: REASON` rows explain which project marker, git remote, or
cache caused each detected grant.

## The method: open what an honest mistake can recover from

Open a directory when an accident in it is recoverable: a cache is disposable, and a toolchain can be reinstalled. Keep credentials, the repository itself, installed binaries, and state closed. Prefer the narrowest cache directory.

When a tool reports `Read-only file system`, add the directory named in the error to `write` or grant the matching tool directory in the session's `[session.confine]` table, not `$HOME`, unless opening the whole home is your deliberate choice. `write = ["~"]` makes everything under home writable except the hides and protected launch inputs; the choice belongs to the user. Whatever you open stays writable for the next unconfined run too. The `cargo` grant opens `~/.cargo` while keeping its credentials hidden; `~/.cargo/bin` remains protected when it is on `PATH`. `~/.cargo/env` and `~/.cargo/config.toml` are writable under that grant, and later builds execute the configuration they contain.

## What stays open

`write = ["~"]` is an operator choice, not a sandbox escape hidden behind a default. It leaves these residuals:

1. The Git history inside a writable Git directory is writable, so an agent can rewrite commits.
2. Harness roots are writable and persistent by default for Claude and OMP, including settings, hooks, plugins and new files. Treat them as trusted program directories; a running unconfined sibling may load changes immediately. Only this session's repository `hooks`/`config` and team file are protected; every other repository's `.git/hooks` and `.git/config`, and every other `a2amx.toml` under a writable home, are writable. Windows drives under `/mnt` are writable if `/mnt` is opened.
3. Hides are decided when the session starts. A file created later at a hidden path is not hidden.
4. Network access is not restricted.
5. A writable home lets the agent edit shell startup files and editor configs that the user's next unconfined run executes. Open it knowing that.
6. A grant opens what its row says and no more. An install directory other than `~/.a2amx` is protected only when it appears as an existing `PATH` directory inside the home.
7. The daemon finds `bwrap` and the `omp` quota probe only in absolute `PATH` directories; relative and empty entries are ignored.
   An `omp` started from a path the session can write (a relative path with a slash, or an absolute one outside the `PATH` directories that `write` opens) is still run by the quota probe outside the sandbox, so keep the harness binary in a read-only directory.

Auto-grant consent is permission for the named grant, not a narrower path limit.
Once accepted, that grant opens every path described by its row; the prompt explains
what those paths expose but does not change the confinement plan.

## Harness login and state files

Claude and OMP sessions bind their real harness configuration root read-write and persistent
by default, so the harness can renew its token and save settings, hooks, skills and new
files. This is broad access to programs that sibling unconfined sessions may load
immediately; use it only for an agent you trust. `/login` and saves to `~/.claude.json`
from inside a confined session still do not work: their lock and temporary files live in
your home, which stays read-only. Run `claude /login` once in a normal terminal; every
session picks up that login. The explicit `harness-config` grant names the same capability
for a session that needs it outside the default harness path.

`cache` and marker-specific toolchain grants are explicit. Auto detection adds them only
for a global team file with `[confine] auto = true`; local and explicit files must list
grants themselves.

## Recipes by ecosystem

### Rust

With `~/.cargo` read-only, a build that needs an uncached crate fails while creating a `.crate` file under `~/.cargo/registry`; the probe reported `humantime-2.0.0.crate` and `Read-only file system`. The `cargo` grant opens `~/.cargo` and `~/.rustup`, including registry and Git caches, installed crates and toolchains. The workspace `target` directory needs no entry.

```toml
# fragment
[session.confine]
grants = ["cargo"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant cargo -- cargo build
```

If you do not want the whole grant, use this narrower `write` fallback:

```toml
# fragment
[session.confine]
write = ["~/.cargo"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.cargo -- cargo build
```

The `cargo` grant keeps `~/.cargo/bin` read-only when it is on `PATH`; `cargo install` there fails, so set `CARGO_INSTALL_ROOT` to a writable directory. `~/.cargo/config.toml` and `~/.cargo/env` remain writable and later builds run what they contain. The `~/.cargo/credentials` and `~/.cargo/credentials.toml` files remain hidden if they exist when the session starts.

The defaults use literal paths such as `~/.cargo` and `~/.cache`; environment-relocated
directories (`CARGO_HOME`, `XDG_CACHE_HOME`, and the like) are not detected or honoured.
The `cache` grant is the named way to open `~/.cache`. Default hides under it protect only
paths that exist when the session starts, so a token created or renamed afterwards is not
hidden; a state directory under `~/.cache` makes the grant unresolved. A `~/.cache`
symlink is never followed or created through.

### Node with npm

Without `~/.npm`, `npm install` fails with `EROFS` under `~/.npm/_cacache`. The `node` grant opens the JavaScript package caches in `~/.npm`, `~/.yarn/berry`, `~/.local/share/pnpm` and `~/.bun/install/cache`.

```toml
# fragment
[session.confine]
grants = ["node"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant node -- npm install
```

If you do not want the whole grant, use this narrower `write` fallback:

```toml
# fragment
[session.confine]
write = ["~/.npm"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.npm -- npm install
```

`~/.npmrc` is hidden by default. Unhide it only when a private registry needs it; see [Credentials a tool needs](#credentials-a-tool-needs). Global installs (`npm install -g`) and version-manager directories are not covered.

### Python with pip

The `python` grant opens pip, uv, poetry, virtualenv and pipx data under `~/.cache` and `~/.local/share`, including tool environments whose programs your own shell may run; `~/.pypirc` stays hidden unless you also give `pypi-token`.

```toml
# fragment
[session.confine]
grants = ["python"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant python -- python -m pip install requests
```

A virtual environment inside the workspace needs no `write` entry. The probe installed `requests` into a workspace virtual environment with `~/.cache` read-only. The `python` grant does not cover conda.

### Go

The `go` grant opens all of `~/go`, including `bin` (which stays read-only only when it is on `PATH`), the module cache, downloaded toolchains and installed binaries, and `~/.cache/go-build`; `~/.netrc` stays hidden unless you also give `netrc`.

```toml
# fragment
[session.confine]
grants = ["go"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant go -- go build
```

If `GOPATH` or `GOCACHE` is relocated, use `go env` for a narrower `write` fallback:

```toml
# fragment
# Replace these placeholders with the paths printed by go env.
[session.confine]
write = ["/path/from/GOMODCACHE", "/path/from/GOCACHE"]
```

Equivalent command-line form after printing the paths:

```sh
gomodcache="$(go env GOMODCACHE)"
gocache="$(go env GOCACHE)"
a2amx new --confine --confine-write "$gomodcache" --confine-write "$gocache" -- go build
```

### Maven

This was not tested by the maintainers. Maven's default local repository is `~/.m2/repository`. The `java` grant opens that repository and the whole `~/.gradle` directory.

```toml
# fragment
[session.confine]
grants = ["java"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant java -- mvn package
```

If you do not want the whole grant, use this narrower `write` fallback:

```toml
# fragment
[session.confine]
write = ["~/.m2/repository"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.m2/repository -- mvn package
```

For the narrower fallback, `~/.m2/settings.xml` can hold server `username` and `password` and is not hidden by default, so add `hide = ["~/.m2/settings.xml"]` under `[session.confine]` when the agent does not need it.

### Gradle

This was not tested by the maintainers. Gradle's user home is `~/.gradle`, or the path in `GRADLE_USER_HOME`. The `java` grant opens all of `~/.gradle`, including its caches, daemon, `init.d`, JDKs and wrapper distributions; it also exposes `gradle.properties`, where repository credentials may be stored, and `init.d` scripts run on every build.

```toml
# fragment
[session.confine]
grants = ["java"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-grant java -- gradle build
```

If you do not want the whole grant, use this narrower `write` fallback:

```toml
# fragment
[session.confine]
write = ["~/.gradle/caches", "~/.gradle/wrapper"]
```

Equivalent command-line options:

```sh
a2amx new --confine \
  --confine-write ~/.gradle/caches \
  --confine-write ~/.gradle/wrapper \
  -- gradle build
```

For any other tool, run it once, read the failing path in the error, and decide with [the method above](#the-method-open-what-an-honest-mistake-can-recover-from).

## Docker and other container engines

By default, the system runtime sockets are hidden, including `/run/docker.sock`,
`/run/podman/podman.sock`, `/run/containerd/containerd.sock`, `/run/crio/crio.sock`,
`/run/libvirt/libvirt-sock`, the LXD and Incus sockets, and the Snap socket. The runtime
variables `DOCKER_HOST`, `DOCKER_CONTEXT`, `CONTAINER_HOST`, `CONTAINER_CONNECTION`,
`CONTAINERD_ADDRESS`, `BUILDKIT_HOST`, and `LIBVIRT_DEFAULT_URI` are unset. Consequently,
`docker ps` fails with `EACCES` on the socket.

Grant the configured host Docker daemon explicitly:

```sh
a2amx new --confine --confine-grant docker -- <command>
```

```toml
# fragment
[session.confine]
grants = ["docker"]
```

This gives the agent root-equivalent power over the host: a member of the `docker` group can
start a container that mounts the host's root, and confinement does not limit what a container
can do. Use the grant only when you accept that. After the grant, the configured daemon socket
is reachable; the maintainers did not test `docker run`, `docker build`, or `docker compose` here.

The Docker Desktop sockets `~/.docker/desktop/docker.sock` and `~/.docker/run/docker.sock` are
also inside the default hide of `~/.docker`. The grant does not lift that hide, so both the grant
and an unhide are needed:

```sh
a2amx new --confine --confine-grant docker \
  --confine-unhide ~/.docker/desktop/docker.sock -- <command>
```

Choose `~/.docker/run/docker.sock` instead when that is the socket path. The Docker CLI
configuration, contexts, and `docker login` credentials are under the default hide of `~/.docker`,
so the CLI configuration is hidden. A private registry login needs an
`unhide = ["~/.docker/config.json"]` entry under `[session.confine]`, with the warning in
[Credentials a tool needs](#credentials-a-tool-needs); this was not tested.

Rootless Docker and Podman cannot be used here: their sockets are under `$XDG_RUNTIME_DIR`,
which is hidden and cannot be unhidden, even with the grant. Use an engine on another machine
(`DOCKER_HOST` passes through with the grant and the network is open), or run the whole session
in a container or VM instead of a2amx confinement. Reaching a remote engine needs its credentials,
for example `~/.ssh` (use the `ssh` grant, which opens it read-write)
or TLS files under `~/.docker`; those are default hides and need an `unhide` entry. None of this remote-engine path was tested.

Workloads the agent starts are not confined by a2amx. A socket created after the session starts
and outside every launch-time hide is visible. Start runtime daemons before the session when
their host socket must be hidden.

## Credentials a tool needs

Credential files are hidden by default. The `gh` grant exposes `~/.config/gh` read-write
and returns `GH_TOKEN`, `GITHUB_TOKEN` and `GH_ENTERPRISE_TOKEN` to that session. The
`git-credentials` grant exposes `~/.git-credentials` and `~/.config/git/credentials`
writable in place. Use the narrowest grant that matches the work:

```toml
# fragment
[session.confine]
grants = ["gh"] # for a session that pushes or opens pull requests with gh
```

`git push` over https needs the credential helper in the user's `~/.gitconfig`; that file is
readable but read-only inside, so run `gh auth setup-git` outside first. The writable `gh` mount
means the one-time migration of an old-format `hosts.yml` can now run in the confined session.
Commands such as `gh auth refresh`, `auth switch` and `config set` can now write that file in a
confined session; those writes and refresh behavior remain untested.

Use `git-credentials` only when the user keeps a plain-text store. The files are writable
in place, but a credential helper that replaces a file still fails: git's `store` helper
writes a lock file in the read-only home directory and renames it. A credential helper
that is a Windows program reached through WSL interop does not work because `/run/WSL` is
hidden. SSH pushes need an `ssh` grant; see [ssh and signed commits](#ssh-and-signed-commits).
A `gh` token kept in the system keyring is unreachable because `~/.local/share/keyrings`
and the session bus are hidden.

The maintainers read `gh`'s token from `hosts.yml` and ran `gh auth status` and
`git push --dry-run` with the path unhidden (2026-11-01). A real push, `gh pr create`
and this recipe in a fresh login are untested; the tests use fixtures.

### Credential environment variables

Every confined session unsets this closed list of credential variables by default:

```text
GH_TOKEN, GITHUB_TOKEN, GH_ENTERPRISE_TOKEN, GITLAB_TOKEN, GITLAB_PRIVATE_TOKEN,
CI_JOB_TOKEN, BITBUCKET_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD,
CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET,
ARM_CLIENT_SECRET, VAULT_TOKEN, HF_TOKEN, HUGGING_FACE_HUB_TOKEN, SONAR_TOKEN,
SENTRY_AUTH_TOKEN, DIGITALOCEAN_ACCESS_TOKEN, HCLOUD_TOKEN, CLOUDFLARE_API_TOKEN,
NETLIFY_AUTH_TOKEN, VERCEL_TOKEN, FLY_API_TOKEN, HEROKU_API_KEY, PGPASSWORD, MYSQL_PWD
```

The list is explicit, not pattern-matched. It never removes LLM provider keys
(`ANTHROPIC_*`, `OPENAI_*`, `GEMINI_*`, `GOOGLE_API_KEY`, `OPENROUTER_*`), `A2AMX_*`,
or harness internals. `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, and
`AWS_SESSION_TOKEN` are the Bedrock login used by the harness; `GOOGLE_APPLICATION_CREDENTIALS`
is only a path. Those four names stay set so Claude Code and OMP can run as the confined child.
The corresponding `~/.aws` and `~/.config/gcloud` files remain hidden by default.

Return one listed variable to one session with `keep_env`:

```toml
# fragment
[session.confine]
keep_env = ["NPM_TOKEN"] # for a session that publishes with npm
```

The `gh` grant is the named alternative when the session needs the three GitHub names.

`keep_env` only returns the value already present in the operator's environment; it does not
read or inject a new secret. It is per-session, and the dry run and launch notice list both
the variables still unset and the names kept. A value read from a file the user writable-binds
is not an environment variable and is unaffected by `keep_env`. Updating the daemon alone
changes the environment of every confined session it launches.

### ssh and signed commits

The grant opens `~/.ssh` read-write and mounts an empty file over `/etc/ssh/ssh_config`, so the
root-owned system configuration and its included files no longer break `ssh`. Settings from the
system configuration therefore no longer apply; copy what you need into `~/.ssh/config`. An
`Include` of a root-owned file there still fails. With the `ssh` grant, a separate `write` entry
for a single file inside `~/.ssh` is redundant and makes that file a single-file bind that cannot
be replaced by a rename (`ssh-keygen -R` fails), so drop it. A `ControlPath` under `~/.ssh` is
also unusable; point it at `/tmp`.

No `SSH_AUTH_SOCK` is passed through, so a passphrase-protected key cannot be used (F63). Signing
commits with an unencrypted listed ssh key works; gpg signing is F54. With `default_hides = false`
in a team file (or `--confine-no-default-hides` on the command line), `~/.ssh` is visible
without the grant, but the system configuration still breaks `ssh`. Keep the HTTPS recipe above if that is preferable.

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `Read-only file system` on a path under `$HOME` | The path is read-only. | Add the directory named in the error to `write` under `[session.confine]`. |
| A tool says it is not logged in, or reports an authentication error | Its credential file or directory is a default hide. | Give the matching grant first (`gh`, `ssh`, `git-credentials`, `sonar`, `aws`, `gcloud`, `azure`, `kube`, `terraform`, `npm-token`, `pypi-token`, `cargo-token`, `netrc`, `huggingface-token`); use `unhide` only for a credential no grant covers; see [Credentials a tool needs](#credentials-a-tool-needs). |
| Claude or OMP cannot update itself (`Read-only file system` under `~/.local/share/claude` or `~/.local/bin`) | A harness binary lives in a read-only directory on purpose: a writable `~/.local/bin` would let a confined agent replace programs that you, or the daemon's `omp` quota probe, run outside the sandbox. Self-update is not supported in a confined session. | Update the harness from a shell outside the sandbox, then restart the session. |
| `permission denied` on a container socket | Container sockets are hidden. | Use [Docker and other container engines](#docker-and-other-container-engines). |
| A file written under `/tmp` is not seen by a peer session | `/tmp` is private per session. | Use a directory in the workspace. |
| Another session cannot find a helper daemon that one session started | A recorded process id is wrong across sessions, and a socket or file under `/tmp` or `$XDG_RUNTIME_DIR` is not shared. | Find the helper by a port or a shared writable directory; see [Background processes](harnesses.md#background-processes). |
| A path in `write`, `hide`, or `unhide` is reported as `unresolved` in `--dry-run` | The operator path could not be canonicalized; the reason is shown as `not found`, `permission denied`, or another filesystem error. | Create or correct the path, or fix its permissions. Missing default hides continue to use `(skipped missing)`. |
| I do not know what a session may touch | The effective confinement plan is recorded by the daemon. | Run `a2amx confine show [SESSION]`. |

## A complete team file

The following file sets team defaults in `[confine]` and uses `cwd = "."`. Grants are
explicit in a local file; use `auto = true` only in the global state-home team file.

```toml
team = "myproject"
[confine]
enabled = true

[[session]]
name = "architect"
role = "architect"
command = ["claude"]
cwd = "."

[[session]]
name = "developer"
role = "developer"
command = ["omp"]
cwd = "."
[session.confine]
grants = ["cargo", "node"]
```
