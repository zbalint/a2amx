# Spec 3eu: the three tests that fail in a confined session pass there (backlog G18)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `d3b0860` or later.
Shared task `context_id`: `a2amx-g18-confined-tests`.

**Scope.** May edit exactly these files and no others.
- `tests/gateway_up.rs` (one test, `sandbox_hides_state_host_and_limits_writes_to_gateway`)
- `tests/confine.rs` (one test, `real_bwrap_confined_tree_hides_state_run_and_secrets`)
- `tests/human_inbound.rs` (one test, `queued_human_message_rechecks_recipient_eligibility_before_delivery`)

Does not touch: `src/`, `tests/common/`, `docs/` (the architect removes backlog row G18 when accepting), `AGENTS.md`,
`README.md`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, every other test and spec. Do not commit, stage or merge.

Public test seams: the three tests themselves, run through `cargo test --test <name> <test>`. No new test is added and
no behavior is newly tested, so no mutation check applies.

## 1. Why

`AGENTS.md` says the suite passes inside a confined a2amx session. It does not: three tests fail there, and only CI
(unconfined) runs them. The architect's, the developer's and the tester's sessions all have a read-only `$HOME`
(a `write` entry for `$HOME` itself is refused by design, and `tests/confine.rs` pins that refusal), so a task that
changes the confinement code cannot verify its safety net where it works.

Root causes, each reproduced on `d3b0860` in the architect's confined session:

1. `sandbox_hides_state_host_and_limits_writes_to_gateway` (`tests/gateway_up.rs:246-249`) and
   `real_bwrap_confined_tree_hides_state_run_and_secrets` (`tests/confine.rs:7015-7016`) create their temp directory
   with `tempfile::tempdir_in($HOME)`, which makes `~/.tmpXXXX` directly in the read-only home:
   `PathError { path: "/home/zbalint/.tmpqaNYOQ", err: Os { code: 30 } }`.
2. `queued_human_message_rechecks_recipient_eligibility_before_delivery` (`tests/human_inbound.rs:204`) creates a
   confined session with `Harness::Omp` (line 216). `confine::ensure_data_dirs` (`src/confine.rs:661`, called from
   `src/daemon.rs:437` with `$HOME` read from the process environment at `src/daemon.rs:425`) then `chmod`s the real
   `~/.omp/cache`. `strace` shows `chmod("/home/zbalint/.omp/cache", 0700) = -1 EROFS`. It passes in the tester's
   session because the tester's own harness is OMP, so `~/.omp` is writable there. That is the backlog's "cause
   unexplained". Every other confined session in this file uses `Harness::Generic` (`rg -n 'Harness::' tests/human_inbound.rs`
   shows only line 216 as another harness), which `ensure_data_dirs` returns early for.

An environment-only probe confirms the diagnosis: with `HOME` pointed at an empty writable directory under `target/`,
all three tests pass.

Alternatives rejected: a `write` grant for `$HOME` (refused by design, and it would remove the protection confinement
exists for); a `home` field on `DaemonConfig` (production change, and the struct is built in 13 places); making
`ensure_data_dirs` tolerate a read-only home (hides a real failure for a real user); setting `HOME` for the whole test
process (global state in a multi-threaded test binary). The `DaemonConfig` count is 13 `DaemonConfig {` occurrences
across `src` and `tests`, the definition included.

## 2. `tests/gateway_up.rs` and `tests/confine.rs`: temp parent outside `$HOME`

In both tests replace the real-`$HOME` parent with `env!("CARGO_TARGET_TMPDIR")`, the directory the neighbouring tests
already use for a path that must be outside the sandbox-private `/tmp` and writable with a read-only `$HOME`
(`tests/confine.rs:5102-5103`, `6508`, `6873`).

- `tests/gateway_up.rs:246-249`: the `let parent = ...HOME... .expect(...)` statement and `tempdir_in(parent)` become a
  single `tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()`.
- `tests/confine.rs:7015-7016`: drop the `HOME` read; `base` is `tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))?`.

Nothing else in either test's text changes. `shell_layout(home.path(), ...)` and `layout::Layout::new(root.path())`
already take an explicit home or root, and no assertion in either test depends on the parent being the real home.

One behavior of the `confine.rs` test does change, and the environment probe of section 1 did not exercise it. The
sandbox plan binds the workspace through `git_common_dir(cwd, home)` (`src/confine.rs:1347`, walk at `925-936`).
Under `~/.tmpXXXX` that walk reaches `$HOME` and returns `None`. Under `<checkout>/target/tmp/...` it finds the real
checkout's `.git`, so the nested sandbox also binds that `.git` writable, with `.git/hooks` and `.git/config`
read-only (`src/confine.rs:1357-1371`). The test's assertions cover paths beside `cwd`, not inside `.git`, so it
should still pass; the acceptance run is the only evidence. If either test fails after the change for any reason
other than the parent directory, or if the `.git` bind is the cause, stop and report `BLOCKED` with the output;
do not change `src/` or the assertions.

## 3. `tests/human_inbound.rs`: no OMP data directories

Change the harness and deliver arguments at lines 216-217 from `Harness::Omp,` and `None,` to `Harness::Generic,` and
`Some(Deliver::Auto),` (Amendment 1; `Deliver` is already imported in this file). The test asserts that a queued human
message whose lease expired becomes `undeliverable` with a given detail. If the test still fails with this pair,
stop and report `BLOCKED`; do not add a production seam.

## 4. Claims checked

| Claim | Where | Verified how |
| --- | --- | --- |
| The three tests fail in a confined session on `d3b0860` | `cargo test --test gateway_up`, `--test human_inbound`, `--test confine` filtered to each test | ran them; EROFS output above |
| They pass with a writable `HOME` | same commands with `HOME=target/scratch/g18-home`, `CARGO_HOME`, `RUSTUP_HOME` set | ran them; 3 passed, scratch deleted |
| The `human_inbound` write is `chmod ~/.omp/cache` | `src/confine.rs:661-684`, `src/daemon.rs:425-437` | read, and `strace -f` showed the EROFS call |
| No other test reads the real `$HOME` as a temp parent | `rg 'var_os\("HOME"\)\|env::var\("HOME"\)\|home_dir' tests` | ran it; only `gateway_up.rs:246`, `confine.rs:7015` and `codex.rs:381` (which already passes, uses it as a value) |
| Only one `human_inbound` test uses `Harness::Omp` | `rg 'Harness::' tests/human_inbound.rs` | ran it; line 216 only |
| `CARGO_TARGET_TMPDIR` is usable in both files | `tests/confine.rs:5103,6508,6873` use `env!` on it; `tests/gateway_up.rs` is also an integration test of the same package | read; `gateway_up.rs` use is checked by the build in acceptance |
| Under `target/tmp` the confine test's plan also binds the real `.git` | `src/confine.rs:925-936,1347,1357-1371` | read by the consultant; the effect on the test is an inference |
| The three tests still pass after the change | not yet run | post-implementation acceptance below |

## 5. Out of scope

Splitting `tests/confine.rs`, renaming tests, any `src/` change, changing `AGENTS.md`'s confined-suite sentence (it
becomes true again), the `tests/codex.rs` `HOME` use, the `docs/backlog.md` row (architect removes it).

## 6. Acceptance

Run in the developer's own confined session on the final tracked snapshot. The two `$HOME` tests fail there on the old
code in any session with a read-only `$HOME`. The `human_inbound` test fails on the old code only when the session's
own harness is not OMP (the architect's run, on the Claude harness, saw it fail); in an OMP session it is already green,
so for it the check is a contract test and the developer reports which case applied.

1. `cargo test --test gateway_up --test confine --test human_inbound` ends with 0 failed in all three targets.
2. `cargo test` (the whole suite) ends with 0 failed; report each target's counts.
3. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` print nothing.
4. `rg -n 'var_os\("HOME"\)' tests/gateway_up.rs tests/confine.rs` matches nothing.
5. `rg -n 'Harness::Omp' tests/human_inbound.rs` matches nothing.
6. `git status --short` lists exactly the three files in §0 and `git diff --stat` shows fewer than 15 changed lines in
   total (the tester or the architect reads it).
7. The developer leaves no directory it created behind (`target/scratch/<its own>`), except its plan file named in the
   assignment.

Known flakiness: none recorded for these three tests. If any other test fails, rerun it once; a repeat failure is
reported as pre-existing with the output, not fixed.

## Amendment 1: section 3 needs `Some(Deliver::Auto)`, not `Generic` alone

The developer reported `BLOCKED — SPEC ADJUDICATION REQUIRED`: with `Harness::Generic` and `deliver: None` the
`human_inbound` test fails at line 252 (`message did not fail after lease expiry`). The report is right that section 3
as first written does not work. Its explanation (the PTY channel submits the message at once) is not what the code
does; the cause is the default delivery mode.

- `Harness::default_deliver` returns `Deliver::Hold` for `Generic` and `Deliver::Auto` for `Omp`
  (`src/harness.rs:68-71`); `src/daemon.rs:599` applies it when the request's `deliver` is `None`.
- `delivery::run` returns as soon as `channel.deliver() == Deliver::Hold` (`src/delivery.rs:410-413`), before the
  human-message lease recheck (`:424-437`). A held session's message is never rechecked, so it never expires.
- With `Deliver::Auto` on `Generic`, the recheck runs first; the lease is valid on the first pass, then
  `channel.begin()` returns `None` because `harness::ready` for `Generic` needs `bracketed_paste` mode
  (`src/harness.rs:372-376`), which `sleep 90` never enables. The message stays pending, the loop's next ticks run the
  recheck after the 1 s lease expires, and it is marked `undeliverable` with the expected detail. This is the path the
  `Omp` native channel took, where no bridge ever attached.

Section 3 now says `Harness::Generic` with `Some(Deliver::Auto)`. The claims table gains these rows:

| Claim | Where | Verified how |
| --- | --- | --- |
| `Generic` defaults to `Hold`, `Omp` to `Auto` | `src/harness.rs:68-71`, `src/daemon.rs:599` | read |
| Hold returns before the lease recheck | `src/delivery.rs:410-413` vs `:424-437` | read |
| `Generic` with `Auto` is not ready without bracketed paste | `src/harness.rs:372-376`, `src/emulator.rs:385` | read; the mode is set only by the program running in the PTY |
| `Generic` plus `Some(Deliver::Auto)` makes the test pass | not run | post-implementation acceptance; `BLOCKED` again if it fails |
