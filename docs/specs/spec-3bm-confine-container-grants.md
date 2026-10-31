# Spec 3bm: One grant per container runtime, `a2amx confine list`

## 0. Status

**LOCKED** (2026-10-31, amended once for the acceptance grep; one consultant review `m_4742`, all findings applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F48). Architect-directed, before 0.2.0.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `7524684` (after spec 3bl); full suite 714 passed, 0 failed, 3 ignored; run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `grant::all`, `grant::lookup`, `confine::plan`, `confine::notice` (`tests/grant.rs`, `tests/confine.rs`),
`team::parse` (`tests/team.rs`), the `a2amx confine list` command (`tests/grant.rs`, runs the binary with no daemon).

**Scope.** May edit exactly these files and no others: `src/grant.rs`, `src/confine.rs`, `src/cli.rs`, `src/main.rs`,
`src/team.rs` (one string, section 3), `tests/grant.rs`, `tests/confine.rs`, `tests/team.rs`, `tests/docs_examples.rs` (only if a fence changes), `a2amx.toml.example`,
`docs/confinement.md`, `docs/cli.md`, `docs/harnesses.md`, `docs/teams.md`. Does not touch: `a2amx.toml`, `CHANGELOG.md`
(the architect edits it; the architect also touches the `grant` row of `AGENTS.md` at commit), `src/daemon.rs`, `src/wire.rs`, `tests/wire.rs`, every other file. Re-anchor every line number against the tree after spec 3bl; the anchors below are from before it. No new dependency. Leave the diff
uncommitted.

## 1. Why

Spec 3bk shipped one all-or-nothing grant, `containers`, that opens every container and virtualization socket on the host
(docker, podman, containerd, lxd, libvirt, snapd and more). A user who wants `docker build` must also hand the agent libvirt
and snapd. The owner's decision: each runtime is enabled on its own, and every grant states what it exposes.

## 2. The grant table (`src/grant.rs`)

`Grant` gains two fields; `containers` is removed (confinement is unreleased, so no released interface breaks):

```rust
pub struct Grant {
    pub name: &'static str,
    pub exposes: &'static str,
    pub team_default_ok: bool,
    /// Host paths hidden unless the grant is given (`~/` is the home directory).
    pub sockets: &'static [&'static str],
    /// Environment names unset unless the grant is given.
    pub env: &'static [&'static str],
}
```

Seven rows, all `team_default_ok: false`, in this order, with exactly the paths and names that `CONTAINER_HIDES`,
`CONTAINER_UNSET` and `CONTAINER_UNSET_EXTRA` hold today (`src/confine.rs:37-62`), distributed as follows; those three
constants are deleted:

| name | sockets | env |
| --- | --- | --- |
| `docker` | `/run/docker.sock`, `~/.docker/desktop/docker.sock`, `~/.docker/run/docker.sock` | `DOCKER_HOST`, `DOCKER_CONTEXT` |
| `podman` | `/run/podman/podman.sock` | `CONTAINER_HOST`, `CONTAINER_CONNECTION` |
| `containerd` | `/run/containerd/containerd.sock`, `/run/containerd/containerd.sock.ttrpc`, `/run/buildkit/buildkitd.sock`, `/run/k3s/containerd/containerd.sock` | `CONTAINERD_ADDRESS`, `BUILDKIT_HOST` |
| `crio` | `/run/crio/crio.sock` | none |
| `libvirt` | `/run/libvirt/libvirt-sock` | `LIBVIRT_DEFAULT_URI` |
| `lxd` | `/var/snap/lxd/common/lxd/unix.socket`, `/var/lib/lxd/unix.socket`, `/var/lib/incus/unix.socket` | none |
| `snap` | `/run/snapd.socket`, `/run/snapd-snap.socket` | none |

`exposes` is one line each (the `containerd` line names buildkit and k3s, the `lxd` line names incus), in the form `<what it opens>: <consequence>`, and says plainly that a socket that controls a
container daemon is effectively root on the host (docker, podman, containerd, crio, lxd), that libvirt controls virtual
machines, and that snapd installs and removes packages.

`validate` is unchanged. `lookup` gains one special case before the closest-match logic: the 3bk name `containers` (distance 1 from
`containerd`, so it would otherwise point at the wrong runtime) errors with `confinement grant "containers" was split into one grant per runtime: docker, podman, containerd, crio, libvirt, lxd, snap; run a2amx confine list`. Otherwise `container` is distance 1 from `containerd` and further from the rest, so it names `containerd`.

Invariant: no socket path and no environment name appears in two rows or in `ALWAYS_UNSET` (test in section 7).

## 3. Plan and notice (`src/confine.rs`)

- Delete `allows_containers()`, the local `allow_containers` and the `allow_containers` parameters of
  `hide_containers`, `release_operator_writes`, `apply_operator_hides`, `unhide` and `check_container_socket` (the refusal
  condition becomes "the path is in `container_hidden`"). `hide_containers` walks `grant::all()` and hides the `sockets` of
  every grant **not** in the request, exactly as `CONTAINER_HIDES` did (same hide kind, same public `Plan.container_hidden:
  Vec<PathBuf>`); a granted runtime's sockets are not hidden. Granted wins: the canonical paths of the granted grants'
  sockets are computed first, and a not-granted socket whose canonical path equals one of them is not hidden (a distribution
  may link `/run/docker.sock` to `/run/podman/podman.sock`). `env_unsets` unsets, after `ALWAYS_UNSET`, the `env` names of every
  grant not given minus the `env` names of every grant given, deduplicated.
- The builder keeps a private `Vec<(PathBuf, &'static str)>` of each hidden canonical socket and its owning grant, filled in
  `hide_containers`. `check_container_socket` names the grant whose canonical socket the path equals or lies under (no parent
  directory matching: unhiding `/run/containerd` stays allowed, the socket stays hidden because the deeper mount wins):
  `{option} path {path} requires the {grant} grant (--confine-grant {grant}, or grants = ["{grant}"])`.
- `src/team.rs:532` (the one allowed edit there): the migration message for `confine_allow_containers` becomes
  `confine_allow_containers was replaced by one grant per runtime, for example grants = ["docker"], in [session.confine]; run a2amx confine list`;
  `tests/team.rs` (assert near `:2782`) changes with it.
- `notice()`: replace the two sentences `Container engines are allowed.` / `Container engines are restricted.` with
  `Container engines are restricted.` when no granted grant has sockets, and otherwise
  `Container engines allowed: {names}. Other container engines are restricted.` where `{names}` are the granted names whose
  row has non-empty `sockets`, joined by `, ` (sorted, as `plan.grants`). The existing `Granted: ...` line stays.
- `~/.docker` stays a default hide: with the `docker` grant the user-home sockets sit inside a hidden directory and stay
  hidden unless the user also unhides `~/.docker/run` or the socket; document it (section 6).

## 4. `a2amx confine list` (`src/cli.rs`, `src/main.rs`)

A new `Command::Confine { action: ConfineAction }` with the fieldless `ConfineAction::List`, which needs no daemon and no `HOME`: its arm goes in the early-return match of `main` next to `Mcp`, `Hook` and `OmpBridge` (`src/main.rs:68-82`), before `resolve_home` (`:142-156`). It does not clash with the session `Command::List`.

```
a2amx confine list
```

prints one line per grant in table order:

```
docker       session only   <exposes>
```

i.e. `{name:<12}{scope:<15}{exposes}` where `scope` is `team or session` when `team_default_ok` and `session only`
otherwise, then a final line `Give a grant with --confine-grant NAME, or grants = ["NAME"] in [session.confine].` Exit
code 0. The `Command` enum already carries `#[allow(clippy::large_enum_variant)]`; the new variant must not need another
suppression. The formatting is a pure function in `src/grant.rs` (`pub fn render_list() -> String`) so the test needs no
process; one binary test runs the command.

## 5. Dry run

Unchanged apart from the grant names: `  grants: docker, podman`. Nothing else prints.

## 6. Docs

`docs/confinement.md`: replace the `containers` text with the table of grants (name, what it exposes) and one example
`grants = ["docker"]` for a session, state that the `~/.docker` sockets also need `unhide`, and mention `a2amx confine list`.
`docs/cli.md`: the `confine list` command and `--confine-grant` (names the valid values by pointing to `confine list`).
`docs/harnesses.md`, `docs/teams.md`, `a2amx.toml.example`: change every `containers` grant to a runtime grant
(`docker` in the example). Every TOML fence still parses (`tests/docs_examples.rs`).

## 7. Tests

`tests/grant.rs`: rewrite the 3bk tests (`containers_is_a_non_team_default_grant`, the repeat test, the closest-match asserts and the valid-list assert, all naming `containers`): `container`, `containrs` and `contaner` now give `closest match: containerd`; `containers` gives the split message. New: (1) the table has the seven names in order, each with a non-empty `exposes`, `team_default_ok` false, and
the `sockets`/`env` of the table in section 2 (literal expectations); (2) `lookup("container")` errors with
`closest match: containerd`; (3) `render_list()` has seven lines plus the final line, each line starts with its name; (4) the
binary `a2amx confine list` exits 0 and prints the same text, also with `HOME`, `XDG_STATE_HOME` and `A2AMX_HOME` unset; (4b) invariant: no socket path and no env name appears in two rows or in `ALWAYS_UNSET` (`ALWAYS_UNSET` is private: assert through `plan` env unsets, or make the check inside `src/confine.rs` tests).
`tests/confine.rs`: replace every use of the `containers` grant (`rg -n -w containers tests/confine.rs`: the `grants` values, the
65-copy count test, the assertions of `containers grant` in the refusal text at the two refusal tests, and the closest-match
assert) with `docker` where the test uses the `~/.docker` fixtures (the tests around `unhide_cannot_lift_container_socket_restriction`
are a drop-in), and keep the asserted paths. Tests must not depend on what the host has installed: a socket outside the
home fixture exists only on some hosts. (5) with only `docker` given, the fixture `~/.docker/run/docker.sock` is not hidden
and `/run/podman/podman.sock` is in `hidden_paths()` or `skipped_paths()` (attempted either way) and `/run/docker.sock` is in
neither; env: `DOCKER_HOST` is not unset with `docker` and is unset without it, `CONTAINER_HOST` is unset with only `docker`;
(6) with only `podman` given and the fixture `~/.docker/run/docker.sock` present, `unhide` of that socket is refused naming
the `docker` grant (the abuse case: another grant never opens a socket); (6b) granted wins: a fixture where the
not-granted socket is a symlink to a granted one is not hidden; (7) the notice with `docker` says `Container engines
allowed: docker. Other container engines are restricted.`, with no grant it says `Container engines are restricted.` (replaces
the two assertions in `notice_describes_restrictions_and_keeps_closing_sentence`).
`tests/team.rs`: update every use of the grant (`rg -n -w containers tests/team.rs`, double- and single-quoted TOML, the
`  grants: containers` dry-run line, the team-level refusal, the repeated-grants and closest-match asserts, the
`confine_allow_containers` migration assert) so grants use a runtime name and the team-level error names it.

## 8. Out of scope

New runtimes beyond the seven, the rootless `$XDG_RUNTIME_DIR` sockets (the runtime directory is hidden wholesale and stays
so), credential grants (`gh`, `git-credentials`, `ssh`, `gpg`), harness grants, any change to network access, any edit of `src/team.rs` other than the one migration string of section 3.

## 9. Trust boundaries

The grant list is client-controlled input to `plan()` and is already validated there against the closed table (spec 3bk);
the change keeps that single gate. An unknown or misspelled name is an error naming the valid names; an extra grant opens
exactly the listed paths and no others. Abuse-case test: test 6 (a socket of a runtime that was not granted stays refused
even when another runtime is granted) and the existing unknown-grant test.

## 10. Placement

The table of what each grant exposes, with its sockets and environment names, is data about grants and lives in
`grant.rs`; `confine.rs` consumes it to build mounts and the notice. `confine list` formatting is pure and sits beside the
table.

## 11. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
rg -n 'CONTAINER_HIDES|CONTAINER_UNSET|allows_containers' src   # no matches
rg -n -w containers src tests docs a2amx.toml.example --glob '!docs/specs/*' --glob '!docs/backlog.md'
# only these may remain: docs/architecture.md and docs/install.md prose about running inside containers; in src/grant.rs the
# special case for the old name `containers` and its message; in tests/grant.rs the test of that message. Any other whole-word
# hit is reworded (a source comment or doc sentence), not allowed. Amendment 1 (m_4775): the migration lookup and its test are
# the intended survivors.
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent`: rerun once and record as
pre-existing. Mutants for the tester: (1) hide every runtime's sockets even when granted; (2) drop one socket from a row;
(3) unset all env names even when granted; (4) refusal names the wrong grant; (5) notice prints the old allowed sentence;
(6) closest-match uses a different threshold; (7) `confine list` omits a grant.

## 12. Claims

Line numbers are from `07bdcf9`, before spec 3bl; re-anchor. Counts use `rg -n -w containers`.

| Claim | Where | Verified how |
| --- | --- | --- |
| The three container constants hold the paths and names of section 2 | `src/confine.rs:36-60` | read |
| Uses of the constants and `allows_containers` | `src/confine.rs` | grep |
| The notice has the two container sentences | `src/confine.rs:1336-1340`, `tests/confine.rs:1692`, `:1705` | grep |
| The word `containers` as a grant name appears in `a2amx.toml.example`, `docs/confinement.md` (a TOML fence at about `:195` that `tests/docs_examples.rs` parses), `docs/cli.md`, `docs/teams.md`, `docs/harnesses.md`, `src/grant.rs`, `src/confine.rs`, `tests/confine.rs`, `tests/grant.rs`, `tests/team.rs`; the only hits in files outside scope are `docs/architecture.md`, `docs/install.md` (prose) and the legacy wire field in `tests/wire.rs` | consultant `m_4742` | grep |
| `main` returns early for `Mcp`, `Hook`, `OmpBridge` before `resolve_home` | `src/main.rs:68-82`, `:142-156` | read (consultant) |
| The migration message for `confine_allow_containers` | `src/team.rs:532`, `tests/team.rs:2782` | read (consultant) |
| `Command` carries one `#[allow(clippy::large_enum_variant)]` from spec 3bk | `src/cli.rs:33` | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer and tester on the
OMP harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed to either).
BLOCKED/amend cycles: 1 BLOCKED with 2 amendments, plus one BLOCKED that was a misreading of the truncated notice (lifted, no amendment).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | Tests 5 and 6 depended on which sockets the host has; two sockets could resolve to one file and defeat a grant (granted must win); the migration message in `src/team.rs` pointed at a removed grant; `confine list` dispatch had to run before the state directory is resolved; unlisted test changes; counts and anchors wrong; the acceptance grep too weak | All fixed in the spec before the lock |
| Implementation | Developer (`gpt-5.6-luna`) | BLOCKED: the fresh developer read the checkout as read-only from the truncated notice | Architect: the working directory is writable; wait lifted; fixed for new sessions by spec 3bl |
| Implementation | Developer | BLOCKED: the acceptance grep for the word `containers` forbade the required migration message and its test | Amendment 1 (`23f9892`) |
| Implementation | Developer | Section 8 listed `src/team.rs` as out of scope while section 3 required one string there | Amendment 2 (`2281c8d`) |
| Implementation | Developer | The granted-wins rule could not be tested through public seams with host-neutral fixtures | Architect approved one private unit test over a table-parameterised helper |
| Architect review | Architect | A test-only trait with a callback and a single-variant enum with an unused parameter were added only to serve one test | Developer replaced them with plain data rows and removed the enum |
| Tester pass | Tester (`gpt-5.6-luna`) | Reversing the order of the unset environment names survived all tests | Accepted: the order of `--unsetenv` arguments has no effect on behavior, and the output is deterministic by construction |
