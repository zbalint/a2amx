# Spec 3eg: one root directory, `~/.a2amx`, with `config/ state/ run/ log/` below it

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
Shared task `context_id`: `a2amx-config-layout-2026-11-13`.
Plan file for the developer: `target/scratch/3eg/plan.md` (gitignored; scratch only under `target/scratch/3eg/`, deleted before the report).

This is spec 1 of 7 in the owner's configuration redesign (root and layout; config loader and
`daemon.toml`; gateway config; client config; team file rules; per-project auto grants; migration;
presence per session). It is a **pure relocation**: no file format changes, no new config file, no
migration. The release that carries it is cut only after the migration spec (spec 6) is accepted;
nothing here is released alone.

**Scope.** May be edited: `src/layout.rs` (new), `src/lib.rs` (one `pub mod layout;` line),
`src/main.rs`, `src/cli.rs`, `src/daemon.rs`, `src/client.rs`, `src/store.rs`, `src/codex.rs`,
`src/omp.rs`, `src/relay_files.rs`, `src/relay.rs`, `src/gateway_proc.rs`,
`src/gateway_launch.rs`, `src/confine.rs`, `src/team.rs`, `scripts/install.sh`, every test file under
`tests/` that names a moved path or the changed texts (section 7), the docs named in section 6, and
(Amendment 1) `SECURITY.md` and `a2amx.toml.example`.
**Does not touch:** `CHANGELOG.md`, `docs/backlog.md`, `AGENTS.md`, `Cargo.toml`, `docs/specs/`
(the architect edits these), `src/messaging.rs`, `src/mcp.rs`, `src/wire.rs`, `src/session.rs`,
`src/hook.rs`, `src/bridge.rs`, `extension/`, every other file. If a file outside this list turns out
to name a moved path, stop and report `BLOCKED — SPEC ADJUDICATION REQUIRED`.

**Public test seams.** The `layout` module's public functions; the daemon through
`Daemon::start(DaemonConfig)`, `Client::connect(<root path>)` and the real binary spawned by
`tests/common`; `confine::Plan` argv for confinement; `gateway_launch::sandbox_argv` for the gateway.
No test calls a private function.

## 1. Why

Owner decision (2026-11-13): the project needs one configuration directory, a hidden directory in the
user's home, `~/.a2amx`, and everything now under `~/.local/state/a2amx` moves below it in
subdirectories. The state directory location was picked by the architect without asking and the
owner never wanted it. Today `~/.a2amx` holds only the installed binary (`~/.a2amx/bin/a2amx`), and the
state directory is a flat mix of operator-written configuration, a secret, durable data, per-run
files and logs. Separating them is the base for the next specs (a config loader, per-component config
files, a migration), and it fixes what confinement can expose: configuration and logs readable, data
and secrets hidden.

Owner decisions this spec carries out:

- The only root is `~/.a2amx`, or `A2AMX_HOME` (env), or `--home`. `XDG_STATE_HOME` is no longer read.
- `A2AMX_HOME` and `--home` name the **root** of the whole tree, `bin/` included. The installer honours
  `A2AMX_HOME`; `A2AMX_INSTALL_DIR` goes away.
- Subdirectories: `bin/` (the installed binary), `config/` (written by the operator), `state/`
  (durable data the daemon and gateway write), `run/` (per-run files, deleted safely while no daemon
  runs), `log/`.
- Confined sessions: `bin/`, `config/` and `log/` are **read-only**; `state/`, `run/` and
  `config/secrets/` are **hidden**. The owner said configuration files hold no secrets except the
  `secrets/` subdirectory, and that agents can read messages anyway, so logs may be read.
- `discord.token` is written by `a2amx gateway token` (`src/main.rs:290-295`), not by the user; it
  lives in `config/secrets/`.

Alternatives rejected while grilling: keeping the state directory and adding `~/.a2amx` only for
config (two roots, the thing the owner objected to); `XDG_STATE_HOME` support (the owner dislikes the
location); hiding logs from sessions (the owner accepts that agents read them; the sampled `tracing`
calls log errors, ids and counts, not message bodies, and a test below pins that for the messages path
this spec touches).

## 2. The `layout` module (`src/layout.rs`, new)

One module owns every path under the root. No other module joins a literal file or directory name
below the root (acceptance 5). It does not read or write files except in `ensure`.

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout { root: PathBuf }
```

Constructors:

- `pub fn new(root: PathBuf) -> Layout`: wraps an already-resolved root.
- `pub fn resolve(explicit: Option<PathBuf>, user_home: Option<PathBuf>) -> anyhow::Result<Layout>`:
  `explicit` (the `--home` value; clap already folds `A2AMX_HOME` into it, `src/cli.rs:14`), else
  `user_home.join(".a2amx")`, else the error `cannot determine the a2amx directory: set HOME or A2AMX_HOME`.
  An empty explicit path is an error (`the a2amx directory must not be empty`).

Directories (each returns `PathBuf`; `root()` returns `&Path`):

| Method | Path |
| --- | --- |
| `root()` | `<root>` |
| `config_dir()` | `<root>/config` |
| `secrets_dir()` | `<root>/config/secrets` |
| `state_dir()` | `<root>/state` |
| `gateway_state_dir()` | `<root>/state/gateway` |
| `run_dir()` | `<root>/run` |
| `link_dir()` | `<root>/run/link` |
| `codex_dir()` | `<root>/run/codex` |
| `codex_session_dir(id: &str)` | `<root>/run/codex/<id>` |
| `omp_dir()` | `<root>/run/omp` |
| `log_dir()` | `<root>/log` |

Files:

| Method | Path | Was (relative to the old state dir) |
| --- | --- | --- |
| `addr()` | `run/addr` | `addr` |
| `admin_token()` | `run/admin.token` | `admin.token` |
| `gateway_token()` | `run/gateway.token` | `gateway.token` |
| `daemon_lock()` | `run/daemon.lock` | `daemon.lock` |
| `gateway_lock()` | `run/gateway.lock` | `gateway.lock` |
| `gateway_pid()` | `run/gateway.pid` | `gateway.pid` |
| `link_addr()` | `run/link/addr` | `link/addr` |
| `link_gateway_token()` | `run/link/gateway.token` | `link/gateway.token` |
| `confine_empty()` | `run/confine-empty` | `confine-empty` |
| `messages_db()` | `state/messages.db` | `messages.db` |
| `auto_grants()` | `state/auto-grants` | `auto-grants` |
| `pairing()` | `state/gateway/pairing.json` | `gateway/pairing.json` |
| `seen()` | `state/gateway/seen.json` | `gateway/seen.json` |
| `team_file()` | `config/team.toml` | `team.toml` |
| `gateway_config()` | `config/gateway.toml` | `gateway/config.toml` |
| `discord_token()` | `config/secrets/discord.token` | `gateway/discord.token` |
| `daemon_log()` | `log/daemon.log` | `daemon.log` |
| `gateway_log()` | `log/gateway.log` | `gateway/gateway.log` |

`messages.db-wal` and `messages.db-shm` are SQLite siblings of `messages.db` and need no accessor.
File names that are not in this table (the Codex socket file names, `extension-<version>.ts`,
`overlay.yml` inside `run/omp`) stay where they are built today (`codex.rs`, `omp.rs`) but are joined
onto `codex_session_dir`/`omp_dir`, never onto the root.

`pub fn ensure(&self) -> anyhow::Result<()>`: the only function that creates directories under
the root. For the root and then `config`, `config/secrets`, `state`, `state/gateway`, `run`, `run/link`,
`log` in that order, create each directory that is **missing** with mode `0700` and an explicit
`chmod 0700` after creating it (the loop in `daemon::ensure_state_dir`, `src/daemon.rs:2285-2311`,
moves here; `ensure_state_dir` is deleted and there is no second public ensure function). A directory
that already exists is not touched: its mode stays as it is (the installer creates `~/.a2amx` with mode
`0755`; `ensure` must not change it), and when every directory exists `ensure` performs no write, so it
is safe to call where the filesystem is read-only. It does not create `bin/`.

## 3. Resolution and call shape

- `main.rs`: `resolve_home` (`src/main.rs:275-289`) is replaced by `Layout::resolve(home_arg, HOME)`.
  The `XDG_STATE_HOME` branch is deleted. The text in `src/cli.rs:13` becomes
  `a2amx directory. Defaults to A2AMX_HOME, else $HOME/.a2amx.`
- **Edges take a root path, code below takes `&Layout`.** `DaemonConfig.state_dir` is renamed `root`
  (`src/daemon.rs:56`); `Client::connect` and `relay_files::load`/`write_*` keep
  taking the root `&Path` and build `Layout::new(root.to_owned())` once (`src/client.rs:46`,
  `src/relay_files.rs:98-175`). **`Client::connect_gateway` takes `&Layout`**: the gateway process
  has no root of its own inside its sandbox, only `run/link`, and `src/relay.rs:31,133,189` pass the
  link directory today. It reads `layout.link_addr()` and `layout.link_gateway_token()`; `Client::connect`
  reads `layout.addr()` and `layout.admin_token()`. `connect_with` takes the two file paths. Every function below an edge that today takes `home: &Path` or
  `state_dir: &Path` to join a moved name takes `&Layout` instead. No code outside `layout.rs`
  names `state_dir` for the root: a variable that holds the root is `root` or `layout`, because
  `state/` now exists and a `state_dir` that means the root would be a trap. Rename mechanically; do
  not rename unrelated variables.
- `DaemonConfig.state_dir` (`src/daemon.rs:56`), `Daemon.state_dir` (`:76`) and `Runtime.state_dir`
  (`:90`) are replaced by `root` and two `layout: Layout` fields (uses at `:416-438,582,775,2332-2430,
  2454,2560,2650-2651`).

## 4. Module changes

Each row replaces a literal join with a `Layout` method; behaviour is otherwise unchanged.

| File | Change |
| --- | --- |
| `src/daemon.rs` | `ensure_state_dir`/`ensure_empty_file` calls (`:2334-2335`) become `layout.ensure()` and the confine-empty file under `run/`; lock `layout.daemon_lock()` (`:2342`); tokens and link files via the table, including the compound literals `link/gateway.token` (`:2409`) and `link/addr` (`:2651`); `addr` (`:2650`); `Host` built from the layout (`:431`); `codex::session_dir` (`:582`) and `codex::start(&self.state_dir, ..)` (`:775`) take the layout. The lock error text `another a2amx daemon is running for this state dir` becomes `another a2amx daemon is running for this a2amx directory`. |
| `src/main.rs` | `daemon.log` (`:480`) becomes `layout.daemon_log()`; `auto-grants` (`:79,84`); `addr` (`:550`); the global team file (`:1210`) becomes `layout.team_file()`; `Host` construction (`:1494-1498`, `:1567-1574`) from the layout; `confine-empty`; `write_token` call (`:295`). `daemon start` (`:485`, the only caller there) calls `layout.ensure()`. `run_gateway_token` calls `layout.ensure()` before `write_token`. |
| `src/client.rs` | `addr` and token file names (`:46-57`) via the layout; the doc comment at `:2` says `a2amx directory`. |
| `src/store.rs` | `messages.db` (`:108`) via the layout (`store::open` takes `&Layout`; it already takes `state_dir: PathBuf`, so the type changes with the parameter). |
| `src/codex.rs` | `session_dir`/`socket_dir` (`:50-55`), `start_unconfined`'s `state_dir.join("codex")` (`:477`), `start_confined` (`:543-556`) use `codex_dir()`/`codex_session_dir(id)`. The unconfined socket file stays `<codex_dir>/<id>.sock`. |
| `src/omp.rs` | `home.join("omp")` (`:28`) becomes `layout.omp_dir()`; `install` takes `&Layout`. |
| `src/relay_files.rs` | `gateway_dir`, `config.toml`, `discord.token`, `pairing.json` and `link_dir` (`:99-175`) via the layout. `load` (`:98`) no longer creates any directory: it runs inside the gateway sandbox (`src/main.rs:378`), where `config/`, `config/secrets` and `run/link` are read-only, and its `ensure_state_dir(&gateway_dir)` is deleted; `write_token` and `write_pairing` call nothing that creates directories either (`gateway up` and `gateway token` call `layout.ensure()` on the host first). `Loaded` keeps `layout: Layout` and drops `home`, `gateway_dir` and `link_dir`. |
| `src/relay.rs` | `loaded.gateway_dir.join("seen.json")` (`:49`) becomes `loaded.layout.seen()`; `Client::connect_gateway(&loaded.link_dir)` (`:31,133`) and the `link_dir` field (`:51,166,189`) become `&loaded.layout` and `layout: Layout`. |
| `src/gateway_proc.rs` | the constants `LOCK_FILE`, `PID_FILE`, `LOG_FILE` (`:11-13`) are deleted and their uses (`:53,80,85,99,170,261`, and `LOG_FILE` in `gateway_launch.rs:241,255`) go through `gateway_lock()`, `gateway_pid()`, `gateway_log()`; `open_log` opens `layout.gateway_log()` (its parameter is no longer a gateway directory); `--home` argument construction (`:228-230`) passes the root; the three `for this state directory` texts (`:210,221,240`) become `for this a2amx directory`. |
| `src/gateway_launch.rs` | `link` and `gateway` paths and the `gateway.pid`/`link` joins (`:59-60,82,121,175,180,216,254,260,289`) via the layout; the confined argv per section 5.3; `up` (`:92`) calls `layout.ensure()` before `relay_files::load`; `check_link_files` (`:289-294`) reads `link_addr()` and `link_gateway_token()`; `DAEMON_REQUIRED` becomes `the a2amx daemon is not running in this a2amx directory; start it first`; `cannot resolve state directory` becomes `cannot resolve the a2amx directory`. `Layout` in this file (`:14-22`, a different struct that carries the sandbox's inputs) is renamed `LaunchPlan` to avoid a clash with `layout::Layout`; its `state_dir` field becomes `layout: crate::layout::Layout`. |
| `src/confine.rs` | per section 5. |
| `src/team.rs` | `check_global_allowed` and `system_reason` (`:307-421`): the parameter `state_home` is the root; the reason text `the daemon state directory or a system directory` becomes `the a2amx directory or a system directory` at both returns (`:404,421`). |
| `src/cli.rs` | `--home` doc (`:13`); the four `global team.toml in the state directory` doc strings (`:272,278,295,307`) say `the global team file in the a2amx config directory`. |
| `scripts/install.sh` | `dest="${A2AMX_HOME:-$HOME/.a2amx}"`. Nothing else changes. |

## 5. Confinement

`confine::Host` (`src/confine.rs:183-189`) becomes `Host { home: PathBuf, layout: Layout, runtime_dir: PathBuf }`;
the field `empty_file` is removed and `layout.confine_empty()` replaces it (`src/main.rs:1498,1574`,
`src/daemon.rs:431`). `Plan` keeps a `layout` in place of `state_dir` (`:195,687,1224,1310`).

1. **Mandatory hides.** `hide_mandatory` (`:1513`) hides `layout.state_dir()`, `layout.run_dir()` and
   `layout.secrets_dir()` where it hid the state directory. The three are added together with
   `runtime_dir` to `mandatory_hidden`. `Host` and `Plan` hold `Layout::new(<canonical root>)`
   (`canonical_existing`, as `Plan` canonicalizes the state directory today, `:1310`), and a missing
   `state/`, `run/` or `config/secrets` makes the plan fail closed (bubblewrap cannot create a mount
   point under the read-only root), so `ensure` must have run; the rest of the root (`bin/`, `config/` except `secrets/`,
   `log/`) stays visible, read-only like the rest of the filesystem (`--ro-bind / /`).
2. **Overlap checks.** Wherever the state directory is passed today, pass `layout.root()`:
   `check_write_allowed` (`:295-330`), `check_unhide_allowed` (`:354-382`), `validate_harness_data_paths`
   (`:599`, its call at `:653` and `:1382`), `check_grant_path_writable` (`:1550`), the `check_unhide_allowed`
   call at `:1726`, `GrantDirScope` and `grant_dir_problem` (`:684-760`), `plan_empty_file` (`:1127`),
   the `Plan` clone (`:1845`) and the two `#[cfg(test)]` `Host` literals (`:2565,2762`). The existing
   explicit/non-explicit split stays: an explicit path is rejected when it is inside the root, a
   non-explicit one when it overlaps it. `state/`, `run/` and `config/secrets` all lie inside the root, so
   one check covers them. Error texts: `writable path {} overlaps the state directory` becomes
   `writable path {} overlaps the a2amx directory` and the unhide text likewise (`:314,367`). The
   consequence is intended: an explicit `write` entry inside the root (for example `~/.a2amx/config`)
   is rejected, and a path that merely contains the root is rejected only when it is non-explicit, as
   `$HOME` is today (`:305`).
3. **Protected root.** Where the existing rule makes `~/.a2amx` read-only because it lies inside a
   writable path (`protect_system_paths`, `:1490-1496`, and the grant check at `:1552`), the same rule
   applies to `layout.root()`; the `~/.a2amx` special case is kept as it is (it protects an install
   that is not the root).
4. **Empty file.** `plan_empty_file` (`:976-1000`) requires the empty file's parent to be
   `layout.run_dir()`; its error text `empty file must be inside the state directory` becomes
   `empty file must be inside the a2amx run directory`. `ensure_empty_file` (`:2316`) creates
   `layout.confine_empty()` and takes `&Layout`.
5. **OMP.** The read-only mount of the OMP overlay directory (`:1954`) uses `layout.omp_dir()`
   (it lies inside the hidden `run/`, a nested mount that the depth ordering at `:1963-1975`
   already places after the hide).
6. **Codex.** `Plan.codex_dir` is `layout.codex_session_dir(id)`; spec 3ed's bind plan is unchanged.
   The socket directory stays `<session dir>/sock`.
7. **Gateway argv** (`gateway_launch::sandbox_argv`, `:60-110`): where it bound `link` read-only and
   `gateway` read-write and used `gateway` as the working directory, it now binds exactly:
   `--ro-bind <run/link> <run/link>`, `--ro-bind <config/gateway.toml> <same>`,
   `--ro-bind <config/secrets/discord.token> <same>`, `--bind <state/gateway> <same>`, and
   `--chdir <state/gateway>`. The two files must exist (`relay_files::load` already requires them
   before launch, `gateway_launch.rs:92`). Nothing else under the root is visible to the gateway. `gateway token`
   (the host-side command that writes `discord.token`, `src/main.rs:290-295`) is unchanged in
   behaviour and runs unconfined on the host.

## 6. Docs

Rewrite the statements that name the old location or the old resolution rule, and only those:
`README.md:73` (path stays), `docs/install.md` (lines `:11,31,37,55-56`: the install directory is
`A2AMX_HOME`, default `~/.a2amx`), `docs/cli.md:186-187`, `docs/getting-started.md:3,11,101`,
`docs/remote-messaging.md` (every `<state>` path, the gateway file table at `:329-340`, and the
`$XDG_STATE_HOME` sentence at `:74`), `docs/confinement.md` (`:10,91,117,134,146,323`), `docs/teams.md`
(the global file location and the `state home` wording), `docs/architecture.md`, `docs/permissions.md`,
`docs/harnesses.md`, `docs/delivery.md` where a path or "state directory" names the old root.
Add one section to `docs/architecture.md`, "Directory layout", with the table in section 2 and the
confinement rule of section 5 in two sentences. `docs/roadmap.md` is not touched. The architect adds
the changelog entry (breaking: paths moved, `XDG_STATE_HOME` and `A2AMX_INSTALL_DIR` no longer read;
existing data is not moved until spec 6).

## 7. Tests

Update every test that names a moved path, the changed texts, or `XDG_STATE_HOME`
(`tests/attach_cli.rs:927`); `rg -l` over `tests/` for the section 8 patterns lists them (about 25 files,
mostly `tests/relay.rs`, `tests/gateway_background.rs`, `tests/gateway_up.rs`, `tests/team.rs`,
`tests/codex.rs`, `tests/confine.rs`, `tests/auto_confine.rs`, `tests/daemon.rs`). The changed
literals: `tests/daemon.rs:1467` (lock text), `tests/gateway_up.rs:535,547,559` (daemon-not-running text),
`tests/team.rs:285` (guard reason), `tests/confine.rs:929,969` (overlap text). Name every updated file in
the report. Add:

- **Layout unit tests** (`tests/layout.rs`, new): `resolve` precedence (explicit wins over HOME;
  HOME gives `<home>/.a2amx`; neither gives the error text; empty explicit is rejected); every accessor
  returns the literal path in the section 2 tables for root `/r`; `ensure` creates exactly the
  directories listed with mode `0700`, does not change the mode of an existing root created `0755`, and
  does not create `bin/`.
- **Overlap:** a confinement plan with an explicit `write = ["R/log"]` and one with `["R/config"]` is
  rejected with the text `writable path {} overlaps the a2amx directory` (new test in `tests/confine.rs`).
- **Daemon:** a daemon started on a fresh root creates `run/addr`, `run/admin.token`, `run/daemon.lock`,
  `run/link/addr`, `run/link/gateway.token`, `state/messages.db`, and creates no entry directly in the
  root except the directories of section 2 (the fixture's own `bwrap-path` directory, which
  `tests/common/mod.rs:54,349` creates in the root temp directory, is excluded; do not move it). The
  tests that read `dir.join("gateway.token")` (`tests/common/mod.rs:171`) and `DaemonConfig { state_dir }`
  (`:58,353`) are updated.
- **Confined session** (real `bwrap`; skips where `confine::preflight` fails, as the existing guest-side
  tests do): with root `R` containing `bin/x`, `config/team.toml`, `config/secrets/discord.token`,
  `state/messages.db`, `run/admin.token`, `log/daemon.log`: inside the session `R/bin/x`,
  `R/config/team.toml` and `R/log/daemon.log` are readable and writing to them fails; listing
  `R/state`, `R/run` and `R/config/secrets` shows no host files (hidden).
- **Gateway argv** (`gateway_launch::sandbox_argv`, existing test style): exactly the five binds and
  the `--chdir` of section 5.7 appear, and no bind of `R`, `R/config`, `R/state` or `R/log`.
- **Logs carry no message body** (`tests/daemon.rs` or `tests/messages.rs`, wherever the log is
  captured today): send a message with a unique marker body through the daemon and assert the marker
  is not in `log/daemon.log`. If the daemon's tracing is not captured by an existing fixture, record
  that and drop this test rather than building a fixture (the architect decides at review).

## 8. Out of scope

Migration of `~/.local/state/a2amx` (spec 6; this spec neither reads nor detects the old directory);
any config file, key or format (`daemon.toml`, `gateway.toml` keys, duration strings, the `_ms`
renames); the team file rules; `auto-grants` content or semantics (the file only moves); renaming
`Layout`'s accessors to fit later specs; the `a2amx config` command; per-session presence (spec 7, draft
3ef is parked); changing which messages or values the daemon logs; `docs/roadmap.md`.

## 9. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
   `scripts/prepush-fast.sh` pass; inside a confined architect session only the documented I24 pair may
   fail. Judge: the architect's fresh run.
2. No literal below-root name outside `src/layout.rs`:
   `rg -n '"(addr|admin\.token|gateway\.token|daemon\.lock|gateway\.lock|gateway\.pid|daemon\.log|gateway\.log|messages\.db|auto-grants|team\.toml|confine-empty|config\.toml|discord\.token|seen\.json|pairing\.json|link|codex|omp|gateway|secrets|config|state|run|log)"|"(link|gateway|codex|omp|run|state|config|log)/' src`
   matches nothing outside `src/layout.rs` except `#[cfg(test)]` modules and strings that are not
   paths. Known non-path hits at `54129ef`: `src/harness.rs:45-59` (harness names `omp`, `codex`) and
   `src/mcp.rs:357` (a JSON key `state`); the developer lists any other remaining hit and why in the
   report. Judge: the architect reruns it.
3. `rg -n "XDG_STATE_HOME|local/state|A2AMX_INSTALL_DIR" src scripts README.md docs --glob '!docs/specs/**' --glob '!docs/backlog.md'`
   matches nothing except the migration note in `docs/architecture.md` if one is written, and
   `CHANGELOG.md` is out of scope. Judge: the architect reruns it.
4. `cargo test --test layout` and `cargo test --test confine` pass; the layout tests and the confined
   tree test are green after and red on the old code (they need `src/layout.rs` and the new paths).
   Contract tests (already green on the old code): the gateway argv test is rewritten and fails on the
   old code, so it is not one.
5. Mutants (the developer breaks each in production code, names the repo test that fails, restores by
   checksum; its tree must equal its own diff afterward): (a) `resolve` prefers `HOME` to `--home`;
   (b) `addr()` returns `<root>/addr`; (c) `ensure` skips `chmod 0700` (killed by `newly_created_parents_are_private_even_with_a_restrictive_umask`, `tests/daemon.rs:2258`, which must exercise the daemon's `Layout::ensure`; the default umask alone cannot kill it); (d) `ensure` changes an existing
   root's mode; (e) `hide_mandatory` leaves `run_dir` out; (f) leaves `secrets_dir` out; (g) hides the
   whole root; (h) the gateway argv binds `config_dir` instead of the two files; (i) the overlap checks compare against `state_dir()` instead of the root (killed by the new test
   that an explicit `write` inside `R/log` and `R/config` is rejected).
6. Trust boundary: a confined session must not read the admin token, the gateway token, the Discord
   token or the message database. Abuse-case test: the confined-tree test of section 7 includes
   `cat R/run/admin.token`, `cat R/config/secrets/discord.token` and a read of `R/state/messages.db`,
   each failing in the session; it is red on the old code only because the old paths do not exist, so it
   is a contract test for the new tree and is named as such. The architect reads it.
7. Logs are readable by sessions now. The developer lists every `tracing` and `eprintln!` call in
   `src/daemon.rs`, `src/relay.rs`, `src/discord.rs`, `src/delivery.rs` that formats a value taken from a
   message body, a token or the Discord configuration; there must be none (the Discord token travels
   only in an `Authorization` header, `src/discord.rs:321-338`, and logged errors are `%error` values).
   Judge: the architect reads the list and the calls.
8. The plan file exists at `target/scratch/3eg/plan.md` before the first tracked edit, and
   `target/scratch/3eg/` is deleted before the report. Judge: the architect checks.

## 10. Claims verified against the tree (at `54129ef`)

| Claim | Where | How |
| --- | --- | --- |
| The state directory is resolved as `--home`, `A2AMX_HOME`, `$XDG_STATE_HOME/a2amx`, `$HOME/.local/state/a2amx`; clap folds `A2AMX_HOME` into `--home` | `src/main.rs:275-289`, `src/cli.rs:13-14` | read |
| `~/.a2amx` is already protected read-only and a grant path inside it is rejected | `src/confine.rs:1490-1496,1552-1556`, `docs/confinement.md:323` | read |
| The state directory is hidden from confined sessions | `src/confine.rs:1513-1527` (`hide_mandatory`) | read |
| Mounts are ordered by path depth, so a read-only mount under a hidden directory (OMP, Codex) works | `src/confine.rs:1963-1975,1954`, spec 3ed | read |
| `admin.token` and `gateway.token` are generated at every daemon start | `src/daemon.rs:2353-2354` | read |
| `discord.token` is written by the host-side `gateway token` command, `pairing.json` by the confined pairing step | `src/main.rs:290-295`, `src/relay_files.rs:159-176`, `src/gateway_launch.rs:110-125` | read |
| The gateway log is opened by the launcher before the sandbox starts, so the sandbox needs no `log/` | `src/gateway_launch.rs:216-230` (`open_log` in `start_background`) | read |
| The installer creates `~/.a2amx` with mode `0755` | `stat` on the owner's install (`755`) and `scripts/install.sh:7` (`mkdir -p`) | probed |
| About 330 test lines name a moved path or the changed texts | `rg` over `tests/` | grep (count by file in section 7) |
| Sampled `tracing` calls log errors, ids and counts, not message bodies; not every call was read | `src/daemon.rs`, `src/relay.rs`, `src/delivery.rs` | grep (partial; the logs test in section 7 pins the messages path) |
| The Codex legacy `s*.sock` files in the old `codex/` directory are not carried | `src/codex.rs:477-500` writes `<id>.sock`; the old files are left by older versions only | read (F92, not part of this spec) |
| The gateway connects to the daemon through the link directory, not the root: `Client::connect_gateway(&loaded.link_dir)` joins `addr` and `gateway.token` onto it | `src/relay.rs:31,133,189`, `src/client.rs:50-66` | read (consultant, confirmed) |
| `relay_files::load` runs inside the gateway sandbox, where `config/`, `config/secrets` and `run/link` are read-only | `src/main.rs:378`, `src/relay_files.rs:98-104`, `src/gateway_launch.rs:60-86` | read (consultant, confirmed) |
| An explicit writable path is rejected only when it is inside the state directory; a non-explicit one when it overlaps it; `$HOME` is rejected only for non-explicit paths | `src/confine.rs:295-320` | read |
| `daemon::ensure_state_dir` chmods only the directories it created | `src/daemon.rs:2285-2311` | read |
| The default umask cannot kill a "skip chmod" mutant; the restrictive-umask fork test can | `tests/daemon.rs:2258` | read (consultant) |
| `tests/common/mod.rs` creates `bwrap-path` inside the root temp directory | `tests/common/mod.rs:54,349` | read |
| The Discord token travels in an `Authorization` header and errors are logged as `%error` | `src/discord.rs:321-338`, `src/relay.rs:67-311` | read (log contents beyond the `%error` formatting not verified) |
| bubblewrap cannot create a mount point under a read-only root, so the hidden directories must exist on the host | consultant's knowledge of bubblewrap, not probed | recall (covered by `ensure` and the fail-closed plan) |

## Amendment 1 (developer BLOCKED m_6945, verified by the architect)

The developer's scoped search found three files outside section 0 that name a moved path:
`AGENTS.md:155` (`<home>/gateway/` for the gateway's config, token and pairing, and `:114` "temp state
dir"), `SECURITY.md:29` (`<state>/link/`) and `a2amx.toml.example:47` (`<state home>/team.toml`). The
architect re-ran a repository-wide search (all files outside `target/`, `docs/specs/`, `docs/backlog.md`,
`CHANGELOG.md`, `src/`, `tests/`) for `<home>/`, `<state>`, `state home`, `/link/`, `local/state`,
`state dir`, `XDG_STATE`, `INSTALL_DIR`: those three files are all, plus `docs/*.md` pages that section 6
already covers (`delivery.md:581`, `teams.md:9,126,131,223`, `permissions.md:43,92,174,247`,
`harnesses.md:42,71,132`, `architecture.md:271,302,343,410,427,437-444`).

Decision:
- `SECURITY.md` and `a2amx.toml.example` are added to the edit scope; the change is wording only:
  `<state>/link/` becomes `<root>/run/link/`, and `<state home>/team.toml` becomes
  `<root>/config/team.toml` (and `a2amx.toml.example:47` keeps its meaning: `auto` is read from any team
  file only after the later spec; until then the comment names the global file's new location).
- `AGENTS.md` stays out of scope. The architect edits it when committing (the `relay_files` row and the
  "temp state dir" line, and a `layout` row in the Modules table); the developer does not.
- No other section changes. Judge for the added files: the architect reads the diff; the section 9 greps
  gain no new match because they cover `src scripts README.md docs`; run
  `rg -n "<home>/|<state>|state home|local/state" SECURITY.md a2amx.toml.example` and it must match nothing.

## Amendment 2 (tester BLOCKED m_7054, verified by the architect)

The tester found three gaps between the diff and the locked text; all three are real.

- **`src/delivery.rs` joins the scope** (test fixture only, `src/delivery.rs:586-588`: it builds a `Layout`,
  calls `ensure()` and passes `&Layout` to `Store::open`). The `Store::open(&Layout)` signature change
  of section 4 forces it; the architect's scope search covered the callers of the moved paths but not
  the callers of `Store::open` inside `#[cfg(test)]` modules. No production code in `src/delivery.rs`
  changes.
- **`<root>` wording.** `SECURITY.md:29` and `a2amx.toml.example:47` say `<home>/run/link/` and
  `<home>/config/team.toml`; Amendment 1 says `<root>/...` and its grep
  `rg -n "<home>/|<state>|state home|local/state" SECURITY.md a2amx.toml.example` must match nothing.
  The two lines change to `<root>/run/link/` and `<root>/config/team.toml`.
- **`docs/architecture.md` "Directory layout"** (section 6) is a required deliverable and is missing:
  add it with the section 2 table and the two-sentence confinement rule.

No other section changes. Judge: the architect reads the three diffs and reruns the Amendment 1 grep.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review; the exact model was not recorded); developer OMP harness, planning, working and advisor models not stated in its report; tester OMP harness, deepseek-v4.1-flash. BLOCKED/amend cycles: 2 (developer m_6945, tester m_7054).

| Found at | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | Ten findings (F1-F10) in the first draft, among them a false claim that a path containing the root is rejected like `$HOME` (the explicit branch only uses `is_within`), `connect_gateway` needed `&Layout`, `ensure` must not run inside the sandbox, the acceptance grep missed compound literals, mutant (c) needed the restrictive-umask fork test | applied to the spec before locking |
| Implementation | developer | BLOCKED m_6945: `AGENTS.md`, `SECURITY.md` and `a2amx.toml.example` name a moved path but were outside the scope (the architect's scope search skipped root files) | Amendment 1 |
| Implementation | developer | Reported a premature "BLOCKED - QUOTA", reverted its own wiring once, deferred required items to the architect | one-item assignments; rejected the deferrals |
| Architect review | architect, Sonnet 5.5 | `confine.rs` overlap guards narrowed from `layout.root()` to `state_dir()`, which would let an explicit write or unhide inside `R/config` or `R/log` pass | reverted to `layout.root()`; new test `explicit_writes_inside_a2amx_root_are_rejected` |
| Architect review | architect, Sonnet 5.5 | `gateway_proc::layout_for` stripped a trailing `gateway` path component, so a root named `gateway` resolved to its parent | heuristic removed; regression test for a root named `gateway` |
| Tester pass | tester, deepseek-v4.1-flash | BLOCKED m_7054: `src/delivery.rs` test fixture outside the scope (the architect's scope search covered the moved paths but not the callers of `Store::open` in test modules); `<home>` instead of `<root>` in `SECURITY.md` and `a2amx.toml.example`; the `docs/architecture.md` Directory layout section missing | Amendment 2 |
| Tester pass | tester | B1: the confined-tree test was vacuous (preflight given a directory and a silent return, fake database, cwd under the root, root shadowed by the plan's `/tmp`) | test rewritten and shown to pass for real |
| Tester pass | tester | B2: team-up preflight built `Host.home` from the a2amx root instead of the real `HOME` | fixed; the architect's mutant survived because no test covered it, so a regression test was added |
| Tester pass | tester | B3: section 5.3 protected root not implemented | implemented with a plan-level test |
| Tester pass | tester | Gateway argv test missed bind modes and a broad root bind; `relay_files::load` creating a directory was untested; fresh-daemon tree test absent; stale texts and names (`cli.rs`, `client.rs`, `gateway_proc.rs`, `state_dir` identifiers), stale docs; `ensure_empty_file` created `run/` itself | fixed; tests added; the log-body test was dropped because no fixture captures the daemon log (allowed by section 7) |
| Architect review | architect, Sonnet 5.5 | The diff removed `--remount-ro /` from the gateway sandbox without reason and the tests asserted its absence; a probe showed it works with the file binds | restored; argv assertions flipped |
| Architect review | architect, Sonnet 5.5 | The developer removed the relay member-limit log text for no reason; a dead `_layout_root` binding and an unused `_host` parameter remained | restored; removed |
| Developer mutants | developer | Mutants e and f (hides of `run` and `secrets`) survived the first `mandatory_hides_remain` test | test strengthened; both killed |
