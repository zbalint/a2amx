# Spec 3bz: toolchain, analyser and credential grants (confinement redesign, step 2)

## 0. Status

**LOCKED** (2026-11-02; one consultant pass applied). Context `a2amx-confine-redesign-2026-11-02`. Location: main checkout, branch `develop`.
Public test seams: `grant::all`, `grant::lookup`, `grant::render_list`, `confine::plan` / `confine::plan_with_path` with a
`ConfineRequest` built directly (`Plan::writable`, `Plan::readonly`, `Plan::protected`, `Plan::hidden_paths`,
`Plan::with_bwrap(..).wrap(..)`), the `a2amx confine list` binary test in `tests/grant.rs`, the existing real-bubblewrap
probe pattern in `tests/confine.rs` (`real_bubblewrap_hides_nested_credentials_but_allows_cache_writes`).

Scope (may edit): `src/grant.rs`, `src/confine.rs`, `tests/grant.rs`, `tests/confine.rs`, `docs/confinement.md`,
`docs/teams.md` (only the sentence naming the team-level grants).
Does not touch: `src/cli.rs`, `src/daemon.rs`, `src/main.rs`, `src/team.rs`, `src/suggest.rs`, `src/wire.rs`,
`docs/cli.md`, `docs/backlog.md`, `CHANGELOG.md`, `a2amx.toml*`, other specs. The grant table keeps
its shape (no new fields), so no other module changes.

This is step 2 of the redesign recorded in backlog F64. Not in this spec (their own steps): dropping the default
build paths and `build_paths`, folding `harness-config` into the default, `confine suggest` printing grants, the
config cleanup, the docs rewrite.

## 1. Why

Owner decision (2026-11-02): the daily driver of confinement is grants, one per common tool, easy to understand, each
all-or-nothing and read-write (spec 3by delivered that for the three path grants that existed). A confined session
today opens a build cache only through the automatic build-path detection, which step 3 removes, or through raw
`write` lines the user has to hunt down. This spec adds the grants that replace both: the toolchain and analyser
caches an agent needs to build and test, and the credential stores that are already hidden by default and that a user
may deliberately open. The rule behind the split: easy to recover and moderate damage is a plain grant (toolchains), but the text still
says that files in a toolchain directory are run by the user's own builds outside the session; hard to recover but
needed is a grant whose text spells out the risk (credentials); containers stay session-only
(unchanged, not touched here).

Two paths the research could not confirm are left out on purpose rather than guessed: the gpg grant (needs a probe of
the gpg-agent socket under confinement) and the dotnet, ruby (bundler), php, JDK/sdkman and version-manager grants
(paths only agent-reported). They are follow-ups (section 6).

## 2. Behaviour

### 2.1 The new grants

Every new grant has `team_default_ok: true`, `sockets: NO_PATHS`, `own_config: false`, `stand_ins: NO_PATHS`. The
`unhides` and `env` values and the exact `exposes` text are in the table; the table order after the existing eleven
rows is the order below. Env names must come from the fixed credential list in `src/confine.rs` (`CREDENTIAL_ENV`).
`~/` is the home directory.

Toolchain grants (cache and build data; no env):

| name | unhides | exposes |
| --- | --- | --- |
| `cargo` | `~/.cargo`, `~/.rustup` | the Rust toolchain, read-write: ~/.cargo (registry and git caches, installed crates; its bin directory stays read-only when it is on PATH) and ~/.rustup (installed toolchains), whose programs and settings (config.toml, env) your own builds outside the session run; your registry login stays hidden while it exists unless you also give cargo-token |
| `go` | `~/go`, `~/.cache/go-build` | the Go toolchain, read-write: ~/go (module cache, downloaded toolchains and installed binaries, which your own builds outside the session may run) and ~/.cache/go-build; ~/.netrc stays hidden unless you also give netrc |
| `python` | `~/.cache/pip`, `~/.cache/uv`, `~/.local/share/uv`, `~/.cache/pypoetry`, `~/.cache/virtualenv`, `~/.local/share/pipx` | the Python tool caches, read-write: pip, uv, poetry, virtualenv and pipx data under ~/.cache and ~/.local/share, including tool environments whose programs your own shell may run; ~/.pypirc stays hidden unless you also give pypi-token |
| `node` | `~/.npm`, `~/.yarn/berry`, `~/.local/share/pnpm`, `~/.bun/install/cache` | the JavaScript package caches, read-write: ~/.npm, ~/.yarn/berry, ~/.local/share/pnpm and ~/.bun/install/cache; ~/.npmrc stays hidden unless you also give npm-token |
| `java` | `~/.m2/repository`, `~/.gradle` | the Java build caches, read-write: ~/.m2/repository and the whole ~/.gradle, where a gradle.properties you wrote may hold repository passwords the agent can then read and an init.d script is run by every gradle build you start outside the session |
| `c-cpp` | `~/.cache/ccache`, `~/.ccache`, `~/.conan2`, `~/.cmake` | the C and C++ build caches, read-write: ccache, conan and cmake data in ~/.cache/ccache, ~/.ccache, ~/.conan2 and ~/.cmake, which your own builds outside the session use |

Analyser grants:

| name | unhides | env | exposes |
| --- | --- | --- | --- |
| `semgrep` | `~/.semgrep` | none | semgrep's data directory ~/.semgrep, read-write: its settings file holds the login token if you ran semgrep login |
| `sonar` | `~/.sonar` | `SONAR_TOKEN` | sonar-scanner's data directory ~/.sonar, read-write, and the SONAR_TOKEN environment variable: the agent can run analyses and publish results to your SonarQube or SonarCloud project as you |

Credential grants (each path is already in the default hides; the grant lifts that hide for it):

| name | unhides | env | exposes |
| --- | --- | --- | --- |
| `aws` | `~/.aws` | none | your AWS CLI configuration and credentials (~/.aws, read-write, including cached SSO tokens): the agent acts in your AWS accounts as you and can change or delete the files |
| `gcloud` | `~/.config/gcloud` | none | your Google Cloud CLI login (~/.config/gcloud, read-write): the agent acts in your Google Cloud projects as you and can change or delete the login |
| `azure` | `~/.azure` | `AZURE_CLIENT_SECRET`, `ARM_CLIENT_SECRET` | your Azure CLI login (~/.azure, read-write) and the AZURE_CLIENT_SECRET and ARM_CLIENT_SECRET environment variables: the agent acts in your Azure subscriptions as you and can change or delete the login |
| `kube` | `~/.kube` | none | your kubectl configuration (~/.kube, read-write): the cluster credentials in it let the agent act in your clusters as you, and it can change or delete them |
| `terraform` | `~/.terraform.d` | none | terraform's user directory (~/.terraform.d, read-write): the saved Terraform Cloud or registry tokens in it let the agent act as you, and it can change or delete them |
| `npm-token` | `~/.npmrc` | `NPM_TOKEN`, `NODE_AUTH_TOKEN` | your ~/.npmrc (read-write in place) and the NPM_TOKEN and NODE_AUTH_TOKEN environment variables: the agent can publish packages as you and change the file |
| `pypi-token` | `~/.pypirc` | `PYPI_TOKEN`, `TWINE_PASSWORD` | your ~/.pypirc (read-write in place) and the PYPI_TOKEN and TWINE_PASSWORD environment variables: the agent can upload packages as you and change the file |
| `cargo-token` | `~/.cargo/credentials`, `~/.cargo/credentials.toml` | `CARGO_REGISTRY_TOKEN`, `CARGO_REGISTRIES_CRATES_IO_TOKEN` | your cargo registry login (~/.cargo/credentials and ~/.cargo/credentials.toml, read-write in place) and the CARGO_REGISTRY_TOKEN and CARGO_REGISTRIES_CRATES_IO_TOKEN environment variables: the agent can publish crates as you and change the file |
| `netrc` | `~/.netrc` | none | your ~/.netrc (read-write in place): every host login and password stored there, which the agent can also overwrite |

"In place" is the single-file limit already documented for `git-credentials`: a single file bind-mounted from the
host can be written and appended to, but not renamed over or deleted (probed in spec 3by for `git-credentials` only; the tester probes the other four files, section 5).

A grant path that does not exist is not opened and is not an error (existing behaviour, test
`granted_paths_may_be_missing_without_becoming_unresolved`). No grant creates a missing directory. A tool that has
never run has no directory yet; the docs say so (2.4).

### 2.2 A grant path with a symlink component is refused

Six toolchain paths sit under `~/.cache`, which a confined session can write by default today (spec 3bx), and a
credential path may be a file. One session could replace `~/.cache/pip` with a symlink to `~/.config/systemd/user`,
`~/.config/git` or `~/.bashrc`, and the next session given `python` would open that target read-write: persistent
code execution outside the session. None of those targets is a default hide, so the hide check does not catch it.
The rule: `PlanBuilder::apply_grant_unhides` fails the plan, for every grant (the three existing path grants
included), when any component of the grant path below the home directory is a symlink, using `symlink_component`
(src/confine.rs:357) on the grant path as written under the (uncanonicalised) home. The error names the grant, the
entry, the symlink and the way out: `confinement grant "python" path ~/.cache/pip passes through the symlink
<link>; name the real path with a write entry instead`. This changes the existing 3by behaviour that a symlinked
grant path opens its canonical target (the accepted-alias half of the test containing
`resolves to the hidden path`, tests/confine.rs:~884-905) is updated; the `harness-config` symlink text (docs/confinement.md:~97) is a different grant and stays. The
inside-a-hidden-path check below stays as defence in depth.

### 2.3 A hidden path inside a granted directory stays hidden

Today `PlanBuilder::apply_grant_unhides` (src/confine.rs:1481-1521) fails the plan when a grant path and any default
hide overlap in either direction (`paths_overlap`), except hides that are the same grant's own paths. The `cargo`
grant would therefore always fail: `~/.cargo/credentials` and `~/.cargo/credentials.toml` are default hides inside
`~/.cargo`. Change the default-hide check only: it fails when the grant path **is or lies inside** the hidden path
(`is_within(&path, &hidden)`, the `gh` aliased-to-`~/.ssh` case pinned by the test containing
`confinement grant "gh" path ~/.config/gh resolves to the hidden path`). A hidden path **inside** the grant path is
fine: it stays hidden (while the file exists at launch; a missing one is not hidden, so `cargo login` can create it
when only `cargo` is given). `mounts()` already mounts the deeper hidden path after the writable parent (sort by depth),
and `real_bubblewrap_hides_nested_credentials_but_allows_cache_writes` already shows it for a cache directory. The
check against `mandatory_hidden` and `container_hidden` (src/confine.rs:1508-1521) stays `paths_overlap` in both
directions: nothing may open a directory that holds the state directory or a container socket.

A credential grant lifts exactly its own paths: `cargo` plus `cargo-token` leaves `~/.cargo` writable and the two
credential files visible and writable; `cargo` alone leaves them hidden. Giving `cargo-token` without `cargo`
opens only the credential files, and `~/.cargo` stays read-only (with `build_paths` false; until step 3 a Rust
build file in the workspace makes `~/.cargo` writable by the build-path default).

### 2.4 Docs

`docs/confinement.md`, "Grants" table: add one row per new grant, taken from `a2amx confine list` (name, exposes
text, Team default "Yes" for all new rows; the existing credential rows show how). Add a short subsection after the
table, "Choosing grants": a grant is all or nothing and read-write; the toolchain grants open caches, never
credentials; a credential grant lifts its own hide only; a path that does not exist is not opened, so run the tool
once outside the session or use a `write` line after it exists; an environment-relocated directory (`CARGO_HOME`,
`GOPATH`, `GRADLE_USER_HOME`, `XDG_CACHE_HOME` and the like) is not followed, use `write` for the real path. Say that a grant lifts only the hide of its own path: `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN`,
`GOOGLE_APPLICATION_CREDENTIALS`, `KUBECONFIG` and `TF_TOKEN_*` are not in the closed credential list and reach every
session either way. A grant path that is a symlink is refused (2.2). Replace the sentence that says only `gh`,
`git-credentials` and `ssh` may be team-wide (docs/confinement.md:~58-60 and ~86, docs/teams.md:~116-118) with: every
grant may be set in a team table except the container grants and `harness-config`. Do not compare with containers or
virtual machines. Nothing else in the docs changes (the build-path text stays until
step 3).

## 3. Claims about existing code

| claim | where | verified |
| --- | --- | --- |
| a grant's `unhides` paths become writable and are canonicalised, missing ones are skipped silently | src/confine.rs:1485-1525, tests/confine.rs `granted_paths_may_be_missing_without_becoming_unresolved` | read |
| the default-hide overlap check bails in both directions | src/confine.rs:1492-1507 (`paths_overlap`, src/confine.rs:215) | read |
| the aliased-`gh` test pins the "path is inside a hidden path" direction | tests/confine.rs:861-880 | read |
| mounts are sorted by depth so a nested hidden path mounts after its writable parent | src/confine.rs:1797-1808 | read |
| a nested credential stays hidden under a writable cache directory in real bubblewrap | tests/confine.rs:4220 | read; run by the tester |
| every credential path in section 2.1 is already a default hide | src/confine.rs:71-101 (`DEFAULT_HIDES`, `DEFAULT_HIDES_EXTRA`) | read (`~/.aws`, `~/.config/gcloud`, `~/.azure`, `~/.kube`, `~/.terraform.d`, `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials`, `~/.cargo/credentials.toml`, `~/.netrc`) |
| env names released by a grant are a subset of `CREDENTIAL_ENV` | src/confine.rs:36-66 | read (SONAR_TOKEN, NPM_TOKEN, NODE_AUTH_TOKEN, PYPI_TOKEN, TWINE_PASSWORD, CARGO_REGISTRY_TOKEN, CARGO_REGISTRIES_CRATES_IO_TOKEN, AZURE_CLIENT_SECRET, ARM_CLIENT_SECRET all present) |
| `team_default_ok` is the only team-level gate | src/team.rs:620 | read |
| the docs examples test only parses TOML blocks, it does not compare the grant table | tests/docs_examples.rs:25-80 | read |
| `~/.semgrep` holds `semgrep.log` and `settings.yml`; `~/.sonar` holds `cache`, `js`, `_tmp` | local listing of names only, 2026-11-02 | probed |
| pip, uv, go, cargo, npm, pnpm-store, gradle, maven default paths | research finding of 2026-11-02 (official pages read by the architect for cargo, go, uv, pip, npm, gradle, maven); poetry, virtualenv, pipx, yarn berry, bun, ccache, conan, cmake agent-reported | the second group is unconfirmed; a wrong path opens nothing |

## 4. Tests

Written first, each failing on the old code:

1. `tests/grant.rs`: extend `grant_table_has_expected_rows` with the 17 new rows (name, sockets, env, unhides, stand_ins,
   team_default_ok, own_config as literals from 2.1); a test that the exposure text of each new row equals the
   literal in 2.1; `team_table_accepts_credential_grants_and_rejects_the_rest` accepts every new grant team-wide
   (the container grants and `harness-config` are still refused); in `grant_validation_rejects_repeats_and_suggests_close_names` the typos `g`, `gx` and `gj` become
   ambiguous (distance 1 from both `gh` and `go`): move them to the ambiguous cases and add `ghh` -> closest match `gh`;
   every other typo case keeps its assertion;
   every new row's `env` is a subset of the credential list (literal list in the test).
2. `tests/confine.rs`: with grant `cargo` and a fixture that has `~/.cargo` with `credentials.toml` and a `bin`
   directory on PATH: the plan succeeds, `~/.cargo` is writable, `~/.cargo/credentials.toml` stays in
   `hidden_paths`, `~/.cargo/bin` is protected. With `cargo` and `cargo-token`: both credential files are visible and
   writable. With `cargo-token` only: `~/.cargo` is not writable. The aliased-`gh` test stays unchanged and green.
   Abuse cases: (a) `~/.cache/pip` a symlink to `~/.ssh` with grant `python` fails; (b) `~/.cache/pip` a symlink to a
   plain directory outside every hide, with `python`, fails with the symlink message, and so does a symlinked
   `~/.cargo` with `cargo`, a symlinked `~/.npmrc` with `npm-token`, and `~/.config/gh` a symlink to an unhidden
   directory with `gh` (this replaces the old accepted-alias half of the test); (c) a symlinked directory above the
   home does not trip the rule. All plan tests in this list use `build_paths: false` (the `ConfineRequest` default).
3. `tests/confine.rs`: one real-bubblewrap probe (skipped without bwrap, as the existing probe) for `cargo`:
   write a file in `~/.cargo`, cannot read or write `credentials.toml`, cannot write `~/.cargo/bin` when it is on PATH.
4. `tests/confine.rs`: each of the nine credential grants and each toolchain grant given alone on a fixture with its
   paths present: the plan's `writable` minus the `writable` of the same request without the grant equals the grant paths that
   exist (a table-driven test is fine), and none of the hidden defaults outside the grant is lifted. For `azure`,
   `npm-token`, `pypi-token`, `cargo-token` and `sonar`, the wrapper's unset list lacks the released env names and
   still holds the rest of the credential list.

## 5. Acceptance

Run from the main checkout, `develop`:

- `env -u A2AMX_BIN -u NO_COLOR cargo test` all green (baseline before the change: 785 passed, 3 ignored, 0 failed).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` clean.
- `scripts/prepush-fast.sh` clean.
- `cargo run -q -- confine list` prints 28 grant rows then the instruction line; the 17 new rows match 2.1 (judge: the
  tester diffs the output against this table).
- Mutation check (the tester, on a scratch copy): (a) restore the old two-direction overlap check on default hides
  -> the `cargo` plan test fails; (b) make the check ignore the inside-a-hidden-path direction -> the aliased-`gh`
  and aliased-`pip` tests fail; (c) drop `~/.cargo/credentials.toml` from `cargo-token` -> the table test fails;
  (d) change a team flag to false -> the team test fails.
- Abuse cases (trust boundaries: a symlink planted in a writable location; a repo-shipped team file naming grants):
  the symlink tests in 4.2; a team file with `grants = ["cargo-token"]` parses (4.1) and its plan releases the env
  (4.4); `grants = ["docker"]` at team level still fails (4.1).
- In-place probe (the tester, real bubblewrap, report only): for `~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials.toml`
  and `~/.netrc` as single files, append works, rename over and delete fail, as for `git-credentials`; the report
  states the result per file.
- Contract tests that already pass on the old code: the aliased-`gh` test and the missing-path test.
- Known flaky: none known; rerun once, then record as pre-existing.

## 6. Out of scope

Dropping the build paths, `build_paths`, `ensure_build_paths`, `suggest` changes (step 3). The `harness-config`
default. New default hides (glab, helm, gem, composer, poetry auth and the like). The gpg grant and any grant needing
a socket. Dotnet, ruby, php, JDK, version managers. Creating missing directories. Following `CARGO_HOME` and the
like. `CHANGELOG.md` (the architect adds the line). Any change to `Grant`'s fields.

## 7. Placement

`src/grant.rs` owns the table, so every new row lives there; the one logic change is in
`PlanBuilder::apply_grant_unhides`, which owns grant-vs-hide checks. Size: grant.rs grows by about 350 lines of data;
the table is data, not logic. `apply_grant_unhides` is near the cognitive-complexity limit (30): the change replaces
one call with another and adds no branch; the developer hand-counts the function's cognitive complexity before and after
and extracts a helper if the change adds a branch; the architect reads the Sonar gate after the push.
