# Confinement

Confinement is a safety net for a trusted but fallible agent, not a security boundary; see [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries).

## Defaults at a glance

| Area | Default |
| --- | --- |
| Writable | The workspace, its Git directory, the harness's data paths, and a private `/tmp`. |
| Read-only | The rest of the filesystem, including `$HOME`; inside the writable Git directory, `hooks`, `config`, `config.worktree`, the directory named by `core.hooksPath`, and the team file are read-only. |
| Hidden | Secret locations such as `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.npmrc`, and `~/.docker`; the state and runtime directories, `/run/WSL`, `/mnt/wslg`, and the container sockets. |

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

`~` and `~/...` expand to your home directory. Other relative paths are relative to the session's working directory. A path that does not exist is skipped. Each of the three lists accepts at most 64 entries.

### Team file

Set `confine = true` at the top level to default it for sessions, or set it on one session. Use the repeatable `confine_write`, `confine_hide`, and `confine_unhide` keys for that session. In the complete example below, `cwd = "."` is relative to the team file's directory; other relative paths are relative to the resolved session working directory.

### Preview

Run this before starting the team:

```sh
a2amx team up --dry-run
```

The dry run shows the effective restrictions. It lists a missing path as `(skipped missing)`. The complete team file at the end of this page parses as a team file.

## The method: open what an honest mistake can recover from

Open a directory when an accident in it is recoverable: a cache is disposable, and a toolchain can be reinstalled. Keep credentials, the repository itself, installed binaries, and state closed. Prefer the narrowest cache directory.

When a tool reports `Read-only file system`, add the directory named in the error to `confine_write`, not `$HOME`. Whatever you open stays writable for the next unconfined run too. The recipe entries below are caches; opening all of `~/.cargo` also opens `~/.cargo/bin` and `~/.cargo/config.toml`, while its `credentials` files stay hidden when they exist at launch.

## Recipes by ecosystem

### Rust

With `~/.cargo` read-only, a build that needs an uncached crate fails while creating a `.crate` file under `~/.cargo/registry`; the probe reported `humantime-2.0.0.crate` and `Read-only file system`. With only `~/.cargo/registry` writable, the build downloaded `humantime 2.0.1` and compiled. The workspace `target` directory needs no entry.

```toml
# fragment
confine_write = ["~/.cargo/registry"]
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
confine_write = ["~/.npm"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.npm -- npm install
```

`~/.npmrc` is hidden by default. Unhide it only when a private registry needs it; see [Credentials a tool needs](#credentials-a-tool-needs). Global installs (`npm install -g`) and version-manager directories are not covered.

### Python with pip

A virtual environment inside the workspace needs no `confine_write` entry. The probe installed `requests` into a workspace virtual environment with `~/.cache` read-only.

```toml
# fragment
# A workspace virtual environment needs no confine_write entry.
```

No recipe is given here for `uv`, `poetry`, `conda`, or `pipx`.

### Go

These paths were not tested by the maintainers. Run `go env GOMODCACHE GOCACHE`, then add the module cache and build cache it prints. The Go documentation says that `GOPATH` defaults to `$HOME/go`, the module cache is `GOPATH/pkg/mod`, and the build cache is a `go-build` directory under the user cache directory. Do not open `~/go/bin`, where `go install` puts binaries you run later.

```toml
# fragment
# Replace these placeholders with the paths printed by go env.
confine_write = ["/path/from/GOMODCACHE", "/path/from/GOCACHE"]
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
confine_write = ["~/.m2/repository"]
```

Equivalent command-line option:

```sh
a2amx new --confine --confine-write ~/.m2/repository -- mvn package
```

`~/.m2/settings.xml` can hold server `username` and `password` and is not hidden by default, so add `confine_hide = ["~/.m2/settings.xml"]` when the agent does not need it.

### Gradle

This was not tested by the maintainers. Gradle's user home is `~/.gradle`, or the path in `GRADLE_USER_HOME`; its layout includes `caches`, `daemon`, `init.d`, `jdks`, `wrapper/dists`, and `gradle.properties`.

```toml
# fragment
confine_write = ["~/.gradle/caches", "~/.gradle/wrapper"]
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

By default, the system container sockets are hidden, including `/run/docker.sock`, `/run/podman/podman.sock`, and `/run/containerd/containerd.sock`. The container variables `DOCKER_HOST`, `DOCKER_CONTEXT`, `CONTAINER_HOST`, `CONTAINER_CONNECTION`, `CONTAINERD_ADDRESS`, `BUILDKIT_HOST`, and `LIBVIRT_DEFAULT_URI` are unset. Consequently, `docker ps` fails with `EACCES` on the socket.

Allow the configured host Docker daemon with the switch or the team-file setting:

```sh
a2amx new --confine --confine-allow-containers -- <command>
```

```toml
# fragment
confine_allow_containers = true
```

This gives the agent root-equivalent power over the host: a member of the `docker` group can start a container that mounts the host's root, and confinement does not limit what a container can do. Use the switch only when you accept that. After the switch, the configured daemon socket is reachable; the maintainers did not test `docker run`, `docker build`, or `docker compose` here.

The Docker Desktop sockets `~/.docker/desktop/docker.sock` and `~/.docker/run/docker.sock` are also inside the default hide of `~/.docker`. The switch does not lift that hide, so both the switch and an unhide are needed:

```sh
a2amx new --confine --confine-allow-containers \
  --confine-unhide ~/.docker/desktop/docker.sock -- <command>
```

Choose `~/.docker/run/docker.sock` instead when that is the socket path. The Docker CLI configuration, contexts, and `docker login` credentials are under the default hide of `~/.docker`, so the CLI configuration is hidden. A private registry login needs `confine_unhide` for the needed file, with the warning in [Credentials a tool needs](#credentials-a-tool-needs); this was not tested.

Rootless Docker and Podman cannot be used here: their sockets are under `$XDG_RUNTIME_DIR`, which is hidden and cannot be unhidden, even with the switch. Use an engine on another machine (`DOCKER_HOST` passes through with the switch and the network is open), or run the whole session in a container or VM instead of a2amx confinement. Reaching a remote engine needs its credentials, for example `~/.ssh` or TLS files under `~/.docker`; those are default hides and need `confine_unhide`. None of this remote-engine path was tested.

Containers the agent starts are not confined by a2amx. A socket created after the session starts and outside every launch-time hide is visible. Start container daemons before the session when their host socket must be hidden.

## Credentials a tool needs

Secrets are hidden by default. `confine_unhide` exposes a path read-only, and you can unhide one file inside a hidden directory without exposing the whole directory. For example, expose the configuration needed by `gh` like this:

```toml
# fragment
confine_unhide = ["~/.config/gh"]
```

The mount is read-only, so a tool that rewrites the file, such as during a token refresh, fails. The network is open, so a readable token lets the agent act as you on that service. Unhide only what the work needs, prefer a token with the least scope, and review what the agent did. The maintainers did not test any `gh` command under confinement.

### ssh and signed commits

Plain `ssh` fails in a confined session even with `~/.ssh` unhidden, with `Bad owner or permissions on /etc/ssh/ssh_config.d/...`. The sandbox cannot map root, so root-owned system files show as owned by `nobody`, and `ssh` refuses a system configuration file it does not trust. `ssh -F /dev/null host` skips the system files and works; a `~/.ssh/config` passed with `-F` should too, but the maintainers did not test it. `git push` over ssh should work with `GIT_SSH_COMMAND='ssh -F /dev/null'` and the key unhidden, untested. No ssh agent is passed through, so a passphrase-protected key cannot be used, and signing commits with gpg or an ssh key is untested and probably fails, because the agent sockets and `~/.gnupg` are hidden. Pushing over https with `gh` avoids all of this. Proper support is backlog item F53 in [the backlog](backlog.md).

## Troubleshooting

| Symptom | Cause | Fix |
| --- | --- | --- |
| `Read-only file system` on a path under `$HOME` | The path is read-only. | Add the directory named in the error to `confine_write`. |
| A tool says it is not logged in, or reports an authentication error | Its credential file or directory is a default hide. | Unhide it with `confine_unhide`; see [Credentials a tool needs](#credentials-a-tool-needs). |
| `permission denied` on a container socket | Container sockets are hidden. | Use [Docker and other container engines](#docker-and-other-container-engines). |
| A file written under `/tmp` is not seen by a peer session | `/tmp` is private per session. | Use a directory in the workspace. |
| Another session cannot find a helper daemon that one session started | A recorded process id is wrong across sessions, and a socket or file under `/tmp` or `$XDG_RUNTIME_DIR` is not shared. | Find the helper by a port or a shared writable directory; see [Background processes](harnesses.md#background-processes). |
| A path in `confine_write` does not appear as writable in `--dry-run` | It does not exist and was skipped. | The dry run lists it as `skipped missing`; create the directory or correct the path. |

## A complete team file

The following file sets `confine = true` at the top level and uses `cwd = "."`. The architect session writes only in the workspace; the developer session also gets the narrow Rust and npm cache directories:

```toml
team = "myproject"
confine = true

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
confine_write = ["~/.cargo/registry", "~/.npm"]
```
