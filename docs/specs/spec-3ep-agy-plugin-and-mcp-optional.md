# Spec 3ep: `a2amx mcp --optional`, the `agy` plugin and its launch install (spec 2a of 4)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`agy-harness`. Spec 2a of the `agy` work: gives an `agy` session the a2amx tools. Spec 3eq (confinement of
`~/.gemini`) follows, then spec 3 (typed delivery) and spec 4 (docs, team file).

**Scope.** May edit exactly these files and no others.
- `src/agy.rs` (new), `src/lib.rs` (one `pub mod agy;` line), `src/layout.rs` (two path functions)
- `src/mcp.rs`, `src/cli.rs`, `src/main.rs`, `src/harness.rs` (`wire_agy_argv` only)
- `tests/mcp.rs` (inert-server tests), `tests/agy_plugin.rs` (new), `tests/agy_wiring.rs`, `tests/daemon_cli.rs`
  (only the `ExitFixture` agy launch and its test, section 6)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md` (the architect adds the `agy` row to its
Modules table), `Cargo.toml`, `Cargo.lock`, `src/confine.rs`, `src/daemon.rs`, `src/session.rs`,
`src/delivery.rs`, `src/emulator.rs`, `src/team.rs`, every other spec. Do not commit, stage or merge.

**Public test seams.** The `a2amx mcp --optional` command as a real process (the `tests/mcp.rs` helper
`spawn`), `agy::install` against a stub `agy` program (a shell script in a temp directory),
`harness::wire_agy_argv`.

## 1. Why

An `agy` session needs the a2amx tools (`send_message`, `list_agents`, ...). `agy --settings` rejects
`mcpServers`, and the owner chose not to edit the user's `mcp_config.json`. The probes established
(`agy` 1.5.9, 2026-11-14/15): a plugin copied by `agy plugin install <dir>` into `~/.gemini/config/plugins/`
carries a stdio MCP server; install is non-interactive, needs no login or network, is idempotent and
supports a `version` field; the plugin's MCP server starts at `agy` launch in **every** `agy` session of the
user and inherits the launch environment; the tool name is `<plugin>_<server>/<tool>`; a permission rule
`mcp(<plugin>_<server>/*)` in `--settings` permissions suppresses the approval card for one session.

So the plugin must (a) be installed automatically by the launcher, (b) do nothing in a session that a2amx
did not start, and (c) be allowed per session without touching the user's global settings.

Decisions (owner, grilling): the plugin is always installed when an `agy` session is launched by `a2amx new`
or `team up`; if no a2amx session environment exists the plugin does nothing; a failed install fails the
launch; the user's global settings are never edited. Known limits, documented by spec 4: the plugin is global to the user, so two a2amx homes on one machine share
it and the last launcher's `<root>/bin/a2amx` wins (installs ping-pong when two homes run the same version);
a home that is deleted leaves the plugin pointing at a missing binary until another launch repairs it; a
plain `agy` started by an agent inside an a2amx session inherits that session's `A2AMX_ADDR`/`A2AMX_TOKEN`, so
its tools act as the parent session (no new authority: the child already holds the token).
Rejected: a workspace `.agents/plugins/` copy (writes
into the user's project); editing `~/.gemini/antigravity-cli/mcp_config.json` (the user's file); a separate
opt-in switch (the owner wants it always on).

## 2. `a2amx mcp --optional` (`src/cli.rs`, `src/mcp.rs`, `src/main.rs`)

1. `Command::Mcp` gains `#[arg(long)] optional: bool`, documented: "serve no tools when this process was not
   started by an a2amx session". `main` passes it to `mcp::run(channel, optional)`.
2. In `mcp::run`: when `optional` is true and **neither** `A2AMX_ADDR` **nor** `A2AMX_TOKEN` is present in
   the environment (`std::env::var_os` is `None` for both), run the inert server; in every other case
   (`optional` false, or at least one variable present) behave exactly as today, including the existing
   `A2AMX_ADDR and A2AMX_TOKEN must be set` error when the pair is incomplete or malformed. `--channel` with
   an inert server is ignored.
3. The inert server is the existing `McpServer` loop with a flag `inert`: `initialize`, `notifications/*`
   and `ping` behave as today; `tools/list` returns `{"tools": []}`; `tools/call` returns the JSON-RPC
   error `-32602` with message `a2amx is not attached to this session`. It never connects to a daemon
   and exits 0 at stdin EOF.
4. Default stays strict: `a2amx mcp` without `--optional` and without the variables still exits non-zero
   with the existing message (contract test, passes on the old code).

## 3. `src/layout.rs`

Add `pub fn agy_dir(&self) -> PathBuf { self.run_dir().join("agy") }` next to `omp_dir`, and `pub fn
installed_binary(&self) -> PathBuf { self.root.join("bin/a2amx") }` (the path `scripts/install.sh` installs to).
`Layout` stays the only place that names these paths.

## 4. `src/agy.rs` (new module: the embedded plugin and its install)

Owns: the plugin's files, the plugin and server names, the permission rule, the install procedure. It spawns
the `agy` program only for `plugin validate`, `plugin list`, `plugin install`.

- `pub const PLUGIN: &str = "a2amx"; pub const SERVER: &str = "tools";`
- `pub fn tool_rule() -> String` returns `mcp(a2amx_tools/*)` built from the two constants.
- `fn manifest() -> String`: JSON `{"name":"a2amx","version":"<CARGO_PKG_VERSION>","description":"a2amx
  messaging tools for agents started by a2amx"}`.
- `fn mcp_config(binary: &Path) -> String`: JSON `{"mcpServers":{"tools":{"command":"<binary>","args":["mcp","--optional"]}}}`
  (built with `serde_json`, never string-formatted).
- `pub fn install(layout: &Layout, agy_program: &OsStr) -> anyhow::Result<()>` (call from a blocking
  thread), in this order:
  1. `binary = layout.installed_binary()`; it must be an existing regular file, else error `a2amx is not
     installed at <path>; the agy plugin needs the installed binary there`. Then run `<binary> mcp --optional`
     with `env_clear()`, stdin `/dev/null`, a 5 s timeout: it must exit 0 at EOF, else error `the installed
     a2amx at <path> does not support mcp --optional; reinstall it` (an older install would make the plugin's
     server fail in every agy session).
  2. Create `layout.agy_dir()` mode 0700; open `agy_dir/install.lock` (0600) and take an exclusive
     `File::lock`, held until return.
  3. Remove any `.install-*` file left in `agy_dir/plugin` by a crashed earlier run (agy copies extra files
     into the global plugin), then write `agy_dir/plugin/plugin.json` and `mcp_config.json` with `omp::write_if_changed` (parent mode 0700).
  4. Run `<agy_program> plugin validate <agy_dir/plugin> --strict --json`. Exit 0 is valid (measured: notes
     such as "no logo" pass even with `--strict`; any error or warning exits 1 with `Error: validation failed...`
     on stderr and the findings JSON on stdout). A non-zero exit is an error carrying at most 2 KiB of stderr.
  5. Run `<agy_program> plugin list --json` (a JSON array of `{name, enabled, version, source, path,
     mcpServers[, hooks]}`; `[]` when empty). Find the entry named `a2amx`. Decide:
     - no entry: install;
     - entry with a version that parses as `MAJOR.MINOR.PATCH` and is **greater** than ours: skip (never
       downgrade; `agy` itself replaces with any version, measured);
     - entry with an equal or lower version: install when its version differs from ours or when the installed
       `plugin.json` / `mcp_config.json` (read from the entry's `path`) differ byte for byte from ours (the
       version is not bumped between releases, so content decides); otherwise skip;
     - an entry whose version does not parse: install.
     `enabled: false` is never changed by a2amx: a reinstall keeps the user's disable (measured), and a
     disabled plugin simply provides no tools; this is documented in spec 4.
     Install is `<agy_program> plugin install <agy_dir/plugin>`; non-zero exit is an error carrying at most
     2 KiB of stderr. After install, re-run `plugin list --json` and error if the entry's version is not ours
     or the file contents still differ.
  Every subprocess uses an argument vector and stdin `/dev/null`; its output is read on helper threads while
  the parent waits with a `try_wait` loop; after the 30 s timeout the child is killed and the call fails
  without waiting for its pipes to close more than 1 s. Reads are bounded (rule 26): stderr 2 KiB kept, `plugin
  list` stdout at most 64 KiB (more is an error), each installed file read for the comparison at most 64 KiB
  (larger counts as different). `plugin validate` output is only passed through on failure.
- The `agy` program is `command[0]` when its basename is `agy`, otherwise `agy` found on `PATH` (a wrapper
  such as `sh -c` or `env X=1 agy` does not name the program). An empty `command` is the error `empty command`.
- Errors name the step. Nothing is swallowed: any failed step returns an error.

## 5. `src/main.rs` and `src/harness.rs`

1. `wired_command`, `Harness::Agy` arm: when `install_omp` is true (the flag the callers already pass as
   `true` for `new` and `false` for the dry run; this is the "install" switch, not OMP-specific; rename it
   `install_support` only if the developer finds the diff stays minimal, otherwise leave the name) call
   `agy::install(&layout, &program)` (program chosen as in section 4) through `spawn_blocking` before wiring `--settings`.
2. `wire_agy_argv`: add `permissions.allow` containing `agy::tool_rule()`, merged into any user
   `permissions.allow` array (added once; a non-object `permissions` or non-array `allow` is the error
   `--settings permissions.allow must be an array`). `toolPermission` is never set.

## 6. Tests (write first)

`tests/mcp.rs`: (1) inert: `--optional` with both variables removed from the environment answers `initialize`,
`tools/list` (`[]`), `ping` and `tools/call` (`-32602`, message `a2amx is not attached to this session`) and
exits 0 at EOF; (2) a sibling of the existing `mcp_missing_or_invalid_environment_exits_without_protocol_output`
(`tests/mcp.rs:808`, which stays unchanged and is the contract test for the strict default, passing on the old
code) that runs the same five environment cases with `--optional` and expects the same exit code 1, empty
stdout and identical stderr: partial or malformed variables stay strict (the first and second cases are the
single-variable cases and prove one present variable is not "absent"). The inert test spawns the binary with
`Command` and `env_remove` of both variables (the suite itself runs inside a2amx sessions that export
them); extending the `spawn` helper for that is allowed.
`tests/agy_plugin.rs`: a stub `agy` shell script in a temp directory that records its argument vectors to a
file and emulates the three subcommands with the measured behaviour: `validate <dir> --strict --json` exits 0
(or 1 with `Error: validation failed: 1 error` on stderr when the dir's `mcp_config.json` is not valid
JSON), `list --json` prints a JSON array read from a state file, `install <dir>` copies the dir into a temp
"plugins" directory and writes the state file with the dir's version (never refusing a lower one, like
`agy`). Cases, each asserting the recorded argument vectors and the resulting state:
fresh install; second call skips (no second `install`); installed higher version is skipped (no downgrade)
and the call succeeds; installed lower version is replaced; equal version with a different installed
`mcp_config.json` is replaced; unparseable installed version is replaced; an installed `enabled: false` stays `false`
after a replace, the call succeeds, and the recorded argument vectors contain no `enable`, `disable` or
`uninstall`; invalid plugin (stub validate exits 1) makes the call fail
with the stderr text and no `install` recorded; missing `bin/a2amx` fails with the message and runs no `agy`
subcommand; `command[0]` of `sh` uses PATH `agy` (stub on PATH) and an empty command errors; a stub that runs `exec sleep 60` (so no grandchild holds the pipes) is killed at a short timeout and the
call fails quickly (seam: `pub fn install_with_timeout(layout, agy_program, timeout: Duration)`, which
`install` calls with 30 s); a `<root>/bin/a2amx` stub script that exits 2 for `mcp --optional` makes the call
fail with the 'does not support' message and runs no `agy` subcommand; a stray `.install-1-0` file in
`agy_dir/plugin` is removed before the files are written; two threads calling `install` at once against one layout record exactly one
`install` and both return `Ok`; the rendered `mcp_config.json` equals the literal
`{"mcpServers":{"tools":{"args":["mcp","--optional"],"command":"<temp root>/bin/a2amx"}}}` (key order as
`serde_json` writes it) and `plugin.json` equals the literal manifest with the crate version.
`tests/agy_wiring.rs`: the allow rule is present exactly once by default (red on the old code); merged with a
user allow list (one assertion: both the user entry and ours); the error case (non-array `allow`).
`tests/daemon_cli.rs`: the `ExitFixture` agy launch (`kill_sends_agy_two_key_exit_sequence`, line ~1457 and the
fixture at ~1364) runs `a2amx new --harness agy`; after this spec `new` installs the plugin first, so the
fixture creates `<root>/bin/a2amx` as a stub that exits 0 for `mcp --optional` and passes a stub script
named `agy` as the command (it answers `plugin validate|list|install` with the success outputs and otherwise
`exec`s the existing python fixture). Behaviour asserted by the test is unchanged.

## 7. Out of scope

Hooks and PreInvocation receipts (spec 3); confinement and `~/.gemini` grants (3eq); typed delivery;
docs; uninstall; marketplace install; a `--no-plugin` switch; per-session plugin disable.

## 8. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass (repo commands).
2. `git status --short` lists exactly the scoped files; `src/agy.rs` and `tests/agy_plugin.rs` are new.
3. `git diff --stat -- src/confine.rs src/daemon.rs src/session.rs src/delivery.rs src/emulator.rs` prints nothing.
4. The existing strict-default test (`tests/mcp.rs:808`) is the contract test and passes on the old code. Every new test
   fails on the old code (compile-red counts); the developer names the red evidence per test.
5. Mutation check, in the checkout with `sha256sum` restore: break each of (a) the `--optional` env test
   (always strict), (b) the never-downgrade comparison, (c) the content comparison on equal version,
   (d) the lock, (e) a call to `plugin enable` after install, (f) the allow-rule merge, (g) the `mcp --optional`
   support check; name
   the failing test per mutant. The tester reruns them.
6. Trust-boundary check (`trust-boundaries`): the plugin runs `a2amx mcp --optional` in every `agy` session
   of the user. Abuse case: a session without the variables must expose no tool and open no connection
   (test 6.1 asserts `tools/list` is empty and `tools/call` fails). The plugin carries no token: `mcp_config.json` has no `env` key (asserted by the
   literal test). The binary path is the installed `<root>/bin/a2amx` and is validated to exist before use.
7. Post-implementation live check (architect, not a developer gate): with a real `agy` under the owner's
   authenticated session, a session started by `a2amx new --harness agy` lists `a2amx_tools/*` tools and calls
   `list_agents` without an approval card; a plain `agy` shows no a2amx tool. The tool name
   `a2amx_tools/<tool>` rests on one earlier observation of the `<plugin>_<server>` pattern.

## 9. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `mcp::run(channel)` is called once, from `main`; `Command::Mcp` has one field | `src/main.rs:169-171`, `src/cli.rs:186`; `rg "mcp::run" src tests` | read, grep |
| Strict env test and its five cases | `tests/mcp.rs:808-836` | read |
| `wired_command` install flag: `true` for `new`, `false` for the dry run | `src/main.rs:1254-1263`, `1693-1702` | read |
| `omp::write_if_changed` is `pub` and writes a `.install-<pid>-<n>` temp beside the target | `src/omp.rs:39-70` | read |
| `scripts/install.sh` installs to `<dest>/bin/a2amx` | `scripts/install.sh:7-15` | read |
| `ExitFixture` launches `new --harness agy -- sh -c ...` | `tests/daemon_cli.rs:1364-1384`, test at `1457` | read (consultant) |
| `agy plugin validate/list/install/disable/uninstall` behaviour (exit codes, JSON shapes, no downgrade protection, disable kept) | tester probes P10, 2026-11-15 | tester measurement, not rerun |
| Tool name `<plugin>_<server>/<tool>` and the allow rule `mcp(<plugin>_<server>/*)` | earlier tester probe L | one observation; section 8.7 rechecks live |
| `File::lock` exists on the pinned toolchain | `rust-toolchain.toml` 1.98.1, `Cargo.toml` rust-version 1.89 | read (consultant); stable since 1.89 |
| No other test launches an agy session | `rg -n "harness agy" tests` | grep (consultant) |

## Amendment 1 (2026-11-15, architect, after the tester's pass on the first implementation)

Not contradictions of the locked text; the tester measured gaps the text implied but did not state. They bind the
fix round.

1. **Pipes and timeout.** After the child exits, the reader threads are waited for at most 1 s; a pipe still
   open after that is the error `agy command left its output pipe open`. The 30 s timeout still kills a child that
   does not exit. The readers keep draining their pipe after the limit (a full pipe must never block the child):
   stderr keeps the first 2 KiB and drops the rest with no error; stdout of `validate` and `install` keeps the
   first 64 KiB and drops the rest with no error (it is only passed through on failure); only `plugin list`
   stdout over 64 KiB is an error, `agy plugin list output exceeds 64 KiB`. A non-zero exit therefore always
   reports `agy plugin <step> failed: <first 2 KiB of stderr>`.
2. **List entries.** `version` is optional in a `plugin list --json` entry; an entry without it, or with a
   version that does not parse, takes the "install" branch; an unrelated entry without `version` no longer
   fails the whole list.
3. **Environment.** Every `agy` subprocess (`validate`, `list`, `install`) runs with `A2AMX_TOKEN`,
   `A2AMX_ADDR` and `A2AMX_SESSION` removed (`env_remove`); the support probe of the installed binary keeps
   `env_clear()`. Reason: a launcher started inside an a2amx session holds that session's token, and `agy` plugin
   commands do not need it.
4. **Tests added** (`tests/agy_plugin.rs`): a stub whose `list` reports a wrong version or different files after
   `install` makes the call fail (post-install verification); a 100 KiB `list` stdout is the named error with no
   `install` recorded; the support-probe stub records its argv (exactly `mcp --optional`) and its environment (no
   `A2AMX_TOKEN` and no `PATH` with `env_clear`); the `agy` stubs record their environment names and none of the
   three session variables appear even when the test process sets them; a `validate` stub that exits 0 but leaves
   a background `sleep 5` holding its pipes fails within 3 s at a 300 ms timeout; a failing `validate` with 3000
   bytes of stderr reports exactly the first 2048 bytes; a 100 KiB `validate` stdout with exit 0 succeeds; a
   list entry for `a2amx` without `version` installs; the PATH-modifying test restores `PATH` with a drop guard.
5. **Known limit (comment, no code):** a relative `command[0]` such as `./agy` is resolved against the
   launcher's directory, not the session's `cwd`; the installer notes this with a `shortcut:` comment in
   `src/main.rs`.
