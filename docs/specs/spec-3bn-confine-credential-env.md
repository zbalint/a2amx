# Spec 3bn: Credential environment variables are unset in a confined session, `keep_env` returns them

## 0. Status

**LOCKED** (2026-10-31, one consultant review `m_4752` applied, no second pass; context `a2amx-confine-grants-2026-10-31`; backlog F49, first part). Architect-directed, before 0.2.0.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `adbe8a6` (after spec 3bm); full suite 719 passed, 0 failed, 3 ignored; run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::plan` / `Plan::wrap` / `confine::notice` (`tests/confine.rs`), `team::parse` (`tests/team.rs`),
`a2amx team up --dry-run` (`tests/team.rs`), the wire round trip (`tests/wire.rs`).

**Scope.** May edit exactly these files and no others: `src/confine.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`,
`tests/confine.rs`, `tests/team.rs`, `tests/wire.rs`, `a2amx.toml.example`, `docs/confinement.md`, `docs/cli.md`,
`docs/teams.md`, `docs/harnesses.md`. Does not touch: `a2amx.toml`, `CHANGELOG.md` (the architect edits it), `src/grant.rs`, `src/daemon.rs`,
`src/wire.rs`, every other file. No new dependency. Leave the diff uncommitted. Re-anchor every line number against the tree
after spec 3bm; the anchors below are from before it.

## 1. Why

A confined agent inherits the user's environment. A token in `GITHUB_TOKEN`, `AWS_SECRET_ACCESS_KEY` or `NPM_TOKEN` is as
exposed as a file in `~/.aws`, which the sandbox already hides, so hiding the files while leaving the variables is a gap in a
net that is supposed to stop an agent from using what the user did not hand over. The owner's rule: the defaults are strict
and safe, and the user opens anything on purpose. The harness's own LLM keys, the a2amx variables and the harness internals
are never touched: the session cannot run without them.

## 2. The list and the setting (`src/confine.rs`)

A constant `CREDENTIAL_ENV: [&str; 29]` next to `ALWAYS_UNSET`, a fixed list, exactly these names and no pattern matching:

`GH_TOKEN`, `GITHUB_TOKEN`, `GH_ENTERPRISE_TOKEN`, `GITLAB_TOKEN`, `GITLAB_PRIVATE_TOKEN`, `CI_JOB_TOKEN`, `BITBUCKET_TOKEN`, `NPM_TOKEN`,
`NODE_AUTH_TOKEN`, `PYPI_TOKEN`, `TWINE_PASSWORD`, `CARGO_REGISTRY_TOKEN`, `CARGO_REGISTRIES_CRATES_IO_TOKEN`,
`AZURE_CLIENT_SECRET`, `ARM_CLIENT_SECRET`, `VAULT_TOKEN`, `HF_TOKEN`, `HUGGING_FACE_HUB_TOKEN`, `SONAR_TOKEN`,
`SENTRY_AUTH_TOKEN`, `DIGITALOCEAN_ACCESS_TOKEN`, `HCLOUD_TOKEN`, `CLOUDFLARE_API_TOKEN`, `NETLIFY_AUTH_TOKEN`,
`VERCEL_TOKEN`, `FLY_API_TOKEN`, `HEROKU_API_KEY`, `PGPASSWORD`, `MYSQL_PWD`.

It deliberately leaves out `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_SESSION_TOKEN` and `GOOGLE_APPLICATION_CREDENTIALS`: they are the login of the harness itself on Bedrock and Vertex (Claude Code and OMP run as the confined child), and `GOOGLE_APPLICATION_CREDENTIALS` is only a path. `~/.aws` and `~/.config/gcloud` stay hidden by default; a user who does not want these variables in the session unsets them in their own shell. It contains no LLM provider key (`ANTHROPIC_*`, `OPENAI_*`, `GEMINI_*`, `GOOGLE_API_KEY`, `OPENROUTER_*` and the like), no
`A2AMX_*` name, and none of the names in `ALWAYS_UNSET` or the container grants' `env`. A unit test in `src/confine.rs` pins
those three exclusions.

`ConfineRequest` (also the wire type) gains `#[serde(default)] pub keep_env: Vec<String>`. `check_request_limits`:
`check_entry_count(&request.keep_env, "keep_env")`; each name must be non-empty, at most 128 bytes, and consist of
`[A-Za-z0-9_]`; each must be in `CREDENTIAL_ENV`, otherwise
`keep_env {name:?} is not unset by default, so there is nothing to keep` (a typo or an LLM key is a loud error, not a silent
no-op); when the name is the `env` of a grant row the message adds `; it returns with the {grant} grant`. Membership is the one
name check (no separate character or length rule; the byte bound of `validate_confine_list` applies at parse). Duplicates are an
error like grants.

The rule: unset = `ALWAYS_UNSET` ∪ the `env` of every grant not given ∪ `CREDENTIAL_ENV`, minus `keep_env`, minus the `env` of
every grant given, deduplicated. Unsetting an absent name changes nothing (bwrap `--unsetenv` is `unsetenv(3)`), so there is no
existence check. The invariant test of spec 3bm (no name in two rows or in `ALWAYS_UNSET`) extends to `CREDENTIAL_ENV`.

`Plan` gains `pub keep_env: Vec<String>` (sorted), as `grants`. The hand-built `Plan` in `mod tests` gets
`keep_env: Vec::new()`.

## 3. Configuration and command line

- `src/team.rs`: `ConfineTable` gains `keep_env: Vec<String>`; the key is `keep_env` in `[confine]` and
  `[session.confine]`; lists add and duplicates drop, as `write`; `keep_env` is per session only, like a credential grant: a team-level `keep_env` is refused with
  `[confine] keep_env: a credential variable is returned per session, not team-wide; put keep_env in [session.confine]`. `own_has_options` counts it, so `keep_env` without `enabled = true` gets the same
  error as the other options. The merged request carries it. `validate_confine_list` bounds its count; `validate_confine_syntax`
  (spec 3bl) does not check it, a name is not a path.
- `src/cli.rs`: `--confine-keep-env NAME`, repeatable, on `new`, with the same "requires --confine" rule as the other
  options (`src/main.rs`, the `has_confine_options` check near `:423-429` includes it). `src/main.rs`: the `new` path builds
  `ConfineRequest` with it; `NewOptions` already carries the whole request and gets no new field; `resolved_confine_request`
  copies it unchanged.
- Older clients: an absent `keep_env` deserializes to empty, so a client from before this spec gets the strict default.

## 4. What the user and the agent see

- Dry run: after the `grants:` line, `  keep_env: A, B` when non-empty (nothing when empty).
- Notice: after the `Granted:` line, `Credential environment variables are unset: {unset}; kept: {kept}.` where `{unset}` are the
  list names not in `keep_env`, in list order, joined by `, ` (the clause `; kept: ...` only when `keep_env` is non-empty).
  The plan never sees the environment, so this is the list minus the kept names, not a count of variables that were present. The
  agent can name the variable it needs when it asks the user.
- `docs/confinement.md`: the list, the rule "never LLM keys, `A2AMX_*` or harness internals", `keep_env` with an example
  (`keep_env = ["GITHUB_TOKEN"]` for a session that pushes with `gh`), the residual that a value read from a file the user
  writable-binds is not an environment variable, the four names left out and why, and that a daemon upgrade alone changes
  the environment of every confined session. `docs/harnesses.md`: Bedrock and Vertex logins are not touched. `docs/cli.md`: the
  flag. `docs/teams.md` and `a2amx.toml.example`: the key.

## 5. Tests

`tests/confine.rs`: (1) with the default request, `Plan::wrap` output contains `--unsetenv NAME` for every name of the
section 2 list (spell the list out as a literal in the test); (2) with `keep_env: ["GITHUB_TOKEN"]` it contains the unset for
every other name and none for `GITHUB_TOKEN`; (3) `ANTHROPIC_API_KEY`, `A2AMX_TOKEN`, `A2AMX_ADDR` and `OPENAI_API_KEY` are
never unset, with and without `keep_env`; (4) `keep_env: ["ANTHROPIC_API_KEY"]` is refused with the "nothing to keep" error and `keep_env: ["DOCKER_HOST"]` with the
"returns with the docker grant" addition (after spec 3bm)
(abuse case: it cannot be used to widen anything, and a typo is loud); duplicate and 65-entry `keep_env` are refused; (5) the
notice has `Credential environment variables are unset: GH_TOKEN, GITHUB_TOKEN, ...` (all 29 names, spelled as a literal in the test)
by default and the same without `GITHUB_TOKEN` and ending `; kept: GITHUB_TOKEN.` with the keep; (6) a unit test in `src/confine.rs` pins that the
list contains none of the excluded prefixes and none of `ALWAYS_UNSET`.
`tests/team.rs`: (7) `keep_env` in `[session.confine]` drops duplicates; in `[confine]` it is refused with the team-wide error; `keep_env` without
`enabled = true` fails like the other options; an unknown name fails at plan time (dry run) with the error; (8) the dry-run
line.
`tests/wire.rs`: (9) `ConfineRequest` round-trips `keep_env` and an old JSON without it deserializes to empty.

## 6. Out of scope

Credential grants that return names (`gh`, `ssh`, a container runtime's token); pattern matching on names; `ALWAYS_UNSET`
changes; protecting harness logins that arrive by variable (the harness is itself the confined child and keeps them); WSL; network.

## 7. Trust boundaries

`keep_env` comes from the client over the wire and from the team file (session level only). It can only return a name that the closed list
already unsets, so it never exposes anything the user did not already have in the environment of their own process, and a
name outside the list is refused in `check_request_limits`, the one gate inside `plan()`. Abuse-case test: test 4.

## 8. Placement

The list and its rule belong to `confine.rs`, which owns what the sandbox removes from the child's environment; the key is a
confinement setting and sits in the existing table.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent`: rerun once and
record as pre-existing. Mutants for the tester: (1) drop one name from the list; (2) `keep_env` ignored; (3) add
`ANTHROPIC_API_KEY` to the list; (4) accept a name outside the list; (5) notice count off by one; (6) team-level `keep_env`
not merged; (7) `keep_env` without `enabled` accepted.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `env_unsets` yields `ALWAYS_UNSET` plus the container names and `Plan::wrap` emits `--unsetenv NAME` | `src/confine.rs` (`env_unsets`, `wrap`) | read |
| `ConfineTable` has `enabled`, `write`, `hide`, `unhide`, `grants`, `default_hides` and `own_has_options` lists them | `src/team.rs:55-68`, `merge_confine` | read |
| `--confine-no-default-hides` carried by `new` into `ConfineRequest` | `src/cli.rs:65`, `src/main.rs:403-476` | read |
| The wire request tolerates missing fields | `ConfineRequest` fields are `#[serde(default)]` | read |
| Existing tests assert `--unsetenv` for names; none counts or enumerates them | `tests/confine.rs:836`, `:855` | grep (consultant `m_4752`) |
| For Claude and OMP the harness argv itself runs under `plan.wrap`, so the harness is the confined child | `src/confine.rs:1455-1456` | read (consultant) |
| `NewOptions` carries only `confine: Option<ConfineRequest>` after spec 3bk | `src/main.rs:521-539` | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer and tester on the
OMP harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed to either).
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | The list unset four names that are the harness's own login on Bedrock and Vertex (the harness is itself the confined child), and the spec wrongly said the harness process is unaffected; the notice gave only a count the agent could not act on; team-level `keep_env` would hand a credential to every session; wrong claims about `NewOptions` and `env --unsetenv`; the `has_confine_options` anchor was missing | The four names left out and documented; the notice lists the names; `keep_env` per session only; wording and anchors fixed before the lock |
| Tester pass | Tester (`gpt-5.6-luna`) | Removing `keep_env` from the `--confine-*` requires `--confine` guard survived all tests | Developer added `new_confine_options_require_confine_before_daemon_connection`; mutant killed |
