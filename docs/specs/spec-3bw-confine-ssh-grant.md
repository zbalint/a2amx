# Spec 3bw: the `ssh` grant, so `git push` over ssh works when the user turns it on (F53, F62 step e)

## 0. Status

**LOCKED** (2026-11-01; one consultant review `m_5013` applied, no second pass). Context
`a2amx-confine-ssh-grant-2026-11-01`. Location: main checkout, branch `develop`.
Public test seams: `grant::all` / `grant::lookup` / `grant::render_list`, `confine::plan_with_path` (with a
`ConfineRequest` built directly) and `Plan::hidden_paths`, `Plan::skipped_paths`, `Plan::wrap`, the `a2amx confine
list` binary test in tests/grant.rs.

Scope (may edit): `src/grant.rs`, `src/confine.rs`, `tests/grant.rs`, `tests/confine.rs`, `docs/confinement.md`.
Does not touch: `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/daemon.rs`, `src/wire.rs`, `CHANGELOG.md`,
`docs/backlog.md`, `a2amx.toml`, other specs.

## 1. Why

Owner decisions (2026-10-31, 2026-11-01): confinement is a safety net against accidents, not a sandbox; a grant
the user names means full access to that integration; "ssh means full ssh". Today `ssh` fails in a confined
session even with `~/.ssh` unhidden, so a team whose remote is `git@github.com:...` cannot push without a
hand-written https recipe (F62 step e, F53). The root cause is not the keys. The sandbox cannot map root, so
`/etc/ssh/ssh_config` shows as owned by `nobody`; `ssh` checks ownership of every file the main config includes
(`Bad owner or permissions on /etc/ssh/ssh_config.d/...`) but not of the main file itself, so an empty file over
the main file ends the failure.

The earlier F53 row asked for a per-key grant with a generated OpenSSH config, a key list and a file-to-other-path
mount seam. That is the sandbox direction the owner rejected. This spec builds the small version: unhide the
directory read-only and show the system ssh config as an empty file. Agent pass-through is left out (follow-up F63).

## 2. Behaviour

1. `grant.rs`: `Grant` gets `stand_ins: &'static [&'static str]`, absolute paths of host files the session sees as
   empty when the grant is given. Every existing grant gets `NO_PATHS`. New grant `ssh` (appended after
   `git-credentials`; `static GRANTS` becomes `[Grant; 11]`): `team_default_ok: false`, `sockets` and `env`
   empty, `own_config: false`, `unhides: ["~/.ssh"]`, `stand_ins: ["/etc/ssh/ssh_config"]`, `exposes`: "your ssh
   keys and config (~/.ssh, read-only): the agent can log in to every host those keys open as you, and push with
   them".
2. `confine.rs`: a new builder step runs right after `apply_grant_unhides` (the call at src/confine.rs:982). For each
   given grant and each of its `stand_ins`: take the canonical path if it exists and `add_unique` it to
   `self.hidden`; if it does not exist add the source path to `self.skipped`. The step runs whether or not
   `no_default_hides` is set (it must sit outside `hide_defaults`, which returns early). `mounts()` already turns a
   hidden non-directory into `MountKind::Empty`, so there is no new mount kind, no `Plan` field, no change to
   `src/main.rs` and no new notice line: the dry run prints `hidden: /etc/ssh/ssh_config` (or `(skipped missing)`)
   and the notice already says a hidden file looks empty. Do not add the path to `default_hidden` (that list is the
   provenance list for the overlap and alias checks).
3. Interactions, each pinned by a test in 5: an explicit operator `unhide` or `write` of exactly the stand-in path
   removes it from `hidden` again (explicit entries win, as for every hide). `grants = ["ssh"]` with
   `hide = ["~/.ssh"]` fails the launch with the existing `confinement grant "ssh" conflicts with the hide of ...`
   message (same as `gh`). `hide = ["~/.ssh/id_work"]` with the grant works and is the way to keep one key out.
4. Docs (`docs/confinement.md`): add the table row after the `git-credentials` row (same text as `exposes`); add
   `ssh` to the credential-grant sentence at lines 79-80; in "Credentials a tool needs" rewrite only the first
   clause of the sentence at line 311 ("SSH pushes need an `ssh` grant (backlog F53, not built)"), keep the keyring
   half; rewrite the "ssh and signed commits" section (lines 354-356) as the working recipe, deleting the failure
   story and the "Pushing over https with `gh` avoids all of this" sentence, and point line 281 at the grant. State
   the limits plainly: settings from the system ssh config no longer apply (copy what you need into `~/.ssh/config`;
   an `Include` of a root-owned file there still fails); `~/.ssh` is read-only, so ssh cannot add a host to
   `known_hosts` (add the host from outside first; do not claim how an unknown host fails), and a `ControlPath`
   under `~/.ssh` is unusable (point it at `/tmp`); no `SSH_AUTH_SOCK` pass-through, so a passphrase-protected key
   cannot be used (F63); signing commits with an unencrypted listed ssh key works, gpg is F54; with `no_default_hides`
   `~/.ssh` is visible without the grant but the system config still breaks ssh. Keep the https recipe for people
   who prefer it.

## 3. Out of scope

`SSH_AUTH_SOCK` pass-through and binding an agent socket (F63), a per-key list, a generated OpenSSH config, a
writable `known_hosts`, gpg (F54), a `(grant ssh)` label on the hidden line (it would need `src/main.rs`),
`a2amx.toml` (the owner's team file), changelog and backlog edits (the architect makes them), any change to
`apply_grant_unhides` or `hide_defaults`.

## 4. Attack paths (trust-boundaries)

| Path | Disposition |
| --- | --- |
| A session without the grant reaches `~/.ssh` or loses the system ssh config | Removed: default hide and mounts unchanged; contract test |
| A team-level table grants `ssh` to every session | Removed: `team_default_ok: false`; contract test names `ssh` |
| The granted session uses the user's keys to log in elsewhere | Accepted by the owner ("ssh means full ssh"); the grant text and docs say so, no tool-output warning |
| `/etc/ssh/ssh_config` is a symlink | Constrained: the canonical path is mounted over, never opened for writing |

## 5. Acceptance

Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` (768 passed, 0 failed, 3 ignored at `9bd6846`),
`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Run before the change and after it.

Existing tests that must change (the new grant breaks them):
- tests/grant.rs `grant_table_has_expected_rows` (line 6): the expected tuples (lines 7-79) gain the stand-in column
  and the `ssh` row.
- tests/grant.rs `credential_grant_rows_have_exact_exposure_text` (line 94): add the `ssh` exposes literal.
- tests/grant.rs `grant_validation_rejects_repeats_and_suggests_close_names`: line 119 `("sh", "gh")` becomes
  `("gj", "gh")` ("sh" is now within edit distance 1 of both `gh` and `ssh`, so no single closest match); line 130
  drops `"ssh"` from the list of names that must be unknown (`gpg` and `git` stay).
- tests/grant.rs `render_list_has_every_grant_and_instruction` (line 152): the `ssh` row is `lines[10]`, the
  instruction line stays last.

New tests (each fails on the old code unless marked contract):
- `grant::lookup("ssh")`: the unhide, the stand-in, session only.
- `plan_with_path` with `grants: ["ssh"]` and a fixture home with `.ssh`: `.ssh` is readable (not hidden); without the
  grant `.ssh` is hidden (contract).
- Host-conditional, because the stand-in is an absolute host path and `Host` has no root to redirect it: if
  `/etc/ssh/ssh_config` exists on the test host, its canonical path is in `hidden_paths()` and `plan.wrap()` contains
  the window `["--ro-bind", <empty file>, <that path>]` (as tests/confine.rs:229 asserts for other hides); without the
  grant it is in neither (contract); if the file is absent it is in `skipped_paths()`. State in the report which
  branch ran.
- Same result with `no_default_hides: true`.
- Explicit `unhide` of the stand-in path drops it from `hidden`; `grants = ["ssh"]` plus `hide = ["~/.ssh"]` fails
  with the conflict message; `hide = ["~/.ssh/id_work"]` plans and hides that file.
- `team::parse` with `ssh` in the team-level table is refused with the existing message (contract once the row
  exists; every `[[session]]` in the fixture needs a name or role).
- Real bubblewrap: copy the Host-fixture pattern near tests/confine.rs:4405-4425 (`confine::resolve_bwrap` with
  `require_bwrap()` fallback, `Harness::Generic` or `without_overlays("unsupported")` so CI bubblewrap 0.9.0 does
  not meet `--overlay-src`), not the daemon test at line 2798. The confined `ssh -G example.invalid` exits 0 with
  the grant; do not pass `-F`. A missing `ssh` binary skips; a missing `bwrap` follows `require_bwrap()`
  (`A2AMX_REQUIRE_BWRAP=1` makes it a failure). The test cannot show the negative half or the unhide, because ssh
  reads the runner's real home; the argv-window test above is what kills the "drop the stand-in" mutant.

Mutation check on a scratch copy of the tree: drop the stand-in step, make `team_default_ok` true, run the step
inside `hide_defaults` (so `no_default_hides` skips it), keep `.ssh` hidden when the grant is given; name the failing
test for each; the checkout matches its own diff afterward. Without `/etc/ssh/ssh_config` on the host the
stand-in mutants survive; say so.

Live check, done by the owner after the install (not a developer command): in a session with `grants = ["ssh"]`,
`git push --dry-run` over the ssh remote works.

## 6. Claims verified

| claim | where | verified |
| --- | --- | --- |
| ssh fails on the root-owned config in a user namespace and works with an empty stand-in over `/etc/ssh/ssh_config` | `bwrap --ro-bind / / --dev /dev --unshare-user [--ro-bind EMPTY /etc/ssh/ssh_config] -- ssh -G example.invalid` | probed by the architect (exit 0 with it; `Bad owner or permissions on /etc/ssh/ssh_config.d/20-systemd-ssh-proxy.conf` without) |
| ssh checks ownership only on included files and user config, not the main system file | OpenSSH `readconf.c` | recalled by the consultant, consistent with the probe |
| a hidden non-directory is mounted as an empty file | `mounts()` src/confine.rs:1684-1690, `MountKind::Empty` :1829-1835 | read |
| a grant's `unhides` make a default-hidden path read-only with no change | `apply_grant_unhides` :1409-1466; `~/.ssh` is `DEFAULT_HIDES[0]` :72 | read by the consultant |
| `Grant` literals exist only in src/grant.rs | `grep -rn 'Grant {' src tests` | probed twice |
| grant name edit-distance limit is 1 for names of 3 characters or fewer | src/grant.rs:168 | read by the consultant |
| the dry run is rendered in src/main.rs `dry_run_start`, so reusing `hidden` avoids that file | src/main.rs ~1158-1210 | read by the consultant |

## Review record

(Appended at acceptance.)
