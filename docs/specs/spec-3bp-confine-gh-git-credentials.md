# Spec 3bp: the `gh` and `git-credentials` grants

## 0. Status

**LOCKED** (2026-11-01, one consultant review `m_4939` applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F51). Architect-directed, before 0.2.0. Depends on spec 3bt (accepted).
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at the commit the assignment records (sha and suite count there; the tree is clean at that commit); run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `grant::all`, `grant::lookup`, `grant::render_list` (`tests/grant.rs`), `confine::plan`, `Plan::wrap` argv and `confine::notice`
(`tests/confine.rs`), `team::parse` and the dry run (`tests/team.rs`), `a2amx confine list`.

**Scope.** May edit exactly these files and no others: `src/grant.rs`, `src/confine.rs`, `tests/grant.rs`, `tests/confine.rs`,
`tests/team.rs`, `a2amx.toml.example`, `docs/confinement.md`, `docs/teams.md`. Does not touch: `a2amx.toml`, `CHANGELOG.md` (the
architect edits it), `docs/backlog.md`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, every other file. No new dependency. Leave the diff uncommitted.
Line numbers below are at `5efdbb6`; re-anchor them against the tree you are given.

## 1. Why

A confined session hides `~/.config/gh`, `~/.git-credentials` and `~/.config/git/credentials` and unsets `GH_TOKEN`, `GITHUB_TOKEN` and
`GH_ENTERPRISE_TOKEN` (spec 3bn). A user who wants the agent to push, open pull requests or read CI results must open exactly that, on
purpose, without opening `~/.ssh` or the whole home. Today that is a hand-written `unhide = ["~/.config/gh"]` plus
`keep_env = ["GITHUB_TOKEN"]`; the owner's decision is a named grant that says what it exposes. Both grants are path-only: they need no new
mount mechanism, because a hidden path is lifted exactly as `unhide` lifts it.

Checked on 2026-11-01 in the architect session, which has `unhide = ["~/.config/gh"]`: `gh auth status` answered from `hosts.yml`, and
`git push --dry-run` over https through `gh auth git-credential` reached GitHub. A real push was not tried.

## 2. The rows (`src/grant.rs`)

`Grant` gains one field after `own_config`, and the two rows are appended after `harness-config` (table order is display order; `GRANTS` becomes `[Grant; 10]`):

```rust
/// Home-relative or absolute paths made visible read-only when the grant is given.
pub unhides: &'static [&'static str],
```

All existing rows get `unhides: NO_PATHS`. New rows, both `team_default_ok: false` (credential grants are per session), `sockets: NO_PATHS`, `own_config: false`:

| name | unhides | env (unset unless given) | exposes |
| --- | --- | --- | --- |
| `gh` | `~/.config/gh` | `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` | `your GitHub CLI login (~/.config/gh) and its tokens in the environment: the agent can push, open and merge pull requests and read private repositories as you` |
| `git-credentials` | `~/.git-credentials`, `~/.config/git/credentials` | none | `the plain-text git credential files: every host and password stored there` |

**The three `gh` names stay in `CREDENTIAL_ENV`** (it stays 29 names; the notice default line and `keep_env` for these names are unchanged, so nothing
that spec 3bn shipped breaks and no 3bn test or example changes). The unset rule changes in two places in `Plan::env_unsets` (`src/confine.rs:1426-1454`). (a) The loop over **ungranted** grants (`:1438-1447`) pushes every grant `env` name
without looking at `keep_env`; with the `gh` names in `gh.env` it would unset `GITHUB_TOKEN` for a session that has `keep_env = ["GITHUB_TOKEN"]` and no `gh` grant. That loop skips names that are in
`CREDENTIAL_ENV`: the `CREDENTIAL_ENV` loop alone owns them and applies `keep_env`. (b) The `CREDENTIAL_ENV` loop adds `&& !released.contains(name)`; `released` is already computed there.
The existing test `wrap_keeps_requested_credentials_and_never_unsets_harness_environment` (`tests/confine.rs`, ~1011-1080: no `--unsetenv GITHUB_TOKEN` with `keep_env`) is the guard for (a). The notice (`confine::notice`, `:1849-1853`) must not list a released name as unset, so the
`released` computation (given grants -> the union of their `env`) moves into one function used by both, e.g. `fn granted_env(grants: &[String]) -> BTreeSet<&'static str>`
(extend, do not duplicate). `keep_env = ["GITHUB_TOKEN"]` keeps working exactly as before; the grant is the named way to the same result plus the path.

The invariant test `grant_socket_and_environment_names_are_unique` (`src/confine.rs:1980`) asserts that no grant `env` name is in `CREDENTIAL_ENV`.
It changes to: a grant `env` name may be in `CREDENTIAL_ENV` only when that row's `unhides` is non-empty (a credential grant); duplicates between rows and
the `ALWAYS_UNSET` check stay.

**`lookup`.** The fixed edit distance of 2 makes `ssh`, `gpg` and `git` candidates for `gh` (distances 2, 2, 2), which would steer a user who wanted
ssh toward their GitHub login. The limit becomes 1 when the table name has three characters or fewer (`gh`, `lxd`), otherwise 2. `gx` and `g`
suggest `gh`; `ssh`, `gpg` and `git` suggest nothing; `gitcredentials` suggests `git-credentials`. A suggestion is only a hint in an error and never acts; `sh` now also suggests `gh` and that is accepted. `lx` suggests `lxd`.

`render_list` needs no change: the name column is already 16 wide (`{:<16}`, `src/grant.rs:176`) and `git-credentials` is 15 characters.

## 3. Plan (`src/confine.rs`)

In `plan_with_path` (`:835-860`) a new step `builder.apply_grant_unhides(&request.grants)` runs after `hide_containers` and before `release_operator_writes`.
For each given grant, for each path in its `unhides`: expand it with `user_path(path, &home)` and `canonical_if_exists` (the same pair `hide_or_skip` used for the default hide,
so the canonical forms match, including a symlinked `~/.config/gh`); when it exists, remove it from `hidden` and `default_hidden`, add it to `readonly` (`add_unique`)
and record `(path, grant name)` in a new `PlanBuilder` field. That is the effect of an operator `unhide` without its refusals (these paths are not the home, the state or runtime
directory, or a container socket). A path that does not exist is left in `skipped` as today and is not an `Unresolved` (spec 3bl: grant paths are not operator entries).
**Alias check.** The canonical form of a grant path can differ from its name: a symlink planted earlier in a session with `write = ["~/.config"]` can turn `~/.config/gh` into a link to `~/.ssh`,
and `hide_or_skip` and the grant would both canonicalize to `~/.ssh`. Before lifting, for each canonical grant path `P`, find every other path `H` in `hidden` that is not itself a canonical path of a given grant: when `P` equals `H`, lies
inside `H`, or contains `H`, fail the plan with `confinement grant "{grant}" path {name} resolves to the hidden path {H}`. This covers the other default hides, the container sockets and the mandatory hides, since all of them are in `hidden`
at this point. A dotfile manager's symlink into a directory that is not hidden still works. The `unhides` come from the static table, not from the client; grant names are validated as before (`grant::validate`, `:843`). Read-only is deliberate: `gh` reads
`hosts.yml`; a command that rewrites it (`gh auth refresh`, `auth switch`, `config set`, gh's one-time migration of an old-format `hosts.yml`) fails with a read-only error.
An exact `write = ["~/.config/gh"]` next to the grant makes it writable through the existing rule in `finish` (`:1269`, explicit writes leave `readonly`); the docs say so.
Under `no_default_hides` the path is not hidden and the grant only adds a harmless read-only bind.

**Contradiction.** An operator `hide` of the exact path a given grant opens re-adds it to `hidden` in `apply_operator_hides`; `mounts()` would then let the `readonly`
entry silently win (`:1389-1397`). In `finish`, next to the `harness_config` checks (`:1271-1284`), for each recorded `(path, grant)` still in `hidden`:
`bail!("confinement grant \"{grant}\" conflicts with the hide of {path}")` (same wording as the harness-config grant). An operator `unhide` of the same path is
redundant, not a conflict. Only the exact path is checked; a `hide` of an ancestor such as `~/.config` leaves the more specific read-only bind in place, as an `unhide` does today.

The notice's `Granted:` line already names the grants (`:1844`). The container-engine sentence is unaffected (only grants with `sockets` count, `:1864-1870`).

## 4. Docs

`docs/confinement.md`: the grant table (`:60-69`) gains the two rows; the paragraph after it says that `gh` and `git-credentials` are credential grants. In
"Credentials a tool needs" (`:277-287`), `grants = ["gh"]` replaces the hand-written `unhide = ["~/.config/gh"]` example for a session that pushes or opens pull requests with `gh`, and the
`keep_env = ["GITHUB_TOKEN"]` example (`:314`) becomes `keep_env = ["NPM_TOKEN"] # for a session that publishes with npm`, with one sentence that the `gh` grant also returns
the three GitHub names. Recipe content, each item stated as it is:
- `git push` over https needs the credential helper in the user's `~/.gitconfig` (`gh auth setup-git`); that file is readable but read-only inside, so run `gh auth setup-git` outside first,
  and run `gh` once outside before the first confined use (a one-time `hosts.yml` migration would otherwise fail on the read-only mount).
- `git-credentials` only for a user who keeps a plain-text store (with the store helper, git's write-back to the file fails on the read-only mount, likely non-fatal).
- A credential helper that is a Windows program reached through WSL interop does not work (`/run/WSL` is hidden).
- What the read-only mount breaks, and that an exact `write` entry for the path lifts it.
- SSH pushes need an ssh grant (backlog F53, not built); a `gh` token kept in the system keyring is unreachable (`~/.local/share/keyrings` and the session bus are hidden).
- Verification statement (it replaces the sentence "The maintainers did not test any `gh` command under confinement." at `docs/confinement.md:287`, which would otherwise contradict the new text): the maintainers read `gh`'s token from `hosts.yml` and ran `gh auth status` and `git push --dry-run` with the path unhidden read-only (2026-11-01); a real push, `gh pr create` and the
  recipe in a fresh login are **untested**; the tests use fixtures. Say so.

`docs/teams.md` (`:115-118`) and `a2amx.toml.example` (`:52`, near the `grants = ["docker"]` line): one example line `grants = ["gh"]` for a session that pushes with `gh`; the `keep_env = ["GITHUB_TOKEN"]` example at `a2amx.toml.example:55` becomes `NPM_TOKEN` like the docs; every TOML fence still parses.

## 5. Tests

`tests/grant.rs`: (1) the table has ten names in order, `gh` and `git-credentials` last, with the `unhides`, `env`, `exposes` above (literal expectations; the existing tuple table gains an `unhides` element);
(2) `render_list` has ten rows and each row is the name, then at least one space, then the scope word (the existing `starts_with(name)` also passes on `git-credentialssession only`), and the
`gh` line equals a literal: `gh` + 14 spaces + `session only   ` + the `exposes` text; (3) `lookup("ssh")`, `lookup("gpg")` and `lookup("git")` do not mention `closest`, `lookup("gx")` says `closest match: gh`,
`lookup("gitcredentials")` says `closest match: git-credentials`; the existing close-typo cases still pass.
`src/confine.rs` unit test: every `unhides` path of every row appears in `DEFAULT_HIDES` or `DEFAULT_HIDES_EXTRA` (the two lists are one fact written twice); the invariant test change of section 2.
`tests/confine.rs` (fixtures create the files first): (4a) `grants: ["gh"]` with `~/.config/gh/hosts.yml`: `plan.readonly` contains the canonical `~/.config/gh`, `plan.protected` does not, `Plan::wrap` has `--ro-bind P P` and
no `--tmpfs P`, `hidden_paths()` lacks it; without the grant it is hidden; (4b) with `write: ["~/.config"]` also given, `plan.protected` contains it and `--ro-bind P P` comes after the `--bind` of `~/.config` (dropping only the hide would pass 4a, not this);
(4c) `write: ["~/.config/gh"]` with the grant: the path is writable and not read-only; (5) with `grants: ["gh"]` the fixture `~/.ssh`, `~/.git-credentials` and `~/.config/git/credentials` stay hidden and `~/.config` is not lifted
(abuse case: a grant opens only its own paths); (5b) `grants: ["gh"]` with `hide: ["~/.config/gh"]` fails with `confinement grant "gh" conflicts with the hide of`; (6) `git-credentials` lifts both credential files that exist and leaves `~/.config/gh` hidden;
(7) with `gh` given there is no `--unsetenv` for `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` and the other 26 names are still unset; without it all 29 are unset (contract test: passes on the old code); `keep_env: ["GITHUB_TOKEN"]` is still accepted;
(7b) the notice with `gh` given lists 26 unset names and no `GH_TOKEN`; without it the line equals the existing 29-name literal (`tests/confine.rs:1140`, unchanged); (8) a granted path that does not exist is not an `Unresolved` and does not fail the plan; (8b) alias abuse case: `~/.config/gh` is a symlink to the fixture `~/.ssh`, `grants: ["gh"]` fails with the `resolves to the hidden path` error (without the check `~/.ssh` would be lifted), and a symlink to a directory that is not hidden is accepted; (8c) `keep_env: ["GITHUB_TOKEN"]` without the grant leaves no `--unsetenv GITHUB_TOKEN` (the existing test above, named as the guard); a `hide` and a `write` of the same exact path together with the grant fail with the conflict error.
`tests/team.rs`: (9) `grants = ["gh"]` in a session parses and the dry run prints `  grants: gh` (pattern of `:1604-1627`); in the team table it is refused with the session-only fix (assert a substring: the message embeds the long `exposes` text; pattern of `:2872`).

## 6. Out of scope

A dedicated write mode for `gh` (the exact `write` entry is the way), the system keyring, `ssh` and `gpg` (F53, F54), the harness grants (F52), `netrc`, moving the three names out of `CREDENTIAL_ENV`, the owner's `a2amx.toml`
(after this lands the architect entry can use `grants = ["gh"]`; the owner edits it).

## 7. Trust boundaries

Credential files are exposed on purpose by a per-session grant that names them. The grant list is validated in `plan()` against the closed table (`grant::validate`, the one gate every entry point passes through),
and a team-level credential grant is refused at parse (`team_default_ok: false`). Paths come from the static table, never from the client. Attack paths and dispositions: a grant opening more than its rows (constrained: exact table paths only; abuse test 5);
a team-wide credential grant (removed: refused at parse; test 9); a hide that the read-only bind would silently override (constrained: refused loudly; test 5b); a symlink planted at a granted path that aliases another hidden path (constrained: the alias check, test 8b); the agent acting as the user on GitHub through the token (monitored: that is
the grant's purpose, stated in `exposes` and the docs; the network is open by design).

## 8. Placement

The paths and environment names a grant opens are data about the grant and sit in `grant.rs`; `confine.rs` consumes them as it does sockets and env. The released-environment computation is one function in `confine.rs` used by the launch
argv and the notice.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent`: rerun once, else record as pre-existing. Contract tests (pass on the old code): the 29-name default notice, test 7 without the grant,
`keep_env` for `GITHUB_TOKEN`. Mutants for the tester (on a scratch copy; its tree matches its diff afterward): (1) `gh` unhides `~/.config` instead of `~/.config/gh`; (1b) the grant removes the hide but does not add `readonly` (killed by 4b);
(1c) the `lookup` limit is 2 for every name; (2) a grant lifts every default hide; (3) the `gh` env names stay unset when granted (killed by 7); (3b) the notice still lists released names (7b); (4) a granted-but-missing path fails the plan;
(5) `git-credentials` drops the second file; (6) `gh` becomes `team_default_ok`; (7) the conflict check in `finish` is removed (5b); (8) `released` ignores the given grants; (9) the ungranted-grant loop in `env_unsets` ignores `CREDENTIAL_ENV` (killed by 8c); (10) the alias check is removed (killed by 8b). Mutant 1b is also killed by 4a.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `~/.config/gh`, `~/.git-credentials`, `~/.config/git/credentials` are default hides | `src/confine.rs` `DEFAULT_HIDES` (`:83-85`) | read 2026-11-01 |
| An operator unhide removes the path from `hidden`, `default_hidden`, `container_hidden` and adds it to `readonly` | `src/confine.rs:1250-1261` | read |
| `mounts()` lets a later `readonly` entry replace a `hidden` one at the same path | `src/confine.rs:1381-1397` | read |
| `finish` removes explicit writes from `readonly` and already refuses harness-config vs hide/unhide | `src/confine.rs:1269-1284` | read |
| `GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN` are in `CREDENTIAL_ENV`, 29 names | `src/confine.rs:39-69` | read |
| `env_unsets` computes `released` and does not apply it to the `CREDENTIAL_ENV` loop | `src/confine.rs:1426-1454` | read |
| The notice lists `CREDENTIAL_ENV` minus `keep_env` and does not list grant env names | `src/confine.rs:1849-1860` | read |
| `render_list` name column is `{:<16}`; `lookup` uses limit 2; `ssh`, `gpg`, `git` are at distance 2 from `gh` | `src/grant.rs:176`, `:137` | read; distances by hand (sub + delete each) |
| No test or doc outside the scoped files spells the grant count or the `render_list` width | `rg 'grant::all\|closest match\|render_list' tests src` | probed 2026-11-01: only `tests/grant.rs` and `tests/confine.rs:3110` |
| `docs/cli.md` and `src/main.rs` do not mention grant names | `rg 'grants\|harness-config' docs/cli.md`; `rg 'confine-grant' src/cli.rs src/main.rs` | probed (cli.md: no hit) |
| `gh` reads its token from `hosts.yml` under the read-only unhide and `git push` over https via the gh credential helper authenticates | architect session, `gh auth status`, `git push --dry-run` | probed 2026-11-01 |
