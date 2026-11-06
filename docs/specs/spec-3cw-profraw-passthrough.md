# Spec 3cw: Test helpers that clear the environment keep the coverage target

## 0. Status

LOCKED.

Scope (files that may be edited): `tests/common/mod.rs` and the test files that start the `a2amx` binary
with a cleared environment: `tests/grant.rs`, `tests/suggest.rs`, `tests/privilege.rs`, `tests/hook.rs`, and any
other `tests/*.rs` the developer's scan finds (listed in the plan before the edit). Does not touch: `src/`,
`scripts/`, `.gitignore`, `docs/backlog.md` and `CHANGELOG.md` (the architect updates the backlog; no changelog
line, nothing user-visible), the existing `default_*.profraw` files in the checkout root (they are
deleted only on the owner's word), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-profraw-2026-11-06`. No public seam: this changes test helpers only.

## 1. Why

Backlog F78. `scripts/prepush-sonar.sh` runs the suite under `cargo llvm-cov`, which sets
`LLVM_PROFILE_FILE` to a file under `target/llvm-cov-target`. A test that starts the instrumented `a2amx`
binary with `Command::env_clear()` drops that variable, so the child writes `default_%m_%p.profraw` into its
working directory, the checkout root, where the coverage report never finds it (`tests/hook.rs:180-190`
already explains and handles this for its own helper). The root held 405 such files (71 MB) on 2026-11-06 and
550 after three more Sonar runs. They are ignored by `.gitignore` but they fill the disk the WSL2 VM lives on
(rule: scratch files stay in `target/`), and the lost profiles also understate coverage of what those
children run.

The daemon case matters most: `common::spawn_daemon_process` clears the environment and starts the daemon, and
the daemon runs in the checkout root, so its own profile lands there. Session children are a smaller case:
`a2amx new` sends the client's environment to the session (`src/main.rs:875`) and the children run in a
test tempdir, so what they lose never reaches the root. The root count therefore measures the daemon and the
direct CLI sites, not child coverage, and this spec does not claim to recover child coverage.

Rejected: ignoring `*.profraw` harder or deleting them in the script (the cause stays and the coverage
stays understated); passing the variable in production code (`src/`) (tests must not shape the product);
setting `LLVM_PROFILE_FILE` globally in `.cargo/config.toml` (it would override the path `cargo llvm-cov`
chooses).

## 2. Change

Add one helper to `tests/common/mod.rs`, for example `pub fn keep_coverage_env(command: &mut Command)
-> &mut Command`, that copies `LLVM_PROFILE_FILE` from the test process to the command when it is set
and does nothing otherwise. Call it on every command that starts the `a2amx` binary (or a harness fake that
runs it) after `env_clear()`:
- `tests/common/mod.rs` `spawn_daemon_process`;
- the `env_clear()` call sites found by `rg -n 'env_clear' tests` (today: `tests/grant.rs`,
  `tests/suggest.rs` six sites, `tests/privilege.rs`, `tests/hook.rs`). A file that does not include
  `mod common;` (`grant.rs`, `suggest.rs`, `privilege.rs`) may include it, or keep a local three-line copy
  like `hook.rs` does; prefer `mod common;` and say which in the plan. `hook.rs` replaces its local
  `clean_command` body with the shared helper only if that is a net simplification; otherwise it stays.
`tests/privilege.rs` `run_command` is the single choke point for every program it runs, including `unshare -Ur
<a2amx>`, which passes the cleared environment on to the binary: it gets the call (the extra variable is
harmless for `unshare -Ur true`). A command that never runs the instrumented binary (a plain `sh`, `sudo`,
`git`) needs no call.

## 3. Out of scope

`src/`, scripts, the `.gitignore`, deleting the existing files, other environment variables, tests that do not
clear the environment, and `src/codex.rs`'s `env_clear()` of the Codex child (a different binary).

## 4. Acceptance

The developer proves the cause and the fix by measurement, with the coverage tooling already installed
(`cargo llvm-cov` is on PATH and `target/llvm-cov-target` exists):
1. Baseline, with no other coverage or Sonar run active (they share `target/llvm-cov-target` and write to
   the same root): save a sorted list of `default_*.profraw` in the checkout root to a file under
   `target/scratch/3cw/`, run `cargo llvm-cov --no-report --test suggest --test grant --test privilege
   --test hook` (one `--test` per invocation if the tool rejects a repeated flag), save the list again and diff
   the two. The new files are the red result (expected: five of the six `suggest.rs` sites, `grant.rs`,
   `privilege.rs`; `suggest.rs:330` uses a tempdir as its working directory and writes there). Repeat for
   `--test explain --test omp_wiring` (they use `spawn_daemon_process`; the daemon writes its own profile
   when it stops). `auto_confine` and `confine_show` also use it and may be added when `bwrap` works.
2. After the change, the same two measurements give a difference of zero. If another source still writes
   files, the developer reports which test (bisect with one `--test` per run) and stops: that is a new
   finding for the architect, not an expansion of scope.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`; `TMPDIR` unset. They
   pass with the ambient environment of a hosted session.
4. Coverage runs write only under `target/`: after the work, the developer removes only the files that
   its own runs created in the root (it knows them by the before/after lists), deletes `target/scratch/3cw`,
   and reports the count; the 550 older files stay.
5. The architect confirms with the next `scripts/prepush-sonar.sh` run: the root count does not grow.
