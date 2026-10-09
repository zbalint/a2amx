# Spec 3s: a build version the binary and the daemon both report

## 0. Status

**LOCKED** (2026-10-09, pre-lock gate run against develop at `9114ce3`; consultant advice `m_894`
and pre-lock review `m_897`, context `a2amx-daemon-version`; gate notes in section 9). Owner request: version the daemon so it is clear whether it was updated.
Owner decisions: the commit hash is enough (no release tags yet); MCP `list_agents` stays out.
Architect decisions are in section 2.

**Baseline:** develop at `9114ce3`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-daemon-version`.
Public test seams: `a2amx::client::version_warning` (pure, `tests/version.rs`), the wire JSON
(`tests/wire.rs`), a raw authenticated socket (`tests/daemon.rs`), and the `a2amx` binary
(`tests/daemon_cli.rs`).

**Scope.** May edit exactly these files and no others.

- `build.rs` (new), `src/lib.rs`, `src/cli.rs`, `src/wire.rs`, `src/daemon.rs`, `src/client.rs`,
  `src/main.rs`
- `tests/version.rs` (new), `tests/wire.rs`, `tests/daemon.rs`, `tests/daemon_cli.rs`
- `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `Cargo.toml` and `Cargo.lock` (cargo finds `build.rs` by itself; no dependency is
added), `scripts/install.sh` (it already prints `--version`), `src/mcp.rs` and every other `src/`
file, every other test file, every spec document, `AGENTS.md`. Do not commit, stage or merge: leave
the diff uncommitted.

## 1. Why

`a2amx --version` prints the constant `0.0.0` (`Cargo.toml` line 3), the daemon reports no version at
all, and a running daemon keeps its old binary after `scripts/install.sh` replaces the file. So after
an install nobody can tell whether the daemon is the new build. This spec stamps every build with
the git description and lets `a2amx daemon status` show the daemon's version and warn when it differs
from the binary you ran.

## 2. Decisions

- **D1. Version string.** `build.rs` sets `A2AMX_VERSION` to the output of
  `git describe --tags --always --dirty` (no tags exist, so today this is the short commit hash,
  with `-dirty` when tracked files differ from HEAD). If git is missing, the directory is not a
  repository, or the command fails or prints nothing, it falls back to `CARGO_PKG_VERSION`
  (`0.0.0`); such builds (a source tarball) all report the same string, so a mismatch cannot be
  detected for them. Accepted and documented.
- **D2. One constant.** `a2amx::VERSION: &str = env!("A2AMX_VERSION")` in `src/lib.rs`. The CLI's
  `--version` (`src/cli.rs` line 11) and the daemon both use it. clap prints `a2amx <VERSION>`.
- **D3. Wire seam: a new request.** `Request::Version` (unit variant, JSON `{"type":"version"}`)
  answered by `Response::Version { version: String }`. The hello reply, `Response::Ok`, `Sessions`
  and `Agents` do not change: the client accepts only `Response::Ok` to hello (`src/client.rs`
  lines 69-72), and `tests/wire.rs` asserts the exact JSON of hello and sessions. Admin only: the
  session-token allow list at `src/daemon.rs` lines 1692-1701 is not extended, so a session token
  gets the existing `not permitted for a session token` error.
- **D4. Old daemon.** A daemon that predates this spec answers an unknown request with
  `Response::Error { message: "invalid control request" }` and keeps the connection usable
  (`src/daemon.rs` lines 1679-1691). The client treats any `Response::Error` to `Version` as
  "daemon predates version reporting".
- **D5. Where it shows.** Only `a2amx daemon status` (`run_daemon_status`, `src/main.rs` line 267)
  and `--version`. Not `list`, `new`, or any other command: a `cargo run` from `target/` against the
  installed daemon would otherwise nag on every call. The MCP, hook and bridge processes never check.
- **D6. Output.** `daemon status` stdout gains one line `version: <daemon version>` directly after the
  `running` line (before the `listening on` lines). Against an old daemon it prints
  `version: unknown (daemon predates version reporting)`. The exit code is unchanged (0 when running).
  A mismatch or unknown version also writes **one line to stderr** from `version_warning` (D7); stdout
  stays parse-stable. Never refuse or fail because of a mismatch.
- **D7. Pure comparison.** `pub fn version_warning(cli: &str, daemon: Option<&str>) -> Option<String>`
  in `src/client.rs`: `None` when `daemon == Some(cli)`; otherwise
  `Some("warning: daemon version {daemon} differs from this binary {cli}; restart the daemon to use this binary (sessions end)")`
  for `Some(daemon)`, and
  `Some("warning: daemon predates version reporting; restart it to use this binary (sessions end)")`
  for `None`. With hashes there is no ordering, so the text never says older or newer.
- **D8. Not in scope: compatibility.** The version string answers "was it updated", not "can these
  talk". A protocol integer (like `BridgeUp::Hello.protocol`) is a separate decision, deferred.

## 3. `build.rs` (new)

Plain `fn main()` with no `unwrap`/`expect` (the crate denies both; write it with `match`/`if let`
and the D1 fallback). Run git with `std::process::Command` and an argument vector, in the manifest
directory (`CARGO_MANIFEST_DIR`), never a shell string.

1. Compute the version per D1 and print `cargo:rustc-env=A2AMX_VERSION=<version>`.
2. Rerun directives. Printing any `rerun-if-changed` disables cargo's default of rerunning on any
   package file change, so list everything the version depends on: `build.rs`, `Cargo.toml`, `src`
   (so `-dirty` flips when a source file is edited), and the git files that move HEAD, resolved with
   `git rev-parse --git-path <name>` (correct in a worktree, where `.git` is a file) for `HEAD` and
   `index`, `logs/HEAD` (every commit, checkout and reset appends to it, which covers a branch ref that
   exists only in `packed-refs`), and for the ref `git symbolic-ref -q HEAD` names, and `packed-refs`. Print a
   `rerun-if-changed` line only for a path that exists (cargo reruns every build for a missing one).
   Leave a `// shortcut:` comment: `-dirty` tracks `src`, `Cargo.toml` and git state, not edits to
   tests or docs (ceiling), and the index file changing on a plain `git status` can trigger a
   harmless rebuild; upgrade trigger: report if a stale or flapping version is seen.
3. When git is unavailable print no git `rerun-if-changed` lines.

## 4. Source changes

- `src/lib.rs`: add `pub const VERSION: &str = env!("A2AMX_VERSION");` with a doc comment saying it
  is the git description of the build. Place it before the `pub mod` list's first item or after the
  module docs, wherever the file's style keeps top-level items; do not reorder modules.
- `src/cli.rs` line 11: `#[command(name = "a2amx", version = crate::VERSION)]`.
- `src/wire.rs`: add `Version,` to `Request` (after `List` if present, otherwise at the end of the
  unit variants; keep the rest unchanged) and `Version { version: String }` to `Response`.
- `src/daemon.rs`: in the request `match` (arms from line 1713; `Request::List` is at 1716) add
  `Request::Version => Response::Version { version: crate::VERSION.to_owned() }`. Nothing else, in
  particular not the session-token allow list.
- `src/client.rs`: add `version_warning` (D7) as a free `pub fn`.
- `src/main.rs`, `run_daemon_status`: after `request_sessions`, send `Request::Version` on the same
  client. `Response::Version { version }` gives `Some(version)`; `Response::Error` gives `None`; any
  other response is an `unexpected daemon response` error as in `request_sessions`. Build the output
  per D6, then if `a2amx::client::version_warning(a2amx::VERSION, daemon.as_deref())` is `Some`, write
  it plus a newline to stderr after stdout is written (use the same blocking-safe write helper
  pattern the file already uses; do not `unwrap`).

## 5. Tests (failing first, expected values are literals unless noted)

Tests 7 and 9 are contract tests: they already pass at `9114ce3` (unknown-request handling is at
`src/daemon.rs` lines 1679-1691, status without a daemon at `tests/daemon_cli.rs` lines 1308-1314) and
must still pass; they are not expected to fail first. The rest fail first (compile error or wrong
output) until the code exists.

`tests/wire.rs`:

1. `serde_json::to_string(&Request::Version)` is exactly `{"type":"version"}` and parses back to
   `Request::Version`.
2. `serde_json::to_string(&Response::Version { version: "ab12cd3-dirty".into() })` is exactly
   `{"type":"version","version":"ab12cd3-dirty"}` and parses back.

`tests/version.rs` (new), pure `version_warning`:

3. `version_warning("ab12cd3", Some("ab12cd3")) == None`.
4. `version_warning("ab12cd3", Some("9f8e7d6"))` is exactly
   `Some("warning: daemon version 9f8e7d6 differs from this binary ab12cd3; restart the daemon to use this binary (sessions end)")`.
5. `version_warning("ab12cd3", None)` is exactly
   `Some("warning: daemon predates version reporting; restart it to use this binary (sessions end)")`.

`tests/daemon.rs`, through a raw authenticated socket (follow `tests/daemon.rs` lines 1035-1046: read
`admin.token`, send hello, expect `Response::Ok`):

6. After hello, `Request::Version` gets `Response::Version { version }` where `version` is non-empty
   (its equality with the binary's `--version` is asserted in test 8; here the daemon runs in-process
   so it equals `a2amx::VERSION` by construction, which is why the literal check lives in test 8).
7. After hello, send the raw frame `{"type":"no_such_request"}`: the reply is
   `Response::Error { message: "invalid control request" }`, and a following `Request::List` on the
   same socket still gets `Response::Sessions`. This is the contract the old-daemon fallback (D4)
   relies on; no test for it exists today.

`tests/daemon_cli.rs`, through the binary:

8. Extend the existing status test around line 1225-1248 (`daemon status` with one running and one
   exited session): `a2amx --version` stdout, trimmed, with the `a2amx ` prefix removed, is the
   expected version `V` (non-empty); expected `daemon status` stdout becomes
   `format!("running\nversion: {V}\nlistening on {address}\nsessions: 1 running, 1 exited\n")` and stderr
   stays `""` (same binary, so no warning). Note: `V` comes from the binary itself, an accepted
   equality of two exposures.
9. `daemon status` without a daemon (lines 1308-1314) is unchanged: `not running\n`, exit 1.

There is no test of the `version: unknown (daemon predates version reporting)` line either (it needs an
old daemon). There is no test of a real old daemon or of a mismatched daemon through the binary; D4 and D7 are
covered by tests 5, 7 and the pure function.

Existing tests: search the whole tree for exact `daemon status` output and for `--version`
(`rg -n "daemon.*status|--version|sessions: " tests README.md docs/architecture.md`). Update only
assertions of the `daemon status` stdout shape (the one in test 8). If any other existing test fails
because of this change, stop and report `BLOCKED — SPEC ADJUDICATION REQUIRED` with the test name.

## 6. Documentation

- `README.md`: line 98 (`a2amx daemon status  # show daemon state and session counts`) mentions the
  version. In "Dev build versus live install" (around line 338-347) add: the binary and the daemon
  carry the git description of their build (`a2amx --version`, `a2amx daemon status`), `-dirty` marks
  a build from a modified tree, and `daemon status` warns on stderr when the running daemon is not
  the build you ran; a source tarball without git reports `0.0.0` and cannot be compared.
- `docs/architecture.md`: where the control requests or the daemon's status output are described,
  one sentence for `Request::Version` and the build-time version. Find the spot with
  `rg -n "daemon status|Request::List" docs/architecture.md`.
- `docs/backlog.md`: add row `| C23 | Build version stamped into the binary; the daemon reports it in daemon status | spec 3s |`
  after row C22.

## 7. Out of scope

A release tagging scheme; bumping `Cargo.toml`'s version; a `version` subcommand; `list`/`list_agents`
or MCP output; a protocol-compatibility integer; checks from MCP, hook, bridge or channel processes;
auto-restarting a mismatched daemon; refusing to run on a mismatch; `scripts/install.sh` changes; any
change to the hello exchange; a test that builds a second, differently stamped binary.

## 8. Acceptance

Post-implementation commands (new code required), run from the main checkout with the ambient
`A2AMX_BIN` and `NO_COLOR` unset:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
git status --short   # only files listed in section 0 scope (the committed spec is not listed)
```

All three pass with no warnings. Then a manual probe on an isolated daemon (temp `--home`, the
built `target/debug/a2amx` as both CLI and daemon): `a2amx --version` prints `a2amx <V>`, and
`a2amx daemon status` prints a `version: <V>` line with empty stderr.

## 9. Gate notes (pre-lock)

- Baseline (develop `9114ce3`, `env -u A2AMX_BIN -u NO_COLOR cargo test`): the first full run failed
  `attach_cli::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (`tests/attach_cli.rs:153`, a
  timing wait); three reruns of `cargo test --test attach_cli mouse_wheel` passed 2/2. Treat it as a
  load-dependent flake: if it fails in the developer's full run, rerun that test alone and report
  both results, not a regression of this spec.
- Scope reconciliation: `rg -n "daemon.*status|--version|sessions: " tests README.md docs/architecture.md`
  matched only `tests/daemon_cli.rs` (in scope), `tests/omp_smoke.rs` (the OMP binary's own
  `--version`, unrelated) and README line 98 (in scope).
- `Request`/`Response` are matched without wildcards only in `src/daemon.rs` (the request arm); the
  client side matches responses with `_ =>`. A compile error in any other `src/` file means stop and
  report `BLOCKED`.
