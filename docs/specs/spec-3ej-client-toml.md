# Spec 3ej: `client.toml`

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from `e1e65db`.
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. This is spec 4 of 7 in the owner's configuration
redesign (root and layout, accepted as 3eg; config loader and `daemon.toml`, 3eh; `gateway.toml`, 3ei; client
config; team file rules; per-project auto grants; migration; presence per session). The flags `--prefix` and
`--viewer-after` and their environment variables keep working; nothing about them breaks. A new file is read by
every command, so an invalid `client.toml` is a new way for a command to fail (section 1).

Public test seams: the `a2amx::config` module API (`load_client`, `ensure_client_file`, `show_client`,
`client_template`) in `tests/config.rs`; `a2amx::prefix::format_prefix` in `tests/prefix.rs`; the `a2amx` binary
(`list`, `confine list`, `daemon start --foreground`, `config show`, `attach`) in `tests/daemon_cli.rs` and
`tests/attach_cli.rs`.

**Scope** (files that may be edited or created):

- `src/config.rs`, `src/prefix.rs`, `src/layout.rs` (one accessor), `src/cli.rs`, `src/main.rs`.
- `tests/config.rs`, `tests/prefix.rs`, `tests/layout.rs` (one table row), `tests/daemon_cli.rs`, `tests/attach_cli.rs`.
- `docs/configuration.md`, `docs/cli.md`, `docs/remote-messaging.md`.

Does **not** touch: `src/relay_files.rs`, `src/relay.rs`, `src/gateway_launch.rs`, `src/daemon.rs`, `src/client.rs`,
`src/team.rs`, `src/messaging.rs`, `gateway.toml`, `daemon.toml` keys, `team.toml`, migration, `AGENTS.md`,
`CHANGELOG.md`, `README.md`, `docs/architecture.md`, `docs/backlog.md`, earlier specs (the architect updates the
changelog, the `AGENTS.md` module table and the architecture page when committing).

## 1. Why

The attach client has two settings, both global flags read on every command: `--prefix` (`A2AMX_PREFIX`, default
`C-b`, parsed by `prefix::parse_prefix` through clap's `value_parser`) and `--viewer-after` (`A2AMX_VIEWER_AFTER`,
default 300 seconds, `0` disables; `src/cli.rs:15-28`). `AttachOptions` is built from them at the top of
`dispatch` (`src/main.rs:151-154`). The owner's redesign puts every user-facing setting into a file under
`config/` with defaults in the code.

Owner decisions (grilled 2026-11-14, recorded; earlier rules of the redesign apply as for `daemon.toml`):

- **Keys: `prefix` and `viewer_after` only.** The flags and environment variables stay as overrides. Rejected:
  defaults for `new` and `team up`, status-bar and picker options (no named need).
- **`daemon start` writes the commented template once**, next to `daemon.toml`; it never rewrites it. Client
  commands only read, so `a2amx list` creates nothing. Rejected: an explicit `config init` command; the first
  `attach` or `new` writing it.
- **An invalid `client.toml` makes commands fail**, naming the file and the key, like `daemon.toml`. Rejected:
  failing only `attach` and `new`.
- Defaults are Rust constants; precedence flag, environment, file, default; durations are strings; every value
  that makes sense for a user is configurable.

Decisions of this spec that the owner did not grill (sent to the owner with the review of this spec):

- **The harness-run adapters do not read `client.toml`**: `mcp`, `hook`, `statusline`, `omp-bridge`, and the
  early-dispatch `confine list` and `confine suggest` (`src/main.rs:156-199`) return before `resolve_home`
  today and keep doing so. A broken file must not stop an agent's hooks or MCP server; these commands use
  neither setting. Every other command, `daemon start` included, fails on an invalid file.
- **`viewer_after` is a duration string** (`"5m"`, `"0s"` = never), whole seconds, `0s` allowed, ceiling `3650d`
  (the `WholeSecondsOrZero` rule). The flag and the environment variable stay integer seconds.
- **`config show` gets a third section.** Its source label is `override` (clap merges the flag and the
  environment variable, so show cannot tell them apart), not `env`.
- **`prefix` in the file uses the existing `parse_prefix` notation** (`"C-b"`, `"C-space"`); show prints it back
  through a new `format_prefix`, in the canonical lowercase form (`"C-B"` prints `"C-b"`).
- **The client table lives in `src/config.rs`** (spec 3eh decided that later specs add their file's table
  there). `config.rs` is 990 lines today and grows by about 200: a closer look per the placement check. It stays
  cohesive (it owns configuration files; the key tables share `resolve`, the duration rules, `read_config` and
  `ensure_file`), so no split is in scope; splitting it into a directory module with one file per config is
  raised to the owner as a follow-up once the team-file spec (5a) lands. Rejected: a `config_client` module now
  (needs the shared helpers extracted first).

## 2. `src/prefix.rs`

`pub fn format_prefix(byte: u8) -> String`, the inverse of `parse_prefix` for every byte it returns: `0` is
`C-space`, `1..=26` except `9`, `10` and `13` is `C-a`..`C-z`, `0x1c` `C-\`, `0x1d` `C-]`, `0x1e` `C-^`, `0x1f` `C-_`. A byte
`parse_prefix` never returns (`0x09`, `0x0a`, `0x0d`, `0x1b` and everything above `0x1f`) prints as
`0x<two hex digits>`; no caller passes one, the form only makes the function total. Property: for every spec
string `parse_prefix` accepts, `parse_prefix(&format_prefix(parse_prefix(spec)?))` returns the same byte.

## 3. `src/layout.rs`

`pub fn client_config(&self) -> PathBuf` returns `config/client.toml`, beside `daemon_config`. Table row
`(layout.client_config(), "/r/config/client.toml")` in `tests/layout.rs`.

## 4. `src/config.rs`

Everything here is added or extended; the daemon and gateway tables and their behaviour stay.

1. **Reader.** `read_config` (used by `daemon.toml`, now also by `client.toml`) names the file in its three literal
   `daemon.toml` texts through `path.file_name()` (`src/config.rs:273,280,283`: `is not a regular file`,
   `exceeds 65536 bytes`, `is not UTF-8`); the daemon file's messages read the same as today.
2. **Types.** `pub struct ClientSettings { pub prefix: u8, pub viewer_after: Option<Duration> }` (`None` means the
   timer is off); `pub struct ClientOverrides { pub prefix: Option<u8>, pub viewer_after_secs: Option<u64> }`;
   `pub struct LoadedClient { pub settings: ClientSettings, pub sources: Vec<(&'static str, Source)> }`. Defaults,
   as constants in `config.rs` replacing the attribute literals at `src/cli.rs:22,27`: `prefix` byte `0x02`
   (`C-b`), `viewer_after` 300 seconds. `sources`
   has one entry per key in table order: `Default`, `File` or `Override`.
3. **File.** A private `ClientFile` (`#[serde(deny_unknown_fields)]`): `prefix: Option<String>`,
   `viewer_after: Option<String>`.
   `pub fn load_client(layout: &Layout, overrides: &ClientOverrides) -> anyhow::Result<LoadedClient>` reads
   `layout.client_config()` with `read_config` (an absent file is defaults), parses with `toml`, validates in
   this order with the form `<path>: key <name>: <problem>` (`file_error`): `prefix` through `parse_prefix` (its
   message is carried inside the key-naming wrapper), `viewer_after` through `parse_limited_duration` with
   `DurationRule::WholeSecondsOrZero` (`0s` allowed, a sub-second value rejected, ceiling `3650d`). The
   `viewer_after_secs` override is not range-checked: the flag and the environment variable stay as they are today
   (any `u64`, `0` off; `tests/attach_cli.rs:2612-2614` runs `--viewer-after 18446744073709551615` and expects
   the session to stay interactive); only the file value has the `3650d` ceiling. Resolution per key through
   `resolve`. `viewer_after` of zero seconds becomes `None` in `ClientSettings`.
4. **Key table, template, writer, show.** A `CLIENT_KEYS: &[ClientKey]` table with a new private
   `struct ClientKey { name, description, example: &'static str, display: fn(&LoadedClient) -> String }` (the
   `Key` and `GatewayKey` types are not reused or generalised). `client_template() -> String` in the daemon
   template's form (`# <name>: <description>` then `# <name> = <example>`), header
   `# A2AMX client settings. Every a2amx command reads this file, so a change applies to the next command. The daemon writes this file once and never rewrites it.`,
   examples `"C-b"` and `"5m"`. `ensure_client_file(layout)` calls the shared `ensure_file` (`create_new`, `0600`).
   `show_client(layout, &ClientOverrides) -> anyhow::Result<String>` reads, never creates: section 6 text,
   `prefix` printed through `format_prefix`, `viewer_after` through `format_duration` (`0s` for off),
   labels `default`, `file`, `override`, produced by the existing `gateway_source` function (rename it to
   `source_label`; no third copy of the match). The template body loop is the same shape as `gateway_template`'s;
   share one private helper over `(name, description, example)` triples rather than writing a third copy.

## 5. `src/cli.rs` and `src/main.rs`

1. `src/cli.rs:15-28`: `prefix` becomes `Option<u8>` and `viewer_after` `Option<u64>`: no `default_value`, the
   `env` and `value_parser = parse_prefix` attributes stay, doc lines say the default comes from `client.toml`,
   then `C-b` and 300 seconds. `--home` is unchanged.
2. `src/main.rs`, `dispatch`: remove `attach_options` from the top (`:151-154`). After `resolve_home` (`:200`, past
   the early-dispatch block that returns for the adapters) call `config::load_client` on `spawn_blocking` with
   `ClientOverrides` from `cli.prefix` and `cli.viewer_after`, and build `AttachOptions` from the returned
   settings. A bad file is the command's error: non-zero exit, message on standard error naming the file and key.
3. `run_daemon` (`:451`): after `config::ensure_daemon_file` call `config::ensure_client_file` inside the same
   `spawn_blocking` closure. `run_daemon_background` does not write (the child does).
4. `Command::Config { action: Show }` arm (`:209-230`): the output is the daemon section, a blank line, the
   gateway section, a blank line, then `config::show_client` with the same overrides.

## 6. `a2amx config show` output

After the gateway section (spec 3ei section 5): one blank line, a first line
`client.toml: <path> (<present|absent>)`, then one line per key in table order,
`<key> = <value>  (<default|file|override>)`, strings as TOML strings. Example, no file and no overrides:

```
client.toml: /home/user/.a2amx/config/client.toml (absent)
prefix = "C-b"  (default)
viewer_after = "5m"  (default)
```

With `A2AMX_PREFIX=C-A` in the environment the line is `prefix = "C-a"  (override)`. A bad file makes
`config show` fail with the load error.

## 7. Trust boundary

`client.toml` is operator-written and read by every client command, including commands that run inside confined
sessions (where `config/` is read-only). Attack paths:

| Path | Disposition | Abuse-case test |
| --- | --- | --- |
| A confined agent edits `client.toml` (remap the prefix, change the viewer timer) | Removed by 3eg: `config/` is read-only in confined sessions; an unconfined agent already has the operator's rights | none new; stated in `docs/configuration.md` |
| A malformed or oversized file makes every command fail, including an agent's hooks | Constrained: the adapters (`mcp`, `hook`, `statusline`, `omp-bridge`, `confine list`, `confine suggest`) never read the file; regular file only, at most 64 KiB, UTF-8, `deny_unknown_fields` | new test: with an invalid `client.toml`, `confine list` exits 0 and `list` fails naming the file and key |
| Non-regular file (FIFO, device) at the path stalls every command | Constrained: `read_config` opens non-blocking and checks the opened file's type | contract: 3eh's FIFO and directory tests cover `read_config`; one new test: a FIFO at `client.toml` makes `load_client` fail without blocking |
| A dangling symlink makes the template write go elsewhere | Removed: `create_new` never follows or replaces | new test, as for the other two templates |
| Values that disable a protection or overflow: `viewer_after = "3651d"` in the file, a prefix that swallows keys | Constrained: ceiling on the file value (the flag keeps today's range), and `parse_prefix` already rejects ESC, Tab, LF and CR | tests for `"3651d"` and `prefix = "C-["` |

## 8. Docs

- `docs/configuration.md`: a `client.toml` section with the full template verbatim (test 12), the key table, the
  override rule (flag or environment beats the file), "every command reads it except the harness adapters", and
  "written once by `daemon start`", and one sentence that an invalid file also stops `daemon stop` and
  `gateway up|stop|status` until it is fixed (the adapters and `confine list|suggest` are exempt). A malformed
  `A2AMX_PREFIX` or `A2AMX_VIEWER_AFTER` environment value fails every command at argument parsing, adapters
  included, as it does today.
- `docs/cli.md`: `--prefix` and `--viewer-after` (`:17`, `:192-194`, `:206`) say the default comes from `client.toml`;
  the `config show` line mentions the client section.
- `docs/remote-messaging.md:28`: `--viewer-after` / `viewer_after` in `client.toml`.

## 9. Tests

`tests/prefix.rs`: (1) the round trip of section 2 over every accepted spec string already in the file's tables
plus `C-B` (prints `C-b`); (2) literals: `format_prefix(0)` is `C-space`, `2` is `C-b`, `0x1c` is `C-\`,
`0x1f` is `C-_`, `27` is `0x1b`, `9` is `0x09`.

`tests/config.rs` (temp root with `Layout::new` and `Layout::ensure`; expected values are literals):

3. `load_client` defaults: no file, no overrides: `prefix == 2`, `viewer_after == Some(300s)`, both sources `Default`.
4. A file with `prefix = "C-a"` and `viewer_after = "10m"`: `1` and `Some(600s)`, both `File`; `viewer_after = "0s"`
   gives `None`.
5. Overrides beat the file per key: `prefix` override `Some(24)` with a file `prefix`: `24`, `Override`; the other key
   keeps its file source.
6. Parse errors, each message contains the file name and the key: unknown key, `prefix = "x"`, `prefix = "C-["`,
   `prefix = ""`, `prefix = 5` (type), `viewer_after = "1500ms"`, `"5w"`, `""`, and
   `viewer_after = "3651d"` (assert the text `between 0s and 3650d`, so the test reaches the ceiling). An
   override of `u64::MAX` seconds loads (`Some(Duration::from_secs(u64::MAX))`, source `Override`).
7. Template round trip: `ensure_client_file` into an empty config dir loads as defaults; uncomment every
   assignment (lines matching `^# [a-z_]+ = `) and it loads to the defaults again; file mode `0o600`.
8. `ensure_client_file` leaves an existing file byte for byte unchanged (empty, invalid body); a dangling symlink:
   `Ok`, target not created, link unchanged; the FIFO case of section 7.
9. `show_client`: section 6's example exactly (temp path in the first line); with `prefix = "C-B"` in the file the
   line is `prefix = "C-b"  (file)`; with an override `Some(1)` it is `(override)`; no file or directory is created.
10. The daemon file's messages still read `daemon.toml ...` (the three `read_config` literals): the existing tests
    pass unchanged.
11. `client_template()` contains no `daemon.toml` or `gateway` wording.
12. `docs/configuration.md` contains `client_template()` verbatim (`include_str!`).

`tests/daemon_cli.rs` (the `a2amx` binary, `--home` in a temp directory; tests 13-15 remove `A2AMX_PREFIX` and
`A2AMX_VIEWER_AFTER` from the child environment, as the existing `config show` test removes
`A2AMX_OWNER_AWAY_SECS`):

13. `daemon start --foreground` in a fresh root creates `config/client.toml` containing `# prefix`.
14. With `config/client.toml` holding `prefix = "x"`: `list` exits non-zero and stderr names `client.toml` and `prefix`;
    `confine list` exits 0; the daemon is not started by either.
15. `config show` on a fresh root prints the daemon, gateway and client sections separated by one blank line each
    (the existing exact test is extended: 11 + 8 + 2 key lines, two blank lines, three first lines, 26 lines in all) and creates no
    config file.

`tests/attach_cli.rs`: 16. a variant of the `A2AMX_PREFIX` attach test (`:985-1000`) with the prefix set only in
`client.toml` (write or overwrite `config/client.toml` with `prefix = "C-a"`; the root may already hold a
daemon-written template), no `A2AMX_PREFIX` in the environment: Ctrl-A then `d` detaches; and the existing environment test
still passes unchanged (environment beats file; add a file `prefix = "C-x"` to a copy of it to pin that).

Contract tests (pass on the old code once written against unchanged behaviour): the existing environment attach
test; `tests/prefix.rs` `parse_prefix` tables; every test that runs the binary with the default prefix.

## 10. Out of scope

- Other client keys (command defaults, status bar, picker); a flag per key; a `config init` command; hot reload.
- `team.toml`, per-project grants, migration, presence; `gateway.toml` and `daemon.toml` keys.
- Splitting `src/config.rs` (follow-up for the owner).
- `CHANGELOG.md`, `AGENTS.md`, `docs/architecture.md`, `docs/backlog.md`: architect, at commit.

## 11. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `--prefix` has `env = "A2AMX_PREFIX"`, default `C-b`, `value_parser = parse_prefix`; `--viewer-after` has `env`, default 300 | `src/cli.rs:15-28` | read |
| `AttachOptions` is built at the top of `dispatch`, before any command match | `src/main.rs:151-154` | read |
| The early-dispatch block returns for `mcp`, `hook`, `statusline`, `omp-bridge`, `confine list`, `confine suggest` before `resolve_home` | `src/main.rs:156-199,200` | read |
| `cli.prefix` and `cli.viewer_after` are used only at `src/main.rs:152-153` | grep `cli\.prefix\|cli\.viewer_after` over `src` | grepped |
| `run_daemon` calls `ensure_daemon_file` then `load_daemon` in one `spawn_blocking`; the background parent does not write | `src/main.rs:451-468` | read |
| `read_config` carries three literal `daemon.toml` texts | `src/config.rs:273,280,283` (at `e1e65db`) | grepped `daemon\.toml` over `src/config.rs` |
| `parse_prefix(&str) -> Result<u8>` accepts `C-space`, `C-a..C-z`, `C-\`, `C-]`, `C-^`, `C-_`, rejects `C-[`, `C-i`, `C-j`, `C-m` | `src/prefix.rs:34-86` | read |
| No `format_prefix` exists | grep `format_prefix` over `src tests` | grepped |
| The env attach test sets `A2AMX_PREFIX=C-a` and detaches with Ctrl-A `d` | `tests/attach_cli.rs:985-1000` | read |
| Only `tests/attach_cli.rs` names `A2AMX_PREFIX`; no test names `A2AMX_VIEWER_AFTER` | grep over `tests src` | grepped |
| `Layout` has no `client_config` | `src/layout.rs:100-112` | read |
| The gateway sandbox is a tmpfs root with only `link/`, `gateway.toml`, `discord.token` and the gateway state bound (`--tmpfs /`, then `--ro-bind` of the two config files), so inside it `config/client.toml` does not exist; `gateway run` reaches `resolve_home` and so loads the client settings, gets not-found and uses defaults, which is not an error | `src/gateway_launch.rs:36,57-83`, `tests/gateway_up.rs:28,67` | read |
| `gateway_proc` and `gateway_launch` call no client settings | grep `AttachOptions\|viewer_after` over `src` | grepped (`src/main.rs` only) |
| Baseline at `e1e65db`: `cargo test --test config --test prefix --test attach_cli --no-fail-fast` pass (48, 22 and 14 tests in the three result lines); `--test daemon_cli` passed 71 at the 3ei acceptance on the same code | run by the architect at lock |

## 12. Acceptance

Run from the checkout, as an ordinary user (the form of `AGENTS.md`):

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test config
cargo test --test prefix
cargo test --test daemon_cli
cargo test --test attach_cli
```

- All pass with no warnings (judge: exit status, read by the architect at final review). Known environment
  failures of the architect's confined session (a read-only `$HOME`) are not the developer's: report which
  tests fail in your session and whether they pass with `HOME` redirected under `target/scratch/3ej/`.
- `rg -n "default_value" src/cli.rs` finds no `prefix` or `viewer_after` default; `rg -n "cli\.prefix|cli\.viewer_after" src/main.rs`
  finds only the `ClientOverrides` construction.
- `git status --short` shows only section 0 scope files and the untracked
  `docs/specs/spec-3ef-per-session-owner-presence.md` (a parked draft; do not touch or commit it).
- Tests 1-16 exist and pass; the contract tests of section 9 pass unmodified.
- Mutation check by the developer (mutate in the checkout, restore by checksum), each mutant killed by a named
  repo test: file/override precedence swapped; `3650d` ceiling removed; `WholeSecondsOrZero` replaced by
  `Millis`; `format_prefix` `C-space` arm removed; `create_new` replaced by `File::create`; the template write
  removed from `run_daemon`; `client.toml` read added before the adapter early return; `deny_unknown_fields`
  removed; the `Source::File` label printed as `default` in `show_client`. Name the killing test for each.
- A scratch directory, if any, is under `target/scratch/3ej/` and deleted before the report.
