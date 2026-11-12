# Spec 3ed: let Codex's own sandbox start inside a confined session (fix for spec 3ec)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
Shared task `context_id`: `a2amx-codex-nested-2026-11-12`.

**Scope.** May be edited: `src/confine.rs`, `src/codex.rs`, `src/daemon.rs` (the one call at line
582 only), `tests/confine.rs`, `tests/codex.rs`, `docs/confinement.md`. **Does not touch:** `CHANGELOG.md` and `docs/backlog.md` (the architect
edits them when committing), `Cargo.toml`, `Cargo.lock`, the rest of `src/daemon.rs`, `src/session.rs`,
`src/delivery.rs`, `src/team.rs`, `src/main.rs`, `src/wire.rs`, `tests/common/*`, the unconfined
Codex path, `docs/delivery.md` (its `<dir>/codex/<session>.sock` text predates spec 3ec and is a
separate stale-docs item), and every other harness.

**Public test seams.** `confine::plan` / `Plan::wrap` / `Plan::wrap_app_server` with
`Harness::Codex`; the daemon through `Request::NewSession` with the fake Codex of
`tests/common/fake_codex.rs`. No test calls a private function.

## 1. Why

Spec 3ec shipped confined Codex sessions. The owner's first real run (Codex 0.167.1, installed
build at `9f89965`, 2026-11-12) showed the confinement notice and message delivery work, but every
tool call fails: Codex's own nested sandbox cannot start and reports
`bwrap: Can't mkdir parents for <state>/codex/<id>: Read-only file system`.

Root cause, reproduced in a scratch directory with the real `codex` and `bwrap` (no login needed):
the guest sees the socket directory as a bind mount at `/tmp/codex-daemon-<uid>` whose mount
source, in `/proc/self/mountinfo`, is the host path `<state>/codex/<id>`. Before each tool call
Codex's linux sandbox re-binds that host path inside its own namespace. Our plan hides `<state>`
behind a tmpfs that is then remounted read-only, so the host path does not exist in the guest and
cannot be created. Codex also refuses the obvious workaround, a bind of the socket directory at
its own host path (`unsupported host mount at <path>; remove the bind-mount alias`).

Alternatives rejected:

| Alternative | Why not |
| --- | --- |
| Bind the socket directory at its host path as well | Codex refuses it (probed). |
| Leave `<state>` visible read-write in the guest | Exposes tokens, databases and other sessions' sockets; the point of the tmpfs. |
| Put the socket directory under `~/.codex` or the workspace | Writes into the owner's harness config or checkout; the socket directory is daemon state. |
| Skip Codex's own sandbox (`--dangerously-bypass-approvals-and-sandbox` style) | Removes a layer instead of fixing the mount, and changes Codex's approval behaviour. |

Chosen shape (probed, scratch directory, real `codex sandbox`): the per-session directory
`<state>/codex/<id>/` (0700) holds the socket directory as a subdirectory `sock/` (0700). The
app-server's plan binds `<state>/codex/<id>` **read-only** at its own host path (so the path Codex
re-binds exists and is not itself a socket-directory mount) and binds `<state>/codex/<id>/sock`
read-write at `/tmp/codex-daemon-<uid>`. Result (probed): `codex sandbox sh -c 'echo nested-ok'`
runs inside the outer sandbox; the app-server can still create its socket in `sock`; and a guest
process cannot rename, replace or add entries beside `sock` (`mv`, `ln -s` fail with `Read-only
file system`). The read-only identity bind is deliberate: the TUI's `bwrap` runs on the host and
resolves `--ro-bind <id>/sock ...` by following symlinks, so a guest that could swap `sock` for a
symlink could make the host mount any path into the TUI (consultant finding on the first draft,
which had the identity bind read-write).

## 2. `src/confine.rs`

1. Add `pub const CODEX_SOCKET_SUBDIR: &str = "sock";`. It names the socket directory inside a
   session's Codex directory; `confine.rs` already owns the guest path
   `/tmp/codex-daemon-<uid>` (`src/confine.rs:2100`).
2. `Plan::with_codex_dir` (`src/confine.rs:1906`) takes the session's Codex directory
   (`<state>/codex/<id>`), not the socket directory. The field `codex_dir` keeps its name and
   now holds that directory.
3. In `wrap_with_codex_bind` (the code at `src/confine.rs:2100`):
   - For the app-server variant only (`app_server == true`), emit
     `--ro-bind <codex_dir> <codex_dir>` after the last mount of the mounts loop and **before**
     the `for path in remounts` loop that emits `--remount-ro` (`src/confine.rs:2097`),
     so bubblewrap can still create the mount point inside the hidden-state tmpfs while that
     tmpfs is writable. It is read-only so the guest cannot replace `sock` (§1). The TUI variant gets no identity bind: it runs no tool commands.
   - The `/tmp/codex-daemon-<uid>` bind keeps its place just before `--chdir`, with source
     `<codex_dir>/sock` (`--bind` for the app-server, `--ro-bind` for the TUI).
4. No other behaviour changes: hidden paths, remounts, notice and grants are as in spec 3ec.

## 3. `src/codex.rs`

1. `socket_dir(state_dir, session_id)` (`src/codex.rs:50`) returns
   `<state>/codex/<id>/sock`. Add `pub(crate) fn session_dir(state_dir, session_id) ->
   PathBuf` returning `<state>/codex/<id>`; `socket_dir` joins
   `crate::confine::CODEX_SOCKET_SUBDIR` onto it.
2. `src/daemon.rs:582` passes `codex::socket_dir(..)` to `Plan::with_codex_dir`; it must pass
   `codex::session_dir(..)` instead. This is a one-token change in an otherwise untouched file and
   is the only edit allowed in `src/daemon.rs`.
3. `reset_socket_dir` (`src/codex.rs:612`) takes the session directory: it removes it with
   `remove_dir_all` (no link following, NotFound tolerated), creates the session directory with
   mode 0o700 and its `sock` subdirectory with mode 0o700, and sets both to 0o700 afterwards
   (umask). It is still called before the app-server starts (`src/codex.rs:553`) and still
   runs on a blocking thread.
4. `pinned_socket` (`src/codex.rs:88`) is unchanged: it still opens the socket directory
   (`<state>/codex/<id>/sock`) with `O_DIRECTORY|O_NOFOLLOW` and applies the entry pin. With the
   read-only identity bind the guest cannot replace `sock`, so the pin on `sock` is defence in
   depth (the test of §5 substitutes it from the host); a symlink, a non-directory, a link target
   outside `/tmp/codex-daemon-<uid>/<name>`, or a non-socket entry must still be refused.
5. `Endpoint::Confined { host_dir, .. }` keeps holding the socket directory.

## 4. `docs/confinement.md`

Update the Codex paragraph (`docs/confinement.md:57-61`) to say: the session's Codex directory
under the state directory is mounted at its own path in the app-server (Codex's own sandbox
needs it) and its `sock` subdirectory at `/tmp/codex-daemon-UID`; only the app-server gets the
Codex directory, and read-only, so the guest cannot replace its socket directory; Codex's own
tool sandbox cannot connect to the socket in its default workspace-write mode (network off; with
network access enabled that is Codex's decision, not a mount property).

## 5. Tests

Update, do not delete:

- `tests/confine.rs:3134` (`codex_plan_wraps_own_socket_dir_with_read_only_and_read_write_variants`)
  and the argv assertions at `tests/confine.rs:3296-3317`: `with_codex_dir` takes the session
  directory; the expected app-server argv has `--ro-bind <session> <session>` after the last mount of the
  mounts loop (the `--bind <home>/.codex` entry) and immediately before the first `--remount-ro`
  (asserted by position, like the `--chdir` check at `tests/confine.rs:3310-3315`), and `--bind <session>/sock /tmp/codex-daemon-<uid>` last
  before `--chdir`; the expected TUI argv has no identity bind and `--ro-bind <session>/sock
  /tmp/codex-daemon-<uid>`.
- `tests/codex.rs:262` (`confined_codex_uses_private_socket_dir_and_delivers`): the host layout is
  `codex/s2/sock`; the stale link is planted in `codex/s2` and in `codex/s2/sock` and both are gone
  after start; both directories have mode 0o700; the fake binds its socket inside `codex/s2/sock`.
- The hostile-link test (`tests/codex.rs:575`): the directory search and the planted socket move to
  `<codex dir>/<id>/sock`.

Add:

- A host-substitution test (contract test; the old layout had no `sock`, so no red phase), like the
  hostile-link test at `tests/codex.rs:637-655`: once the `app.sock` link appears in
  `<session>/sock`, the test renames `sock` away on the host, creates a substitute directory
  holding an `app.sock` link to a live `FakeCodex`, and symlinks `sock` to it. The request ends in
  `Response::Error` containing `unexpected socket link` and the `FakeCodex` saw zero connections
  (`connections() == 0`, as at `tests/codex.rs:670-678`). The live listener is what makes mutant
  7.5(f) fail.
- A real-bubblewrap test (the fixture of `tests/common/mod.rs:52`, which skips when bubblewrap is
  unusable): the fake `codex` shell script, running as the app-server guest, receives the host
  session directory in an env variable through `NewSession` env (the id is predictable, as
  `tests/codex.rs:311` relies on), lists the parent of `<session>` (only `<id>`) and
  `<state>/codex` (only `<id>`), and tries `mv <session>/sock <session>/sock2`,
  `ln -s /etc <session>/x` and `touch <session>/y`; each must fail and the script writes the
  results to a file in its cwd that the test asserts. It must still be able to create its socket
  link inside `/tmp/codex-daemon-<uid>`.

## 6. Out of scope

Reading the Codex hook failure (`Hook failed: failed to get client`) seen in the same run; the
configurable private `/tmp` size (backlog F89); any change to Codex's own sandbox settings;
the stale `docs/delivery.md` text; moving the daemon's connect to a different mechanism.

## 7. Acceptance

All from the main checkout, as an ordinary user.

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; in a
   confined architect session only the documented I24 pair
   (`gateway_up::sandbox_hides_state_host_and_limits_writes_to_gateway`,
   `human_inbound::queued_human_message_rechecks_recipient_eligibility_before_delivery`) may fail.
   Judge: the architect's fresh run.
2. `scripts/prepush-fast.sh` passes. Judge: the architect's fresh run.
3. Real Codex nested sandbox (tester, no login needed): from a throwaway state directory under
   `target/scratch/3ed/`, build the app-server argv with the new `Plan` (or read it from a running
   confined session) and run
   `codex sandbox sh -c 'echo ok'` as the wrapped command inside it. It prints `ok` and does not
   print `Can't mkdir parents`. The same run on `9f89965` prints the error. Judge: the tester's
   command output, rerun by the architect.
4. Trust-boundary abuse cases: the guest-side test of §5 shows the app-server guest cannot replace
   `<session>/sock` (red on the read-write variant, mutant 7.5(g)); the host-substitution test of
   §5 shows the daemon refuses a substituted `sock`. Information, not a gate: a tool command in
   Codex's nested sandbox, default workspace-write mode, cannot connect to
   `/tmp/codex-daemon-<uid>/app.sock` or `<session>/sock/app.sock` (probed on scratch:
   `Operation not permitted`, most likely Codex's own network-off seccomp, not the mounts); the
   tester repeats that probe through the real argv, once with `network_access=true`, and reports
   the output.
5. Mutation check: the developer breaks each of these in production code and names the repo test
   that fails, then restores it: (a) drop the identity bind; (b) emit it after the `--remount-ro`
   loop; (c) emit it for the TUI variant; (d) bind `<session>` (not `<session>/sock`) at
   `/tmp/codex-daemon-<uid>`; (e) change the post-create `set_permissions` of `sock` in `reset_socket_dir` to 0o755;
   (g) make the identity bind read-write (`--bind`);
   (f) make `pinned_socket` follow a symlinked `sock`. Mutate one file in the checkout and restore
   it by checksum; the tree must equal the developer's diff afterwards.
6. After the architect's commit and push, the owner repeats the authenticated real-Codex check
   (confinement notice, a tool call, a denied write outside the grants, message delivery); the
   result is recorded in the spec's review record.

## 8. Claims verified against the tree

| Claim | Where | How |
| --- | --- | --- |
| The guest's mount source for the socket dir is the host path | `/proc/self/mountinfo` of a scratch bwrap with `--bind <dir> /tmp/codex-daemon-1000` | probed |
| Nested `codex sandbox` fails with `Can't mkdir parents` when the state dir is a read-only tmpfs | scratch `bwrap ... --tmpfs S0 --remount-ro S0 --bind S0/codex/s9 /tmp/codex-daemon-1000 codex sandbox sh -c 'echo ok'` | probed |
| Codex refuses a bind of the socket dir at its host path | same, with `--bind S S` | probed, `unsupported host mount` |
| The parent identity bind plus `sock` subdirectory lets it run | same, with `--bind P P` before `--remount-ro` and `--bind P/sock /tmp/codex-daemon-1000` | probed, prints `ok` |
| The same with `--ro-bind P P`: nested sandbox runs, the guest can create `/tmp/codex-daemon-1000/app.sock`, and `mv P/sock`, `ln -s` in P fail with `Read-only file system` | scratch run, 2026-11-12 | probed |
| The TUI's bwrap runs on the host and resolves its bind source by path, following symlinks | consultant read of `src/daemon.rs:584-613` | read |
| Tool commands cannot connect to the socket at any of its three paths | real `codex sandbox python3` connecting to each path | probed, `Operation not permitted` |
| Codex requires the socket dir to be user-owned 0700 | scratch run with a 0755 dir | probed, `must be a user-owned directory with mode 0700` |
| Real-bubblewrap tests exist for Codex confinement | `tests/common/mod.rs:52`, `tests/codex.rs:262` | read |
| No other callers of `Plan::with_codex_dir` or `codex::socket_dir` | `grep -n 'with_codex_dir\|socket_dir' src tests` shows `src/daemon.rs:582`, `src/codex.rs`, `tests/confine.rs:3149`, `tests/codex.rs` | grep |

## Review record

Models: architect Claude Sonnet 5.5 (`claude-sonnet-5-5`); consultant Claude Opus 5.5
(`claude-opus-5-5`); developer OMP `openai-codex/gpt-5.6-luna` (planning and working slots; the
advisor model was not exposed); tester OMP `opencode-go/deepseek-v4.1-flash` (planning and working
slots; the advisor model was not exposed).

BLOCKED/amend cycles: 0. This spec is itself the fix for a problem found after the push of spec 3ec.

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| After the push of 3ec (owner's real-Codex run) | owner, architect (Sonnet 5.5) | Every tool call in a confined Codex session failed: Codex's nested sandbox re-binds the mount source path of `/tmp/codex-daemon-<uid>` (the host path under the hidden state directory) and cannot create it. Spec 3ec's acceptance had no authenticated tool-call check, so no review stage could catch it. | This spec; the owner repeats the login check after the push (§7 item 6). |
| Draft review | consultant (Opus 5.5) | The first draft bound the session directory read-write at its own path, so the app-server could swap `sock` for a symlink and the TUI's host-side `bwrap` would follow it. | Identity bind made read-only (probed: nested sandbox still runs, `mv`/`ln` in the session directory fail); no `src/session.rs` change. |
| Draft review | consultant | The hostile-`sock` test could not be built as written (guest fake is a shell script, `tests/common/*` out of scope); mutant (f) could survive without a live listener; mutant (e) was equivalent. | Test redesigned (host-side substitution with a live fake, plus a guest-side test); (e) redefined. |
| Draft review | consultant | The claim that tool commands cannot reach the socket was attributed to the mounts; the probe's EPERM is most likely Codex's own seccomp. Line anchors and the identity-bind position were loose. | Claim limited to probed behaviour, anchors and position wording fixed. |
| Architect review | architect, from the tester's information finding | `docs/confinement.md` said the tool sandbox cannot connect "because networking is off"; the tester's probe with network access enabled also failed, so the cause was not established. | Wording changed to what was probed, not to a cause. |
| Tester pass | tester | Minor: two listings in the guest test script expand the same directory; the stale-link test keeps no outside witness. | Recorded, not fixed: no behaviour impact; the stale-link removal is also covered by the mode and stale assertions. |

Mutants: the developer's a-g and the tester's eleven (a-g plus four of its own) were all killed;
the architect reran mutant g (identity bind read-write) and it fails the argv test and the
guest-side test. Developer's full `cargo test` run was unresolved (one `tests/team.rs` failure,
not reproduced by the tester in two runs); the architect's fresh full run passed.

Open at acceptance: the owner's authenticated real-Codex check (backlog U11), now also covering a
Codex session started with `--remote both` that messages `human:owner`.
