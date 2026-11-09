# Spec 3dn: `a2amx gateway up`, the confined launch of the gateway (F81)

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared context `a2amx-remote-human-messaging-2026-11-08`. Public test seams: the `a2amx` binary (`gateway up`, `gateway run`), the pure `gateway_launch::sandbox_argv`, the daemon's files in the state directory, and `Client::connect_gateway`.

Scope (files that may change): `src/gateway_launch.rs` (new), `src/lib.rs`, `src/cli.rs`, `src/main.rs`, `src/daemon.rs`, `src/relay.rs`, `src/relay_files.rs`, `tests/gateway_up.rs` (new), `tests/relay.rs`, `tests/daemon.rs`, `tests/human_inbound.rs`, `tests/common/mod.rs` (the daemon tests that assert `addr` and the tokens are removed gain the link-file assertions; `tests/relay.rs:1178` writes a fake `addr` for the reconnect test and must write it to `link/` instead).

Does not touch: `src/confine.rs` (used as is), `src/gateway.rs`, `src/discord.rs`, `src/client.rs`, `src/wire.rs`, Cargo files, `deny.toml`, docs, CHANGELOG, `AGENTS.md` (the architect adds the module row and docs when committing).

## 1. Why

`a2amx gateway run` (spec 3dm) is the process that talks to Discord. It parses data from the internet (message text, JSON, websocket frames), so a bug in it must not reach the owner's home directory, the repository, the daemon's message database or the admin token. The owner decided: one command, `a2amx gateway up`, starts the gateway, and it is always confined (no unconfined mode), with its own small bubblewrap profile (no home, no repository, no harness config; only the gateway's state and its two daemon files).

Facts that shaped the design (all read in the tree):
- The daemon writes `addr` and `gateway.token` in the state directory, with the same truncate-and-rewrite (`write_state_file`, `src/daemon.rs:2524-2526`), and removes `addr` at clean shutdown (`src/daemon.rs:2475`) and both tokens when binding fails (`src/daemon.rs:2256`). A bind mount of a single file keeps the old inode, so after a daemon restart a file mounted into the sandbox would be stale, and the gateway would reconnect with a dead address or token (3dm reconnects every 5 s and rereads both files). A bind mount of a directory sees every later change. So the gateway must read these two files from a directory of its own, which the sandbox can mount.
- The state directory also holds `admin.token`, every session token and the message database; none of it may be visible to the gateway.
- A sandbox that keeps the whole file system read-only (`--ro-bind / /`, the session profile) still lets the process `connect()` to every same-user socket under `/run` (the session D-Bus, whose `systemd-run --user` runs a command outside any sandbox, `docker.sock`, WSL interop). The session profile closes them with hide lists in `src/confine.rs`. The gateway needs none of that, so its profile is an allowlist: an empty root with only the system directories and its own files mounted.
- The network cannot be limited by name with bubblewrap. The allowlist (`discord.com`, `*.discord.gg`) is a firewall note in the setup guide (3do); this spec does not enforce it and says so.
- Alternatives rejected: the session profile with its hide lists (a denylist; every new socket path is a hole); mounting the whole state directory read-only (shows admin token and messages); mounting the two files (stale after a restart); a second daemon port for the gateway only (new surface); `--unshare-net` (the gateway needs Discord).

## 2. `a2amx gateway up [--pair]`

`src/cli.rs`: add `GatewayAction::Up { pair: bool }` (`--pair` as in `run`). `Run` stays, with `#[command(hide = true)]` and the help text "Internal: the unconfined relay that `up` starts inside the sandbox." `Token` is unchanged.

`src/main.rs`: `Up` calls `gateway_launch::up(&home, pair)`, which never returns on success (it replaces the process with `exec`) and returns an error otherwise. The error is printed like every other command error, exit code 1.

`up` does, in this order, and stops at the first failure with a message naming what to do:
1. `relay_files::load(&home)` (config, token and pairing files are valid; without `--pair` a pairing record must exist, as in 3dm). The result is discarded; the inner process loads again.
2. `<home>/link` is a real directory and holds `addr` and `gateway.token` as regular files, all checked with `symlink_metadata` (a symlink is refused, since bubblewrap resolves bind sources on the host); otherwise "the a2amx daemon is not running in this state directory; start it first".
3. `confine::resolve_bwrap(None)` then `confine::preflight(&bwrap)`; either failing is fatal: "bubblewrap is required to run the gateway; it never runs unconfined". There is no flag, environment variable or fallback that skips this.
4. `gateway_launch::sandbox_argv` builds the argument vector (below); `up` runs it with `std::os::unix::process::CommandExt::exec`, so a terminal Ctrl-C reaches bubblewrap and its child directly and the gateway stops gracefully. A SIGTERM sent to the process kills the gateway at once (`--die-with-parent` and `--unshare-pid`), which at-least-once delivery tolerates; a graceful stop by signal belongs to a later `gateway down`. If `exec` returns, that error is returned.

`up` also computes `resolv_target` before building the vector: `None` when `/etc/resolv.conf` is not a symlink or its canonical target is under `/etc` or `/usr` (already mounted); `Some(canonical target)` when it is a symlink to a regular file elsewhere; and an error naming `/etc/resolv.conf` ("cannot resolve /etc/resolv.conf; the gateway needs DNS") when it is a symlink that cannot be canonicalized or whose target is not a regular file. A missing `/etc/resolv.conf` is `None` (the gateway then fails to resolve, with its own error).

The inner command line is the vector `[<current_exe canonical>, "--home", <canonical state directory>, "gateway", "run"]` (a relative `--home` would resolve against the sandbox's working directory, so the canonical path is used everywhere) plus `"--pair"` when asked. No shell is involved anywhere.

### 2.1 `gateway_launch::sandbox_argv`

A pure function, `sandbox_argv(layout: &Layout, inner: &[String]) -> Vec<String>`, with `Layout { bwrap: PathBuf, state_dir: PathBuf, exe: PathBuf, rust_log: Option<String>, resolv_target: Option<PathBuf> }` (absolute and canonical; `state_dir` is the resolved `--home`; `resolv_target` is decided by `up`, see step 2 below, so the function reads nothing from the host and the worked-example test fixes every input). Output, in this order:

1. `<bwrap> --die-with-parent --unshare-user --unshare-ipc --unshare-pid` (the network is shared; the gateway needs Discord and the loopback daemon).
2. An empty root. Then, read-only: `--ro-bind /usr /usr`, `--ro-bind-try /bin /bin`, `--ro-bind-try /sbin /sbin`, `--ro-bind-try /lib /lib`, `--ro-bind-try /lib64 /lib64`, `--ro-bind /etc /etc` (the program, its libraries, DNS and name service files; on a merged-`/usr` system `/bin` and `/lib` are links into `/usr`, which `--ro-bind-try` follows). When `layout.resolv_target` is `Some(path)`, that path is bound read-only at its own path too (`--ro-bind <path> <path>`, placed right after `/etc`; on WSL it is `/mnt/wsl/resolv.conf`). Then `--proc /proc`, `--dev /dev`, `--size 1073741824 --tmpfs /tmp`.
3. `--ro-bind <exe> <exe>` (the binary, wherever it lives; bubblewrap creates the parent directories in the empty root).
4. `--ro-bind <state_dir>/link <state_dir>/link` and `--bind <state_dir>/gateway <state_dir>/gateway` (the only writable path besides `/tmp`).
5. `--remount-ro /` (the empty root and its created directories).
6. `--clearenv`, `--setenv PATH /usr/local/bin:/usr/bin:/bin`, and `--setenv RUST_LOG <value>` only when it was set. With `--chdir` bubblewrap also sets `PWD`; nothing else exists (no `HOME`, no `A2AMX_*`, no token, no harness variable).
7. `--chdir <state_dir>/gateway`, `--`, then `inner`.

Nothing else of the host is present: no `/root`, `/var`, `/opt`, and no `/run` content; `/home`, `/mnt` or `/run` exist only as the parent directories of mounted paths (probed with bubblewrap 0.11.1: `/` lists `bin dev etc home lib lib64 mnt proc sbin tmp usr`, the state directory lists `gateway link`, `admin.token` is absent, `link` is read-only, `gateway` is writable, `discord.com` resolves). Same-user Unix sockets in the file system (session bus, `docker.sock`, WSL interop) are therefore absent. Not closed, and named in 3do: abstract-namespace Unix sockets and loopback TCP services, because the network namespace is shared.

No `--new-session` (the pairing prompt reads the terminal and Ctrl-C must reach the process). TIOCSTI injection from a sandboxed process is closed by the kernel by default since 6.2 and the process is a2amx's own code; noted in 3do as a residual risk.

## 3. The link directory: `src/daemon.rs`, `src/relay.rs`, `src/relay_files.rs`

`src/daemon.rs` start-up (the block at `2205-2262` and the address write at `2272-2274`): `daemon::ensure_state_dir(&state_dir/link)` (existing helper, mode 0700) runs before the first write. `link/gateway.token` is written right after `gateway.token` (`write_state_file`, 0600, same content) and `link/addr` right after `addr`. A failed `link/gateway.token` write removes `gateway.token` and `link/gateway.token` and fails the start, as the `admin.token` failure does today (`2222-2238`); the bind-failure path (`2251-2258`) also removes `link/gateway.token`. Clean shutdown removes `link/addr` next to `addr` (`2472-2483`); `gateway.token` and `link/gateway.token` persist across a clean shutdown, as `gateway.token` does today. The files in the state directory stay, unchanged, for every other reader.

`relay_files::Loaded` gets `link_dir: PathBuf` (`<home>/link`), and `relay::run` and its reconnect path call `Client::connect_gateway(&link_dir)` instead of `&home`. `Client::connect_gateway(dir)` is unchanged (it reads `dir/addr` and `dir/gateway.token`). The daemon never removes or recreates `link/` itself while running; 3do notes that doing it by hand leaves a running gateway with a stale mount until it restarts.

## 4. Attack paths (trust-boundaries)

| entry point | abuse | disposition | abuse-case test (`tests/gateway_up.rs`, real bubblewrap) |
|---|---|---|---|
| Discord data parsed by the gateway (a compromised relay) | read `admin.token`, session tokens, the message database, the owner's home, the repository | removed: an empty root; the state directory holds only `link/` (read-only) and `gateway/` | inner `sh -c` probes through `sandbox_argv`: `cat <state>/admin.token` fails; the entries of `<state>` are exactly `gateway` and `link`; `/root` does not exist and `/home` holds only the path components down to the state directory and the executable; writing `<state>/link/x` and `<state>/marker` fails; writing `<state>/gateway/x` succeeds |
| same-user Unix sockets (session D-Bus, `docker.sock`, WSL interop) | a command outside the sandbox | removed: `/run` (except the parent directories of a `resolv.conf` target that lives there), `/var` and the rest of the host are absent | the test creates a real listening Unix socket under a temporary directory outside the mounts and one at `$XDG_RUNTIME_DIR/bus` when that exists; inner `connect()` (a small `python3` or `perl` one-liner, skipped with a reason if neither exists) to each fails; the entries of `/` are the allowlist plus the parent components of the mounted paths (`home`; `mnt` or `run` when `/etc/resolv.conf` points into them) and nothing else (no `var`, `opt`, `root`, `srv`), and when `run` exists it holds no socket |
| environment of the launcher | tokens or harness variables reach the gateway | constrained: `--clearenv`, `PATH`, `PWD` and `RUST_LOG` only | inner `env` prints only `PATH`, `PWD` and (when set) `RUST_LOG`, with the launcher's environment carrying a fake `A2AMX_TOKEN` and `HOME` |
| missing or broken bubblewrap | the gateway runs unconfined | removed: no unconfined path in `up` | `up` with a valid config, token, pairing, running daemon and an empty `PATH` exits non-zero with the message; no `gateway run` process starts |
| `--home` or paths with spaces, quotes, a leading `-` | argument injection into bubblewrap | constrained: argument vector, absolute canonical paths, the inner command after `--` | a state directory name with spaces and a leading `-` stays one argument in the vector, and nothing after `--` is an option |
| `link` or its files are symlinks | bubblewrap exposes a host directory | removed: `up` refuses a symlink | `up` with `link` a symlink to another directory exits non-zero and starts nothing |
| stale daemon files after a restart | the gateway keeps an old address or token | removed: directory mount, files rewritten in place | restart the daemon while the sandbox runs; the inner `cat link/addr` shows the new address |
| `gateway run` called directly | unconfined gateway | monitored: hidden internal command, documented in 3do; the owner can always run any binary | none (accepted) |

## 5. Out of scope

The network allowlist and firewall note, the setup guide, the residual-risk text, CHANGELOG and docs (3do); changing `confine.rs` or its grants; a `gateway down` or service file; running `up` as a daemon child; `--unshare-net` or a proxy; moving the existing `addr`/`gateway.token` files; the live test with the owner's bot.

## 6. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo deny --locked check`, `scripts/prepush-fast.sh` pass; `git status --short` shows only section 0 files.
2. `tests/gateway_up.rs` has one test per row of the section 4 table (real bubblewrap, skipping with a printed reason only when `bwrap` is missing and `A2AMX_REQUIRE_BWRAP` is unset; CI's `check` job has bubblewrap, installed in `ci.yml`), plus: the exact `sandbox_argv` for a worked example (literal expected vector), `--pair` is passed through, `up` fails with a clear message when config, token or link files are missing, and the daemon writes and removes the link files like the originals (`tests/daemon.rs` for the start and shutdown, `tests/human_inbound.rs` for the bind-failure removal), and `tests/relay.rs` proves the relay reconnects through `link/` after a daemon restart.
3. Mutation check (each restored by checksum, `timeout 120` on each run, naming the failing test): `--clearenv` removed; `/run` (or `--ro-bind / /`) mounted; `link` mounted writable; `gateway` mounted read-only; the state directory mounted whole; `--remount-ro /` removed; the bwrap preflight skipped; the unconfined fallback on a missing bubblewrap; the daemon not removing `link/addr` at shutdown; `relay` reading the old directory.
4. Complexity: each function in `gateway_launch.rs` stays small (the developer states hand counts for `up` and `sandbox_argv`); Sonar after the push, and `scripts/prepush-sonar.sh` by the architect before it.

Pre-lock runs: baseline claims below were read, not run; the acceptance commands need the new code and run after implementation.

| claim | where | how |
|---|---|---|
| `write_state_file` truncates in place | `src/daemon.rs:2524-2526` | read |
| `addr` removed at clean shutdown; tokens removed on bind failure | `src/daemon.rs:2475`, `2256` | read |
| `connect_gateway` rereads both files on each call | `src/client.rs:50-73` | read |
| `confine::resolve_bwrap` and `preflight` exist and are public | `src/confine.rs:2187`, `2240` | read |
| session sandbox base arguments and `--remount-ro` use | `src/confine.rs:2008-2100` | read |
| CI has bubblewrap for sandbox tests | `.github/workflows/ci.yml:107` | read |
| readers of `gateway.token` or `addr` in tests keep working because the originals stay; the one that must change is the fake `addr` at `tests/relay.rs:1178` | `tests/relay.rs`, `tests/human_inbound.rs:1430`, `tests/teams.rs:38`, `tests/common/mod.rs:171`, `tests/daemon_cli.rs:1299`, `tests/broker.rs:2538`, `tests/attach_cli.rs:1968` | grep (consultant) |
| bubblewrap 0.11.1 accepts the empty-root order (binds below created parents, `--remount-ro /`, `--chdir`, `--clearenv`) and DNS works | probed by the architect with the exact section 2.1 order | probed |

## 7. Amendment 1 (after the developer's BLOCKED)

Section 2.1 called `sandbox_argv` pure but made its output depend on the host's `/etc/resolv.conf`. `Layout` now carries `resolv_target: Option<PathBuf>`, decided by `up` (section 2, the paragraph before the inner command line), with the failure rule stated there. `sandbox_argv` reads nothing from the host. Acceptance additions: a worked-example test with `resolv_target` `Some("/mnt/wsl/resolv.conf")` and one with `None` (two literal vectors), and a test of the `up` decision for a symlink into `/mnt`, a symlink under `/etc`, a regular file, and a dangling symlink (error). Gate recheck: the section 4 rows and section 6 mutation list are unchanged; no other section states the resolv.conf rule.
