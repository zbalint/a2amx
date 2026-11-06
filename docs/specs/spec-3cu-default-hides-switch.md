# Spec 3cu: One name for the home-hides switch

## 0. Status

LOCKED.

Scope (files that may be edited): `src/cli.rs` (the `new` flag), `src/main.rs` (the destructuring
at line 503, the `--confine-*` requires check at lines 524-531 and the request at line 577),
`tests/confine_show.rs`, `docs/cli.md`, `docs/confinement.md`.
Does not touch: every other file under `src/` and `tests/`, `src/team.rs` and the team file keys,
`ConfineRequest` and the wire field `no_default_hides`, `CHANGELOG.md` and `docs/backlog.md` (the
architect updates both), `AGENTS.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-confine-config-cleanup-2026-11-06`. Public test seams: the `a2amx` binary (`new`) and
`a2amx::cli`.

Public interface: this removes a command-line flag (`--confine-no-default-hides`) and adds one
(`--confine-default-hides`). It is a breaking change of the CLI, allowed before 1.0 (owner decision
2026-11-02); the changelog entry says so.

## 1. Why

Backlog F64 step 4. The same setting has two names with opposite polarity: the team file key
`default_hides = false` and the flag `--confine-no-default-hides`. The team file moved to the
positive name when the file format was reworked (the old key `confine_no_default_hides` is rejected
with a message pointing to `default_hides = false`, `src/team.rs:595`), and the flag was left. The
owner decided (2026-11-06) on one positive name. The internal and wire field `no_default_hides` stays:
it is the daemon's protocol (`ConfineRequest`, `tests/wire.rs:1187`), and renaming it would change the
protocol for no user gain.

The owner also decided, in the same round, to keep `--confine`, `[confine] enabled` and the global `auto`
as three ways to turn confinement on (they differ in scope: one session, one team, every project), and
the raw `unhide` and `keep_env` stay as escape hatches. Nothing in this spec touches them.

Rejected: keeping both flags (the point is to remove the duplicate); a bare `--confine-default-hides`
switch with no value (the default is already true, so a flag that can only say "true" is useless;
`false` is the only value anyone passes); dropping the switch entirely (the owner chose to keep the
setting).

Trust boundary: none. A command-line flag of the user's own process, no new input from a less trusted
source.

## 2. `src/cli.rs`

In the `New` command (line 69-70) replace

```rust
#[arg(long)]
confine_no_default_hides: bool,
```

with

```rust
/// Set to false to show the optional home credential locations that are hidden by default.
#[arg(long = "confine-default-hides", value_name = "BOOL")]
confine_default_hides: Option<bool>,
```

(`--confine-default-hides=false` and `--confine-default-hides false` both parse; clap accepts `true`
and `false` for a `bool` value.)

## 3. `src/main.rs`

- Line 503: destructure `confine_default_hides` instead of `confine_no_default_hides`.
- Lines 524-531: add `|| confine_default_hides.is_some()` to `has_confine_options`, so the flag without
  `--confine` is refused with the existing message `--confine-* flags require --confine`. (Today
  the old flag is silently ignored without `--confine`; this is a deliberate tightening, listed in the
  changelog entry. `--confine-default-hides=true` without `--confine` is refused too: the user named a
  confine option.)
- Line 577: `no_default_hides: confine_default_hides == Some(false),`.

## 4. Tests (`tests/confine_show.rs`)

Written first. Both use the existing `run_binary` and the real-bwrap guard used by the neighbouring
tests (skip with a message when `bwrap` is missing).

1. Visible and hidden through the binary. The daemon takes the plan's home from its own process
   environment (`src/daemon.rs:378-380`), and a session's `/tmp` is a private tmpfs
   (`src/confine.rs:1899`), so the test starts the daemon with
   `common::spawn_daemon_process(state, user_home, runtime)` (`tests/common/mod.rs:56`; it sets `HOME` to the
   fixture home and uses `bwrap` from `PATH`), and creates the fixture home, the runtime directory and the
   session directory with `tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR"))` (precedent
   `tests/confine.rs:3776`). It skips with a message when `bwrap --version` fails (the pattern at
   `tests/auto_confine.rs:383`). Create `<home>/.ssh/marker` before the `new` calls (a missing hide source is
   skipped, `hide_or_skip`). Start two detached sessions in the session directory, each running
   `sh -c 'if test -e "$HOME/.ssh/marker"; then echo visible > out-NAME; else echo hidden > out-NAME; fi'`
   with `NAME` distinct (the session directory is writable in the session, as the neighbour tests rely on):
   one with plain `--confine`, one with `--confine --confine-default-hides=false`. After each, poll the output
   file with a bounded wait of 10 s; the first contains `hidden`, the second `visible`. Kill both sessions
   at the end, as `show_reports_off_mode_by_name_and_id` does. It fails if the mapping is inverted or
   dropped (one test for both mutants). `~/.ssh` is `DEFAULT_HIDES[0]` and `hide_defaults` skips it when
   `no_default_hides` is set (`src/confine.rs:74`, `:1439-1442`); the `ssh` grant is not involved.
2. Old flag rejected (needs no daemon and no bwrap): `new --confine --confine-no-default-hides -- sh -c true` exits with a failure and stderr
   contains `unexpected argument '--confine-no-default-hides'`, so it is never skipped. (It fails on the old
   code; the wording is clap 4's, `Cargo.lock` has 4.6.7.)
3. Requires `--confine`: `new --confine-default-hides=false -- sh -c true` fails with stderr containing
   `--confine-* flags require --confine` (`src/main.rs:530`, raised before the daemon connection at `:550`, so
   no daemon is needed). On the old code it fails only because the new flag does not exist; say so in the
   test's comment. The requires check is killed by this test alone (mutant 3 below).

## 5. Docs

- `docs/cli.md:115`: replace `--confine-no-default-hides` disables the optional home hides with
  `--confine-default-hides=false` shows the optional home hides (the home credential locations hidden by
  default), keeping the rest of the sentence (the mandatory hides stay).
- `docs/confinement.md:33`: the option block lists `--confine-default-hides=false`; line 37: the same
  replacement in the sentence; line 624: `(or --confine-default-hides=false on the command line)`.
- The content search for the old text is acceptance 2; outside `docs/specs/` and `CHANGELOG.md` the old
  hyphenated text exists today only at `docs/cli.md:115` and `docs/confinement.md:33, 37, 624`
  (grepped, `README.md`, `docs/teams.md`, `scripts/`, `a2amx.toml.example` and `tests/docs_examples.rs` do
  not contain it). No test asserts `new --help` text.

## 6. Out of scope

`no_default_hides` in `ConfineRequest`, the wire, `src/confine.rs` and the tests that set it; the team
file; the other `--confine-*` flags; making `--confine-write` and its siblings require `--confine`
checks differently; renaming anything else; the enable switches; a deprecation alias for the old flag
(none: a breaking change is allowed and an alias would keep the duplicate).

## 7. Acceptance

1. `cargo test --test confine_show` and then `cargo test`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`, all pass. The tester's checks and mine use `TMPDIR` unset.
2. `rg -n 'no-default-hides' --glob '!docs/specs/*' --glob '!CHANGELOG.md' --glob '!tests/*' .` matches
   nothing (`tests/` is excluded because test 2 holds the old flag as a string; the underscore forms
   `confine_no_default_hides` in `src/cli.rs` and `src/main.rs` are renamed by the compiler).
3. `a2amx new --help` lists `--confine-default-hides <BOOL>` and not `--confine-no-default-hides` (the
   tester reads it from the built binary).
4. `git status --short` lists exactly the five files of section 0's scope.
5. Mutants (developer, each restored by checksum): invert `== Some(false)` to `== Some(true)`; drop the
   mapping (`no_default_hides: false`); drop `|| confine_default_hides.is_some()`. Each must be killed by a
   named test of section 4. Mutants 1 and 2 run where `bwrap` works: a skipped test 1 is "not run", never
   "killed", and the developer reports whether it ran.

Known flaky tests: I7, I14, I18 in `tests/attach_cli.rs` and `tests/daemon_cli.rs`; a failure there on
the first run is rerun once, and recorded as pre-existing when it passes (spec 3cq prints diagnostics).

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5; developer and tester OMP
`openai-codex/gpt-5.6-luna` (slot names not exposed). BLOCKED/amend cycles: 0.

| Where found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant, Opus 5.5 | Test 1 could not pass as drafted: the daemon takes the plan's home from its own environment and a session's `/tmp` is a private tmpfs, so a plain tempdir home is never visible | Test uses `spawn_daemon_process` and fixtures under `CARGO_TARGET_TMPDIR` |
| Draft review | consultant, Opus 5.5 | Scope said five files, acceptance said six; two different greps; requires-block anchor off by one | Fixed in the spec |
| Tester pass | tester, gpt-5.6-luna | Two mutants survived: the requires guard tested only the `false` value, and `value_name` could change from `BOOL` | Requires test loops over both values; a help-text test was added |
| Architect review | architect, Sonnet 5.5 | The bwrap availability check used a blocking `std::process::Command` in an async test body (Sonar rust:S7487, flagged the same way in spec 3cs) | Moved into a sync helper `bwrap_available` |

Accepted survivors: none. One transient `tests/explain.rs` timeout on the developer's first full
run passed on rerun (known flaky class, not part of this diff).
