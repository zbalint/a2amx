# Spec 3at: split the attach test's `fake_connection` into named steps

## 0. Status

**LOCKED** (2026-10-27; pre-lock gate run against develop at `7919789`; consultant review `m_4069`, findings adopted).
Architect-directed fix of the red CI Sonar gate on `develop` at `7919789`, context
`a2amx-sonar-s3776-2026-10-27`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `7919789`; `env -u A2AMX_BIN -u NO_COLOR cargo test --test attach_cli` passes 30/30, and the two
tests that use the fake daemon, `status_toggle_reattaches_to_resize_the_session` and
`switch_waits_for_old_attachment_eof_with_delayed_fake_daemon`, pass.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the real `a2amx attach` binary under a PTY against the fake daemon, through the two tests
above. The new helpers are private to the test file and are not tested directly.

**Scope.** May edit exactly one file.

- `tests/attach_cli.rs`: `fake_connection` (line 138), `run_fake_daemon` (line 217), the two tests named above
  (their setup, lines 1832-1843 and 1864-1875, and the reads of `attach_count` and `early_switch`), and new test
  helpers placed directly before `fake_connection`.

Does not touch: every other source and test file, `docs/`, `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`,
`sonar-project.properties`, `.semgrep/`. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

The CI SonarQube gate on `develop` is red at `7919789`: `rust:S3776` (cognitive complexity over 30) flags
`fake_connection` in `tests/attach_cli.rs` at 36 (a hand count gives 36, which matches Sonar; the gate condition
`new_code_smells_severity` reads 20 against 14). The function is new in spec 3as (`d0ef6c7`). It is a request loop
with a match, and one match arm holds the whole attach exchange: it counts the attach, detects an early switch,
sends three frames, and then runs a second loop that answers the client's detach, with two different endings. The
spec 3as reviews did not count it, and the push skipped `scripts/prepush-sonar.sh`. A hand count is the only check
here, because no local command measures S3776.

The point is not to pass the gate. The attach exchange and the detach handshake are two separate protocol steps
that deserve names, and the team plus the four shared handles that travel together through three functions belong in one
small type. Behavior of the fake daemon must not change.

## 2. Target shape

All items are private to `tests/attach_cli.rs`.

| Item | One responsibility | Signature |
|---|---|---|
| `FakeDaemon` | The fake daemon's team and its shared test state | `#[derive(Clone)] struct FakeDaemon { team: TeamScope, old_open: Arc<AtomicBool>, early_switch: Arc<AtomicBool>, attach_count: Arc<AtomicUsize>, done: Arc<tokio::sync::Notify> }` with `fn new(team: TeamScope) -> Self` (flags false, count 0) |
| `run_fake_daemon` | Accept loop (body unchanged except the parameters) | `async fn run_fake_daemon(listener: TcpListener, daemon: FakeDaemon) -> anyhow::Result<()>` |
| `fake_connection` | Answer Hello, List and unsupported requests; hand an Attach to `fake_attach` | `async fn fake_connection(mut stream: TcpStream, daemon: FakeDaemon) -> anyhow::Result<()>` |
| `fake_attach` | The attach exchange: count it, detect an early switch, send `AttachedTop`, data and status, mark the old attachment open, then run the detach handshake | `async fn fake_attach(stream: &mut TcpStream, session: &str, daemon: &FakeDaemon) -> anyhow::Result<()>` |
| `fake_serve_detach` | Wait for the client's `Detach` frame and answer it: for the first attach of `s1`, a late data frame, a 300 ms pause and then the old attachment closes; otherwise a `Detached` frame and the `done` notification | `async fn fake_serve_detach(stream: &mut TcpStream, session: &str, attach_number: usize, daemon: &FakeDaemon) -> anyhow::Result<()>` |

`FakeDaemon` replaces the four loose `Arc`s and the team argument that today travel together through
`run_fake_daemon` and `fake_connection`. `fake_attach` and `fake_serve_detach` each take at most four parameters, none a
field spread of another structure.

Behavior that must stay exactly as it is: the order and content of every frame sent, the `fetch_add` and `store`
orderings (`SeqCst`), the `s1` and `s2` branches, the 300 ms sleep before `old_open` is cleared, the `break` out of the
detach loop and the return from `fake_connection` after an attach (the connection ends there), and the ignoring of
requests that do not parse. `fake_connection` returns `fake_attach(..)`'s result from the Attach arm.

## 3. `tests/attach_cli.rs`

Move the Attach arm's body into `fake_attach` and its inner `while let` into `fake_serve_detach`, as in the table. The
Attach arm becomes `return fake_attach(&mut stream, &session, &daemon).await;` (the connection must end there, not fall
back into the request loop); `fake_attach` ends with `fake_serve_detach(..).await?` followed by `Ok(())`, and the `break`
stays inside `fake_serve_detach`'s loop. Replace
the `old_open`, `early_switch`, `attach_count` and `done` locals in the two tests with
`let daemon = FakeDaemon::new(fake_team());`, pass `daemon.clone()` to `run_fake_daemon`, and read
`daemon.attach_count` and `daemon.early_switch` where the tests read their clones today. Lines 70-136
(`fake_team`, `fake_sessions`, `fake_status`) and the frame helpers at 51-68 stay as they are.

Hand counts under Sonar's rules (+1 per `if`, `else`, loop and `match`, plus the nesting depth; +1 per run of `&&`/`||`;
`?`, closures and `return` add nothing), recorded here so the developer and the reviewer can check them:

| Function | Count | Margin to 30 |
|---|---|---|
| `fake_connection` before (current code) | 36 | -6 |
| `fake_connection` after | 6 (`while let` 1, `if let` 2, `match` 3) | 24 |
| `fake_attach` | 5 (`if` and `&&` 2, `if`/`else` 2, `if` 1) | 25 |
| `fake_serve_detach` | 7 (`while let` 1, `if !matches!` 2, `if`/`else` 2 with `&&` 1 and `else` 1) | 23 |

## 4. Out of scope

Changing what the fake daemon sends or when; changing the two tests' assertions; fixing the weak points of the fake
(for example the fixed 500 ms and 300 ms sleeps); other tests in the file; any `src/` file; `scripts/prepush-sonar.sh`
or a local S3776 proxy (backlog F25); the active-member highlight; the CHANGELOG.

## 5. Acceptance

Commands use the form from AGENTS.md. Checks that need the new code run after implementation.

1. `env -u A2AMX_BIN -u NO_COLOR cargo test --test attach_cli` passes 30/30 (judge: the command output; the count
   is unchanged because no test is added or removed).
2. `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass
   with no warnings.
3. `git status --short` shows only ` M tests/attach_cli.rs`, and `git diff --stat` shows no other file.
4. `rg -n 'async fn fake_connection' -A 40 tests/attach_cli.rs` shows no inner `while let` and no
   `old_open`, `early_switch` or `attach_count` reference inside `fake_connection` (judge: the reviewer reads it).
5. Mutation check, because the behavior is only exercised through two tests. On a scratch copy
   (`git archive` into the scratchpad, or the assigned checkout restored byte for byte afterward), break each of
   the following, name the test that fails, and restore it:
   - change `session == "s2" && old_open` to `session == "s2"` in `fake_attach` (the early-switch flag is then set on
     every `s2` attach): `switch_waits_for_old_attachment_eof_with_delayed_fake_daemon` fails on its
     `!early_switch` assertion;
   - replace `attach_count.fetch_add(1, ..) + 1` with `let attach_number = 1` and no increment:
     `status_toggle_reattaches_to_resize_the_session` fails on its count assertion;
   - remove the `old_open.store(false, ..)` after the late frame: `old_open` stays true from the `s1` attach, the later
     `s2` attach is flagged early, and `switch_waits_for_old_attachment_eof_with_delayed_fake_daemon` fails on the same
     assertion.
   Known surviving mutants, not defects of this spec: removing `done.notify_one()` (neither test waits on it; both
   end with `wait_exit` and `server.abort()`) and removing `old_open.store(true, ..)` after the status frame (the flag
   then stays false and `early_switch` is never set). The developer reports them as survivors. Each mutant is a
   probe; the tree must match the developer's own diff afterward.
6. Complexity (judge: the architect at final review, with a hand count of the committed code, and the CI Sonar gate on
   `develop` after the push): `fake_connection` ≤ 30 and no new `rust:S3776` issue in `tests/attach_cli.rs`. The
   developer reports its own hand count per function next to section 3's table.
7. No suppression comment, `#[allow]` or lint relaxation is added.

## 6. Review record

Models: architect Claude Sonnet 5.5; consultant Claude (model not reported); developer and tester OMP, working model
openai-codex/gpt-5.6-luna, planning and advisor slots not exposed. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| After the push of spec 3as | CI SonarQube gate | `fake_connection` was at cognitive complexity 36 (limit 30); the 3as reviews recorded no hand count for the new helper and the push skipped `scripts/prepush-sonar.sh` | This spec; the 3as record carries the same row |
| Draft review | consultant, Claude | Mutant "remove `done.notify_one()`" cannot fail either test (both end with `wait_exit` and `server.abort()`), so the spec's "times out" was false | Mutant dropped and listed as a known survivor, with `old_open.store(true)` |
| Draft review | consultant, Claude | Acceptance 3 (`git status` shows one file) would fail with the spec file untracked; the `fake_serve_detach` count was 8 where Sonar's rules give 7 (the `&&` counted twice); mutants 1 and 3 had the wrong failure mode | Spec committed first; count, margin and mutant wording fixed |
| Tester pass | tester, OMP gpt-5.6-luna | Own mutant "late first-attach data frame sent as `Detached`" survives both fake-daemon tests: no assertion on the late frame's type or content | Not changed: it is a gap in the 3as tests, outside this behavior-preserving refactor |
| Design read | consultant, Claude | `fake_attach` ends with `fake_serve_detach(..).await?; Ok(())` where returning the call directly is equivalent | Not changed: optional nit, behavior identical |

The architect's own fresh checks found no further problems: `attach_cli` 30/30, clippy and fmt clean, one mutant (`attach_number == 1`
to `== 2`) killed by both fake-daemon tests; a second, a `sed` swap of the ALPHA and BETA output, survived because the switch test
waits for text that is already on screen after the first attach (the tester's own swap was killed).
