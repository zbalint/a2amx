# Spec 3bf: a daemon test for the `env` preflight of confined Generic sessions

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `a9a9156`; consultant review `m_4586`, findings 1-9 adopted). Architect-directed, backlog F36, owner chose
option A on 2026-10-30. Context `a2amx-confine-env-test-2026-10-30`.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `a9a9156`; the full suite passes there (697 passed, 0 failed, 3 ignored, writable HOME; the exact
environment shows only the known read-only-HOME failure F40).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** a real daemon started with `DaemonConfig`, driven through `Client` in `tests/confine.rs`.

**Scope.** May edit exactly these files and no others:

- `src/daemon.rs`: `DaemonConfig` (line 52), `struct Runtime` (line 74), `Daemon::start` (line 1636, the `bwrap_path`
  computation at 1639), the `Runtime { .. }` literal (line 1713, `bwrap_path,` at 1723), `Runtime::plan_confinement` (line 345)
- `src/main.rs`: the `DaemonConfig` literal at line 172
- `tests/confine.rs`, `tests/common/mod.rs`, `tests/daemon.rs`, `tests/broker.rs`, `tests/attach_cli.rs`: an added
  `env_path` line in each `DaemonConfig` literal (section 2 item 4), and in `tests/confine.rs` also the helper and the new test

Does not touch: every other file, including `CHANGELOG.md` and `docs/` (the field is a test seam, not user-visible).
No new dependency. Leave the diff uncommitted.

## 1. Why

`Runtime::plan_confinement` calls `confine::preflight_env(Path::new(confine::ENV_PATH))` for every confined Generic
session so that a missing `env --ignore-signal`/`--default-signal` is refused with a clear message instead of a session that
dies at start (spec 3ba). Nothing tests that call: removing it passes the whole suite (the spec 3ba tester's mutant e). The
function `preflight_env` is tested directly, but the daemon's use of it is not. A daemon test cannot make `/usr/bin/env`
fail, so it needs a seam: `DaemonConfig` already carries `bwrap_path` for the same reason, and gets `env_path` (the owner
chose this over dropping the test).

## 2. Changes

1. `DaemonConfig` gets a field after `bwrap_path`:

   ```rust
   /// `env` program probed by the Generic confinement preflight; `None` means `confine::ENV_PATH`.
   /// Only the probe uses it: the launched argv always names `confine::ENV_PATH`.
   pub env_path: Option<PathBuf>,
   ```

2. `struct Runtime` (not `Daemon`: `plan_confinement` is a `Runtime` method and reads `self.bwrap_path`, line 359) gets a
   private field `env_path: PathBuf` after `bwrap_path` (line 84). `Daemon::start` computes it next to the `bwrap_path`
   computation at line 1639: `config.env_path.clone().unwrap_or_else(|| PathBuf::from(confine::ENV_PATH))`, and sets it in
   the `Runtime { .. }` literal next to `bwrap_path,` (line 1723).
3. `plan_confinement` clones it next to `bwrap_path` (line 359) and passes it:
   `confine::preflight_env(&env_path)?` in place of `confine::preflight_env(Path::new(confine::ENV_PATH))` (line 363).
   Nothing else in the function changes; the order resolve bwrap, preflight env, preflight bwrap stays.
4. Every `DaemonConfig { .. }` literal gets an `env_path` line. The sites on `a9a9156` (grep `DaemonConfig {`):
   `src/main.rs:172`; `tests/common/mod.rs:45`; `tests/attach_cli.rs:757`; `tests/broker.rs:2471`;
   `tests/daemon.rs:184, 1333, 1838, 1857, 2136` get `env_path: None,` (nine literals); the tenth, `tests/confine.rs:101`,
   moves into the helper of item 5 and gets `env_path,`. The compiler lists any missed one.
5. `tests/confine.rs`: a new helper `start_confine_daemon_with_env(state, bwrap_dir, env_path: Option<PathBuf>)` holds the
   body of `start_confine_daemon`; `start_confine_daemon` calls it with `None`. Its other callers do not change.

## 3. Test to add (name is a suggestion)

`confined_generic_session_is_refused_when_env_lacks_the_signal_flags` in `tests/confine.rs`, `#[tokio::test(flavor =
"multi_thread", worker_threads = 2)]`, in the style of `missing_bwrap_path_refuses_confined_session`:

1. The daemon's `bwrap_path` is a directory holding a `bwrap` stub that this test (or a new private helper next to
   `probe_fail_stub_path_dir`) writes: a `/bin/sh` script that exits 0 for `--version`, exits 0 when its first argument starts
   with `--ignore-signal` (so it passes the `env` probe if the daemon ever runs it as `env`), and exits 42 otherwise (so it
   fails the bubblewrap probe). Do not use `probe_fail_stub_path_dir()` itself: it exits 42 for every call but `--version`,
   so it would also fail the `env` probe and the test could not tell the configured `env` path from the `bwrap` path
   (Amendment 1). It needs no real bubblewrap, so the test never skips, and on every mutant the request
   stops at `confine::preflight` with an error that does not name the flags, before `HOME` and `XDG_RUNTIME_DIR` are read
   or any session is spawned. (`stub_path_dir(0)` would let a mutant that probes `env` after `bwrap` survive and would
   spawn a real session on some mutants.) A failing `env` stub (`stub_path_dir(42)`, its `bwrap` file used as the env program, as
   `preflight_env_refuses_an_env_without_the_flags` does) is `env_path: Some(..)`.
2. Send `confined_shell_request(&cwd, "printf ran > marker", vec![], ConfineRequest::default())` (Generic).
3. Assert the response is `Response::Error { message }` and `message` contains `--ignore-signal` and `--default-signal`
   (literals), and that `cwd/marker` does not exist.
4. Contract test (passes on the new production code; the red phase is the compile error before the field exists, and
   the mutants of section 5 are its value).

## 4. Rule

Tests may use only public interfaces (`DaemonConfig`, `Client`, `Request`, `Response`). Expected values are literals.
No change to `confine::preflight_env`, `launch_argv` or `ENV_PATH`.

## 5. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test confine
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
rg -o 'env_path: None' src tests | wc -l   # 9: src/main.rs 1, tests/common 1, attach_cli 1, broker 1, tests/daemon.rs 5
git status --short                         # only the files of section 0
```

- In a confined session the exact `cargo test` fails on `real_bwrap_confines_the_session_process` (backlog F40); use
  `HOME=<writable dir> CARGO_HOME=/home/zbalint/.cargo RUSTUP_HOME=/home/zbalint/.rustup` and report both results.
  Known flaky: backlog I9 and F37, rerun once and record as pre-existing.
- The tester applies each mutant to a scratch copy of the final snapshot (`git archive`, one shared `CARGO_TARGET_DIR`,
  restore after each) and names the test that fails on it. All must be killed by the new test:
  1. remove the `preflight_env` call from `plan_confinement` (the F36 mutant);
  2. keep the call but probe `Path::new(confine::ENV_PATH)` instead of the configured path;
  3. change the guard `matches!(harness, Harness::Generic)` so the probe never runs (same effect as 1; a cheap extra check,
     not separate evidence);
  5. pass the `bwrap` path to `preflight_env` instead of `env_path` (the variable swap): killed because the `bwrap` stub then
     passes the `env` probe and the request fails later at the bubblewrap probe, without the flag names.
  Mutant 2, and a mutant that makes `Daemon::start` ignore `config.env_path`, are killed only if the host's `/usr/bin/env`
  supports `--ignore-signal` (GNU coreutils 8.31 or newer, or uutils): the tester records `env --version`, and a survivor
  on an older `env` is reported, not a fix request. The tester also runs mutants of its own (for example: probe after
  `confine::preflight(&bwrap)`, which the bubblewrap probe failure kills), and reports survivors. A survivor in this diff's scope is a fix request.

## 6. Out of scope

A Claude or OMP session is not probed (the guard is Generic only): no test for the negative side. `launch_argv` using
`confine::ENV_PATH` while the probe uses the configured path (documented in the field comment; only tests set the
field). `CHANGELOG.md`. Every other test and file.

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `plan_confinement` calls `preflight_env(Path::new(confine::ENV_PATH))` for Generic only, after `resolve_bwrap` and before `preflight(&bwrap)` | `src/daemon.rs:358-366` | read |
| `ENV_PATH` is `/usr/bin/env` | `src/confine.rs:25` | read |
| `preflight_env` runs the probe and its error text names `--ignore-signal` and `--default-signal` | `src/confine.rs:1239-1250` | read |
| `DaemonConfig` has five fields and ten literals exist, all listing every field (no `..Default`) | `rg -n 'DaemonConfig \{' src tests` | grep |
| `resolve_bwrap(Some(dir))` finds an executable `bwrap` file in a directory path | `src/confine.rs:1174-1199`; used with `stub_path_dir` by existing tests | read |
| `stub_path_dir(n)` writes an executable `bwrap` script that exits `n` | `tests/confine.rs:69-78` | read |
| `preflight_env_refuses_an_env_without_the_flags` uses the stub's `bwrap` file as the env program and expects both flag names in the message | `tests/confine.rs:2369-2375` | read |
| `missing_bwrap_path_refuses_confined_session` is the model for the daemon-level test (client request, `Response::Error`, marker absent) | `tests/confine.rs:2152-2172` | read |
| `probe_fail_stub_path_dir` answers `--version` with 0 and everything else with 42; run as `env` it fails the probe, so it cannot serve as this test's `bwrap` | `tests/confine.rs:80-94` | read; survivor of mutant 5 (architect and tester) |
| `plan_confinement` is a `Runtime` method that reads `self.bwrap_path` (a `Runtime` field); `Daemon` has no `bwrap_path` | `src/daemon.rs:74-92`, `:345`, `:359` | read (consultant `m_4586`) |
| `DaemonConfig` does not derive `Default` and no literal uses `..` | `src/daemon.rs:52` | read (consultant) |
| With the call removed, a stub bwrap that exits 0 lets the create proceed and no env error appears | not run | to confirm by the tester's mutant 1 |
| A failed plan reaches the client as `Response::Error` with `error.to_string()`; the `preflight_env` error text starts with the requirement naming both flags | `src/daemon.rs:386`, `src/confine.rs:1247-1249` | read |
| `DaemonConfig` is not part of the public interface of `AGENTS.md` (CLI, config and team files, wire protocol, database schema, MCP tools) | `AGENTS.md`, Versioning | read |

## Amendment 1 (the bwrap stub must pass the env probe; no scope change)

2026-10-30, after the architect's and the tester's mutation passes (`m_4602`). Mutant 5 (probing the `bwrap` path instead of
`env_path`) survived the first implementation: section 3 told the developer to use `probe_fail_stub_path_dir()` as
`bwrap`, which exits 42 for every call except `--version`, so run as `env` it fails the `env` probe with the same message.
Section 3 item 1 now specifies a `bwrap` stub that exits 0 when its first argument starts with `--ignore-signal`, and
section 5 lists the variable swap as mutant 5. Gate step 7 re-run: `probe_fail_stub_path_dir` appears in this spec only in
section 3 item 1 (as the stub not to use), in this amendment and in the claims table, where the row now describes it as
rejected for this test. Only `tests/confine.rs` changes; scope is unchanged.
