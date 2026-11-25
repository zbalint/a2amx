# Spec 3hb: two environment variables that tell a session who it is

## 0. Status

LOCKED (2026-11-24) after the pre-lock gate against `develop` at 3aaec03 and two consultant reviews (three blocking findings in the first, two amendments in the second, all applied; see section 8). Owner priority: urgent, ahead of 3gy. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context `a2amx-session-identity-env`. Public test seams: the daemon through the control protocol (`tests/broker/`; the model is the supplied-token test at `tests/broker/sessions_and_naming.rs:35-70`), the real daemon process with a cleared environment (`common::spawn_daemon_process`, `tests/common/mod.rs:282`), and the fake-Codex daemon case (`tests/codex/`).

Scope (edit): `src/daemon/launch.rs` (`create_session`, `prepare_confined_and_codex`, `prepare_codex`, `LaunchPrep`/`CodexPrep`, and one new function next to `prepare_session_identity`); tests: `tests/broker/sessions_and_naming.rs` (new tests), `tests/codex/main.rs` (`Case::launch`, `:157`, and one line in `FAKE_CODEX`, `:24-28`, only), `tests/common/mod.rs` (one new helper, step 3b), `tests/codex/app_server_launch.rs` (`:89-90`, only); docs: `docs/architecture.md` (the "Identity from the token" bullet, `:474-476`), `docs/harnesses.md` (the "Harness profiles" paragraph that says what `a2amx mcp` reads from its environment, `:28-31`). Architect in the accepting commit: `CHANGELOG.md` and the review record. Does not touch: `src/commands/*`, `src/session/spawn.rs`, `src/agy.rs`, `src/codex.rs`, `src/confine*`, `src/wire.rs`, `src/cli.rs`, `src/mcp.rs`, every other part of `tests/common`, every other test and spec, `AGENTS.md`, `Cargo.toml`. Do not commit, stage or merge: leave the diff uncommitted.

Owner request: hooks and scripts that run inside a session need to know which session they run in and what role it has. The owner said it is enough for the spec to say a2amx adds two new environment variables, because they are useful for other purposes too. Owner decisions (2026-11-24): `A2AMX_SESSION` is the full `name@host` address; the Codex child may see its role in its environment (spec 3h's `unset` rule is reversed for it, see section 3 step 4), because the agent already has its role in its instructions and `list_agents` shows every team member's address and role.

User-visible, additive change to the environment a session sees: no existing variable changes. One changelog line under Unreleased.

## 1. Why

A session sees `A2AMX_ADDR` and `A2AMX_TOKEN` and nothing that says which session it is or what role its operator gave it. A hook, a status-line command or a user script has to ask the daemon with the token, or guess. The daemon knows both at spawn.

Rejected while designing: a model, team or harness variable (the model comes from the harness payload; team and harness derive from the address); a config switch to turn the variables off (labels with no authority); setting them in the client (a raw wire client could forge them; the daemon is where the token and address are already set); leaving `A2AMX_ROLE` unset for an unroled session (the child inherits the daemon's own environment, so an unset variable would show the role of whatever session the daemon was started from, see the first review's finding B3); stripping the daemon's own environment at startup (touches process-wide state and the startup code for no gain over setting an empty value).

## 2. Contract

For every session the daemon spawns, the child's environment carries both variables, set by the daemon:

- `A2AMX_SESSION`: the address peers use, `messaging::address(name, id, host_name)`: `<name>@<host>`, or `<id>@<host>` (for example `s3@host-a`) when unnamed.
- `A2AMX_ROLE`: the session's validated `role` field (`messaging::validate_role`, spec 3h), or the empty string when the session has no role. A reader treats empty as "no role". The value never comes from the launch request's `env` and never from the daemon's own environment.

Neither is secret or derived from a token. Every harness gets them: Claude, OMP, Codex (its TUI and its app-server), agy and generic, confined sessions included. A session already running keeps its old environment until restarted.

## 3. Changes (one step at a time; suite green after each)

0. Baseline: record `cargo test --no-fail-fast` counts.
1. One push site, before the Codex app-server starts. `codex::start` runs inside `prepare_codex` (`src/daemon/launch.rs:585`), which `prepare_confined_and_codex` (`:307`) calls, so a push after that returns reaches only the Codex TUI. In `create_session` (`:360`), between `prepare_session_identity` (`:392`) and `prepare_confined_and_codex` (`:393`): (a) read the Codex role carrier (the last `A2AMX_ROLE` pair of `env`, as `prepare_codex` does at `:574-577`) and remove every `A2AMX_ROLE` and `A2AMX_SESSION` pair from `env`; (b) push `A2AMX_SESSION` and `A2AMX_ROLE` as the contract says. Add `role` to the `Request::NewSession` destructure (the `..` hides it today). Remove the carrier read and strip from `prepare_codex` (`:574-578`) and hand it the carrier value read in (a), through `LaunchPrep`/`CodexPrep` (their `env` is still `&mut`, the read and removal of `A2AMX_NO_AUTHORIZE_PEERS` at `:569-572` stays). Codex keeps getting its role instruction from the carrier exactly as today; the environment variable comes from the validated `role` field. Put (a) and (b) in one new function next to `prepare_session_identity`, taking `&self`, the session id, the name, the role and `&mut env` (five parameters is the limit; a small struct if a sixth appears), returning the carrier value; the developer names it.
2. The real client sends the carrier and the `role` field with the same value (`src/commands/launch.rs:446-449` and the request's `role`). A raw wire client that sends only the carrier gets the instruction and an empty `A2AMX_ROLE`; one that sends only the field gets `A2AMX_ROLE` and no instruction.
3. `tests/codex/main.rs`, `Case::launch` (`:157`): send `role: Some(value)` where `value` is the last `A2AMX_ROLE` pair of the case's env (the carrier is still passed through `env`), and `None` when there is none. This is the minimum for the daemon's field-based variable to be set in those cases. In `FAKE_CODEX` (`:24-28`) add one line recording `${A2AMX_SESSION:-unset}` per role, like the three lines there, so a test can see the address in the app-server and the TUI; nothing else in that file changes.
3b. `tests/common/mod.rs`: add `spawn_daemon_process_with_env(state, home, runtime, extra: &[(&str, &str)])` holding the body of `spawn_daemon_process` plus `.envs(extra)` after the `env_clear`, and make `spawn_daemon_process` call it with `&[]`. Its seven callers (`confine_show`, `omp_wiring`, `explain`, four in `auto_confine`) do not change. This is the seam for acceptance test 4.
4. `tests/codex/app_server_launch.rs:89-90`: the three roled cases expect `architect`, `consultant`, `architect` for `server.role` and `tui.role`, and `agent-impl@host-a` for the new `server.session` and `tui.session` records (session name and host, `tests/codex/main.rs:115,213`) (the fake Codex prints `${A2AMX_ROLE:-unset}`, so an empty value reads `unset`). `:141-148` (no role, client holds `A2AMX_ROLE=leak`) stays `unset`. Spec 3h made the role reach "only the operator instructions"; that guarded against a role in the user's shell reaching a session, which still cannot happen (the daemon sets the value itself), and against leaking the carrier, which is now removed and replaced by the daemon's own value.
5. Docs: `docs/architecture.md:474-476` says the child's environment also holds `A2AMX_SESSION` and `A2AMX_ROLE`, what each is, that an unroled session gets an empty role, that both are labels and not credentials, and that a running session keeps its old environment. `docs/harnesses.md:28-31`: one sentence after the `a2amx mcp` environment sentence naming the two variables as part of every session's environment, and that whether a harness passes its environment on to its hooks is the harness's own behavior (Claude Code and OMP do; Codex and agy are unverified).
6. Run `cs check` on `src/daemon/launch.rs` and report each new finding.

## 4. Claims verified against the tree (drafting and consultant review, 2026-11-24, `develop` at 3aaec03)

| Claim | Where | How |
| --- | --- | --- |
| `codex::start` (the app-server spawn, `env_clear` then `envs(env)`) runs inside `prepare_codex`, called by `prepare_confined_and_codex` | `src/daemon/launch.rs:585-597`, `:307-344`; `src/codex.rs:501-503` | read |
| `prepare_session_identity` has one caller, before `prepare_confined_and_codex` | `src/daemon/launch.rs:392-393`; `rg -n prepare_session_identity src tests` | grepped |
| `prepare_codex` reads the last `A2AMX_ROLE` pair and strips every one | `src/daemon/launch.rs:574-578` | read |
| The client strips an inherited `A2AMX_ROLE` and pushes the carrier only for Codex | `src/commands/launch.rs:435`, `:446-449` | read |
| The spawn builds a map from the env pairs, a later pair wins; the session child also inherits the daemon's own environment (the pairs only add; the shortcut comment says so) | `src/session/spawn.rs:241-246`; supplied-token test `tests/broker/sessions_and_naming.rs:35-70` | read; inheritance by comment and by `AGENTS.md`'s statement that daemon-environment-dependent tests run a real daemon with a cleared environment |
| The background daemon start passes the client's whole environment through; the in-process test daemon is the test process | `src/commands/daemon.rs:85-110` (no `env_clear`/`env_remove`); `tests/common/mod.rs:339`; `common::spawn_daemon_process` clears it, `:282-289` | read |
| `Case::launch` sends `role: None`; the role reaches the fake Codex only as the carrier | `tests/codex/main.rs:157`; `tests/codex/app_server_launch.rs:57,66,73-74` | read |
| The fake Codex records `${A2AMX_ROLE:-unset}` for server and TUI | `tests/codex/main.rs:27` | read |
| The daemon holds the host name and `messaging::address` builds `name@host` or `id@host` | `src/daemon.rs:76`, `:208`; `src/messaging.rs:467-474` | read |
| Confinement unsets neither name | `src/confine.rs:45-85`; `rg -n A2AMX src/confine src/grant.rs` finds nothing; `confine::launch_argv` changes only argv, `into_session_spec` passes `env` through | read, grepped |
| Tests pinning a Codex child's role as `unset`: `tests/codex/app_server_launch.rs:89-90` (roled), `:141-148` (no role) | `rg -n "\.role" tests/codex` | grepped |
| No other test or doc reads either name from a launched session; the agy plugin tests set `A2AMX_SESSION` themselves | `tests/agy_plugin.rs:100,181,583`; `rg -n "A2AMX_SESSION\|A2AMX_ROLE" src tests docs README.md` | grepped |
| `spawn_piped_command` (agy) strips `A2AMX_ADDR`, `A2AMX_TOKEN`, `A2AMX_SESSION` from the commands `agy::install` runs; it is a client-side path, not the daemon | `src/agy.rs:351-366`, `:466`; `tests/agy_plugin.rs:583-593` | read; callers not fully traced |

## 5. Acceptance

- `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; counts before and after, also run `A2AMX_ROLE=developer A2AMX_SESSION=x@y cargo test --no-fail-fast`, which simulates a roled session for the in-process test daemon: the suite must not depend on the ambient values. `git status --short` lists only the files of the scope list.
- New tests in `tests/broker/sessions_and_naming.rs`, written first (they fail on the old code). Each launches a `sh -c` session, harness generic, that writes `${A2AMX_SESSION-unset}` and `${A2AMX_ROLE-unset}` to a file, like the supplied-token test; expected values are literals for host `host-a`, never recomputed with `messaging::address`:
  1. a named session with a role: `<name>@host-a` and the role;
  2. an unnamed session, created first in a fresh daemon or its id read from the `Created` reply: `<id>@host-a` and the empty string (the variable is set and empty, not `unset`);
  3. the request's `env` carries `A2AMX_SESSION=leak` and `A2AMX_ROLE=leak`, the session has another name and no role: the child prints its real address and the empty string;
  4. a session whose daemon was started with `A2AMX_ROLE=ambient` in its environment (`common::spawn_daemon_process`-style, `spawn_daemon_process_with_env` with `A2AMX_ROLE=ambient`, then `Client::connect(state)` as `tests/explain.rs:49-53` does) and that has no role: the child prints the empty string, not `ambient`. That daemon has no `--host-name`, so this test asserts only the role.
- `tests/codex`: the three roled cases expect the role in server and TUI (step 4); the inheritance case stays `unset`. The roled cases go red on the old code (role `unset`); the inheritance case passes on old and new code (a contract test).
- Confined sessions: nothing changes the confinement plan or unsets either name (section 4). If a bwrap-backed seam can launch a `sh` probe in this environment, one test shows both variables inside; if not, the developer says so and does not claim a test.
- Tester pass, mutants in a scratch snapshot per the tester skill, each with its killing test named: remove the carrier read (the Codex instruction is lost: killed by the `developer_instructions` assertion of `codex_role_reaches_only_the_operator_instructions`); push the labels after `prepare_confined_and_codex` (the app-server gets neither: the roled Codex cases for the role, the new `server.session` record for the address); no push of an empty role when absent (broker test 2/3/4: only these can kill the "unset instead of empty" and the "inherited value leaks" mutants, since the fake Codex's `:-unset` hides empty); push the role as `"None"` when absent; use the name when unnamed or the id when named; drop `host_name`; push `A2AMX_SESSION` only for some harnesses (broker tests cover generic, the Codex records cover Codex; an agy, Claude or OMP-only skip is an expected survivor with a single unbranched push site); remove the pre-removal (an equivalent mutant: the later pair wins in the spawn map and in `Command::envs`; the carrier read must stay ahead of the push either way). Equivalent mutants listed with a reason.

## 6. Out of scope

- Any reader of the variables; hooks and scripts live outside this repository.
- `A2AMX_ROLE` reaching the commands `agy::install` runs: `spawn_piped_command` strips three names and not the role, so a roled session that runs the agy install passes its role label to those commands. A label, not a secret; left as is. Adding it to that strip list is a follow-up.
- The inherited `A2AMX_NO_AUTHORIZE_PEERS` hole noted in spec 3h, and any other daemon-environment variable a session inherits.
- Exposing the values on the wire, in `list` or in `explain` (address and role are already there).
- `docs/delivery.md:419-420` and `:615` (they say how the hook and the OMP extension reach the daemon, not what a session's environment holds).
- Retiring the Codex role carrier in favor of the `role` field: it needs the client and the 3h tests; a separate spec if wanted.
- Restarting or updating sessions that are already running.

## 7. Owner decisions

1. DECIDED (2026-11-24): `A2AMX_SESSION` is the full `name@host` address.
2. DECIDED (2026-11-24): the Codex child's role is visible in its environment; spec 3h's acceptance 5 (`unset`) is reversed for roled sessions.
3. Decided by the architect within spec authority; the owner was told and may veto: an unroled session gets `A2AMX_ROLE` set to the empty string rather than absent, so the daemon's own environment cannot show through (the hook contract already treats empty as null).

## Review record

Models: architect Claude Sonnet 5.5; consultant on the `claude` harness (model not reported in its reports); developer on the `agy` harness (Antigravity CLI, Gemini 4 Hydrogen Medium as it reported; planning and advisor models not reported separately); tester on the `omp` harness (`opencode-go/deepseek-v4.1-flash` as it reported).

BLOCKED/amend cycles: 0 after the lock. The spec was amended twice before the lock, from the two consultant reviews.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Before the draft | Owner | The first draft was written without loading the `architect` and `spec-writing` skills | Both loaded afterwards; the draft went through the gate and two reviews |
| Draft review | Consultant | B1: the Codex app-server starts inside `prepare_codex`, so a push after `prepare_confined_and_codex` misses it | One push site before it; the carrier read moved to the new function |
| Draft review | Consultant | B2: the Codex tests carry the role only as the carrier, so a field-based variable is never set in them | `A2AMX_ROLE` from the validated field; `Case::launch` sends it |
| Draft review | Consultant | B3: the child inherits the daemon's own environment, so an unset role would show the role of the session the daemon started from | An unroled session gets an empty `A2AMX_ROLE` |
| Draft review | Consultant | Line anchors off, the agy install note wrong, unnamed-session id not `s1`, docs placement, an empty-role mutant | Text corrected before lock |
| Draft review | Architect | The first draft's gate had not read the Codex ordering or the tests pinning the role as `unset` | Found by the gate; spec rewritten |
| Second draft review | Consultant | Test 4 cannot be written with `spawn_daemon_process` as it is; no test can kill a Codex-skip mutant of the address | `spawn_daemon_process_with_env` in scope; one `A2AMX_SESSION` record in the fake Codex |
| Architect review | Architect | A shortcut comment in `prepare_codex` said the role field is display-only | Developer reworded it (fix request m_9159) |

Tester pass (no behavior problem): 12 mutants, ten killed, two survivors both expected (a skip for OMP, Claude or agy only, which no repository test launches; the removal of the pre-removal, equivalent). Accepted gaps: no repository test for a Codex session with an empty role, for a raw client sending only the carrier or only the field, or for reading the variables inside a launched Claude, OMP or agy session. The tester noted the name `codex_role_reaches_only_the_operator_instructions` now overstates what it asserts.
