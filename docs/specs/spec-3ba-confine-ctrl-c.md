# Spec 3ba: let a confined Generic command receive Ctrl-C

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `17a7ddf`; consultant review `m_4481`, findings 1-10 adopted, 1 as option (a), refuse). Architect-directed follow-up to spec 3ax, context
`a2amx-confine-sigint-2026-10-30`. Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `17a7ddf`; full suite and CI green there.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `confine::launch_argv`, `confine::preflight_env` and real confined sessions through the daemon and
the attach stream in `tests/confine.rs`. New helpers are private.

**Scope.** May edit exactly these files and no others.

- `src/confine.rs`: `launch_argv` (line 1316), a new `preflight_env`, a new constant for the `env` path
- `src/daemon.rs`: `Runtime::plan_confinement` (line 345), one call
- `tests/confine.rs`: the tests of section 5
- `docs/harnesses.md` (lines 66-70) and `docs/cli.md` (lines 56-59): replace the limitation text (section 6)

Does not touch: every other file, including `src/session.rs`, `src/harness.rs` and the changelog (the architect adds the
entry). No new dependency. Leave the diff uncommitted.

## 1. Why

In a confined session whose command runs with the terminal in cooked mode (a shell or script under the Generic
harness), one Ctrl-C ends the session and the command's INT trap never runs. a2amx writes the byte 0x03 to the PTY
master; the terminal's line discipline sends SIGINT to the whole foreground process group, which contains `bwrap`
itself. `bwrap` has no SIGINT handler and dies, and with `--die-with-parent` and `--unshare-pid` it takes the command
with it. An unconfined session behaves differently: the command gets SIGINT and its trap runs.

Probe, 2026-10-30, a scratch pty with `bwrap 0.11.1` (uutils `env`), the sandbox flags of the real plan minus its mounts.
Today: a shell with an INT trap, a plain `sleep`, and a command that ignores INT are all killed by signal 2; a raw-mode
reader receives the byte. With `env --ignore-signal=INT bwrap ... -- env --default-signal=INT <command>`: the trap runs
(exit 7), a plain `sleep` still dies (exit 130), a command that ignores INT survives, the raw-mode reader still gets the
byte. An ignored disposition is inherited across `exec`, so the outer `env` keeps `bwrap` alive and the inner `env`
restores the default for the command; the outer `env` execs `bwrap` in place, so no extra process stays. The probe's
first two scripts were harness bugs (a dead process reported as alive; markers under the sandbox's hidden `/tmp`) and were
discarded; only the third is evidence.

## 2. Scope decision: Generic sessions only

The running Claude Code and OMP processes catch SIGINT (their `SigCgt` mask has bit `0x2`) and their terminals read
`-isig -icanon`, so the line discipline generates no SIGINT for them: Ctrl-C reaches them as the byte 0x03. They do not
hit the bug in steady state. The start-up and exit windows, where the terminal may still be cooked, were not observed;
an unconfined harness also dies on Ctrl-C there, so no regression is expected. Therefore the Claude and Omp launch
argument vectors stay exactly as they are (no interplay with `harness::append_system_prompt`, which scans the command
after the first `--`), and Codex stays refused.

## 3. Behavior

For a confined session with `Harness::Generic`:

1. The launched vector is `ENV --ignore-signal=INT <bwrap> <bwrap options> -- ENV --default-signal=INT <command...>`,
   where `ENV` is the absolute path `/usr/bin/env`. Both `ENV` are the same executable; the second one runs inside the
   sandbox, which binds `/` read-only, so the path exists there unless the operator hides `/usr` or `/usr/bin`
   (the default and mandatory hides never do; such a hide already breaks `sh` on merged-`/usr` hosts).
2. `ENV` is a fixed absolute path. It is never looked up through the session's `PATH` (the confined environment is
   controlled by the agent) and never through the daemon's `PATH` at launch time.
3. `launch_argv(plan, notice, argv, Harness::Generic)` builds it: prefix `argv` with the inner `env`, call `Plan::wrap`,
   prefix the result with the outer `env`. `Plan::wrap` is not changed (it is also the `plan()` preflight and is shared
   with Claude and Omp). The inner prefix goes into `wrap`'s `argv`, so `wrap`'s `MAX_ARGV_BYTES` check already counts
   it. Only the outer prefix, a constant 33 bytes with the NULs, is not counted; `MAX_ARGV_BYTES` (256 KiB) is a
   self-imposed cap far below `ARG_MAX`, so those bytes cannot cause `E2BIG`. Add a one-line `// shortcut:` comment that
   says so; no second check and no second error path.
4. For `Harness::Claude` and `Harness::Omp` the vector is what `launch_argv` returns today. For `Harness::Codex` the error is
   unchanged.
5. `confine::preflight_env(env: &Path) -> Result<()>` runs the production shape once, through `command_with_timeout`
   (the helper `preflight` uses): `<env> --ignore-signal=INT <env> --default-signal=INT <env>`. The last `env` has no
   arguments, prints to the null stdout and exits 0. This tests both flags the way launch uses them, the inner absolute
   path, and needs no `true` looked up through the daemon's `PATH`. On failure it returns an error that names the
   requirement ("confinement of a Generic session needs env with --ignore-signal and --default-signal: GNU coreutils
   8.31 or newer, or uutils") and carries the underlying cause (missing file versus exit status), so an operator can
   tell "missing" from "too old". `Runtime::plan_confinement` calls it with the constant for Generic sessions only, next
   to `confine::preflight(&bwrap)`, so an `env` that lacks the flags refuses the session with that error.

   Decision (architect, consultant option (a)): refuse, do not degrade. A host with an older `env` (GNU coreutils before
   8.31: Ubuntu 18.04 and 20.04, Debian 10, RHEL 8 and similar; busybox `env`) can no longer start a confined Generic
   session. Ctrl-C killing `bwrap` does not weaken the sandbox, so the cost is a compatibility break, accepted because
   it is explicit and fixable, it matches how `preflight` already refuses a broken `bwrap`, and a degrade would need a
   second launch path and a flag nobody can see. The changelog says so. The owner may reverse this choice.
6. Everything else in the plan and the notice is unchanged. A raw-mode command still receives Ctrl-C as input.

## 4. Target shape

Two functions in `src/confine.rs`, one constant, one call in `daemon.rs`:

| Item | Responsibility | Signature |
| --- | --- | --- |
| `ENV_PATH` | the fixed `env` path | `pub const ENV_PATH: &str = "/usr/bin/env";` |
| `preflight_env` | run the flag probe and name the requirement on failure | `pub fn preflight_env(env: &Path) -> Result<()>` |
| `launch_argv` | build the vector of section 3 item 1 for Generic | unchanged signature |

The Generic arm of `launch_argv` becomes a few lines; if it grows past about 10 lines, extract the two-prefix step into
one private function with the signature `fn ctrl_c_wrapper(wrapped_inner: Vec<String>) -> Vec<String>`. Do not add a
parameter to `Plan::wrap`, a `Plan` field, or a second spawn path. Do not use `pre_exec`, `signal()` or `libc` in
`session.rs` (the terminal types stay inside the allowed modules).

## 5. Tests (public seams; write the failing test first)

Real-behavior tests need bubblewrap and user namespaces; CI now provides both and sets `A2AMX_REQUIRE_BWRAP=1` (they
skip locally without them, like the sibling tests). Use the Generic harness and a confined session created through the
daemon as the existing real-bwrap tests do, and send the byte as an attach-stream input frame, the path a user's Ctrl-C
takes after the attach client forwards it: `Client::connect(state).await?.attach(&id, false, cols, rows)` (`src/client.rs`
line 115), then `attachment.send(ClientFrame::Input(vec![0x03]))` as in `tests/daemon.rs` line 867; write the byte as
`vec![0x03]`, not as a string literal. Read the exit code by polling the session list's `exit_code` (`eventually` in
`tests/common/mod.rs` line 67; add `mod common;` to `tests/confine.rs`, it is allowed there). Markers go to a directory
the plan binds writable (the workspace), never to `/tmp` (the sandbox mounts a fresh tmpfs there).

**Race rule for tests 1-4.** The byte can arrive before `trap` or `stty raw` has run. Each script therefore writes a
`ready` marker into the workspace after its setup, and the test polls for that file before it sends the byte (the
existing real-bwrap test polls a `done` marker the same way). For test 4 send the byte during the `sleep`, after the
`ready` marker. Bound every poll (about 10 seconds) so a broken change
fails the assertion instead of hanging the suite.

1. `ctrl_c_runs_the_trap_of_a_confined_shell`: a command `trap 'echo got-int > MARKER; exit 7' INT; while :; do sleep 0.2; done`; after the byte the
   marker holds `got-int` and the session's exit code is 7. This fails on the current code (session killed, no marker).
2. `ctrl_c_still_ends_a_confined_command_without_a_trap`: `sleep 30`; after the byte the session has exited with code
   130 (not still running). Contract test that passes identically before and after: today `bwrap` is killed by signal 2
   and `exit_code_from_status` (`src/session.rs` line 315) maps a signal death to 128+signal; after the change
   `bwrap` propagates the child's signal death as 128+N. It cannot tell the two cases apart; its job is to show the
   command still dies.
3. `ctrl_c_reaches_a_confined_raw_mode_reader_as_input`: `stty raw -echo; head -c1 > MARKER`; the marker holds the single
   byte 0x03 and the session exits 0. Contract test (passes today).
4. `ctrl_c_does_not_end_a_confined_command_that_ignores_it`: `trap '' INT; sleep 2; echo survived > MARKER`; the marker
   holds `survived`. Fails on the current code.
5. `launch_argv_prefixes_generic_with_env_wrappers`: `launch_argv` for Generic starts with `["/usr/bin/env",
   "--ignore-signal=INT", <bwrap>]` and contains `["--", "/usr/bin/env", "--default-signal=INT", <first command word>]`
   in that order, with literal expected tokens. A second assertion: for `Harness::Claude` and `Harness::Omp` the vector
   begins with the bwrap path and contains no `--default-signal=INT` (contract test).
6. `preflight_env_refuses_an_env_without_the_flags`: a stub executable that exits non-zero for these arguments makes
   `preflight_env` fail with an error containing `--ignore-signal` and `--default-signal`; a stub that exits 0 passes; a
   path that does not exist fails with an error that names the missing file. Use the existing stub helpers
   (`stub_path_dir`, `tests/confine.rs` line 67; pass its `bwrap` file as the env path, the stub ignores its arguments).

The pure tests (5, 6) must run without bwrap. Tests 1-4 are marked as real-bwrap tests in the same way as
`real_bwrap_confines_the_session_process`.

## 6. Documentation

- `docs/harnesses.md` lines 66-70 and `docs/cli.md` lines 56-59: remove the cooked-mode limitation. Say instead that
  a confined Generic command receives Ctrl-C as it would unconfined (the sandbox wrapper ignores SIGINT and the command
  gets the default back), that Claude and Omp are unaffected because they run the terminal in raw mode and receive the
  byte as input, and that confinement of a Generic session needs `env` with `--ignore-signal` and `--default-signal`
  (GNU coreutils 8.31 or newer, uutils).
- Spec 3ax stays as it is: its limitation line is a historical record.

## 7. Trust boundary

The change adds one executable, `env`, launched outside the sandbox by the daemon on behalf of the session request.

| Attack path | Disposition |
| --- | --- |
| An agent or repo-shipped config chooses which `env` runs outside the sandbox (PATH, a workspace-relative `env`) | Removed: the path is the constant `/usr/bin/env`, never looked up |
| The inner `env` is shadowed by a hostile file inside the sandbox | Removed: also an absolute path; and anything running inside the sandbox is already the confined agent |
| `env --ignore-signal=INT` weakens the sandbox | Not applicable: it changes only how `bwrap` itself handles SIGINT; the mount plan, namespaces and `--die-with-parent` are untouched, and the daemon stops sessions with SIGHUP and SIGKILL, not SIGINT (claims table) |
| A missing or old `env` drops the fix | Constrained: `preflight_env` refuses the session with a clear error. Losing the fix would be a usability bug, not a breach, so the refusal is a compatibility choice (section 3 item 5) |

Abuse-case tests: 6 (an `env` without the flags refuses), and 5 (the vector uses the absolute path and not a bare
`env`). The daemon's kill paths use SIGHUP and SIGKILL (`src/session.rs` lines 290, 1155, 1165, 1340), so ignoring
SIGINT in `bwrap` does not weaken stopping a session.

## 8. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test confine
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files of section 0
```

- Tests 1 and 4 fail on the unchanged code first (record it); tests 2, 3, 5 (second assertion) are contract tests.
- The tester mutates a scratch copy of the final snapshot (`git archive`, one shared `CARGO_TARGET_DIR`, restore after each):
  remove the outer `--ignore-signal=INT` (test 1 must fail), remove the inner `--default-signal=INT` (test 1 must fail:
  the trap would be ignored), apply the wrapper to Claude (test 5's second assertion must fail), drop the
  `preflight_env` call in `plan_confinement` (documented gap if no test reaches it: the tester says so), and replace
  the constant with `"env"` (test 5 must fail).
- The tester also runs the four cases against a real Generic session by hand through `a2amx attach` in an isolated daemon
  (temp home, the installed binary, never the working daemon) and reports what it saw.
- The tester's report ends with `df -h /tmp` and `ls -d /tmp/a2amx-*` and the deletion of every directory the tester
  created; the architect checks both.
- Known flaky: `tests/daemon.rs::exited_claude_session_never_reports_needs_input`, `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`
  and `tests/daemon_cli.rs::activity_notices_report_peer_transitions_once_in_order`: rerun once, record as pre-existing if it then passes.

## 9. Out of scope

Claude, Omp and Codex launch vectors, `Plan::wrap`, `src/session.rs`, the confinement notice, the changelog,
`--new-session`, graceful session stop, and a minimum-version policy for bubblewrap.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `launch_argv` is at `src/confine.rs:1316` and its Generic arm returns `wrapped` unchanged | `src/confine.rs:1316-1332` | read |
| `plan_confinement` is at `src/daemon.rs:345` and calls `confine::preflight(&bwrap)` at line 362 | `src/daemon.rs` | read |
| `preflight` runs `--version` then a flag probe through `command_with_timeout` | `src/confine.rs:1212-1234` | read |
| The process launched is `launch_argv[0]` with the rest as arguments | `src/session.rs:380-405` | read |
| The daemon stops sessions with SIGHUP and SIGKILL, never SIGINT | `src/session.rs:290,1155,1165,1340`; `src/daemon.rs:1559` | grep |
| The limitation text lives at `docs/harnesses.md:66-70` and `docs/cli.md:56-59` | those lines | read |
| Claude and OMP catch SIGINT and run on `-isig -icanon` terminals | `/proc/<pid>/status` and `stty -a` of the running processes, 2026-10-30 | probed (steady state only) |
| GNU coreutils `env` has `--ignore-signal`/`--default-signal` since 8.31 (2019-03-10) | coreutils 8.31 announcement and man page | read (web) |
| The probed behavior of the mechanism, section 1 | scratch pty, bwrap 0.11.1, uutils env 0.8.0 | probed; GNU `env` not probed locally |
| The CI runner's `env` supports the flags | Ubuntu 24.04 ships coreutils 9.4 | recall, not read: the first CI run of the tests settles it |
| No existing test sends 0x03 through the attach stream into a confined session | `tests/*.rs` grep for `0x03` | grep: hits only in `tests/prefix.rs` and `tests/prop_wire.rs`, unrelated |
| The attach stream carries input as `ClientFrame::Input(Vec<u8>)` | `src/wire.rs:493` | grep (anchor corrected by the consultant) |
| `tests/confine.rs` has no attach helper and no `mod common;`; the pattern is `Client::attach` then `ClientFrame::Input` | `src/client.rs:115`, `tests/daemon.rs:867`, `tests/common/mod.rs:67` | consultant read; architect did not re-read |
| The changelog's Unreleased section still states the limitation | `CHANGELOG.md:27` | consultant read; the architect edits it (out of the developer's scope) |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (model name not exposed to the architect);
developer and tester on the OMP harness, model `openai-codex/gpt-5.6-luna` (the advisor model was not exposed to
either). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant | Several line anchors and the claim about the test helper (`mod common;`, attach pattern) were off or unread | Anchors corrected, the helper pattern named in the spec |
| Draft review | Architect (Claude Sonnet 5.5) | Section 8 called the by-hand check one against the "installed binary"; the installed daemon predates the change, so it would show the old behavior | Assignment says the built `target/debug` binary with an isolated daemon |
| Implementation | Developer | Edited the tree after its REPORT, so the tester's hash no longer matched | Freeze rule: a REPORT means the tree is frozen; the tester reran against the new hash |
| Implementation | Developer | The Generic arm of `launch_argv` was longer than the spec's target | Extracted the private `ctrl_c_wrapper` the spec allowed; no semantic change |
| Tester pass | Tester (gpt-5.6-luna) | Mutant (f), dropping the nested `env` call in `preflight_env`, survived the first test | Architect fix request m_4511: the test uses a recording stub and asserts the four argv literals; the rerun kills it |
| Tester pass | Tester (gpt-5.6-luna) | Mutant (e), removing the daemon's `preflight_env` call, survives the suite | Not fixed here; backlog F36 |
| Architect review | Architect | A full run during the tester's mutant builds failed one test through the shared `CARGO_TARGET_DIR`; the developer saw the same once | Not a defect; the rerun with no concurrent build passed 695/0/3. Mutation runs and checkout runs should not overlap |
| Architect review | Architect, own mutants | none survived (deleting either env flag push was killed by three tests each) | none needed |
