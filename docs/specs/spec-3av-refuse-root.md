# Spec 3av: Refuse to run as root

## 0. Status

**LOCKED** (2026-10-28; pre-lock gate run against `develop` at `a0c5034`; consultant review `m_4206`, findings A-F and the tidy-ups adopted). Architect-directed, owner
decision (2026-10-28), context `a2amx-refuse-root-2026-10-28`. Implementation goes to `a2amx-developer` only.

**Breaking change** under SemVer (CLI behavior an operator depends on): `a2amx` exits with an error when its effective
uid is 0. The major version is 0, so the architect raises the minor version at the next release and adds a `Changed`
entry to the changelog; the developer does neither.

**Baseline (run 2026-10-28 on `a0c5034`):** `env -u A2AMX_BIN -u NO_COLOR cargo test` passed 629 tests, 0 failed, 3
ignored (the developer records its own count before the first edit); `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` are the other two acceptance
commands. Known flaky test: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (backlog I14); a
failure is rerun once and recorded as pre-existing if it passes.

**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `a2amx::privilege::ensure_not_root_for` (pure, `tests/privilege.rs`); the `a2amx` binary run as
uid 0 through `unshare -Ur` (`tests/privilege.rs`, skipped when `unshare -Ur` cannot run).

**Scope.** May edit exactly these paths and no others.

- `src/privilege.rs` (new, 3.1), `src/lib.rs` (3.2), `src/main.rs` (3.3)
- `tests/privilege.rs` (new)
- `AGENTS.md` (the Modules table, one row), `docs/install.md` (3.4), `SECURITY.md` (3.4)

Does not touch: `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `docs/backlog.md`, `docs/specs/`, `.github/`, every other file.
No new dependency. Leave the diff uncommitted.

## 1. Why

Owner decision: `a2amx`, the daemon included, is never supposed to run as root or under `sudo`. The daemon starts agents
that run arbitrary commands, so a root daemon turns an agent's honest mistake (`rm -rf $var` with an empty `$var`) into
damage to the whole system. State files rely on owner-only modes (`0600`, `0700`) to keep other users out, which root
bypasses. Nothing in the code refuses today (`rg -n -i "geteuid|getuid|sudo" src` finds no check). The owner also
decided: a hard stop with no override, and containers are not an exception ("no one should use containers with root
inside"; users deal with it).

## 2. Rule

Every `a2amx` command except argument parsing itself refuses to run when the effective uid is 0.

- Check the **effective** uid (`rustix::process::geteuid().as_raw() == 0`). Under `sudo`, under `su`, as a root login,
  as a setuid-root binary, and inside a user namespace that maps the caller to root (rootless containers) the effective
  uid is 0, so all of them are refused. `SUDO_UID` and similar variables are not consulted.
- `--help`, `--version` and argument errors keep working as root: clap handles them in `Cli::try_parse` before the check
  (src/main.rs:49-55, the `Err` arm), and that stays.
- There is **no** override: no flag, no environment variable, no config key.
- The refusal is a normal command failure: the existing error path in `main` prints `a2amx: <message>` to stderr and
  exits with `ExitCode::FAILURE` (1).
- The message, exactly: `refusing to run as root (effective uid 0): run a2amx as an ordinary user`.
- The check runs before any command runs: no state directory is created, no file is read or written, no connection is
  opened. (Logging setup and the async runtime start first; neither writes a file.)

## 3. Changes

### 3.1 `src/privilege.rs` (new)

Doc comment on the module: why (one sentence). Two public functions, no state:

```rust
/// Err when `euid` is 0.
pub fn ensure_not_root_for(euid: u32) -> anyhow::Result<()>

/// Checks the process's effective uid.
pub fn ensure_not_root() -> anyhow::Result<()>
```

`ensure_not_root_for(0)` returns `Err` with the message in section 2; any other value returns `Ok(())`.
`ensure_not_root` is `ensure_not_root_for(rustix::process::geteuid().as_raw())`. The `process` feature of `rustix` is
already enabled (`Cargo.toml:16`); no new feature or crate.

### 3.2 `src/lib.rs`

Add `pub mod privilege;` in the alphabetical position (between `prefix` and `quota`).

### 3.3 `src/main.rs`

In `main`, in the `Ok(cli)` arm (line 50), call `a2amx::privilege::ensure_not_root()` first and run `dispatch(cli)` only
when it returns `Ok`; on `Err` the result is that error. This is the single place every command passes through,
including `mcp`, `hook` and `omp-bridge`, which `dispatch` handles before it resolves the home directory
(src/main.rs:68-77). Do not add a second check anywhere. (Use the crate's existing path style for `privilege`; the
file already imports its modules the same way for `mcp`, `hook` and `bridge`.)

### 3.4 Docs

- `docs/install.md`: one short paragraph at the end of "Install a release": a2amx and its daemon refuse to run as root
  or under `sudo`, install and run it as an ordinary user, there is no override, and containers must use a non-root
  user too.
- `SECURITY.md`: one bullet in "What is in scope" or one sentence below it saying the same and why (state-file
  permissions and agent commands assume an ordinary user).
- `AGENTS.md`: one row in the Modules table, directly after the `prefix` row: `privilege` | Refusal to run with
  effective uid 0. And one sentence under "Commands": the test suite must run as an ordinary user, because the tests that
  spawn the binary are refused as root.

## 4. Tests

`tests/privilege.rs`, written first:

1. `ensure_not_root_for(0)` is `Err` and its text contains `refusing to run as root`; `ensure_not_root_for(1000)` and
   `ensure_not_root_for(1)` are `Ok`. (Literal expected values.)
2. Abuse cases through the binary run as uid 0: `unshare -Ur <a2amx> --home <empty temp dir> list`, `daemon status`,
   `daemon start` (background form, no `--foreground`), `mcp`, `hook`. Each exits with status 1 and prints a line
   containing `refusing to run as root` on stderr. For `list`, `daemon status` and `daemon start` the temporary home
   is still empty afterwards (`mcp` and `hook` ignore `--home`, so the emptiness assertion does not apply to them).
   These tests fail on the old code (the old binary does not refuse). Why no `--foreground`: on the old code
   `daemon start --foreground` does not fail, it binds a loopback port and waits for a signal (src/main.rs:179-185),
   so a red run would hang; do not add that case.
   Every child in this file is run with `stdin = Stdio::null()`, a 10-second timeout (kill and wait on expiry; a
   timeout is a test failure), an explicit `--home`, and a cleared environment except `PATH` (so a test run inside an
   a2amx-hosted session cannot reach the live daemon through `A2AMX_*` variables). On the old code `mcp` and `hook`
   read stdin until EOF, which is why the null stdin is required.
3. Contract test: `unshare -Ur <a2amx> --version` exits 0 and prints the version (it already passes on the old code).
4. Contract test: the same binary run as the ordinary test user does not print the refusal: `list` against an empty
   temporary `--home` fails with the unreachable-daemon error (`client::UNREACHABLE`, "cannot reach the a2amx daemon")
   and its stderr does not contain `refusing`.
5. Skip rule for tests 2-3: when `unshare -Ur true` cannot run, print a notice and return, unless
   `A2AMX_REQUIRE_USERNS=1`, which makes the skip a failure. CI does not set it (the GitHub runner's image may restrict
   unprivileged user namespaces; set it only after one CI run proves it works).
6. The suite must run as an ordinary user: 13 of the integration test files spawn the `a2amx` binary, and as root they
   would all be refused. State this in `AGENTS.md` under "Commands" in one sentence (the file is already in scope).

## 5. Out of scope

- Any override flag, environment variable or config key.
- A real-uid check, `SUDO_*` variables, supplementary groups, capabilities, or `setuid` handling beyond the effective
  uid.
- Checking at the daemon separately from the client commands; the single check in `main` covers both.
- Refusing a daemon that is already running as root (it is not touched; the next start refuses).
- Anything about confinement, containers or the harnesses. `CHANGELOG.md` and the version (architect, at commit).

## 6. Acceptance

Judge: the commands below; the tester or the architect reads the diff for the single call site.

- `env -u A2AMX_BIN -u NO_COLOR cargo test` passes; the new tests exist and test 2 fails on the old code
  (record the red run before the change).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n "ensure_not_root" src` finds the definition in `src/privilege.rs` and exactly one call in `src/main.rs`.
- By hand after implementation: `unshare -Ur target/debug/a2amx list` prints the refusal and exits 1;
  `unshare -Ur target/debug/a2amx --version` exits 0.
- `git status --short` shows only the scope paths.

## 7. Claims (gate step 9)

| Claim | Evidence | Verified how |
| --- | --- | --- |
| No root check exists today | `rg -n -i "geteuid\|getuid\|sudo\|euid" src` finds only unrelated `process` lines | probed 2026-10-28 |
| `rustix` has the `process` feature and `geteuid`, `Uid::as_raw` | `Cargo.toml:16`; `Cargo.lock` pins rustix 1.1.5; its `src/process/id.rs:53`, `src/ugid.rs:46` | read (consultant, 1.1.5 source) |
| On the old code `daemon start --foreground` hangs, `mcp`/`hook` read stdin until EOF, `list` fails with `client::UNREACHABLE` | `src/main.rs:179-185`, `src/mcp.rs:62`, `src/hook.rs:30-45`, `src/client.rs:16` | read (consultant); hang inferred |
| The background daemon child re-executes the same binary through `main`, so it hits the check | `src/main.rs:216-220` | read (consultant) |
| 13 integration test files spawn the binary | `rg -l CARGO_BIN_EXE_a2amx tests` plus `tests/common/pty.rs:55` | counted (consultant) |
| `main` is the single entry; `--help`/`--version` go through the clap `Err` arm before dispatch | `src/main.rs:49-55` | read |
| `mcp`, `hook`, `omp-bridge` return from `dispatch` before home resolution | `src/main.rs:68-77` | read |
| `unshare -Ur id -u` prints `0` on the architect's host | probed 2026-10-28 | probed |
| The GitHub runner may not allow `unshare -Ur` | `ubuntu-latest` (`.github/workflows/ci.yml:22`) | recalled, unverified: tests skip, variable not set in CI |
| No existing test or CI job runs a2amx as uid 0 | `rg` over `tests/`, `.github/`, `scripts/` for `unshare`, `geteuid`, `sudo`, `container:` finds nothing; workflows use `ubuntu-latest` / `ubuntu-22.04` | grepped (consultant) |
| The release workflow only runs `./a2amx --version`, which stays exempt | `.github/workflows/release.yml:52` | read (consultant) |
| Exit code for a failure is 1 | `src/main.rs:60` | read |

## 8. Trust boundary

No new input from a less trusted source. The check reads the process's own effective uid and has no override, so there is
no attack path to constrain. The abuse case is a user running a2amx as root; section 4 test 2 shows it fails.
