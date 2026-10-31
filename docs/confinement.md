# Confinement

Confinement is a safety net for a trusted but fallible agent, not a security boundary; see [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries).

## Defaults at a glance

| Area | Default |
| --- | --- |
| Writable | The workspace, its Git directory, the harness's data paths, and a private `/tmp`. |
| Read-only | The rest of the filesystem, including `$HOME`; inside the writable Git directory, `hooks`, `config`, `config.worktree`, the directory named by `core.hooksPath`, and the team file are read-only. |
| Hidden | Secret locations such as `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.npmrc`, and `~/.docker`; the state and runtime directories, `/run/WSL`, `/mnt/wslg`, and the container sockets. |


Hide and write are strict by default; you open what you name.

## How to set it up

You can configure confinement on one command, in a team file, or by previewing a team file before starting it.

### One command

Use repeatable path flags with `a2amx new`:

```sh
a2amx new --confine \
  --confine-write PATH \
  --confine-unhide PATH \
  --confine-hide PATH \
  -- <command>
```

`~` and `~/...` expand to your home directory. Absolute paths are accepted; other relative paths are relative to the session's working directory. Environment variables are not expanded, and `~user` forms are rejected. To reach a relative directory literally named `$x`, write `./$x`. A path that does not exist is retained in the dry run as an unresolved operator entry with its option and reason; missing defaults remain skipped. Each of the three lists accepts at most 64 entries.

### Team file

Use one optional `[confine]` table for team defaults and `[session.confine]` for per-session settings. Set `enabled = true` in either table; lists add and scalars override. `write`, `hide`, and `unhide` are arrays, `default_hides` is a boolean, and `grants` names a capability for that session. In the complete example below, `cwd = "."` is relative to the team file's directory; other relative paths are relative to the resolved session working directory.

### Preview

Run this before starting the team:

```sh
a2amx team up --dry-run
```

The dry run shows the effective restrictions. An operator `write`, `hide`, or `unhide` entry that cannot be resolved is printed as `unresolved: PATH (write|hide|unhide: REASON)`; missing default hides and other optional paths remain `hidden: PATH (skipped missing)`. The complete team file at the end of this page parses as a team file.

### How the options combine

The effective session setting uses the session table when a scalar is present, otherwise
the team table, otherwise the default (`enabled = false`, `default_hides = true`). The
`write`, `hide`, and `unhide` lists add in team-then-session order, with exact duplicates
kept once; `grants` is per session only. `enabled = false` needs no other key. A team table
with options but no `enabled` anywhere confines nothing, while team `enabled = false` and
session `enabled = true` confines that session and still applies the team's lists. A
credential or privileged grant must be named by the session that needs it, never team-wide.

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

Grant names are closed and validated before launch. Each runtime is a separate opt-in;
these grants do not make a session a sandbox. Run `a2amx confine list` for the current
table and descriptions.

## The method: open what an honest mistake can recover from

Open a directory when an accident in it is recoverable: a cache is disposable, and a toolchain can be reinstalled. Keep credentials, the repository itself, installed binaries, and state closed. Prefer the narrowest cache directory.

When a tool reports `Read-only file system`, add the directory named in the error to `write` in the session's `[session.confine]` table, not `$HOME`. Whatever you open stays writable for the next unconfined run too. The recipe entries below are caches; opening all of `~/.cargo` also opens `~/.cargo/bin` and `~/.cargo/config.toml`, while its `credentials` files stay hidden when they exist at launch.

## Recipes by ecosystem

### Rust

With `~/.cargo` read-only, a build that needs an uncached crate fails while creating a `.crate` file under `~/.cargo/registry`; the probe reported `humantime-2.0.0.crate` and `Read-only file system`. With only `~/.cargo/registry` writable, the build downloaded `humantime 2.0.1` and compiled. The workspace `target` directory needs no entry.

```toml
# fragment
[session.confine]
write = ["~/.cargo/registry"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.cargo/registry -- cargo build
```

Add `~/.cargo/git` for Git dependencies; that path was not tested. Opening all of `~/.cargo` also makes `~/.cargo/bin` (where the `cargo` and `rustc` proxies run unconfined) and `~/.cargo/config.toml` writable. The `~/.cargo/credentials` and `~/.cargo/credentials.toml` files remain hidden if they exist when the session starts.

### Node with npm

Without `~/.npm`, `npm install` fails with `EROFS` under `~/.npm/_cacache`. With `~/.npm` writable, the probe completed with `added 2 packages`.

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

A virtual environment inside the workspace needs no `write` entry. The probe installed `requests` into a workspace virtual environment with `~/.cache` read-only.

```toml
# fragment
# A workspace virtual environment needs no write entry.
```

No recipe is given here for `uv`, `poetry`, `conda`, or `pipx`.

### Go

These paths were not tested by the maintainers. Run `go env GOMODCACHE GOCACHE`, then add the module cache and build cache it prints. The Go documentation says that `GOPATH` defaults to `$HOME/go`, the module cache is `GOPATH/pkg/mod`, and the build cache is a `go-build` directory under the user cache directory. Do not open `~/go/bin`, where `go install` puts binaries you run later.

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

This was not tested by the maintainers. Maven's default local repository is `~/.m2/repository`.

```toml
# fragment
[session.confine]
write = ["~/.m2/repository"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.m2/repository -- mvn package
```

`~/.m2/settings.xml` can hold server `username` and `password` and is not hidden by default, so add `hide = ["~/.m2/settings.xml"]` under `[session.confine]` when the agent does not need it.

### Gradle

This was not tested by the maintainers. Gradle's user home is `~/.gradle`, or the path in `GRADLE_USER_HOME`; its layout includes `caches`, `daemon`, `init.d`, `jdks`, `wrapper/dists`, and `gradle.properties`.

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

`gradle.properties` is often used for repository credentials, and `init.d` scripts run on every build. Do not open all of `~/.gradle` unless you accept that.

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
for example `~/.ssh` or TLS files under `~/.docker`; those are default hides and need an `unhide`
entry. None of this remote-engine path was tested.

Workloads the agent starts are not confined by a2amx. A socket created after the session starts
and outside every launch-time hide is visible. Start runtime daemons before the session when
their host socket must be hidden.

## Credentials a tool needs

Secrets are hidden by default. `unhide` under `[session.confine]` exposes a path read-only, and you can unhide one file inside a hidden directory without exposing the whole directory. For example, expose the configuration needed by `gh` like this:

```toml
# fragment
[session.confine]
unhide = ["~/.config/gh"]
```

The mount is read-only, so a tool that rewrites the file, such as during a token refresh, fails. The network is open, so a readable token lets the agent act as you on that service. Unhide only what the work needs, prefer a token with the least scope, and review what the agent did. The maintainers did not test any `gh` command under confinement.

### ssh and signed commits

Plain `ssh` fails in a confined session even with `~/.ssh` unhidden, with `Bad owner or permissions on /etc/ssh/ssh_config.d/...`. The sandbox cannot map root, so root-owned system files show as owned by `nobody`, and `ssh` refuses a system configuration file it does not trust. `ssh -F /dev/null host` skips the system files and works; a `~/.ssh/config` passed with `-F` should too, but the maintainers did not test it. `git push` over ssh should work with `GIT_SSH_COMMAND='ssh -F /dev/null'` and the key unhidden, untested. No ssh agent is passed through, so a passphrase-protected key cannot be used, and signing commits with gpg or an ssh key is untested and probably fails, because the agent sockets and `~/.gnupg` are hidden. Pushing over https with `gh` avoids all of this. Proper support is backlog item F53 in [the backlog](backlog.md).

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `Read-only file system` on a path under `$HOME` | The path is read-only. | Add the directory named in the error to `write` under `[session.confine]`. |
| A tool says it is not logged in, or reports an authentication error | Its credential file or directory is a default hide. | Add it to `unhide` under `[session.confine]`; see [Credentials a tool needs](#credentials-a-tool-needs). |
| `permission denied` on a container socket | Container sockets are hidden. | Use [Docker and other container engines](#docker-and-other-container-engines). |
| A file written under `/tmp` is not seen by a peer session | `/tmp` is private per session. | Use a directory in the workspace. |
| Another session cannot find a helper daemon that one session started | A recorded process id is wrong across sessions, and a socket or file under `/tmp` or `$XDG_RUNTIME_DIR` is not shared. | Find the helper by a port or a shared writable directory; see [Background processes](harnesses.md#background-processes). |
| A path in `write`, `hide`, or `unhide` is reported as `unresolved` in `--dry-run` | The operator path could not be canonicalized; the reason is shown as `not found`, `permission denied`, or another filesystem error. | Create or correct the path, or fix its permissions. Missing default hides continue to use `(skipped missing)`. |

## A complete team file

The following file sets team defaults in `[confine]` and uses `cwd = "."`. The architect session writes only in the workspace; the developer session also gets the narrow Rust and npm cache directories:

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
write = ["~/.cargo/registry", "~/.npm"]
```
