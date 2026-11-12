# Spec 3ec: confine Codex sessions (F31)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
Shared task `context_id`: `a2amx-codex-confine-2026-11-12`.

**Scope.** May be edited: `src/confine.rs`, `src/codex.rs`, `src/daemon.rs`, `src/session.rs`,
`src/team.rs`, `src/delivery.rs`, `tests/confine.rs`, `tests/codex.rs`, `tests/team.rs`,
`tests/common/fake_codex.rs` (and `tests/common/mod.rs` only if a helper must be shared),
`README.md`, `docs/confinement.md`, `docs/harnesses.md`, `docs/remote-messaging.md`,
`docs/permissions.md`, `docs/teams.md`, `docs/backlog.md` (close F31 only). **Does not touch:**
`Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `src/grant.rs` (the `codex` harness grant, F52, stays
refused), `src/main.rs`, `src/wire.rs` (so `a2amx explain` output is unchanged: the socket dir is
not added to `ConfineInfo`), the unconfined Codex path, Codex's own sandbox settings, any other
harness's behaviour.

**Public test seams.** `confine::plan` / `Plan::wrap` / `confine::launch_argv` with
`Harness::Codex`; the daemon through `Request::NewSession` with the fake Codex of
`tests/common/fake_codex.rs`; `a2amx team up` validation. No test calls a private function.

## 1. Why

A confined session today is Claude, OMP or generic; Codex is refused in three places
(`src/confine.rs:1093`, `src/confine.rs:2532`, `src/daemon.rs:547`) plus the team file check
(`src/team.rs:1113`). The reason: a Codex session's tools run in its private `codex app-server`,
which the daemon starts in `Daemon::prepare_codex` (`src/daemon.rs:743`) through `codex::start`
(`src/codex.rs:349`) with a plain tokio `Command`, outside `Session::spawn` and so outside the
bubblewrap plan; the TUI in the PTY is only a client. Wrapping the PTY alone would confine nothing
that matters. Confinement shipped in 0.2.0 without Codex on purpose (backlog F31, owner decision
2026-10-29); the owner asked on 2026-11-12 to add it. Remote messaging needs a confined session
(`src/daemon.rs:540`), so this also opens remote messaging to Codex agents.

**Probed facts that shape the design** (codex 0.167.1, bubblewrap 0.11.1, scratch `CODEX_HOME`,
2026-11-12; an earlier belief that a short socket path avoids Codex's symlink was false):
1. `codex app-server --listen unix://P` always puts the real socket at
   `/tmp/codex-daemon-<uid>/<64 hex>` and makes `P` a symlink to it, for requested paths of 14, 57,
   61, 71, 81, 91, 101, 107 and 108 bytes alike. `TMPDIR` does not move it.
2. Under confinement `/tmp` is a private tmpfs per sandbox, so that link dangles for the daemon and
   for the TUI (its own sandbox, its own `/tmp`).
3. With `bwrap --tmpfs /tmp --bind <hostdir> /tmp/codex-daemon-<uid>` the real socket and its lock
   file appear in `<hostdir>`; the link target is the in-sandbox path.
4. A host client cannot connect to `<hostdir>/<hash>` when `<hostdir>` is longer than about 42
   bytes (`sun_path` is 108 bytes); `/proc/self/fd/<dirfd>/<hash>` is short enough.
5. A read-only bind of that directory is enough for a client inside a sandbox to connect (probed
   with a Python client and server).
6. Nested bubblewrap works and Codex's own workspace-write sandbox stacks inside ours (probed on
   0.165, 2026-10-28; re-check on 0.167.1 with a real tool call is a post-implementation check).

**Alternatives rejected.** A short socket path (the owner's first choice; impossible per fact 1).
Binding the host's `/tmp/codex-daemon-<uid>` into every session (all sessions would share one
directory and could replace each other's sockets, which is the exact flaw F31 names). Starting the
app-server inside the PTY sandbox instead of from the daemon (breaks `--remote` and the daemon's
delivery channel).

## 2. Design

Both Codex processes run under the **same** `Plan` (same mounts, hides, env unsets, notice). They
differ only in one mount: a per-session directory `<state_dir>/codex/<session id>/` (the *socket
dir*) appears at `/tmp/codex-daemon-<uid>` inside each sandbox, read-write for the app-server and
read-only for the TUI. `<uid>` is the daemon's effective uid (`rustix::process::geteuid`). The
app-server listens on `unix:///tmp/codex-daemon-<uid>/app.sock`; Codex then creates the real
socket `<hash>` beside it and `app.sock -> /tmp/codex-daemon-<uid>/<hash>`. The TUI gets
`--remote unix:///tmp/codex-daemon-<uid>/app.sock`. The daemon, outside, reaches the socket
through the socket dir (§2.3).

### 2.1 `src/confine.rs`

- `PlanBuilder::session_harness_root` returns `~/.codex` for `Harness::Codex`. In
  `plan_with_preflight` the automatic `harness-config` grant, and in `bind_harness_config` the
  harness check, cover Codex like Claude and OMP (owner decision 2026-11-12: `~/.codex` read-write
  and persistent by default, `auth.json` visible; the existing default-hide exemption at
  `src/confine.rs:1297` already keeps it visible). The error text `needs a Claude or OMP session`
  names Codex too. `harness_data` stays empty for Codex because the whole root is bound.
- Remove `check_harness`'s Codex refusal (`:1093`) and the `launch_argv` Codex refusal (`:2532`).
  `launch_argv` for Codex returns `plan.wrap(argv)` with no appended system prompt (the notice
  reaches Codex through `developer_instructions`, §2.2).
- New `Plan` state `codex_dir: Option<PathBuf>`, set by `Plan::with_codex_dir(PathBuf)` (builder
  style, like `with_bwrap`). When set, `wrap` adds `--ro-bind <dir> /tmp/codex-daemon-<uid>` as the
  **last mount option, immediately before `--chdir`** (`src/confine.rs:~2093`), so no operator
  `write`, `hide` or `unhide` entry at or above `/tmp` can cover it; new
  `Plan::wrap_app_server(argv)` is `wrap` with `--bind` (read-write) instead. One private helper
  builds the shared argument vector; the two public methods differ by the bind kind only. A plan
  without `codex_dir` produces exactly today's argv.
- The auto-grant block in `plan_with_preflight` (`:1138-1145`) has an `unreachable!()` arm for
  Codex that becomes the `~/.codex` root. Tests that pin the changed texts:
  `tests/confine.rs:6318` (`needs a Claude or OMP session`, update the expected text) and
  `tests/confine.rs:3010` (`codex_is_refused_and_write_guards_are_reciprocal`: rewrite the Codex
  half to assert acceptance, keep the write-guard half).
- `CODEX_HOME` is not honoured: the root is `~/.codex`. A session launched with `CODEX_HOME`
  elsewhere gets that directory read-only; `docs/confinement.md` says so.

### 2.2 `src/daemon.rs`, `src/codex.rs`

- Order in the new-session handler: build the confinement plan **before** `prepare_codex` (today
  after). A plan failure returns before any app-server exists; a `prepare_codex` failure after a
  plan leaves nothing to undo. Drop the refusal at `:547`.
- One function derives the socket dir: `codex::socket_dir(state_dir, session_id) -> PathBuf`
  (`<state_dir>/codex/<id>`). Right after `plan_confinement` succeeds for a Codex session the
  handler does `plan = plan.with_codex_dir(codex::socket_dir(..))`; that single plan value feeds
  both `codex::start` (via `prepare_codex`, which receives `Option<&Plan>`) and `SessionSpec.confine`
  (so the TUI's `launch_argv` -> `plan.wrap` carries the read-only mount). Without this the TUI's
  sandbox has no `/tmp/codex-daemon-<uid>/app.sock` and cannot connect.
- `codex::start` for a confined session: creates the socket dir (0700, as today's dir) and removes
  stale entries in it without following links (`remove_dir_all` semantics, then recreate), runs
  `plan.wrap_app_server([program, "app-server", "--listen", "unix:///tmp/codex-daemon-<uid>/app.sock", ...config])`,
  and `server_config` gets the confinement notice appended to `developer_instructions`
  (`format!("{peer prompt}\n\n{notice}")`, `confine::notice(&plan)`; when there is no peer prompt
  the notice alone). The unconfined start is byte-for-byte unchanged. Sizes are within the plan's
  existing limits (`NOTICE_LIMIT` 12 KiB, `MAX_ARG_BYTES` 128 KiB); `wrap` already fails cleanly
  past them.
- **The confined app-server is spawned on the dedicated `a2amx-spawn` worker thread**, not on a
  tokio or blocking-pool thread: `--die-with-parent` binds to the forking *thread* (spec 3ax), and a
  blocking-pool thread that idles out would kill the app-server. Generalise the worker's job
  (`SpawnTask`, `src/session.rs:36-60`, typed to return `Session` today) into a
  `Box<dyn FnOnce() + Send>` that owns its reply, keeping `catch_unwind`, so
  `Session::spawn_confined` and the app-server spawn share it. The async caller captures
  `tokio::runtime::Handle::current()`, moves it into the job, and the job does
  `let _guard = handle.enter(); command.spawn()` (tokio's `Command::spawn` needs a reactor). Only
  the spawn moves; the readiness loop stays async on the returned `Child`.
- `Link` keeps today's `socket` for the unconfined path and gains the endpoint for the confined
  one. A small `Endpoint` type (`Path(PathBuf)` or `Confined { host_dir: PathBuf, guest: String }`)
  replaces `socket: PathBuf`; `Rpc::connect(&Endpoint)`; `wire_argv` takes the guest URL.
  Callers to update: `src/codex.rs:401,524`, `src/delivery.rs:259`, `src/daemon.rs:784`.
- Confined connect (the daemon, outside; the agent controls everything in the dir): open
  `host_dir` with `O_DIRECTORY | O_NOFOLLOW`; `readlinkat` `app.sock` in it; require the target to
  be exactly `/tmp/codex-daemon-<uid>/<name>` with `<name>` one normal path component (no `/`, not
  `.` or `..`, at most 80 bytes); `openat(dirfd, name, O_PATH | O_NOFOLLOW)`; `fstat` that fd and
  require `S_ISSOCK`; then connect to `/proc/self/fd/<that fd>`. The agent-named path is never
  resolved a second time: connecting through `<dirfd>/<name>` would follow a symlink the agent
  planted as `<name>` (probed 2026-11-12: `/proc/self/fd/<dirfd>/evil` followed a link to an
  outside listener, while `O_PATH|O_NOFOLLOW` returned a non-socket and the real entry connected
  through its `O_PATH` fd). Anything else is a connect error naming "unexpected socket link"; a
  failed attempt retries like today's startup loop until the 30 s deadline. Use `rustix` for the
  `openat`, `readlinkat` and `fstat` calls.
- Each confined `start` removes everything in the socket dir without following links
  (`remove_dir_all` semantics, then recreate 0700) before spawning. The dir is not removed when the
  session ends (`// shortcut:` comment naming that: dirs pile up per session id until the next
  session with the same id clears one; remove at `Link::kill` if it matters).

### 2.3 `src/team.rs`, `src/session.rs`

- `validate_session_confine_options` drops the Codex refusal (`src/team.rs:1113`).
- `Session::spawn` needs no Codex branch beyond `launch_argv` (`src/session.rs:393`); the Codex
  link stays in `SessionSpec.codex`.

### 2.4 Docs

Every place that says Codex cannot be confined or names only Claude and OMP as harness-root
sessions: `README.md:15`; `docs/confinement.md:9,52,213,288,309,322` (the `~/.codex` root, the
`/tmp/codex-daemon-<uid>` mount, the `CODEX_HOME` note, the app-server note); `docs/harnesses.md:60,62,79`
(the notice reaches Codex through `developer_instructions`, not an appended system prompt);
`docs/remote-messaging.md:36-37` (Codex agents can use remote messaging when confined);
`docs/permissions.md:73` (eligibility condition 5); `docs/teams.md:143,150-151`;
`docs/backlog.md` (F31 moves to Closed; F52 keeps the `codex` grant error). Confirm the list with
`rg -n -i 'codex' docs README.md` for the words "cannot be confined", "refused", "not supported".

## 3. Trust boundaries

The confined agent controls the contents of its socket dir (read-write bind in the app-server).

| Attack path | Disposition | Abuse-case test |
| --- | --- | --- |
| `app.sock` replaced by a link to another path (`../../x`, an absolute path, a path in another session's dir) to make the daemon connect elsewhere | Constrained: only the final component of an exact-prefix target is used, opened relative to the dir fd | Fake app-server plants each hostile link; the daemon reports "unexpected socket link" and never connects outside the dir |
| `<hash>` planted as a symlink (or non-socket) pointing at a host socket outside the dir (another session's app-server, an agent socket) | Constrained: `openat(O_PATH\|O_NOFOLLOW)` plus `S_ISSOCK`, then connect through that fd; no second path resolution | A host listener outside the dir, a `<hash>` link to it: the listener never sees a connection and the daemon reports "unexpected socket link". Red on a naive `<dirfd>/<name>` connect |
| Link or socket planted in the dir by an earlier session with the same id (daemon restart) | Removed: stale contents are removed without following links before the app-server starts | Pre-populate the dir with a hostile link, start a session, entry gone |
| TUI sandbox writes the dir | Constrained: `--ro-bind` | Argv test asserts the TUI mount is read-only |
| Another session reaches this session's socket | Removed: only this session's dir is bound; no shared directory | Argv test asserts the bind source is the session's own dir |
| The agent runs its own server on the socket | Monitored: it is its own session; the daemon speaks the existing protocol with its existing frame bounds | None (unchanged) |

## 4. Claims about existing code

| Claim | Where | Verified |
| --- | --- | --- |
| Codex refused at four sites | `src/confine.rs:1093,2532`, `src/daemon.rs:547`, `src/team.rs:1113` | read (`rg 'not supported for Codex'`) |
| `prepare_codex` runs before `plan_confinement` | `src/daemon.rs:577-592` | read |
| App-server spawned with tokio `Command`, `kill_on_drop`, `env_clear` | `src/codex.rs:375-386` | read |
| `Link.socket` consumers | `src/codex.rs:401,524`, `src/delivery.rs:259`, `src/daemon.rs:784` | read (`rg 'Rpc::connect\|\.socket\(\)'`) |
| Confined sessions spawn on a dedicated worker thread | `src/session.rs:36-60,373` | read |
| `~/.codex/auth.json` hidden except for Codex | `src/confine.rs:1297` | read |
| `harness_data`, `ensure_data_*`, `validate_harness_data_paths` return nothing for Codex | `src/confine.rs:416,656,593` | read |
| Tests that pin the refusal | `tests/confine.rs:3010`, `tests/confine.rs:6318`, `tests/team.rs:1287` | read (`rg`; the last two by the consultant) |
| The fake Codex in tests links the requested path to a short socket | `tests/codex.rs:23-32` | read |
| Remote-messaging code tests only `session.confined()` and the mode, never the harness | `src/daemon.rs:1604-1650`, `:2149` counts Codex as native receipt | read (consultant) |
| A confined `write = ["/tmp"]` is accepted and emitted after `--tmpfs /tmp` | `src/confine.rs:299-310,2034` | read (consultant); hence the last-mount rule |
| `/proc/self/fd/<dirfd>/<link>` follows a planted symlink; `O_PATH\|O_NOFOLLOW` plus `fstat` rejects it and connects through the fd | n/a | probed 2026-11-12 (Python) |
| Real Codex 0.167.1 behaviour in §1 facts 1-5 | n/a | probed 2026-11-12 |

## 5. Out of scope

The `codex` harness grant (F52); changing Codex's own sandbox or approval settings; the unconfined
Codex path; an ignored real-Codex smoke test (U4); restructuring `src/confine.rs` (about 2,500
lines; the change adds small branches to code that already owns the harness decisions, so it
stays there); version and changelog (architect).

## 6. Acceptance

Run all as an ordinary user. Pre-existing: `gateway_up` and `human_inbound` queued-recheck tests
may fail in a confined session (backlog I24); record, do not chase.

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`: judge is the
   command output.
2. New tests (names are the developer's; each must fail on the old code except the contract
   tests named): argv tests for `Plan::wrap` and `Plan::wrap_app_server` with and without
   `codex_dir` (a plan without it equals today's argv: *contract test*; with it the bind is
   read-only for `wrap`, read-write for `wrap_app_server`, its source is the session's own dir and
   it is the last mount option, also with an operator `write = ["/tmp"]`); `launch_argv` for Codex
   wraps and appends nothing; `plan(Harness::Codex)` binds `~/.codex` writable with `auth.json`
   visible; the tests at `tests/confine.rs:3010`, `tests/confine.rs:6318` and `tests/team.rs:1287`
   are rewritten as §2.1 says; and a daemon test with the fake Codex under real bubblewrap
   (`A2AMX_REQUIRE_BWRAP=1`, serial, like the existing ignored confinement tests). Workable shape
   for the daemon test: the fake script links `app.sock` to `/tmp/codex-daemon-<uid>/fake` (the
   `$3` URL is the in-sandbox path), writes its `ARGS_DIR` files under the session's cwd (writable
   in the sandbox), and the test, while `NewSession` is pending on another thread, waits for the
   link to appear in `<state>/codex/<id>/` and then binds `FakeCodex` at `<dir>/fake`; the daemon's
   connect loop retries until it does. Asserted: the session is created, a delivery reaches the
   fake app-server, the server's `developer_instructions` argument contains the confinement
   notice, both processes see a pid namespace different from the daemon's
   (`readlink /proc/self/ns/pid`; a pid number proves nothing because of `--unshare-pid`), and the
   app-server process is gone after the session exits. The abuse cases of §3 (both rows plus the
   stale-entry removal) are tests of the confined connect through the daemon with the same fake.
3. Mutation check by the developer on the checkout (restore by checksum; tree equals its own
   diff afterwards): break each of (a) the read-only flag of the TUI bind, (b) the exact-prefix
   target check, (c) the `O_NOFOLLOW`/`S_ISSOCK` check (revert to connecting through
   `<dirfd>/<name>`), (d) the stale-entry removal, (e) the notice append, (f) the plan-before-start
   order, (g) the last-mount placement; name the repo test that fails on each.
4. Post-implementation, by the tester on an isolated daemon with a real login (judge: the
   tester's report with command output): a confined real Codex 0.167.1 session starts and its
   TUI connects with the **read-only** socket-dir mount (if the real TUI cannot start with it,
   report BLOCKED; the architect then amends the TUI mount to read-write, its own dir only); the
   model, asked what restrictions it has, repeats the confinement notice; a model shell command
   works; a write outside the grants is denied; a message arrives and is received; the app-server
   is still alive after 60 s idle and gone after the session exits.
5. `git status --short` shows only the files in §0 scope.

## Review record

Pending.
