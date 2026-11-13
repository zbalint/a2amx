# Spec 3eh: config loader and `daemon.toml`

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, from `6ca176f`.
Shared task `context_id`: `a2amx-config-layout-2026-11-13`. This is spec 2 of 7 in the owner's configuration
redesign (root and layout, accepted as 3eg; config loader and `daemon.toml`; gateway config; client config;
team file rules; per-project auto grants; migration; presence per session).

Public test seams: the `a2amx::config` module API (`load_daemon`, `ensure_daemon_file`, `show_daemon`,
`parse_duration`, `format_duration`) in a new `tests/config.rs`; the `a2amx` binary through
`daemon start` and `config show` in `tests/daemon_cli.rs`.

**Scope** (files that may be edited or created):

- `src/config.rs` (new), `src/lib.rs` (one `pub mod config;` line), `src/layout.rs` (one accessor, section 2), `src/cli.rs`, `src/main.rs`.
- `tests/config.rs` (new), `tests/daemon_cli.rs`, `tests/layout.rs` (one table row).
- `docs/configuration.md` (new), `docs/cli.md`, `docs/remote-messaging.md`.

Does **not** touch: `src/messaging.rs` (`Limits` and its `Default` stay as they are), `src/daemon.rs`
(`DaemonConfig` keeps its shape, so the twelve construction sites in `src/main.rs` and `tests/` need no edit),
`src/relay_files.rs`, `src/team.rs`, `gateway.toml`, `client.toml`, `team.toml`, migration,
`AGENTS.md`, `CHANGELOG.md`, `README.md`, `docs/architecture.md`, `docs/backlog.md` (the architect updates the
`AGENTS.md` module table, the changelog and the architecture page when committing).

## 1. Why

The owner found operator policy hardcoded: the per-recipient pending cap (50), the per-agent rate (20/min), the
human inbound and outbound rates (20 and 30/min), the stored-message cap (10,000), retention (7 days), the lease
(24 h) and the pairing-code lifetime (10 min) are literals in `Limits::default` (`src/messaging.rs:160-173`);
only the owner-away grace has a flag, and its default 60 is written twice (`src/cli.rs:224` and
`Limits::default`). The owner called hardcoding operator policy a design crime and decided (2026-11-13,
grilled): defaults stay Rust constants; a file overrides them; precedence is flag, environment, file, default;
an invalid file makes the daemon refuse to start and print the problem; `a2amx config show` prints each value
and its source; the daemon writes the file with every key commented out and never rewrites an existing one;
durations are strings; a restart applies changes. Every value that makes sense for a user is configurable;
fixed are the Discord message limit, untrusted-input bounds, protocol constants and team-file sanity bounds.

This spec delivers the loader and `daemon.toml` only. `gateway.toml` and its secrets, `client.toml`, the team
file rules and the migration are later specs; `config show` grows a section per file as they land.

Decisions of this spec, with the alternatives rejected:

- **Key set.** `listen`, `host_name`, and the nine `Limits` fields (`away_secs` is `owner_away`): eleven keys. Fixed constants, by the
  owner's rule: `MAX_WRONG_CODES` (brute-force bound on the pairing code), `MAX_MESSAGE_BYTES`,
  `MAX_HUMAN_BODY_BYTES`, `MAX_SUBJECT_BYTES`, `HEARTBEAT_MAX` (untrusted-input bounds), and the timing
  heuristics in `src/daemon.rs:43-51` and `src/session.rs:30-35` (activity quiet window, needs-input delay,
  quota refresh and staleness, reset and graceful-exit timeouts): they are implementation tuning, not
  something an operator decides. Rejected: making the heuristics keys (more surface to document and test,
  no operator need named).
- **Old flags stay as overrides** (owner, 2026-11-13): `--listen`, `--host-name`, `--owner-away-secs` and
  `A2AMX_OWNER_AWAY_SECS`. The other keys are file-only. Rejected: a flag per key (large CLI surface).
- **`config show` is in this spec** (owner, 2026-11-13), daemon keys only, read-only, works without a daemon.
- **`DaemonConfig` is unchanged**: the loader produces its inputs at the binary edge (`src/main.rs`), so the
  library entry point `Daemon::start` stays free of file reads and the tests that call it directly keep working.
- **A new `src/config.rs`** owns file shape, defaults-vs-file-vs-override resolution, value parsing, the
  template text and `show`; later specs add their file's table to it. Rejected: putting the loader in
  `src/daemon.rs` (3,884 lines, owns serving, not configuration) or `src/main.rs` (4,255 lines, dispatch).
- **Own duration parser and formatter.** No existing parser (`grep "fn parse_duration" src` finds none).
  `messaging::display_duration` (`src/messaging.rs:89`) floors (`90s` prints `1m`), so it cannot show a
  configured value exactly; it stays for human-facing text. Rejected: a `humantime` dependency for four
  units.
- **Template written with `create_new`, not `relay_files::atomic_write`.** `atomic_write` (`src/relay_files.rs:247`)
  renames over the target, which is exactly the overwrite this feature forbids. `OpenOptions::create_new`
  (O_EXCL) fails if anything exists, a dangling symlink included, and never follows one.

## 2. `src/config.rs` (new): target shape

```rust
pub enum Source { Default, File, Override }

pub struct Overrides {                    // flag-or-environment values, already parsed by clap
    pub listen: Vec<SocketAddr>,          // empty = not given
    pub host_name: Option<String>,
    pub owner_away_secs: Option<u64>,
}

pub struct DaemonSettings {               // exactly the inputs of DaemonConfig that policy decides
    pub listen: Vec<SocketAddr>,
    pub host_name: Option<String>,
    pub limits: Limits,
}

pub struct LoadedDaemon { pub settings: DaemonSettings, pub sources: Vec<(&'static str, Source)> }

pub fn load_daemon(layout: &Layout, overrides: &Overrides) -> anyhow::Result<LoadedDaemon>;
pub fn ensure_daemon_file(layout: &Layout) -> anyhow::Result<()>;
pub fn show_daemon(layout: &Layout, env_owner_away_secs: Option<u64>) -> anyhow::Result<String>;
pub fn parse_duration(text: &str) -> Result<Duration, String>;
pub fn format_duration(duration: Duration) -> String;
```

One responsibility per function:

- `load_daemon` reads `layout.daemon_config()` (a new accessor in `src/layout.rs` next to `gateway_config`, `<root>/config/daemon.toml`; `layout` is the only module that names paths under the root, and `tests/layout.rs:53-54` is the table to extend), applies defaults, then the file, then the overrides. Only `NotFound` on the file itself means absent and loads defaults; any other
  failure to read it (`ENOTDIR`, `EACCES`, ...) is an error. A present file that is not a regular file, is over 64 KiB, is not UTF-8, does not parse, has an
  unknown key, or has an invalid value is an error. `sources` holds one entry per key in table order, named
  with the key's dotted path (`listen`, `host_name`, `owner_away`, `limits.rate_per_minute`, ...).
- `ensure_daemon_file` creates `daemon.toml` with `create_new(true)`, mode `0o600`, containing the template
  of section 2.2, only when the path does not exist; `AlreadyExists` is success and the file is left
  byte-for-byte alone. It creates no directory (the caller ran `Layout::ensure`). On a write error it removes
  the partial file it created and returns the error.
- `show_daemon` returns the text of section 4. It reads the file as `load_daemon` does and creates nothing.
- `parse_duration` / `format_duration`: section 2.3.

### 2.1 Keys, defaults, validation

| Key | Type | Default (source constant) | Valid |
| --- | --- | --- | --- |
| `listen` | array of socket-address strings | `["127.0.0.1:0"]` (the literal now at `src/main.rs:428`) | at least one entry; each parses as `SocketAddr` |
| `host_name` | string | unset (`Option`) | non-empty when set |
| `owner_away` | duration | `60s` (`Limits::default().away_secs`) | `0s` to `3650d` |
| `limits.max_pending_per_recipient` | integer | 50 | 1..=`u32::MAX` |
| `limits.rate_per_minute` | integer | 20 | 1..=`u32::MAX` |
| `limits.human_inbound_per_minute` | integer | 20 | 1..=`u32::MAX` |
| `limits.human_outbound_per_minute` | integer | 30 | 1..=`u32::MAX` |
| `limits.max_stored_messages` | integer | 10000 | 1..=`u32::MAX` |
| `limits.retention` | duration | `7d` | `1s` to `3650d` |
| `limits.lease` | duration | `24h` (`LEASE_MAX`) | `1s` to `3650d` |
| `limits.code_ttl` | duration | `10m` (`CODE_TTL`) | `1s` to `3650d` |

A zero rate or cap would refuse every message, so it is rejected, not honoured. Every duration has an upper bound because the values reach `Instant + Duration` (`src/daemon.rs:1496,1554`, `src/messaging.rs:224`), which panics on overflow; the bound applies to the final value, so `--owner-away-secs` and `A2AMX_OWNER_AWAY_SECS` above `3650d` (315,360,000 seconds) are rejected with the section 2.4 error. `3650d` is a shortcut: ten years, far past any sensible grace, lease or code lifetime; lower it if a use ever needs it (leave a `// shortcut:` comment in the code). Defaults come from
`Limits::default()` and the `listen` literal; the loader does not restate the numbers. The file layout is a
top-level table for `listen`, `host_name`, `owner_away` and a `[limits]` table for the rest.
`serde(deny_unknown_fields)` on both structs, as `src/relay_files.rs:30,54` and `src/team.rs:17` do.

Precedence per key: override (flag or environment, when given) over file over default. `--listen` given at
all replaces the file's whole `listen` list; it does not merge. Only `listen`, `host_name` and `owner_away`
have overrides; the `limits.*` keys do not.

### 2.2 The template

`ensure_daemon_file` writes, and `docs/configuration.md` shows, a file of this shape, generated from the same
key table as the validation so the two cannot drift: a header comment (what the file is, that a restart
applies changes, that the daemon never rewrites it), then for every key one comment line of description and one
line `# key = <default>` (inside a `# [limits]`-style grouping: the `[limits]` table header is itself commented
out, so the file as written parses to an empty table). Durations are written in the `format_duration` form (`"1m"` for the default 60 seconds, `"1d"` for 24 hours), the
address list as `["127.0.0.1:0"]`, `host_name` as `# host_name = "host-a"` (an example, since it has no
default). A key line is exactly `# <key> = <value>` (it matches `^# [a-z_]+ = `) and the table header is exactly `# [limits]`; no description or header line may match the key-line pattern, so a test can uncomment mechanically.

### 2.3 Durations

Grammar: decimal digits followed by exactly one of `s`, `m`, `h`, `d` (`"60s"`, `"10m"`, `"24h"`, `"7d"`); no
spaces, no sign, no fractions, no compound forms, no bare numbers. Overflow (`u64::checked_mul`) is an error.
`format_duration` prints the largest unit that divides the value exactly (`86400s` prints `1d`, `90s`
prints `90s`, `0s` prints `0s`); `parse_duration(format_duration(d)) == d` for every whole number of seconds.
The error text names the offending text and the accepted form.

### 2.4 Errors

Every error names the file path and the key, and for a TOML syntax error the line and column the `toml` crate
reports. Examples of the required content: `daemon.toml: key limits.rate_per_minute: 0 is not allowed; use 1 or more`,
`daemon.toml: unknown key limits.rate`. Error text carries no file contents beyond the offending key and
value. The daemon prints the error to standard error and exits non-zero without starting.

## 3. `src/cli.rs` and `src/main.rs`

1. `src/cli.rs:222-225`: `owner_away_secs` becomes `Option<u64>` with no `default_value_t`; the `env`
   attribute stays; its doc line says the default comes from `daemon.toml`, then 60 seconds. `--listen` and
   `--host-name` are unchanged.
2. `src/cli.rs`: a new top-level `Command::Config { action: ConfigAction }` with `ConfigAction::Show`
   (a subcommand enum, so later specs add `show` sections and possibly `path`, not new top-level commands).
3. `src/main.rs`, `run_daemon` (`:420`): build `Overrides` from the flags, call
   `layout.ensure()` (a blocking call, as `run_daemon_background` does at `:477-478` through `spawn_blocking`),
   then `config::ensure_daemon_file`, then `config::load_daemon`, and construct `DaemonConfig` from the
   returned settings, replacing the `127.0.0.1:0` literal (`:426-433`) and the `Limits { away_secs, ..Default }`
   expression (`:439-442`). File reads and the file create run on `spawn_blocking`.
4. `src/main.rs`, `run_daemon_background` (`:460`): inside the existing `spawn_blocking` closure (`:477`), after
   `Layout::ensure()` (`:478`) and before the `daemon.log` open, call `config::load_daemon` (no template write) so a
   bad file is reported on the user's terminal and spawns no child, and so the existing
   `background_reports_a_startup_failure` (`tests/daemon_cli.rs:203`, root is a regular file) still fails in `ensure`
   with the message it asserts, before any config read. The child command line passes `--owner-away-secs` only when the value is `Some`
   (today it always passes it, `:495`), and `--listen` and `--host-name` as before.
5. `src/main.rs`, `Command::Config` arm: `ConfigAction::Show` calls `config::show_daemon` with
   `A2AMX_OWNER_AWAY_SECS` read by `std::env::var` and parsed as `u64`, as the daemon flag's `env` does (the
   subcommand declares no daemon flags, so clap never sees the variable); a non-numeric or non-UTF-8 value is an
   error naming the variable, exit non-zero, and the value is then range-checked like the flag (section 2.1). Exit code 0; a bad file exits non-zero with the section 2.4
   error. `config show` never creates any file or directory.

## 4. `a2amx config show` output

One line per key in table order, `<dotted key> = <value>  (<source>)`, where `<source>` is `default`, `file` or
`env`; the unset `host_name` prints `host_name = (unset)  (default)`. Values print in the file's form: durations
via `format_duration`, addresses as TOML strings. A first line `daemon.toml: <path> (<present|absent>)` names the
file. `config show` has no flags for the daemon keys, so its only override source is `A2AMX_OWNER_AWAY_SECS`;
it reports what a daemon started without flags would use. It does not show a running daemon's values.

Example (file present with `limits.retention = "3d"`, no environment):

```
daemon.toml: /home/user/.a2amx/config/daemon.toml (present)
listen = ["127.0.0.1:0"]  (default)
host_name = (unset)  (default)
owner_away = "1m"  (default)
limits.max_pending_per_recipient = 50  (default)
limits.rate_per_minute = 20  (default)
limits.human_inbound_per_minute = 20  (default)
limits.human_outbound_per_minute = 30  (default)
limits.max_stored_messages = 10000  (default)
limits.retention = "3d"  (file)
limits.lease = "1d"  (default)
limits.code_ttl = "10m"  (default)
```

(`60s` prints as `1m` and `24h` as `1d` under the largest-exact-unit rule of section 2.3. In tests the first line holds the real temp-root path.)

## 5. Trust boundary

The file is operator-written and read by the daemon at start. Attack paths:

| Path | Disposition | Abuse-case test |
| --- | --- | --- |
| A confined agent edits `daemon.toml` to raise limits or open a listener | Removed by 3eg: `config/` is read-only in confined sessions, an unconfined agent can already write the file with the operator's rights (same trust as the flags) | none new (3eg's confine tests cover the read-only mount); stated in `docs/configuration.md` |
| Oversized, non-regular (FIFO, device, directory) or non-UTF-8 file stalls or exhausts the daemon | Constrained: regular file only, at most 64 KiB read, UTF-8 | `config.rs` tests: a 64 KiB + 1 file, a FIFO at the path (opened non-blocking, with the regular-file check on the opened file's metadata so the open cannot block), a directory, invalid UTF-8: each is an error |
| `daemon.toml` is a symlink or dangling symlink so the template write goes elsewhere | Removed: `create_new` never follows or replaces | test: dangling symlink at the path, `ensure_daemon_file` returns `Ok`, the link target is not created, the link is unchanged; `load_daemon` then reads `NotFound` and loads defaults (documented, not a defect) |
| Values that disable protection (zero rate, zero cap) or overflow arithmetic | Constrained: section 2.1 minimums, `checked_mul` | tests for each zero and for `99999999999999999999d` |
| `listen` exposes the daemon beyond loopback | Accepted: operator choice, same as `--listen` today; AGENTS.md forbids warnings on operator-chosen addresses | none |

## 6. Docs

- `docs/configuration.md` (new): the root and `config/` directory (link to the layout section of
  `docs/architecture.md`), `daemon.toml` with the full template and a table of keys, defaults and limits,
  precedence, durations, "restart applies changes", "the daemon writes the file once and never rewrites it",
  `config show`, and a sentence that confined sessions see `config/` read-only.
- `docs/cli.md`: the `daemon start` line (`:10`) and a `config show` line.
- `docs/remote-messaging.md:14-15`: the grace period is `owner_away` in `daemon.toml`, flag `--owner-away-secs`.

## 7. Tests

New `tests/config.rs` (through the public API, against a temp root created with `Layout::new` and
`Layout::ensure`; expected values are literals):

1. Defaults: no file, no overrides: `listen == ["127.0.0.1:0"]`, `host_name == None`, `limits` equal to
   `Limits::default()`, every source `Default`.
2. A full file overrides every key: each setting equals its literal, sources `File`.
3. An override beats the file for `owner_away_secs`, `host_name` and `listen` (the file's list is replaced
   as a whole); sources `Override`; keys without an override keep `File`.
4. Template round trip: `ensure_daemon_file` into an empty config dir; the written file loads as defaults;
   uncomment every line matching `^# [a-z_]+ = ` and the header `# [limits]` (strip the leading `# `) and it loads
   to the defaults except `host_name == Some("host-a")`, the template's example; the file mode is `0o600`.
5. `ensure_daemon_file` leaves an existing file byte-for-byte unchanged, including an empty file and a file
   with an invalid body.
6. Parse errors: unknown top-level key, unknown `limits` key, wrong type (`rate_per_minute = "x"`), zero for
   each of the five counters and for the three duration keys except `owner_away`, `owner_away = "0s"`
   accepted, bad duration forms (`"60"`, `"1.5h"`, `"1h30m"`, `"-1s"`, `""`, `"5w"`, `213503982334602d` (the first value whose seconds exceed `u64::MAX`, reaching `checked_mul`), `3651d` for each of the four duration keys, and an `owner_away_secs` override of `315360001`), empty
   `listen`, bad address, empty `host_name`. Each asserts the message contains the file name and the key.
7. Section 5 abuse cases: oversized, FIFO, directory, invalid UTF-8; dangling symlink (`ensure_daemon_file` is `Ok`, and `load_daemon` then reads `NotFound` and loads defaults).
8. `parse_duration` and `format_duration` table cases and the round trip over a set of literals.
9. `show_daemon`: the text of section 4's example (the file holds `[limits]` then `retention = "3d"`; first line with the temp path), exactly; with `env_owner_away_secs = Some(5)` the line is
   `owner_away = "5s"  (env)`; with no file the first line says `absent` and no file or directory is created.

`tests/daemon_cli.rs` (the `a2amx` binary, `--home` in a temp directory):

10. `daemon start --foreground` with an invalid `daemon.toml` exits non-zero, prints the key to standard
    error, and starts no daemon (no `run/addr`).
11. `daemon start` (background) with an invalid file fails on the terminal with the same message, spawns no
    child (no `run/addr`, no `log/daemon.log` content from a child).
12. `daemon start --foreground` in a fresh root creates `config/daemon.toml` containing `# owner_away`.
13. A daemon started with `limits.retention` etc. in the file runs (smoke: start, `daemon status`, stop).
14. `config show` (subprocess with `A2AMX_OWNER_AWAY_SECS` removed from its environment) prints the section 4 first line and 11 key lines on a fresh root and creates no
    `config/daemon.toml`.

15. `docs/configuration.md` contains the generated template verbatim (`include_str!("../docs/configuration.md")` and the template text from `config`), so the docs copy cannot drift.

Contract tests (pass on the old code once written against the unchanged behaviour): the existing
`Daemon::start` tests at the twelve `DaemonConfig` sites keep passing unmodified; no new test targets them.

## 8. Out of scope

- `gateway.toml`, `client.toml`, `team.toml` and their keys (`viewer_after` and `prefix` stay global flags in
  this spec); migration; per-project auto grants; per-session presence (the grace default stays 60).
- Hot reload; a `config path`/`config edit` subcommand; showing a running daemon's values.
- A flag per key; configurability of the fixed constants of section 1; changing `Limits`.
- Refactors of `src/main.rs` or `src/daemon.rs` beyond the call sites named.
- `CHANGELOG.md`, `AGENTS.md` module row for `config`, `docs/architecture.md` module table: architect, at commit.

## 9. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `Limits::default` holds the nine values, `away_secs` 60 | `src/messaging.rs:160-173` | read |
| `--owner-away-secs` has `default_value_t = 60` and env `A2AMX_OWNER_AWAY_SECS` | `src/cli.rs:224-225` | read |
| `run_daemon` builds `DaemonConfig` with the `127.0.0.1:0` literal and `Limits { away_secs, ..Default }` | `src/main.rs:420-445` | read |
| `run_daemon_background` always passes `--owner-away-secs` and checks "already running" first | `src/main.rs:460-500` | read |
| `Limits` fields are read in `src/daemon.rs:1348,1353,1398,1496,1502,1554,1715,1746,2426`, passed whole at `:2331,:2414`, and `src/store.rs:499,500,892` | grep `limits\.` | grepped |
| `DaemonConfig` is constructed at (twelve sites) `src/main.rs:435`, `tests/broker.rs:2722`, `tests/daemon.rs:226,1453,1986,2008,2291`, `tests/attach_cli.rs:915`, `tests/human_inbound.rs:1440`, `tests/common/mod.rs:57,352`, `tests/confine.rs:131` | grep `DaemonConfig {` | grepped |
| No duration parser exists | `grep "fn parse_duration\|fn parse_.*dur" src` found none | grepped |
| `deny_unknown_fields` is the house style for config structs | `src/relay_files.rs:30,54`, `src/team.rs:17,26,69,90` | grepped |
| `Layout` has `config_dir()`, accessors such as `gateway_config` (`tests/layout.rs:53-54` tables them) and `ensure()` creates `config/` at 0700; `ensure` is idempotent | `src/layout.rs:32,118-150` | read |
| `atomic_write` renames over the target | `src/relay_files.rs:247-271` | read |
| Confined sessions get the root read-only except the hides | spec 3eg §1 and §5 (`docs/specs/spec-3eg-root-and-layout.md:47-54,184`) | read (accepted spec; the 3eg tests pin it) |
| Docs that name the flag: `docs/cli.md:10`, `docs/remote-messaging.md:14-15`, `CHANGELOG.md:35`; no test names `--owner-away-secs` | grep `owner-away` | grepped |
| `Command` has no `Config` variant | `src/cli.rs` | grepped (`ConfigAction\|Config {` empty) |

## 10. Acceptance

Run from the checkout, as an ordinary user:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo test --test config
cargo test --test daemon_cli
```

- All pass with no warnings (judge: the commands' exit status, read by the architect at final review).
- `rg -n "default_value_t = 60" src/cli.rs` finds no `owner_away_secs` default; `rg -n '"127.0.0.1:0"' src/main.rs`
  finds no match (the literal lives in `src/config.rs` only).
- `git status --short` shows only the section 0 scope files and the untracked `docs/specs/spec-3ef-per-session-owner-presence.md` (a parked draft; do not touch or commit it).
- Tests 1-15 exist and pass, and the `tests/layout.rs` table row for `daemon_config()` (`/r/config/daemon.toml`).
  Mutation check by the developer (mutate in the checkout, restore by checksum), each mutant killed by a named
  repo test: `rate_per_minute`
  minimum removed, the `3650d` upper bound removed, precedence of override and file swapped, `create_new` replaced by `File::create`, the 64 KiB
  cap removed, `deny_unknown_fields` removed; name the killing test for each.
- A scratch directory, if any, is under `target/scratch/3eh/` and deleted before the report.
- Contract tests of section 7 pass without edits to the twelve `DaemonConfig` sites.
