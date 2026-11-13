# Spec 3ei: `gateway.toml` on the config loader

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from `ad54e22`.
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. This is spec 3 of 7 in the owner's configuration
redesign (root and layout, accepted as 3eg; config loader and `daemon.toml`, accepted as 3eh; gateway config;
client config; team file rules; per-project auto grants; migration; presence per session). Spec 3eg already put the
files in place (`config/gateway.toml`, `config/secrets/discord.token`); this spec moves the gateway's settings onto
the loader of spec 3eh. It breaks the public interface (the keys of `gateway.toml`); nothing is released before
the migration spec.

Public test seams: the `a2amx::config` module API (`load_gateway`, `ensure_gateway_file`, `show_gateway`,
`gateway_template`, `parse_duration`, `format_duration`) in `tests/config.rs`; `a2amx::relay_files::load` and
`Loaded` in `tests/relay.rs`; the `a2amx` binary through `gateway token`, `gateway up` and `config show` in
`tests/gateway_up.rs` and `tests/daemon_cli.rs`.

**Scope** (files that may be edited or created):

- `src/config.rs`, `src/relay_files.rs`, `src/relay.rs`, `src/main.rs`, `src/gateway_launch.rs`.
- `tests/config.rs`, `tests/relay.rs`, `tests/gateway_up.rs`, `tests/daemon_cli.rs`.
- `docs/configuration.md`, `docs/remote-messaging.md`, `docs/cli.md`.

Does **not** touch: `src/layout.rs` (the accessors exist), `src/discord.rs` (`DiscordConfig` keeps its shape),
`src/gateway.rs`, `src/gateway_proc.rs`, `src/cli.rs`, `src/daemon.rs`, `src/messaging.rs`, `src/team.rs`,
`client.toml`, `team.toml`, migration, `AGENTS.md`, `CHANGELOG.md`, `README.md`, `docs/architecture.md`,
`docs/backlog.md`, and every earlier spec under `docs/specs/` (the architect updates the changelog, the `AGENTS.md`
module table and the architecture page when committing).

## 1. Why

Spec 3eh made operator policy a file with defaults in the code, a validated loader, a commented template and
`config show`. The gateway still has its own, older settings reader: `relay_files::GatewayConfig`
(`src/relay_files.rs:29-51`) with `serde` default functions, integer-millisecond intervals (`sync_ms`, `idle_ms`,
`member_check_ms`, `reconnect_ms`), a hand-written validator (`:167-190`), an error text that still says
`config.toml` (`:169,176,186`), no template and no place in `config show`. A new operator creates
`gateway.toml` by hand with `install -m 600 /dev/null` (`docs/remote-messaging.md:85-93`).

Owner decisions (grilled 2026-11-13, recorded; the first four were asked for this spec, the rest are earlier
rules of the redesign):

- **`api_base` and `gateway_url` stay accepted as file keys but hidden**: not in the template, not in
  `config show`, documented as test-only. They exist only so tests can point the gateway at a fake Discord.
  Rejected: shown as ordinary keys (invites pointing the token at a proxy); fixed constants with an environment
  hook (a new mechanism for a test seam).
- **Intervals become duration strings, renamed**: `sync`, `idle`, `member_check`, `reconnect`. Rejected: keeping
  `_ms` integers (breaks "durations are strings").
- **The duration format gains an `ms` unit.** About ten tests in `tests/relay.rs` run the relay with 10 to 50
  millisecond intervals (one sets `idle_ms = 0` and expects a refusal, one needs a 50 ms against a 400 ms
  reconnect), which a one-second minimum would slow about a hundredfold. Rejected: hidden integer test keys;
  rewriting the tests for a one-second minimum.
- **The gateway commands write the template**: `gateway token` and `gateway up` create `config/gateway.toml`
  once, when it is absent, with every key commented out; they never rewrite it. The daemon does not touch it.
  `gateway run` never writes (it runs confined, with `config/` read-only).
- Defaults are Rust constants; precedence is file, then default (the gateway has no flags or environment
  overrides for these keys); an invalid file refuses to start and names the file and the key; `config show`
  prints each value and its source; every value that makes sense for a user is configurable.

Decisions of this spec that the owner did not grill (the architect's, sent to the owner with the review of this
spec):

- **`gateway.toml` keeps the strict read** of `relay_files::read_secure` (owner-only mode, regular file, no
  symlink, 64 KiB) although the redesign says `config/` holds no secrets. Loosening a check that tests pin
  (`load_rejects_loose_modes_for_each_secret_file`, `load_rejects_symlinked_secret_file`) is not part of a
  settings migration. The template is created `0600`, so it passes. Rejected: the lenient `read_config` of
  `daemon.toml`.
- **A missing `gateway.toml` loads as defaults**, and `relay_files::load` then fails on the missing `guild_id`
  with the file and key named. `config show` works without the file.
- **`max_members` must be at least 1** (like the counters of `daemon.toml`); today `0` pauses the relay for good.
- **The old `_ms` keys are not accepted** (unknown field error); the migration spec converts files.
- **The gateway table lives in `src/config.rs`**, beside the daemon table, as spec 3eh decided ("later specs add
  their file's table to it"). Placement check: `config.rs` owns configuration files and their rules, and
  grows from 636 to about 900 lines; `relay_files.rs` keeps what is specific to the gateway's secrets and state
  (secure reads, token, pairing, `Loaded`). Rejected: a `config_gateway` module (the key table, `resolve`,
  duration rules and template writer are shared and would need a third module); generalising `Key` over its
  settings type (a refactor of the daemon table, out of scope).

## 2. `src/config.rs`

Everything here is added or extended; the daemon table, `load_daemon`, `ensure_daemon_file`, `show_daemon` and
`daemon_template` keep their behaviour and signatures.

1. **`ms` unit.** `parse_duration` accepts `ms`, `s`, `m`, `h`, `d`. The unit is matched longest first (`ms`
   before `s`: `10ms` must not parse as `10m` plus `s`). The result is a `Duration` built from milliseconds with
   `checked_mul` (the overflow error stays: `99999999999999999999d` fails). Bare numbers, signs, fractions and
   compounds stay errors. `format_duration` prints a value that has a sub-second part as whole milliseconds
   (`1500ms`, `10ms`) and is otherwise unchanged (`0s`, `90s`, `1m`, `1h`, `1d`); `parse_duration` of its output
   returns the same value.
   The overflow check now runs on milliseconds, so a value between about `2.1e11d` and `2.1e14d` that used to
   fail the `3650d` range check fails the `overflows u64` parse check instead; both name the file and key, and
   no test asserts on the range text for those values. The parse error text becomes `use digits followed by ms,
   s, m, h or d` (no test pins the old text).
2. **A duration rule.** Replace `validate_duration_secs(path, key, seconds: u64, allow_zero)` and
   `parse_limited_duration(path, key, text, allow_zero) -> u64` (`:268-296`) with
   `validate_duration(path, key, Duration, DurationRule) -> anyhow::Result<Duration>` and
   `parse_limited_duration(path, key, text, DurationRule) -> anyhow::Result<Duration>`, with a small
   `enum DurationRule { WholeSeconds, WholeSecondsOrZero, Millis }`. Daemon callers take `.as_secs()` of the
   result; the `owner_away_secs` override (`:436-439`) passes `Duration::from_secs(value)`.
   `WholeSeconds` and `WholeSecondsOrZero` are today's daemon behaviour (minimum `1s` or `0s`) and now also
   reject a value with a sub-second part, with `duration must be a whole number of seconds` (without this,
   `owner_away = "500ms"` would truncate to `0s` and be accepted; `"0ms"` is `0s` and is accepted for
   `owner_away`, rejected for the other three). `Millis` takes any value from `1ms`. All rules
   keep the `3650d` ceiling. Out-of-range messages name the range (`between 1ms and 3650d`).
3. **Gateway settings.** `pub struct GatewaySettings { pub control_category: String, pub control_channel: String,
   pub max_members: u64, pub sync: Duration, pub idle: Duration, pub member_check: Duration,
   pub reconnect: Duration, pub api_base: String, pub gateway_url: String }` and
   `pub struct LoadedGateway { pub guild_id: Option<String>, pub settings: GatewaySettings,
   pub sources: Vec<(&'static str, Source)> }` (every field `pub`: the integration tests read them). Defaults, as constants moved out of
   `src/relay_files.rs:10-19`: `control_category` `a2amx`, `control_channel` `control`, `max_members` 2,
   `sync` 15 seconds, `idle` 1 second, `member_check` 60 seconds, `reconnect` 5 seconds, `api_base`
   `https://discord.com/api/v10`, `gateway_url` `wss://gateway.discord.gg`. `sources` has one entry per public key
   in table order (section 2.5): `Default` or `File` (never `Override`).
4. **Gateway file.** A private `GatewayFile` (`#[serde(deny_unknown_fields)]`): `guild_id`, `control_category`,
   `control_channel`, `max_members`, `sync`, `idle`, `member_check`, `reconnect`, `api_base`, `gateway_url`, all
   `Option`. `pub fn load_gateway(layout: &Layout) -> anyhow::Result<LoadedGateway>` reads
   `layout.gateway_config()` through `relay_files::read_secure` (made `pub(crate)`), after the same
   `symlink_metadata` not-found probe that `relay_files::load` uses for the pairing file (`:126-139`): an absent
   file is an empty file. It parses with `toml`, then validates each present key, in this order, with
   the message form `<path>: key <name>: <problem>` (`file_error`):
   `guild_id` is 1 to 20 digits (`relay_files::valid_id`, already `pub(crate)`);
   `control_category` and `control_channel` match `relay_files::valid_name` (made `pub(crate)`); `max_members` is at
   least 1 (`parse_counter` takes `u64` or is made generic over the two integer types, developer's choice);
   the four intervals use `DurationRule::Millis`. `api_base` and `gateway_url` are not validated (as today).
   `load_gateway` does not require `guild_id`.
5. **Key table and show.** A `GATEWAY_KEYS: &[GatewayKey]` table, with a new private
   `struct GatewayKey { name: &'static str, description: &'static str, example: &'static str,
   display: fn(&LoadedGateway) -> String }` (the daemon's `Key` is not reused or generalised: its `default` and
   `display` are typed to the daemon). `example` is the TOML text the template shows after `=` (the default,
   or for `guild_id` the example). Eight public keys in this order: `guild_id`,
   `control_category`, `control_channel`, `max_members`, `sync`, `idle`, `member_check`, `reconnect`.
   `api_base` and `gateway_url` are not in it. `pub fn show_gateway(layout) -> anyhow::Result<String>` prints
   section 5's text and reads, never creates.
6. **Template.** `pub fn gateway_template() -> String`: a header comment (`# A2AMX gateway settings. Restart the
   gateway to apply changes. a2amx writes this file once and never rewrites it.`), then for each key, in the
   daemon template's form (`src/config.rs:586-588`), a description line `# <name>: <description>` and a commented
   assignment `# <name> = <example>` (the assignment lines are the ones matching `^# [a-z_]+ = `; `guild_id` has no default: its example is
   `# guild_id = "123456789012345678"`, and its description says it is required). Uncommenting every assignment
   loads to the defaults except `guild_id == Some("123456789012345678")`.
7. **Template writer.** `pub fn ensure_gateway_file(layout) -> anyhow::Result<()>`. Extend the body of
   `ensure_daemon_file` (`create_new`, mode `0600`, remove the file if the write fails) into a private
   `ensure_file(path, text)` that both functions call; no second copy.
8. `read_config` stays the daemon file's reader and is not changed; the gateway file does not use it.

## 3. `src/relay_files.rs`

1. Delete `GatewayConfig`, its `default_*` functions, the `DEFAULT_*` constants it used, `validate_config` and the
   `config.toml` message texts. Keep `valid_id`, `valid_name` (now `pub(crate)`), `read_secure` (now
   `pub(crate)`), `atomic_write`, the token and pairing functions, `Pairing` and `MAX_FILE_BYTES`.
2. `Loaded` becomes `{ pub layout, pub token, pub guild_id: String, pub config: crate::config::GatewaySettings,
   pub pairing }`. `load` keeps its order (state directory check, token, then settings, then pairing): after the
   token it calls `config::load_gateway(&layout)`, takes `guild_id` or fails with `<path>: key guild_id: not set;
   add guild_id = "<your Discord server id>"` (formatted with `config::file_error`, made `pub(crate)`), and
   passes `guild_id` to `validate_pairing` as today.
3. `load_does_not_create_missing_home` (`tests/relay.rs:63`) keeps passing: the state directory check still comes
   first and nothing is created.

## 4. Consumers

- `src/relay.rs`: the `Relay.config` field type (`:170`) is `config::GatewaySettings`; the three intervals
  (`:72-74`) and the two reconnect sites (`:181,196`) use the `Duration` fields directly (`Duration::from_millis`
  goes); `loaded.config.guild_id` (`:378`) becomes `loaded.guild_id`. `max_members`, `control_category` and
  `control_channel` stay `loaded.config.<name>`.
- `src/main.rs`: `run_gateway` (`:392`) reads `loaded.guild_id`; the two `discord.*` lines (`:393-394`) are
  unchanged. `run_gateway_token` (`:305`) calls `config::ensure_gateway_file` after `relay_files::write_token` succeeds, so a rejected empty token
  leaves no file; the new call runs inside `spawn_blocking`. The existing inline `Layout::ensure` (`:306`) and
  `write_token` (`:308`) calls are not changed. The `Command::Config` arm (`:209-226`) builds the output as the daemon section, one blank
  line, then `config::show_gateway`, in one `spawn_blocking`.
- `src/gateway_launch.rs`, `up` (`:95-97`): after `Layout::ensure()` and before `relay_files::load`, call
  `config::ensure_gateway_file`.
- `gateway run` is unchanged and writes nothing.

## 5. `a2amx config show` output

The daemon section is unchanged (spec 3eh section 4). After it, one blank line and the gateway section: a first
line `gateway.toml: <path> (<present|absent>)`, then one line per public key in table order,
`<key> = <value>  (<default|file>)`; an unset `guild_id` prints `guild_id = (unset)  (default)`; strings print as
TOML strings; durations through `format_duration`. A bad `gateway.toml`, including one with broad permissions or
a symlink, makes `config show` fail with the load error, as it does for `daemon.toml`.

Example, no `gateway.toml`:

```
gateway.toml: /home/user/.a2amx/config/gateway.toml (absent)
guild_id = (unset)  (default)
control_category = "a2amx"  (default)
control_channel = "control"  (default)
max_members = 2  (default)
sync = "15s"  (default)
idle = "1s"  (default)
member_check = "1m"  (default)
reconnect = "5s"  (default)
```

## 6. Trust boundary

`gateway.toml` is operator-written and read by the gateway at start; the gateway holds the Discord bot token.
Attack paths:

| Path | Disposition | Abuse-case test |
| --- | --- | --- |
| A confined agent edits `gateway.toml` (change `guild_id`, `api_base`) | Removed by 3eg: `config/` is read-only in confined sessions; an unconfined agent already has the operator's rights | none new (3eg's confine tests); stated in `docs/remote-messaging.md` as today |
| Oversized, loose-mode, wrong-owner or symlinked `gateway.toml` | Constrained, unchanged: `read_secure` | contract tests that already pass on the old code, all in `tests/relay.rs`: `load_rejects_loose_modes_for_each_secret_file` (reaches `gateway.toml` through its `config.toml` loop name), `load_rejects_symlinked_config_and_pairing` (symlinks `gateway.toml`), and `load_rejects_bad_values_and_oversized_config` (its `idle_ms = 0` case changes only to `idle = "0ms"`, which must still fail) |
| A dangling symlink at `gateway.toml` makes the template write go elsewhere | Removed: `create_new` never follows or replaces | new test: `ensure_gateway_file` returns `Ok`, the link target is not created, the link is unchanged |
| Values that disable a protection or overflow arithmetic: `0ms`, `max_members = 0`, a `3651d` interval, an overflowing duration | Constrained: section 2 rules | new tests for each |
| `gateway token` or `gateway up` overwrite an operator's file | Removed: `ensure_file` is `create_new` | new test: existing file, even an empty or invalid one, stays byte for byte |
| `api_base` or `gateway_url` redirect the bot token to another host | Accepted: operator's file (see row one); the keys are hidden so they are not suggested | none |

## 7. Docs

- `docs/configuration.md`: a `gateway.toml` section with the full template (verbatim, test 12), the key table
  (key, default, rule), the `ms` unit in the duration paragraph (daemon keys stay whole seconds), the
  `config show` gateway section, "written once by `gateway token` or `gateway up`", and the strict-read note
  (the file must be owner-only; a created template is). `api_base` and `gateway_url` are named once as test-only.
- `docs/remote-messaging.md`: replace the hand-creation instructions (`:85-93`) with "`gateway token` creates the
  file; set `guild_id`"; replace the key table (`:96-107`) and every `*_ms` mention (`:162,166,314` and the
  paragraph after the table) with the new names and duration strings.
- `docs/cli.md`: `gateway token` also writes the commented `gateway.toml` template; `config show` prints the
  gateway section.

## 8. Tests

`tests/config.rs` (public API, temp root from `Layout::new` and `Layout::ensure`; expected values are literals):

1. `load_gateway` defaults: no file: `guild_id == None`, every setting equals its literal default, every source
   `Default`.
2. A full file sets every key (`guild_id = "123"`, names, `max_members = 5`, `sync = "10ms"`, `idle = "2s"`,
   `member_check = "3m"`, `reconnect = "4h"`, `api_base`, `gateway_url`): each equals its literal; the eight
   public sources are `File`.
3. Template round trip: `ensure_gateway_file`, loads as defaults; uncomment every assignment, loads to defaults
   except `guild_id`; the file mode is `0o600`. In `tests/relay.rs` (the `valid_gateway` helper pattern), a
   template written by `ensure_gateway_file` with `guild_id` uncommented passes `relay_files::load`.
4. `ensure_gateway_file` leaves an existing file byte for byte unchanged (empty, invalid body); a dangling symlink:
   `Ok`, target not created, link unchanged.
5. Parse errors, each message contains the file name and the key: unknown key (`sync_ms = 10`, the old name),
   wrong type (`sync = 10`), `guild_id = "abc"`, `guild_id = ""`, a 21-digit `guild_id`, `control_channel =
   "a_BAD"`, `control_category = ""`, `max_members = 0`, `idle = "0ms"`, `sync = "0s"`, bad forms (`"10"`,
   `"1.5s"`, `"1m30s"`, `"5w"`, `"ms"`, `"10 ms"`), `3651d` for each of the four intervals, `213503982334602d`.
6. Daemon rules after the `DurationRule` change: `owner_away = "500ms"`, `limits.lease = "1500ms"` each fail with
   `whole number of seconds` naming the key; `owner_away = "0s"` still loads; all earlier `tests/config.rs`
   cases pass unchanged.
7. `parse_duration` and `format_duration` table: `10ms`, `1500ms`, `1000ms` (prints `1s`), `250ms`, `0ms` (prints
   `0s`), `1ms`, `60s`, `90s` (prints `90s`), and the round trip over a set of literals including sub-second ones.
8. `show_gateway`: section 5's example exactly (first line with the temp path, `absent`); with
   `guild_id = "123"` and `sync = "10ms"` in the file the lines are `guild_id = "123"  (file)` and
   `sync = "10ms"  (file)`; no file or directory is created.
9. `relay_files::load` and the missing `guild_id`: with token and no `gateway.toml`, the error contains
   `gateway.toml`, `guild_id` and `not set` (also the case of a `gateway.toml` that holds only comments).
10. A gateway file with broad permissions makes `show_gateway` fail naming the file.
11. `gateway_template()` contains no `api_base` and no `gateway_url`.
12. `docs/configuration.md` contains `gateway_template()` verbatim (`include_str!`), like the daemon template.

`tests/relay.rs`: every `*_ms` key in a written `gateway.toml` becomes the renamed duration string (lines `560`
`idle = "0ms"`, `900`, `973`, `1149`, `1239` (`sync = "1m"`), `1384`, `1459`, `1551`, `1957`: the same values in
milliseconds, so the timing of every test is unchanged); `:485` reads `loaded.guild_id`; no other change, and no test is weakened or removed.

`tests/gateway_up.rs`: (13) `gateway token` on a fresh root creates `config/gateway.toml` mode `0600` containing
`# guild_id`; `gateway up` with no `gateway.toml` creates it and then fails with an error containing
`guild_id` and `not set`; the existing refusal tests (`:421,461`) keep their assertions. The case at `:424-427`
(it deletes `gateway.toml` and expects an error naming it) now passes through the new missing-`guild_id` error;
its assertion stays and test 13 covers that path by name.

`tests/daemon_cli.rs`: (14) the `config show` subprocess test prints the daemon section (first line and the 11 key
lines), a blank line and the gateway section (first line and 8 key lines) on a fresh root, and creates neither
file.

Contract tests (pass on the old code once written against the unchanged behaviour): the `relay_files::load`
security tests named in section 6; every relay test other than the renamed keys. Test 6 also pins
`owner_away = "0ms"` as accepted (it is `0s`) and `limits.lease = "0ms"` as rejected.

## 9. Out of scope

- `client.toml`, `team.toml`, team-file rules, per-project grants, migration (including conversion of an old
  `gateway.toml`), per-session presence.
- Flags or environment overrides for gateway keys; hot reload; making the Discord message limit, the untrusted
  input bounds, or `DiscordConfig`'s other fields configurable; generalising `Key`.
- A check that `api_base` or `gateway_url` is a URL.
- Moving `valid_id`, `valid_name` or the token and pairing code; `src/discord.rs`, `src/gateway.rs`.
- `CHANGELOG.md`, `AGENTS.md`, `docs/architecture.md`, `docs/backlog.md`: architect, at commit.

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `GatewayConfig` has serde defaults, `_ms` intervals, `deny_unknown_fields`, no `guild_id` default | `src/relay_files.rs:29-94` | read |
| `validate_config` says `config.toml` and rejects `0` intervals; names must match `valid_name`, `guild_id` `valid_id` | `src/relay_files.rs:167-215` | read |
| `load` reads token, then config through `read_secure`, then pairing behind a `symlink_metadata` not-found probe | `src/relay_files.rs:96-148` | read |
| `read_secure` refuses symlinks, non-regular, other owner, broad mode, over 64 KiB | `src/relay_files.rs:217-245` | read |
| `atomic_write` renames over the target, so it cannot write the template | `src/relay_files.rs:247-271` | read |
| `config.rs` has `resolve`, `file_error`, `parse_counter`, `validate_duration_secs`, `parse_limited_duration`, `ensure_daemon_file` (`create_new`, `0600`), `show_daemon`, `parse_duration` (units `s m h d`, `checked_mul`), `format_duration` | `src/config.rs:211-636` | read |
| `parse_duration`'s unit search is `["s","m","h","d"]` with `strip_suffix`, so `10ms` is today an error (`10m` then fails the digit check) | `src/config.rs:596-600` | read |
| Consumers of the settings: intervals `src/relay.rs:72-74,181,196`; `max_members` `:145,149,321,325`; names `:153,157`; `guild_id` `:378`; `src/main.rs:392-394`; `tests/relay.rs:485` | grep `loaded\.config\|config\.(sync\|idle\|member\|reconnect\|max_members\|control)` | grepped |
| No code outside `relay_files.rs` names `GatewayConfig` | grep `GatewayConfig` over `src tests` | grepped (only `src/relay.rs:170`) |
| Tests that write `*_ms` keys: `tests/relay.rs:560,900,973,1149,1239,1384,1459,1551,1957`; none elsewhere in `src tests` | grep `sync_ms\|idle_ms\|member_check_ms\|reconnect_ms` | grepped |
| Tests that write `gateway.toml` with only `guild_id` and `api_base`: `tests/gateway_up.rs:163`, `tests/gateway_background.rs:260,818` | grep | read |
| `gateway token` writes the token only; `gateway up` runs `Layout::ensure` then `relay_files::load` | `src/main.rs:305-309`, `src/gateway_launch.rs:95-97` | read |
| `config show` prints the daemon section only | `src/main.rs:209-226` | read |
| Docs naming the old keys: `docs/remote-messaging.md:85-107,162,166,314`; none in `docs/cli.md` or `docs/configuration.md` | grep `_ms` over `docs` excluding `docs/specs` | grepped |
| Baseline at `ad54e22`: `cargo test --test config` 12 passed, `--test daemon_cli` 71 passed, `--test relay` 35 passed, `--test gateway_up` 13 passed and 1 failed: `sandbox_hides_state_host_and_limits_writes_to_gateway` creates a temp directory in `$HOME`, which is read-only in the architect's confined session (environment, not code); the developer's run reports whether it passes in its session, else records it as pre-existing | run by the architect at lock |

## 11. Acceptance

Run from the checkout, as an ordinary user (the form of `AGENTS.md`):

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test config
cargo test --test relay
cargo test --test gateway_up
cargo test --test daemon_cli
```

- All pass with no warnings (judge: exit status, read by the architect at final review).
- `rg -n "(sync|idle|member_check|reconnect)_ms" src tests docs --glob '!docs/specs/*'` finds nothing, and
  `rg -n "config\.toml" src/relay_files.rs` finds nothing.
- `rg -n "GatewayConfig" src tests` finds nothing.
- `git status --short` shows only section 0 scope files and the untracked
  `docs/specs/spec-3ef-per-session-owner-presence.md` (a parked draft; do not touch or commit it).
- Tests 1-14 exist and pass; the loose-modes and symlinked-config contract tests of section 6 pass unmodified,
  and `load_rejects_bad_values_and_oversized_config` passes with only its `idle_ms = 0` key renamed.
- Mutation check by the developer (mutate in the checkout, restore by checksum), each mutant killed by a named
  repo test: `ms` matched after `s` (`10ms` fails or parses as minutes), the `WholeSeconds` sub-second rejection
  removed, the `3650d` bound removed, `max_members` minimum removed, `create_new` replaced by `File::create`
  in `ensure_file`, the template written by `gateway up` removed, the missing-`guild_id` error removed,
  `deny_unknown_fields` removed on `GatewayFile`; name the killing test for each.
- A scratch directory, if any, is under `target/scratch/3ei/` and deleted before the report.
