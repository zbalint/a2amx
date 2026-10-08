# Spec 3q: a session with a `role` and no `name` uses the role as its name

## 0. Status

**LOCKED** (2026-10-08, pre-lock gate run against develop at `308a599`; gate notes in section 8;
consultant review `m_797`, no blocking findings, advisories applied). Owner request in this session: "when in the config the name is not set but the role is
set than we should use the role as the name as well". Owner decisions: a role that is not a valid
name fails fast with an error (no silent conversion); a session that has a `name` keeps using it
exactly as today and the example file keeps its explicit names. Architect decisions are in section 2.

**Baseline:** develop at `308a599`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-role-as-name`.
Public test seam: `team::parse` (`tests/team.rs`, pure, no daemon). No binary or daemon test is
needed: the change happens before any request is built.

**Scope.** May edit exactly these files and no others.

- `src/team.rs`
- `tests/team.rs`
- `README.md`, `a2amx.toml.example`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/main.rs` (the flag form `NAME=EXECUTABLE` has no role), `src/messaging.rs`
(`validate_name` and `validate_role` stay as they are), `src/cli.rs`, `src/daemon.rs`,
`src/wire.rs`, every other test file, every spec document, `AGENTS.md`, the owner's gitignored
`a2amx.toml`. No new dependency, no wire change. Do not commit, stage or merge: leave the diff
uncommitted in the working tree.

## 1. Why

In a team file the session `name` and `role` are usually the same word (`architect`, `developer`,
`consultant`). Today `name` is required, so every entry says it twice. A role-only entry should be
enough. The role label itself (what the session sees in its system prompt) does not change.

## 2. Decisions

- **D1. Rule.** In a `[[session]]` table with no `name` key and a string `role` key, the name is the
  role. It is filled in before the table is deserialized, so everything after it (the team prefix,
  `base_names` for `watch` and `control_from` references, `validate_sessions`, duplicate detection,
  `plan`, the launch) sees an ordinary session with that name. The `role` stays set, so the session
  has both with the same value.
- **D2. Explicit name wins.** A table with a `name` key behaves exactly as today, whatever its
  `role`. The example file keeps its explicit names.
- **D3. Fail fast.** The derived name goes through `validate_name` immediately. A role that is not a
  valid name (capitals, spaces, longer than 63 characters, empty, a reserved name such as `s12`) is an
  error naming the session position and the role, and telling the operator to set `name`:
  `session N: name taken from role "ROLE": REASON; set name explicitly` where `REASON` is the text of
  the `validate_name` error. There is no conversion to a different spelling. The check runs on the
  bare role, before the team prefix is applied; a valid role that gives a too-long prefixed name is
  caught later by the existing `validate_sessions` check, unchanged. A role with leading or trailing
  whitespace fails the same way. An explicit `name = ""` is an explicit name (D2) and gets the
  existing `validate_sessions` error.
- **D4. Neither key.** A table with neither `name` nor `role` is an error:
  `session N: name or role is required`. Both D3 and D4 errors are returned directly with `bail!`
  inside the closure, not wrapped in the closure's `session N` context (they already carry the
  position, and wrapping would print `session 1: session 1: ...`). A table whose `role` is present but not a string is left to
  the normal deserialization error (`session N` context, role type error), exactly as today.
- **D5. Duplicates.** Two role-only entries with the same role are a duplicate name; the existing
  `session NAME: duplicate name` error stays (NAME is the final, prefixed name). No new wording.
- **D6. Flag form.** `NAME=EXECUTABLE` items have no role and are unchanged.

## 3. `src/team.rs`

In `parse`, inside the closure that turns each `toml::Table` into a `TeamSession` (the
`.map(|(index, table)| { ... })` over `session`), before `try_into`: make the table mutable; if it
has no `"name"` key, then if it has a `"role"` value that is a `toml::Value::String`, validate that
string with `validate_name` (D3 message, returned with `bail!` and no extra `session N` context, `index + 1` is the session position) and insert it as
`"name"`; if there is no `"role"` key, return the D4 error; any other `role` type falls through to
`try_into`. Reuse `validate_name`; add no new helper. A comment says why (name defaults to role).
Nothing else in `parse`, `validate_sessions` or `plan` changes.

## 4. Tests (failing first, expected values are literals)

`tests/team.rs`, pure `team::parse`:

1. `[[session]]` with `role = "architect"` and `command = ["cat"]` and no name: the parsed session has
   `name == "architect"` and `role == Some("architect")`.
2. The same inside a file with `team = "t"` and a second session `name = "developer"` with
   `watch = ["architect"]` (the bare name, resolved against `base_names`, which includes the derived
   name): the first session is `t-architect`, and the second one's `watch` is
   `["t-architect"]` (the derived name takes part in reference resolution).
3. `name = "worker"` plus `role = "architect"`: `name == "worker"`, `role == Some("architect")`.
4. `role = "Senior Reviewer"` and no name: the error text contains `session 1: name taken from role
   "Senior Reviewer"` and `set name explicitly`. A `role = "s12"` and no name fails the same way.
5. A table with neither key (only `command`): the error text contains `session 1: name or role is
   required`.
6. Two role-only entries with `role = "developer"`: the error text contains `duplicate name`.
7. `role = " architect"` (leading space) and no name fails with the D3 text.
8. The existing tests in `tests/team.rs` pass unchanged.

## 5. Documentation

- `README.md` "Teams": one sentence next to the `role` paragraph: a session may omit `name` when it
  has a `role`, the role is then the name (it must be a valid name, otherwise set `name`), and an
  explicit `name` always wins.
- `a2amx.toml.example`: one comment line in the existing `role` comment block near the top (about
  lines 12 to 14), not inside a session: `# name may be omitted when role is set; the role is then the name`. The explicit names stay.
- `docs/architecture.md`: one sentence in the paragraph that starts `The pure `team` module` (about line 315).
- `docs/backlog.md`: Closed row `C21`, `A session with a role and no name uses the role as its name`,
  `spec 3q`.

## 6. Out of scope

Converting an invalid role into a name, role-based `watch`/`control_from` references beyond what the
derived name already gives, a `name` default for the flag form, any change to how the role is shown
to the session, `src/main.rs`.

## 7. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list. These
commands need the new code and run after implementation.

## 8. Pre-lock gate notes

- Baseline: `env -u A2AMX_BIN -u NO_COLOR cargo test` passed on `188a404` (430 passed, 27 suites, 3
  ignored); `308a599` changed only documentation.
- Facts checked against the code: `TeamSession.name` is a required `String` (`src/team.rs` line 16)
  and a missing key today gives serde's missing-field error under `session N`; no existing test
  asserts that text (`rg "missing field" tests src` has no hits). `base_names` is built from the
  deserialized names, so filling the key before `try_into` is enough for references. `validate_name`
  already rejects capitals, spaces, empties, over-long labels and `s[0-9]+` ids.
- Files named by the commands or inspected: `src/team.rs`, `tests/team.rs` and the four docs are all in
  section 0's scope; `rg "TeamSession \{" src tests` hits only `src/team.rs` (flag form, no change)
  and `tests/team.rs`; no struct field is added, so no literal changes.
- Rules stated twice, to diff after the last edit: the D3 error text (section 2, test 4); the D4
  error text (section 2, test 5).
- Not probed: the interactive effect on a live team (`team up` of a role-only file); it needs the new
  code.
