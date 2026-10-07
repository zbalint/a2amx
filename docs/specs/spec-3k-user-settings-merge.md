# Spec 3k: merge a user-supplied `--settings` into the Claude launch instead of overriding it

## 0. Status

**LOCKED** (2026-10-07, pre-lock gate run against develop at `3888dcb`; consultant report `m_702`,
findings dispositioned in section 8). The owner reported that a
`--settings` flag given in a team file's `command` is lost: a2amx appends its own `--settings`
last, and Claude Code uses the last one. The owner confirmed passing `--settings` is a legitimate
use (the ctrscm plugin needs it), and chose inline JSON in argv for the file form (D2).

**Baseline:** develop at `3888dcb`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-user-settings-merge`. Public test seams: `harness::wire_claude_argv` and
`wire_claude_channel_argv` (`tests/messaging.rs`, `tests/claude_channel.rs`).

**Scope.** May edit exactly these files and no others.

- `src/harness.rs`, `src/main.rs` (`create_session`, the two wire calls only)
- `tests/messaging.rs`, `tests/claude_channel.rs` (its two wire calls only)
- `README.md`, `docs/architecture.md`, `docs/delivery.md`, `docs/backlog.md`

Does not touch: `src/team.rs`, `src/cli.rs`, `src/codex.rs`, `src/daemon.rs`, `src/hook.rs`, the
OMP and Codex wiring, every other test file, every other spec, `AGENTS.md`. No new dependency.
Do not commit, stage or merge: leave the diff uncommitted in the working tree.

## 1. Why

`claude_argv` (`src/harness.rs:259`) appends `--settings '<hook json>'` through `insert_extras`
without looking at the argv it was given. A user `--settings` earlier in `command` is therefore
followed by a2amx's. Probe on Claude Code 2.1.292: `claude -p --settings /nonexistent-first.json
--settings /nonexistent-second.json hi` fails with `Settings file not found:
/nonexistent-second.json`, so the last flag is the one used and the user's settings never load.
The loss is silent. Claude takes one `--settings`, so the fix is one merged value.

## 2. Decisions

- **D1. Detection.** Only arguments before the first literal `--` are scanned (as `insert_extras`
  does). Both `--settings VALUE` and `--settings=VALUE` count. When there are several, the last
  one is the user's effective value (Claude's own rule); every occurrence is removed from the
  output. A `--settings` with no following value, and `--settings=` with an empty value, are
  errors (Claude silently ignores the empty form; D4 is stricter on purpose).
- **D2. Value.** After trimming whitespace, a value that starts with `{` AND ends with `}` is
  inline JSON (Claude 2.1.292 probe: `{bad` is treated as a path, `{bad}` as inline JSON); anything
  else is a file path, read at launch, relative to the session's cwd (the `cwd` already passed to
  `create_session`; for a team session that is the team-file-relative cwd; `~` is not expanded,
  as in Claude). **Owner decision (2026-10-07):** the merged result is passed inline in argv for
  the file form too, so a settings file's whole contents, including any `env` block, appear in
  the process arguments, `ps` and `a2amx list --details`. This is documented in section 6, not
  prevented.
- **D3. Merge.** The value must parse to a JSON object. a2amx's hook entry (today's
  `UserPromptSubmit` element, unchanged) is added: `hooks` absent → created; `hooks` an object
  without `UserPromptSubmit` → the key is created with the one entry; `UserPromptSubmit` an array
  → the a2amx entry is pushed last. A `hooks` that is not an object, or a `UserPromptSubmit` that
  is not an array, is an error. Every other key and every existing hook is left as the user wrote
  it. A user `disableAllHooks` is kept as written and would disable the a2amx hook; this is
  documented in section 6, not an error.
- **D4. Errors fail the launch.** Unreadable file, invalid JSON (including a file Claude would
  silently ignore), a non-object, or a wrong-shaped `hooks` makes `a2amx new` / `team up` fail with a message naming `--settings` and the cause. No
  fallback to a2amx's settings alone, because that is today's silent loss.
- **D5. Output.** Exactly one `--settings <compact JSON>`, in the position a2amx's flag has today
  (last among its extras, before any `--`). With no user `--settings` the output is byte-identical
  to today.
- **D6. Signature.** `wire_claude_argv` and `wire_claude_channel_argv` gain a `cwd: &Path`
  parameter (after `exe`) and return `anyhow::Result<Vec<String>>`. `create_session` runs them in
  `tokio::task::spawn_blocking` because the file read can stall (AGENTS.md). No third helper is
  added beyond one private merge function in `src/harness.rs`.
- **D7. Not extended.** `--mcp-config`, `--allowedTools` and `--append-system-prompt` supplied by
  the user have the same duplicate-flag shape and are not changed here; see section 8 and the
  backlog row in section 6.

## 3. `src/harness.rs`

`claude_argv` first calls a private `take_user_settings(argv, cwd) -> Result<(Vec<String>,
Option<serde_json::Value>)>` implementing D1 and D2, then builds a2amx's settings value as today
and, when a user value exists, merges per D3 before `settings.to_string()`. `wire_claude_argv`
and `wire_claude_channel_argv` thread `cwd` and the `Result`. The hook JSON and every other extra
are unchanged.

## 4. `src/main.rs`

`create_session` passes `&cwd` to the two wire calls and wraps them in `spawn_blocking` (as the
OMP branch does for `omp::install`), with `?` on both layers. The closure is `'static`: `command`
and `exe` move in, and `role` is cloned into it (the original is moved later into the Codex
environment). `cwd` must be cloned before it is turned into a `String` at line 474 (it is the
same value the session is created with).

## 5. Tests (failing first, expected values are literals)

`tests/messaging.rs` (existing calls gain the `cwd` argument and `.expect(..)`):

1. No user `--settings`: output is exactly today's (existing assertions hold).
2. Inline `--settings '{"model":"opus"}'` before a2amx's extras: one `--settings` in the output,
   whose value parses to `{"model":"opus","hooks":{"UserPromptSubmit":[<a2amx entry>]}}`; the
   user's original pair is gone; a trailing `-- prompt` is untouched after it.
3. `--settings=<json>` form behaves as 2.
4. Existing hooks: user `{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"x"}]}],"Stop":[1]}}`
   gives `UserPromptSubmit` of two entries (user's first, a2amx's last) and `Stop` unchanged.
5. File form: a temp file with the JSON, passed by absolute path and by a path relative to `cwd`,
   merges the same as 2.
6. Two `--settings` flags: only the last one's contents appear.
7. Errors (each `Err`, message containing `--settings`): missing file; a file holding invalid
   JSON; the value `{bad` (treated as a path, so missing file); the value `{bad}` (invalid inline
   JSON); a JSON array in a file; `"hooks": 3`; `"hooks":{"UserPromptSubmit":"x"}`; a trailing
   `--settings` with no value; `--settings=` with an empty value.
7a. Inline value with surrounding whitespace, ` {"a":1} `, is accepted and merged.
8. `wire_claude_channel_argv` merges the same as 2.

## 6. Documentation

- `README.md` (the `--settings` sentence near line 132), `docs/architecture.md` (line 425) and
  `docs/delivery.md` (line 363): one sentence each that a `--settings` in the command is merged
  with the hook rather than replaced. `docs/delivery.md` (line 363) also states: the file form is
  passed inline, so its contents (including any `env` secrets) appear in argv and `a2amx list
  --details`; an invalid file fails the launch where Claude alone would ignore it; a user
  `disableAllHooks` would disable the a2amx hook.
- `docs/backlog.md`: Closed row `C15`, `User --settings merged into the Claude launch`, `spec 3k`;
  one new Open row for D7 (`--mcp-config`, `--allowedTools`, `--append-system-prompt` supplied by
  the user; behavior with duplicates unprobed, `--append-system-prompt` probably last-wins),
  evidence `code`, with the ID following the file's open-item numbering.

## 7. Out of scope

Everything in D7; OMP and Codex wiring; a user-visible warning; changing the hook entry; a
`settings` field in the team file; merging settings keys other than the hook.

## 8. Pre-lock gate notes (to complete before locking)

- Callers of the two wire functions (rg over `src` and `tests`): `src/main.rs` 445 and 452;
  `tests/messaging.rs` 159, 207, 225, 242, 249, 262, 272, 298; `tests/claude_channel.rs` 415 and
  446. No other callers.
- Probed on Claude 2.1.292 (architect and consultant): last `--settings` wins; `--settings=VALUE`
  is accepted; inline iff trimmed value starts `{` and ends `}`; relative paths resolve against the
  process cwd; empty `=` value is ignored by Claude; an invalid file is ignored by Claude.
- Consultant `m_702` dispositions: F1 owner decision (inline, D2); F2 adopted (D2); F3 error (D1);
  F4 confirmed; F5 documented (section 6); F6 `disableAllHooks` documented, hook order irrelevant
  (recalled, not probed); F7 callers re-verified; F8 D6 kept with the `role` clone (section 4); F9
  backlog wording (section 6).
- D1 against D2 worked example: `--settings ' {"a":1} '` is inline and merged; `--settings={bad`
  is a path, so a missing-file error; `--settings=` is an error.
- Acceptance commands need the new code and run after implementation.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (spec 2n) are rerun once and reported. Report
`git status --short` showing only files in section 0's scope.
