# Spec 3cz: The container-runtime grants carry an escape text, so the launch notice has something to say and a test (F73)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/grant.rs` (one constant and five `escape` fields), `tests/grant.rs` (two new
tests and the rewrite of one) and `tests/auto_confine.rs`. Does not touch: `src/main.rs`, `src/team.rs` (the notice wiring stays as it is),
`docs/` (the architect updates the backlog, the changelog and the docs line), the `exposes` texts, the
`harness-config` grant, every other grant, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seams: `a2amx::grant::escape_notice` and `consent_lines`, and the `a2amx team up
--dry-run` command with a team file.

## 1. Why

Backlog F73, a survivor of spec 3cf and the open disclosure part of the escape audit (F71). Since spec 3cf,
`run_team_up` (`src/main.rs:1555`) prints `session <name>: grant <grant> can escape: <text>` on stderr for every
grant a session names explicitly (`team::escape_notices`, tested with injected rows at `src/team.rs:1365`),
and `consent_lines` (`src/grant.rs:544`) and `render_list` (`src/grant.rs:632`, the `a2amx confine list` output)
print the same text under a grant row when `escape` is set. The text
comes from `Grant::escape`, and all 30 rows in `GRANTS` have `escape: None`, so the notice never prints and the
wiring in `run_team_up` has no end-to-end test: nothing can be asserted about an output that does not exist.

The owner's line for the audit (2026-11-06, `docs/philosophy.md`): what one or two ordinary commands reach is
closed, a crafted binary or kernel exploit is out of scope, and what stays open is disclosed. A container
runtime socket is the clearest disclosed case: the five grants `docker`, `podman`, `containerd`, `crio` and
`lxd` hand the agent a root-owned daemon, and a deliberate privileged container then reaches the host. Their
`exposes` text already says so in the table; the notice repeats it where the owner sees it at launch, for a
grant the owner named by hand (a grant found by auto mode is already shown at the consent prompt).

Owner decision, 2026-11-06: the escape text goes on the container-runtime grants only. The implicit
`harness-config` grant gets no launch notice (it is on for every session, a notice would print on every
start, and the confinement page and the grant table already say what it opens).

Rejected: moving the sentence out of `exposes` into `escape` (the docs table in `docs/confinement.md` mirrors
`exposes` word for word and `tests/docs_examples.rs:127` pins that, so the move would change five docs rows
for no gain); an escape text on `libvirt` and `snap` (their `exposes` text names machine control and package
changes, not a path to root on the host; say so rather than guess); a launch notice for `harness-config`
(decided above).

## 2. Change in `src/grant.rs`

Add one constant near the other grant constants (before `GRANTS`, line 96) holding the text, for example
`const ROOT_DAEMON_ESCAPE: &str = "a privileged container started through the daemon mounts the host
filesystem and runs as root on the host";` and set `escape: Some(ROOT_DAEMON_ESCAPE)` on `docker` (line 99),
`podman` (119), `containerd` (132), `crio` (145) and `lxd` (171). Line numbers are today's `escape: None` lines
under those names; re-read them, do not trust the numbers. No other field changes.

Three consumers read `escape` (grep `\.escape` and `escape` in `src/`): `escape_notice` (the launch notice),
`consent_lines` and `render_list` (`src/grant.rs:650-651`, prints `  can escape: {escape}` under the row). So
setting the five texts also adds five lines to `a2amx confine list`, and a second line under an
auto-proposed `docker` in `consent_lines` (`docker` is the only one of the five with detection markers). That is
the intended disclosure ("disclose the rest in `confine list`", F71) and is accepted; the existing code
already renders it.

Consequence for the existing test `render_list_has_every_grant_and_instruction` (`tests/grant.rs:676`):
it asserts `lines.len() == all().len() + 1`, zips the lines with `grant::all()` and pins `lines[8]`,
`lines[9]`, `lines[10]` by index; five escape lines shift every row after `docker`, `podman`, `containerd`,
`crio` and `lxd`. The developer rewrites it (section 3, test 4).

## 3. Tests

1. `tests/grant.rs`: `container_runtime_grants_carry_an_escape_text`: for each of `docker`, `podman`,
   `containerd`, `crio`, `lxd`, `grant::lookup(name)` has `escape` that is `Some` and non-empty, and for
   every other row in `grant::all()` `escape` is `None` (this pins the owner decision: `harness-config`,
   `libvirt` and `snap` stay `None`). Red on the old code.
2. `tests/grant.rs`: `escape_notice_renders_for_a_real_container_grant`: `grant::escape_notice("worker",
   &["docker".to_owned()], grant::all())` returns exactly one line starting with `session worker: grant docker
   can escape: `. Red on the old code.
3. `tests/auto_confine.rs`: a `team up --dry-run` test next to `auto_merge_preserves_explicit_grant_and_reports_added_reasons`
   (line 726) with the team file of that test minus `auto`: `[confine]\nenabled = true\n[[session]]\nname =
   \"worker\"\ncommand = [\"cat\"]\n[session.confine]\ngrants = [\"docker\"]\n` (`enabled = true` is
   required, no `auto`, no project markers needed): stderr contains `session worker: grant docker can escape: ` and the command succeeds. This is the missing
   end-to-end test of the wiring. A second case, same file, `grants = ["cargo"]`: stderr does not contain `can
   escape`. Red on the old code for the first case.

4. `tests/grant.rs`: rewrite `render_list_has_every_grant_and_instruction`. Keep every existing assertion's
   intent, but pair rows and lines after removing the lines that start with `  can escape: `; assert the
   rendered line count is the literal 36 (30 rows, 5 escape lines, 1 instruction line); pin the `gh`,
   `git-credentials` and `ssh` rows by content (the exact strings the test has today) found by their
   name prefix instead of by index; and assert that the line right after the `docker` row is `  can
   escape: ` plus the `docker` row's `escape` text. `binary_confine_list_needs_no_home_or_daemon`
   (line 728) compares with `render_list()` itself and stays unchanged.

## 4. Out of scope

`harness-config`, the project-files docs line, the omp quota probe `PATH` (F56), shell startup files, hides
that exist only at start, the consultant's escape enumeration (all F71 residuals), wording of `exposes`,
the code of `render_list`, `consent_lines`, `escape_notice` and `team::escape_notices` (they already do the
right thing), and the docs (`docs/cli.md:99` says "one-line exposure description"; the architect fixes that
line when committing).

## 5. Acceptance

1. `cargo test --test grant --test auto_confine` passes; before the change the developer shows the new tests
   1, 2, the rewritten test 4 (36 lines) and the first case of 3 failing.
2. `cargo test --test docs_examples` passes unchanged (the docs table mirrors `exposes`, which this spec does
   not touch).
3. Mutation check by restore-by-checksum edits in the checkout (the tree must equal the developer's own diff
   afterward): (a) set one of the five back to `None`, test 1 fails; (b) set `escape` on `libvirt`, test 1
   fails; (b2) drop the escape line from `render_list` (`src/grant.rs:650`), test 4 fails; (c) in `escape_notice` (`src/grant.rs:565`) drop the `session {session}:` prefix, test 2 fails;
   (d) in `team::escape_notices` (`src/team.rs:132`) skip every session, test 3 fails (the two unit tests at
   `src/team.rs:1365` and `:1397` fail too, which is fine for a probe). The developer names the
   failing test for each (the `src/` mutants are probes only).
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient
   environment of a hosted session. Known flakes I7, I14 and I18: rerun once and record as pre-existing (I14
   is fixed by spec 3cx once that lands).
5. Scratch files stay under `target/scratch/3cz/` and are removed before the report.
