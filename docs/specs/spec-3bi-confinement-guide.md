# Spec 3bi: a confinement guide with recipes for languages, credentials and Docker

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `a4932d4`; consultant review `m_4629`, findings 1-9 adopted; no second pass: the rewrite follows the findings and adds no new rule). Owner
request, 2026-10-30: set safe defaults and document how to configure confinement so users can develop with different
languages and technologies, and how an agent can use Docker under confinement. Owner decisions: no change to the defaults
and no profiles (docs recipes only); the guide is a new page `docs/confinement.md`, with examples also in
`a2amx.toml.example`. Context `a2amx-confinement-guide-2026-10-30`.
Implementation goes to `a2amx-developer` only; no tester pass; the architect checks every sentence of the guide against the
claims table (section 7) at final review.

**Ordering:** after spec 3bj (`~` in confinement paths) is committed: the examples use `~`, which the CLI and the team
commands silently skipped before 3bj.
**Baseline:** develop at `a4932d4` (spec 3bj committed); docs plus one new test file.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** `a2amx::team::parse` through the new `tests/docs_examples.rs`.

**Scope.** May edit exactly these files and no others: `docs/confinement.md` (new), `tests/docs_examples.rs` (new),
`a2amx.toml.example`, `README.md` (one list entry), and one sentence each in `docs/cli.md`, `docs/harnesses.md` and
`docs/teams.md`. Does not touch: `src/`, other tests, `docs/backlog.md`, `CHANGELOG.md`, every other file. No new
dependency (the test uses plain string handling and the `a2amx` crate). Leave the diff uncommitted.

## 1. Why

The confinement docs say what the flags do and where the feature stops (spec 3bb) but not how to develop under it. By
default only the workspace, its git directory and the harness's data paths are writable, secrets are hidden, and the rest
of `$HOME` is read-only, so the first `cargo build`, `npm install` or `docker ps` in a confined session fails with `Read-only
file system` or `permission denied`. The owner decided to keep the defaults minimal and predictable and to teach the
operator the few `confine_write`, `confine_unhide` and `confine_allow_containers` lines each ecosystem needs. The guide must
be honest about what was tested: the maintainers checked Rust, npm and pip on 2026-10-30 and could not check the others.

## 2. Content of `docs/confinement.md`

Short sections in the style of `docs/cli.md`; every command and TOML example must be supported by the section 7 claims.
Required sections, in this order:

1. **What confinement is for.** One sentence (a safety net for a trusted but fallible agent, not a security boundary) and
   a link to [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries). Then **the defaults at
   a glance**, a table of what spec 3ax section 2.3 defines: *writable* = the workspace, its git directory, the harness's
   data paths and a private `/tmp`; *read-only* = the rest of the filesystem including `$HOME`, plus inside the writable
   git directory `hooks`, `config` and `config.worktree`, the directory named by `core.hooksPath`, and the team file; *hidden* =
   secret locations such as `~/.ssh`, `~/.aws`, `~/.config/gh`, `~/.npmrc` and `~/.docker` (do not give a count), the
   state and runtime directories, `/run/WSL` and `/mnt/wslg`, and the container sockets.
2. **How to set it up.** The three ways, each with one example that matches the code: `a2amx new --confine
   [--confine-write PATH]... [--confine-unhide PATH]... [--confine-hide PATH]... -- <command>`; a session in a team file
   with `confine = true` (top level or per session) and the repeatable keys `confine_write`, `confine_hide`,
   `confine_unhide`; and `a2amx team up --dry-run` to preview the effective restrictions. Say: `~` and `~/...` are expanded
   to your home; other relative paths are relative to the session's working directory (in a team file, relative to the
   file's directory when `cwd` is relative); a path that does not exist is skipped and listed by `--dry-run` as
   `(skipped missing)`; each list takes at most 64 entries. The team-file example here is a complete file (it parses in
   the test of section 5).
3. **The method: open what an honest mistake can recover from.** Open a directory when an accident in it is recoverable (a
   cache is disposable; a toolchain can be reinstalled); keep closed what is not (credentials, the repository itself, the
   installed binaries and state). Prefer the narrowest cache directory. When a tool fails with `Read-only file system`,
   the error names the path: add that directory to `confine_write`, not `$HOME`. Whatever you open stays writable for the
   next unconfined run too, so say which recipe entries are caches and which hold things that run later (below).
4. **Recipes by ecosystem**, one subsection each with a TOML fragment (first line `# fragment`) and the equivalent flags.
   *Checked by the maintainers on 2026-10-30:*
   - Rust: `confine_write = ["~/.cargo/registry"]`. With `~/.cargo` read-only a build that needs a crate that is not
     cached fails with `Read-only file system` creating a `.crate` file under `~/.cargo/registry` (probed); with only
     `~/.cargo/registry` writable the build downloads and compiles (probed). Add `~/.cargo/git` for git dependencies
     (not tested). Opening the whole `~/.cargo` also makes `~/.cargo/bin` (the `cargo` and `rustc` proxies you run
     unconfined) and `~/.cargo/config.toml` writable: say so, and that `~/.cargo/credentials` and `credentials.toml`
     stay hidden if they exist at launch (default hides). The `target` directory inside the workspace needs no entry.
   - Node with npm: `confine_write = ["~/.npm"]`. Without it `npm install` fails with `EROFS` under `~/.npm/_cacache`
     (probed); with it the install works (probed). `~/.npmrc` is a default hide (registry tokens): a private registry
     needs it unhidden (item 6). Global installs (`npm install -g`) and version-manager directories are not covered.
   - Python with pip: a virtual environment inside the workspace needs no entry; pip installed a package with `~/.cache`
     read-only (probed). No recipe is given for `uv`, `poetry`, `conda` or `pipx`.
   *Not tested by the maintainers. The directories come from each tool's documentation; run the tool once and let the
   error name any other path:*
   - Go: the module cache and the build cache. Run `go env GOMODCACHE GOCACHE` and add those two directories (the Go
     documentation: GOPATH defaults to `$HOME/go`, the module cache is `GOPATH/pkg/mod`, the build cache is a `go-build`
     directory under the user cache directory). Do not open `~/go/bin`, where `go install` puts binaries you run later.
   - Maven: `~/.m2/repository` (Maven's documentation: the default local repository). `~/.m2/settings.xml` can hold server
     `username` and `password` and is not a default hide, so it is readable in the session; suggest
     `confine_hide = ["~/.m2/settings.xml"]` when the agent does not need it.
   - Gradle: `~/.gradle/caches` and `~/.gradle/wrapper` (Gradle's documentation: the Gradle user home is `~/.gradle`, set
     with `GRADLE_USER_HOME`, and holds `caches`, `daemon`, `init.d`, `jdks`, `wrapper/dists` and `gradle.properties`).
     `gradle.properties` is often used for repository credentials and `init.d` scripts run on every build: do not open
     `~/.gradle` as a whole unless you accept that.
   Closing sentence: for any other tool, run it once, read the failing path in the error, and decide with item 3.
5. **Docker and other container engines.** Facts only from the code and the specs (claims table):
   - By default the system container sockets are hidden (`/run/docker.sock`, `/run/podman/podman.sock`,
     `/run/containerd/containerd.sock` and the others the code lists) and the container variables (`DOCKER_HOST`,
     `DOCKER_CONTEXT`, `CONTAINER_HOST`, `CONTAINER_CONNECTION`, `CONTAINERD_ADDRESS`, `BUILDKIT_HOST`,
     `LIBVIRT_DEFAULT_URI`) are unset, so `docker ps` fails (the audit saw `EACCES` on the sockets).
   - To let the agent use the host's Docker daemon: `a2amx new --confine --confine-allow-containers -- <command>`, or
     `confine_allow_containers = true` in the team file. State plainly, as spec 3ax requires: this gives the agent
     root-equivalent power over the host, because a member of the `docker` group can start a container that mounts the
     host's root; confinement does not limit what a container can do. Use it only when you accept that.
   - Docker Desktop: its socket `~/.docker/desktop/docker.sock` (or `~/.docker/run/docker.sock`) is also inside the
     default hide of `~/.docker`, and the switch does not lift that hide. Both are needed: the switch and
     `confine_unhide` of the socket path (a test pins this).
   - `~/.docker` (the CLI's configuration, contexts and `docker login` credentials) is a default hide, so the CLI sees none
     of it; a private registry login needs `confine_unhide` of the file, with the warning of item 6. Not tested; say
     "the CLI configuration is hidden" and no more.
   - Rootless Docker and Podman cannot be used: their sockets live under `$XDG_RUNTIME_DIR`, which is hidden and
     cannot be unhidden, even with the switch. Options: an engine on another machine (`DOCKER_HOST` passes through with the
     switch and the network is open; reaching it needs its credentials, for example `~/.ssh` or TLS files under
     `~/.docker`, which are default hides and need `confine_unhide`; nothing here was tested); or run the whole session in a
     container or VM instead of a2amx confinement.
   - Containers the agent starts are not confined by a2amx. A socket created after the session started and outside every
     launch-time hide is visible: start container daemons before the session (existing `docs/cli.md` statement).
   Do not claim that `docker run`, `docker build` or `docker compose` work with the switch on; the maintainers did not test
   that. Say "after the switch the daemon socket is reachable" and no more.
6. **Credentials a tool needs.** Secrets are hidden by default. `confine_unhide` exposes a path read-only; a single file
   inside a hidden directory (one key under `~/.ssh`) can be unhidden on its own (3ax). Example:
   `confine_unhide = ["~/.config/gh"]` for the GitHub CLI. Say that the mount is read-only, so anything that rewrites the
   file (a token refresh) fails, and that the network is open, so a readable token lets the agent act as you on that service:
   unhide only what the work needs, prefer a token with the least scope, and review what the agent did. Do not claim which
   `gh` commands work; the maintainers had not tested `gh` under confinement when this was written.
7. **Troubleshooting**, a table `symptom | cause | fix`: `Read-only file system` on a path under `$HOME` | the path is
   read-only | add the directory named in the error to `confine_write`; a tool says it is not logged in, or an
   authentication error | its credential file or directory is a default hide | `confine_unhide` it (item 6); `permission denied`
   on a container socket | container sockets are hidden | item 5; a file written under `/tmp` is not seen by a peer session |
   `/tmp` is private per session | use a directory in the workspace; another session cannot find a helper daemon that one
   session started | a recorded process id is wrong across sessions, and a socket or file under `/tmp` or `$XDG_RUNTIME_DIR` is
   not shared | find the helper by a port or a shared writable directory (harnesses.md "Background processes"); a path in
   `confine_write` does not appear as writable in `--dry-run` | it does not exist and was skipped (the dry run lists it as
   `skipped missing`).
8. **A complete team file.** One TOML example (no `# fragment` marker) with an architect and a developer session,
   `confine = true` at the top level, per-session `confine_write` for a Rust-and-Node project, and `cwd = "."`. It must parse
   (section 5).

## 3. Style

Plain, short, second person for the operator. No product names other than the tools being configured (Rust, npm, pip, Go,
Maven, Gradle, Docker, Podman, `gh`). No mention of the maintainers' own agents or tooling, and no a2amx development
trivia. Fictional names only (`myproject`). Do not describe the threat model again; link it.

## 4. Other files

1. `a2amx.toml.example`: after the last session, add the lines `# --- confinement example ---`, then a complete
   commented `[[session]]` named `worker` (`# [[session]]`, `# name = "worker"`, `# command = ["omp"]`,
   `# confine = true`, `# confine_write = ["~/.cargo/registry", "~/.npm"]`, and a commented
   `# confine_allow_containers = true` line marked "root-equivalent, read docs/confinement.md"), then
   `# --- end confinement example ---`. Keys after a `[[session]]` header belong to that session; this block is its own
   session, so it must not set top-level keys. The existing comments and sessions are unchanged.
2. `README.md`: add `- [Confinement](docs/confinement.md) — set up --confine for your languages, credentials and Docker.`
   after the "Harnesses and delivery" entry.
3. `docs/cli.md` (end of the Confinement section), `docs/harnesses.md` (end of the confinement section) and `docs/teams.md`
   (end of "Confinement settings"): one sentence each, `See [Confinement](confinement.md) for recipes by ecosystem,
   credentials and Docker.`

## 5. The examples must parse: `tests/docs_examples.rs`

One integration test, `toml_examples_in_the_confinement_guide_parse`. It reads `docs/confinement.md` with
`include_str!("../docs/confinement.md")` (the file must exist before the crate's tests compile: until then the whole
`cargo test` fails to compile) and extracts every fenced block that starts with ```` ```toml ````. A block whose first line is
`# fragment` is skipped; every other block must parse with `a2amx::team::parse` (it requires `command` and a `name` or a
`role` per session, at least one session, and rejects unknown keys). The test asserts at least one non-fragment block was
parsed (the complete file of item 8). It also reads `a2amx.toml.example` with `include_str!`, takes the lines between
`# --- confinement example ---` and `# --- end confinement example ---`, strips one leading `# ` from each, appends them
to the file's other lines and asserts the result parses (the commented lines of the existing file stay comments); the
block's `confine_allow_containers` line stays commented in the file, so the test only asserts that the uncommented block
without it parses. `parse` checks syntax and team-file rules only: it never resolves paths, so a green test does not mean
a recipe works. Public interfaces only; expected values are literals.

## 6. Out of scope

`src/`, new flags, profiles, default changes, the `gh` and Docker end-to-end results (to be added by an amendment after the
architect runs them in a restarted team), `uv`, `poetry`, `conda`, `pipx`, `pnpm`, `yarn`, .NET and other ecosystems,
translations, screenshots, any change to `docs/backlog.md`, and backlog F43 (rootless engines) and F44.

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| Defaults: writable workspace, git common dir, harness data paths, private `/tmp`; read-only protected: `hooks`, `config`, `config.worktree`, the `core.hooksPath` directory, the team file | spec 3ax section 2.3 items 1-3 | read |
| Default hides include `~/.ssh`, `~/.docker`, `~/.config/gh`, `~/.npmrc`, `~/.cargo/credentials`, `~/.aws`; mandatory hides are the state and runtime directories, `/run/WSL`, `/mnt/wslg` | `src/confine.rs:63-95`, `:800-813` | read (consultant `m_4629`) |
| `~/.cargo/credentials*` stays hidden inside a writable `~/.cargo` if it exists at launch | `hide_or_skip` skips missing files (`src/confine.rs:852-861`); nested hides apply last (spec 3ax 2.3 item 6; `release_operator_writes` `:884-891`) | read (consultant) |
| Container sockets and variables hidden/unset by default; the switch turns both off | `CONTAINER_HIDES`, `CONTAINER_UNSET*`, `src/confine.rs:37-61`, `env_unsets` `:1028-1043` | read |
| The switch does not lift the `~/.docker` default hide; the socket needs `confine_unhide` plus the switch | `tests/confine.rs:728-776`; spec 3ax `:194` | read (consultant) |
| Root-equivalent wording required by the owner | spec 3ax item 4a | read |
| Rootless sockets stay hidden with the switch (`$XDG_RUNTIME_DIR` mandatory; unhide and write refused) | `src/confine.rs:224-233`, `:255`, `:801` | read (consultant) |
| `~` and `~/...` are expanded end to end | `expand_home` (`src/confine.rs:272-278`) plus spec 3bj's fix of `resolve_confine_paths` (`src/main.rs:592`) and its dry-run test | read; **valid only after spec 3bj** |
| Other relative paths are joined to the session's working directory; in a team file `cwd` is relative to the file's directory | `src/main.rs:461-463`, `:1039-1042` | read (consultant) |
| Missing paths are skipped and listed by `--dry-run`; lists take at most 64 entries | spec 3ax 2.3; `src/main.rs:1107-1109`; `docs/cli.md` | read |
| Cargo with `~/.cargo` read-only fails: `failed to open ...humantime-2.0.0.crate ... Read-only file system`; with `~/.cargo` writable the build succeeds | architect probe 2026-10-30, nested `bwrap`, rustup cargo | probed |
| Cargo with `~/.cargo` read-only and only `~/.cargo/registry` writable downloads `humantime 2.0.1` and builds | architect probe 2026-10-30 (no `~/.cargo/git` directory existed on the machine) | probed |
| npm with `~/.npm` read-only fails with `EROFS ... .npm/_cacache/tmp/...`; with `~/.npm` writable (tmpfs over it) `added 2 packages` | architect probe 2026-10-30, node v24 / npm 12 | probed |
| pip installed `requests` into a workspace venv with `~/.cache` read-only | architect probe 2026-10-30 | probed |
| Go: GOPATH defaults to `$HOME/go`; the module cache is `GOPATH/pkg/mod`; GOCACHE is a `go-build` subdirectory of the user cache directory | pkg.go.dev/cmd/go and go.dev/ref/mod, fetched 2026-10-30 | read in the vendor docs; not run |
| Maven: user settings `${user.home}/.m2/settings.xml`; default local repository `${user.home}/.m2/repository`; servers hold `username` and `password` | maven.apache.org/settings.html, fetched 2026-10-30 | read in the vendor docs; not run |
| Gradle: user home `~/.gradle`, `GRADLE_USER_HOME`; contains `caches`, `daemon`, `init.d`, `jdks`, `wrapper/dists`, `gradle.properties` | docs.gradle.org directory layout, fetched 2026-10-30 | read in the vendor docs; not run. That `gradle.properties` is often used for credentials and `init.d` scripts run on every build is general knowledge, not in the fetched page: the guide says "can" and "often" and the architect re-checks before commit |
| The audit saw `EACCES` on docker, containerd and snapd sockets from a confined session | architect probes and tester escape pass, 2026-10-30 | probed |
| `a2amx::team::parse(&str)` is public; `command` required; `name` or `role` required; at least one session; unknown keys rejected; confined `codex` and `claude --append-system-prompt-file` rejected | `src/team.rs:415`, `:28`, `:540-554`, `:801-803`, `:16`, `:858-879` | read (consultant) |
| No test pins README, `docs/cli.md`, `docs/harnesses.md`, `docs/teams.md` text; no test parses `a2amx.toml.example` | `rg` of `tests/` and `src/` | grep (consultant) |
| End-to-end `docker ps` and `gh` under confinement | not run | the guide must not claim it |

## 8. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test docs_examples
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short            # only the files of section 0
rg -n -i 'ctrscm|saltmdb' docs/confinement.md a2amx.toml.example README.md   # must print nothing
```

- The test fails before the guide exists (compile error: the `include_str!` file is missing) and passes after; the developer
  shows a block with a deliberate TOML mistake making it fail, then removes the mistake.
- The architect reads the final guide against the section 7 table; a sentence the table does not support is removed.
- Known flaky tests (backlog I9, F37): rerun once and record.

## 9. Pre-lock record

Consultant review `m_4629` adopted: `~` bug (spec 3bj, committed `a4932d4`), narrow cache paths, default-hide count, Docker
Desktop socket and remote-engine credentials, test and example-block traps, troubleshooting rows, honesty of the untested
group. Gate step 7 re-run: the process-list and state-directory wording of spec 3bb is not repeated here; "default hides"
are described without a count in item 1 and in the claims table only. Anchors checked at lock: `README.md:94`,
`docs/teams.md:110`, `docs/cli.md:49`, `docs/harnesses.md:50`, `a2amx.toml.example` ends with the developer session.

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer on the OMP
harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed); no tester pass
(documentation). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | `~` was not expanded by the CLI or the team commands, so every recipe would have been a silent no-op; the architect's claim came from reading `expand_home` and spec 3ax, not from running an example through the user-facing path | Spec 3bj (`a4932d4`) fixed it before this spec was assigned |
| Draft review | Consultant | The recipes opened whole homes (`~/.cargo`, `~/go`, `~/.gradle`, `~/.m2`), against the guide's own rule; `~/.cargo/bin` and `init.d` carry code that runs later | Narrow cache paths; the architect probed that `~/.cargo/registry` alone is enough for a cargo build |
| Draft review | Consultant | Default hides were undercounted (24 vs 28 plus three more); the Docker Desktop socket needs an unhide besides the switch; a remote engine needs hidden credentials; a troubleshooting row gave the wrong cause for a missing helper; maintainer trivia in a user guide | All adopted in the locked spec |
| Draft review | Consultant | Test design traps: the two marker rules conflicted, "at least two blocks" was not guaranteed, keys after a `[[session]]` header in `a2amx.toml.example` belong to that session | Marker rule only, at least one complete block, the example block is its own session |
| Architect review | Architect | The complete team file ran `cargo build` and `npm install` as session commands, which is not what a team file starts; the example block in `a2amx.toml.example` would have enabled `confine_allow_containers` when uncommented | Commands changed to `claude` and `omp`; the container line is double-commented |
| Architect review | Architect | Stale installed binary (`v0.1.2-46`, before spec 3bj): the developer's smoke test on it showed `~` paths skipped; the guide's claim holds on a current build | Operator note to reinstall before the restart; not a source defect |
