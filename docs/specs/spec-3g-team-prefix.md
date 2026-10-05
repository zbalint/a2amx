# Spec 3g: a name prefix for the team file, and a guard for a command given as one string

## 0. Status

**LOCKED** (2026-10-05, pre-lock gate run against develop at `5d65285`; gate notes in section 7).
The owner pointed out that session names are global to the daemon, so every team file has to
prefix each name by hand (`myproject-architect`, and again in `watch` and `control_from`). The
owner decided: a top-level `prefix` key, the tool adds the dash, a `prefix` applies only to names
that appear in the same file, `team down NAME` takes full names. The owner also hit a mistake in
`command`: writing the whole command as one string (`["claude --model Opus 'prompt'"]`) fails
at spawn and infers the wrong harness. Decisions D1 to D8 are in section 2; D6 deviates from the
owner's wording and is flagged there. Reviewed by the consultant (`m_404` and `m_407`, consult
context `a2amx-team-prefix-role`); its findings are D3, D4, D5, D6 and the naming note in section 3.

**Baseline:** develop at `5d65285`; `cargo test --test team` passes (17 tests). **Location and
branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `a2amx-team-prefix`. Public test seams: the pure functions in `src/team.rs`
(`parse`) and the `a2amx` binary against a real daemon in a temp state dir (`tests/common`,
`run_binary` in `tests/team.rs`).

**Scope.** May edit exactly these files and no others.

- `src/team.rs`, `src/main.rs` (`run_team_down` only)
- `tests/team.rs`
- `README.md`, `a2amx.toml.example`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/cli.rs`, `src/messaging.rs`, `src/harness.rs`, `src/daemon.rs`,
`src/wire.rs`, every other test file, `tests/common`, every other spec document (spec 2o stays as
it was locked), `AGENTS.md`, your own gitignored `a2amx.toml`. No new dependency. Do not commit,
stage or merge: leave the diff uncommitted in the working tree.

## 1. Why

Two sessions of two projects cannot both be called `architect`, and the daemon is shared by every
project on the machine, so the example file tells users to write `myproject-architect` and repeat
that in `watch` and `control_from`. The prefix moves that into one line. Separately, `command`
is an argv array, but a user who writes the whole command line as one string gets a program named
`claude --model Opus ...` that cannot spawn, and the harness is inferred as `generic`. Failing at
parse time with a pointer to the fix is cheaper than the spawn failure.

## 2. Decisions

- **D1.** The file may start with an optional top-level `prefix = "myproject"`. Each session's final
  name is `<prefix>-<name>`: the tool adds the dash. Without `prefix`, every name is unchanged.
- **D2.** `prefix` is applied inside `team::parse`; `parse` returns final names. `plan`, the
  duplicate check, the self-watch check, the conflict text and `team down` all see only final
  names and need no change.
- **D3.** The prefix applies to `name`, and to a `watch` or `control_from` entry only when that
  entry equals the base `name` of a session in the same file. Every other entry (a session in
  another file, or an already-final name such as `myproject-developer`) is passed through
  unchanged. If an entry equals a base name it is prefixed, even when it also equals some
  already-final name: the base-name match wins. Session names are per daemon, so an entry may
  refer to a session started by another team file.
- **D4.** An empty `prefix`, or one that ends in `-`, is an error that starts with `prefix`
  (`prefix "myproject-" must not end in '-'; the dash is added for you`). Anything else is
  checked through the final names: each final name must pass `validate_name`, so an invalid
  character, the 63-character limit and the reserved names fail with the existing messages, which
  name the final name.
- **D5.** The flag form (`NAME=EXECUTABLE`) has no file and no prefix.
- **D6.** `team down NAME ...` takes full session names, as `list` and `kill` show them. **Deviation
  from the owner's "make it fail":** a name with no session keeps the exit status 0 (spec 2o D11,
  the README, and the repeated-`down` tests say so, and a scripted teardown relies on it). Instead,
  when any explicitly given name has no session, `down` also prints one line to stderr after the
  loop: `hint: names given to team down are full session names (with any team prefix); see a2amx list`. A file-derived name that
  has no session prints no hint. The stdout lines are unchanged.
- **D7.** `command[0]` must not contain whitespace (`char::is_whitespace`, as `flag_sessions`
  tests it). `parse` fails with `session NAME: command[0] "…" contains whitespace; give each argument as its own array element`, where NAME is the final
  name (`validate_sessions` only sees final names).
  A real executable path containing a space is therefore rejected (accepted limit, documented).
  `flag_sessions` keeps its own check and message.
- **D8.** Out of scope for this spec and kept for a later spec: a per-session `role` label.

## 3. `src/team.rs`

`TeamFile` gains `#[serde(default)] prefix: Option<String>` (keeping `deny_unknown_fields`). The
public types and signatures do not change; `TeamSession.name`, `watch` and `control_from` hold final
names after `parse`. (The internal name `prefix` has no clash here; `src/main.rs` calls the attach
key `prefix` and is not touched.)

In `parse`, after the sessions are deserialized and before `validate_sessions`:

1. If `prefix` is `Some(p)`: error `prefix must not be empty` for `""`; error
   `prefix "p" must not end in '-'; the dash is added for you` for a trailing `-`.
2. Collect the set of base names (the deserialized `name` of every session).
3. For each session: `name = format!("{p}-{name}")`; each `watch` and `control_from` entry that is
   in the base-name set becomes `format!("{p}-{entry}")`; any other entry is left as it is.
4. Then `validate_sessions` runs on the final values, as it does today.

`validate_sessions` gains, per session after the empty-command check: if `command[0]` contains
whitespace, bail with the D7 message. `flag_sessions` output (a bare executable with no
whitespace) is unaffected.

## 4. `src/main.rs` (`run_team_down` only)

`names` is shadowed today (`let names = if names.is_empty() { file names } else { names }`). Keep
whether the names were explicit (`let explicit = !names.is_empty();` before the shadowing), set a flag
when an explicit name has no session, and after the loop and before the failure return, write the D6
hint to stderr once. Exit status logic does not change.

## 5. Tests (failing first, expected values are literals)

`tests/team.rs`, pure:

1. `parse` with
   `prefix = "a2amx"`, session `architect` (`watch = ["developer", "outsider"]`,
   `control_from = ["developer"]`) and session `developer` (`control_from = ["architect"]`) returns
   names `a2amx-architect`, `a2amx-developer`; the architect's `watch` is
   `["a2amx-developer", "outsider"]`, its `control_from` `["a2amx-developer"]`; the developer's
   `control_from` is `["a2amx-architect"]`.
2. An already-final entry is unchanged: with the same prefix, `watch = ["a2amx-developer"]` stays
   `["a2amx-developer"]`.
3. Rejected: `prefix = ""` and `prefix = "a2amx-"` (error text contains `prefix`);
   `prefix = "Bad"` with a session `architect` (error contains `Bad-architect`); a prefix of 55
   `a` characters with session `architect` (final name is 65 characters, error contains the final
   name); `prefix = 5` (parse error); a session that watches its own base name with a prefix
   (error contains `watches itself`).
4. Whitespace guard: `command = ["claude --model opus"]` is rejected with an error containing
   `own array element`; `command = ["claude", "--model", "opus"]` is accepted unchanged.
   (`flag_sessions(["a=cat -n"])` still fails, covered by an existing test.)

`tests/team.rs`, binary against a real daemon:

5. A file with `prefix = "pfx"` and two `sh -c "sleep 30"` sessions `architect` and `developer`:
   `team up --detach --file team.toml` prints exactly
   `started pfx-architect s1\nstarted pfx-developer s2\n`. Then
   `team down architect developer` exits 0 with stdout exactly
   `no session architect\nno session developer\n` and stderr containing `full session names`. Then
   `team down --file team.toml` prints exactly `killed pfx-architect s1\nkilled pfx-developer s2\n`
   and a second run of it prints `no session pfx-architect\nno session pfx-developer\n` with an
   empty stderr.

The existing `down_kills_only_named_team_sessions_and_reports_missing_names` test keeps passing
unchanged (its explicit-name call now also writes the hint to stderr; the test does not assert
stderr).

## 6. Documentation

- `README.md` "Teams": the example file starts with `prefix = "myproject"`; one paragraph says the
  tool adds the dash (`architect` becomes `myproject-architect`), the prefix applies to `name` and
  to `watch` and `control_from` entries that name a session in the same file (other entries are
  used as written, so a team file may refer to another file's sessions), the flag form has no
  prefix, and `team down NAME` takes full names (the README sentence about missing names mentions
  the stderr hint). One sentence: each `command` element is one
  argument, so `["claude", "--model", "opus"]`, not `["claude --model opus"]`; an executable path
  containing a space is not accepted.
- `a2amx.toml.example`: use `prefix = "myproject"` and the unprefixed names; update the header
  comment (the three lines about prefixing by hand) to say the prefix key does it.
- `docs/architecture.md`, after "The pure `team` module parses files and flags and plans
  launches.": one sentence that an optional file-level `prefix` is applied in `parse` to names and
  same-file `watch` and `control_from` entries, so `plan` and `down` only see final names.
- `docs/backlog.md`: a Closed row `C11`, `Team-file name prefix and a guard for a one-string command`,
  `spec 3g`.

## 7. Pre-lock gate notes

- Baseline run: `cargo test --test team` gives 17 passed, 0 failed on `5d65285`.
- Scope reconciliation: `team::parse`, `flag_sessions`, `TeamSession` and `team::` appear only in
  `src/main.rs`, `src/lib.rs` (a `pub mod`, unchanged) and `tests/team.rs`; `tests/daemon_cli.rs`
  uses the binary with `watch` and `team down worker` and does not use a prefix or a one-string
  command (grep `command = \["[^"]* [^"]*"` over `tests`, `README.md`, `docs` and `a2amx.toml*`
  finds nothing), so it needs no change. The old text of the hand-prefixing comment in
  `a2amx.toml.example` appears only there.
- Worked example for D3 against D2/D4: prefix `a2amx`, a session `developer` that watches
  `developer` becomes `a2amx-developer` watching `a2amx-developer`; the existing self-watch check
  in `validate_sessions` then fires, as test 3 expects.
- Worked example for D6 against the existing tests: `team down unrelated missing` still exits 0
  with the same stdout; `team down --file` with all names missing stays silent on stderr.
- Length check for test 3: 55 + 1 + 9 (`architect`) = 65 > 63.
- Acceptance commands need the new code and run after implementation.

## 8. Out of scope

- A `role` label (D8), per-session harness or launch options, defaults blocks.
- Making `team down` exit non-zero for missing names, or reading the file for explicit names.
- Prefixing in the flag form, in `new`, or in the daemon; any wire or daemon change.
- Checking that `watch` and `control_from` entries exist anywhere.
- Rewriting the owner's gitignored `a2amx.toml`.

## 9. Acceptance

After implementation, from the main checkout, on the final tracked snapshot:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list, and a
manual transcript: a temp state dir daemon, a two-session file with a `prefix`, `a2amx team up
--detach`, `a2amx list`, `a2amx team down architect` (shows the hint), `a2amx team down --file`.
