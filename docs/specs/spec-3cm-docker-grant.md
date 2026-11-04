# Spec 3cm: The docker grant is detected and may be named team-wide; container grant texts say what they add

## 0. Status

LOCKED. Code, tests and docs; no change to the wire protocol.

Scope (files that may be edited): `src/grant.rs`, `src/cli.rs`, `docs/confinement.md`,
`docs/cli.md`, `docs/teams.md`, `docs/philosophy.md`, `tests/grant.rs`, `tests/suggest.rs`,
`tests/team.rs`.
Does not touch: every other `src/` file, `CHANGELOG.md`, `docs/backlog.md`, `AGENTS.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base `7d99811`.
Context id: `a2amx-docker-grant-2026-11-04`. Public test seams: `a2amx::suggest::detect`,
`a2amx::suggest::explain`, `a2amx::grant::all`, `a2amx::grant::render_list`, `a2amx::team::parse`,
and the text of `docs/confinement.md` and `docs/philosophy.md`.

## 1. Why

`docs/philosophy.md` says a grant's text says what the grant adds, not a worst-case claim that is
false for some setups, and that a tool-picked grant is shown with its exposure and needs a yes.
Today the five container grants (`docker`, `podman`, `containerd`, `crio`, `lxd`) say "effectively
root on the host" (`src/grant.rs:98, 111, 124, 137, 163`), which is not true of every setup the
`docker` grant covers (Docker Desktop's daemon runs in a VM; its per-user socket is also in the
grant's socket list, though `~/.docker` stays hidden unless the user also unhides it). And `a2amx confine suggest` never offers `docker`: a user whose project needs
Docker gets an agent that cannot run `docker ps`, with no hint.

Owner decisions (2026-11-04): (1) only `docker` is detected; `podman`, `containerd`, `crio`,
`libvirt`, `lxd` and `snap` stay explicit; (2) consent stays as it is: auto mode asks once per
grant name (default No), the accepted name is remembered for all projects; (3) the texts say what
the grant adds and how the risk depends on the setup, in the "accident net" wording below. A
pre-lock consultant review found that the wording first chosen (a hardened daemon, with
`userns-remap` or a sandboxed runtime, is much less risky) is false for the holder of the daemon
socket: a client can start a container with `--userns=host` and `--privileged`, and can choose the
runtime per container (Docker documents `--userns=host` as the per-container opt-out and requires
it for `--privileged` under remapping). Only rootless Docker, or running the agents in a VM
(philosophy rule 8), removes host root. The owner therefore chose the wording below and the
rewrite of the philosophy page passages that repeated the false claim (2026-11-04).

Why `docker` must become team-default-ok: the detector only reports grants with
`team_default_ok` (`src/suggest.rs:10`), and a team file refuses a grant without it
(`src/team.rs:700`). `confine suggest` prints a `grants = [...]` line that the user pastes into a
team file (philosophy rule 4), so a suggested `docker` that the team file then refuses would be a
broken first run. `docker` therefore becomes a grant a team file may name. The other container
grants keep `team_default_ok: false`.

Rejected: a per-project consent store for docker (the owner kept one rule for every grant);
detecting `podman` from the same markers (the owner chose docker only); a `Containerfile`
marker (it belongs to podman).

## 2. `src/grant.rs`

For the `docker` entry (`:96-109`): `team_default_ok: true`, and `detect` the existing-name list
`&["Dockerfile", "compose.yaml", "compose.yml", "docker-compose.yaml", "docker-compose.yml",
".devcontainer"]` (a file or a directory; the detector uses `dir.join(name).exists()`,
`src/main.rs:775`).

New `exposes` texts, one line each, replacing the five "effectively root on the host" strings.
They must be word for word the same in `docs/confinement.md` (section 3). They contain no `|` and
no backtick (the docs test splits table rows on `|`):

- `docker`: `docker daemon sockets: any docker command as you; an accidental rm -rf ~/ still fails, but on a rootful daemon a deliberate privileged container is root on the host, and userns-remap or a sandboxed runtime does not stop that`
- `podman`: `podman daemon socket: any podman command through it; the system socket belongs to a root service on a default install, so a deliberate privileged container is root on the host`
- `containerd`: `containerd, buildkit and k3s daemon sockets: any command through them; these daemons run as root on a default install, so a deliberate privileged container is root on the host`
- `crio`: `crio daemon socket: any command through it; the daemon runs as root on a default install, so a deliberate privileged container is root on the host`
- `lxd`: `lxd and incus daemon sockets: any command through them; these daemons run as root on a default install, so a deliberate privileged container is root on the host`

`libvirt` and `snap` texts stay as they are. No other field changes. The "default install" and
"rootful daemon" statements come from the upstream documentation of each daemon, not from a test;
the texts qualify them for that reason (philosophy rule 10). `rm -rf ~/` failing is philosophy
rule 6's example and follows from the read-only home; only the docker row says it.

## 3. `docs/confinement.md`, `docs/philosophy.md`, `docs/cli.md`

- `docs/confinement.md`: the five rows of the grant table (`:155-160`) get the new `exposes` texts
  and the docker row's third cell starts with `Yes` (for example `Yes; team or session.`); the
  other four keep `No; grant it per session.`. The marker table (`:204-216`) gains a `docker` row
  listing the six markers. The section "Docker and other container engines" (`:496`): the
  paragraph that says the grant "gives the agent root-equivalent power over the host" says
  instead what the grant adds (the accident-net wording of section 2: an accidental `rm -rf ~/`
  still fails; a deliberate privileged container is root on a rootful daemon; `userns-remap` or a
  sandboxed runtime does not stop it because a container can opt out), names rootless Docker and a
  VM as the real limits (see `docs/philosophy.md` rule 8), says that `confine suggest` and global
  auto mode now offer `docker` when a marker is present, with the usual consent prompt, and that a
  Docker Desktop user whose only socket is under `~/.docker` also needs the unhide the section
  already describes, because the grant does not lift that hide. Read the section whole before
  editing and keep its commands and the Docker Desktop paragraph. Line `:146` ("every grant may be
  set in the team table except container grants and harness-config, which remain session-only")
  now names `harness-config` and the container grants other than `docker`.
- `docs/teams.md:116`: "container grants and `harness-config` remain session-only" becomes true to
  the code: `harness-config` and the container grants other than `docker`.
- `docs/philosophy.md`, four passages, nothing else: (1) delete the sentence at `:46-47` "Some
  container grant texts are stricter than this today and will be brought in line."; (2) rule 3's
  example (`:44-46`, "A user with a hardened Docker daemon ... should not be told that Docker
  access is root on the host"), (3) rule 1's example (`:28-29`, "Docker can be set up securely, or
  run as root with none of that") and (4) the `docker` worked example (`:144-152`, which presents
  `userns-remap` and `runsc` as hardening of this grant) are rewritten so no sentence says that
  `userns-remap` or a sandboxed runtime limits what the holder of the docker socket can do. Each
  rewrite keeps its passage's point (rule 1: the result is as secure as the user configures it;
  rule 3: do not state a worst case that is false for some setup, with an example that is true,
  for instance that Docker Desktop's daemon runs in a VM; the worked example: what a careful user
  does, which is rootless Docker or running agents in a VM, with the grant's limits stated). Claims
  stay at what Docker's documentation says (philosophy rule 10); the architect reviews the wording
  at acceptance.
- `docs/cli.md`: check the `confine suggest` text (`:70`) still matches the code after this spec
  and the clap text of section 4; edit only if it does not.
  `docs/backlog.md:93` (F64, "high damage (containers) is session-only") is history and stays; the
  architect closes F74 there.

## 4. `src/cli.rs` (F74)

The doc comment on `ConfineAction::Suggest` (`:184`) says "Print a [session.confine] fragment for
the build tools found in DIR." It prints a `grants = [...]` line from project markers and git
remotes. Replace it with: `Print a grants line for the project markers and git remotes found in
DIR.` Check `a2amx confine --help` shows it.

## 5. Tests

Through the seams in section 0; expected values are literals.

- `tests/suggest.rs`: `detect(|name| name == "Dockerfile")` is `["docker"]`; each of the other five
  markers alone gives `["docker"]`; `detect(|name| name == "Containerfile")` is empty;
  `explain` reports `docker` with the reason `<marker> found` for the first marker in list order
  when several exist.
- `tests/grant.rs:658-668` and `tests/team.rs:2975-2982`: `docker` moves from the "session-only"
  loop to the "team-wide allowed" case; `harness-config` stays refused; add `podman` to the
  refused loop so one other container grant pins the rule.
- `tests/docs_examples.rs` (unchanged) is the contract test for the docs table; it fails on the old
  docs after the code change and passes after section 3.
- No test asserts the new `exposes` wording beyond the docs sync test; do not add one.

## 6. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| `detect` skips grants with `team_default_ok` false | `src/suggest.rs:6-17` | read |
| A team file refuses a grant without `team_default_ok` | `src/team.rs:696-706` | read |
| Markers are checked with `dir.join(name).exists()` | `src/main.rs:775` | read |
| `docker` has `team_default_ok: false` and `detect: NO_PATHS` | `src/grant.rs:96-108` | read |
| The docker grant's socket list includes `~/.docker/desktop/docker.sock` and `~/.docker/run/docker.sock`, but `~/.docker` is a default hide the grant does not lift | `src/grant.rs:34-38`, `src/confine.rs:78`, `docs/confinement.md:520-526` | read (consultant); architect read `src/grant.rs` |
| The docs table is synced to `exposes` and `team_default_ok` by a test | `tests/docs_examples.rs:100-154` | read |
| Tests pin `docker` as refused team-wide | `tests/grant.rs:658-668`, `tests/team.rs:2975-2982` | read |
| `render_list` prints the scope from `team_default_ok` | `src/grant.rs:634-639`, `tests/grant.rs:670-700` | read |
| Auto mode's consent prompt applies to every grant auto adds | `src/main.rs` `confirm_new_grants` (spec 3cf), `tests/auto_confine.rs` | read: `confirm_new_grants` at `src/main.rs:411` exists; behavior from spec 3cf and its tests, not re-read in full |
| No other test or doc pins the old "effectively root" text | `src`, `tests`, `docs` | grep: `src/grant.rs` and `docs/confinement.md` only; older specs 3bk and 3bm mention it as history |
| Statements that container grants stay session-only exist outside the table | `docs/confinement.md:146`, `docs/teams.md:116` | consultant grep; `docs/backlog.md:93` is history |
| Docker documents `--userns=host` as a per-container opt-out and requires it for `--privileged` under `userns-remap` | Docker Engine docs, "Isolate containers with a user namespace" | read by the consultant (web); not tested here, so the texts stay at what the docs say |

## 7. Out of scope

- The other container grants' detection and `team_default_ok`.
- `libvirt` and `snap` texts.
- Any change to the consent store, `--yes`, or the auto-grants file format.
- Rootless Docker and Podman under `$XDG_RUNTIME_DIR` (backlog F43).
- `CHANGELOG.md` and the backlog rows (the architect adds them at acceptance).
- A new test for the wording of `exposes` beyond the docs sync.

## 8. Acceptance

All run in the main checkout, as an ordinary user. This is a public-interface change: a team file
may now name `docker` (breaking nothing that worked before; a file that was refused now parses).

1. `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   pass. Judge: the architect reruns them.
2. Red phase: after the tests of section 5 are written and before the code, `cargo test --test
   suggest --test grant --test team` fails on the new and moved tests, except the `Containerfile`
   negative test, which passes on the old code (a contract test). The developer records it.
3. `a2amx confine list` shows `docker` as `team or session` and the other container grants as
   `session only`. Judge: the tester runs the built binary.
4. In a temp directory containing an empty `Dockerfile`, `a2amx confine suggest .` prints a line
   that contains `"docker"`; in an empty temp directory it does not. Judge: the tester, with the
   built binary and `HOME` set to a temp directory.
5. `rg -n 'effectively root' src docs/confinement.md docs/philosophy.md docs/teams.md` finds
   nothing. `rg -n 'will be brought in line' docs/philosophy.md` finds nothing.
   `rg -n 'userns-remap|runsc|sandboxed runtime' docs src` finds no sentence that presents them as
   limiting the docker grant (the architect reads the hits; the docker text and the Docker section
   may name them to say they do not).
6. `a2amx confine --help` shows the new `suggest` text; `rg -n 'fragment for the build tools'
   src docs/cli.md` finds nothing (`docs/backlog.md` and older specs keep it as history).
7. Mutation check by the developer, in the checkout, restored by checksum: (a) `docker`'s
   `team_default_ok` back to false: the moved team and grant tests fail; (b) drop one marker from
   the list: the per-marker `detect` test fails; (c) add `Containerfile` to the list: the negative
   test fails; (d) change one `exposes` word in `src/grant.rs` only: `grant_table_matches_the_code`
   fails.
8. `git status --short` shows only files in scope. Scratch files only under `target/scratch/3cm/`,
   deleted before reporting.

Known flaky tests: I7 `tests/attach_cli.rs::attach_does_not_capture_session_already_in_drag_mode`
and I14 `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`: rerun that
file once; if it passes, record it as pre-existing.
