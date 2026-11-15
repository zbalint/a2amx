# Spec 3eo: two corrections to the `agy` profile after the 2026-11-15 probes

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop` (HEAD `7d7fa46` or
later). Shared task `context_id`: `agy-harness`. A correction to spec 3en, found by the tester's probes
P1-P9 (2026-11-15) against `agy` 1.5.9. Spec 2 (plugin) follows.

**Scope.** May edit exactly these files and no others: `src/quota.rs`, `src/harness.rs`,
`tests/quota.rs`, `tests/agy_wiring.rs`.

Does not touch: every other file, `docs/`, `CHANGELOG.md`, every other spec. Do not commit, stage or
merge: leave the diff uncommitted.

**Public test seams.** `quota::from_agy_status` (pure) and `harness::wire_agy_argv` (deterministic over its
arguments and the filesystem they name).

## 1. Why

The probes measured two things spec 3en got wrong or never checked:

1. **Model ids are display strings.** With `--model <slug>` or a model chosen in the picker, the status-line
   payload's `model.id` is the display name, not the slug: `"Gemini 4 Argon (Medium)"`,
   `"Claude Opus 5.5 (Low)"`, `"GPT-OSS 120B (Medium)"`, `"Claude Sonnet 5.5 (Medium)"`. (An earlier probe
   with the default model saw `"gemini-3.8-flash-low"`; both forms occur.) `model_group` in `src/quota.rs`
   compares case-sensitive lowercase prefixes, so every display string gives group `None`, and the quota
   report silently becomes the pessimistic minimum over all buckets instead of the model's own bucket group.
2. **`trustedWorkspaces` needs the exact resolved path.** The probes showed `agy` compares the entry with the
   kernel-resolved working directory as a plain string: a symlink path, a trailing slash and a trusted parent
   directory each still show the trust dialog. `wire_agy_argv` injects the `cwd` it is given. If the launch
   directory is not canonical (a symlink component, or a `.`/`..` component), the injected entry never matches
   and the trust dialog appears in a session no human attends. The realistic source is a team file: `team up`
   joins the file's directory and the session's `cwd` (`src/main.rs:1663-1666`), which can carry both; a
   `new` session takes `current_dir()`, which is already resolved, so there the fix changes nothing.

Rejected: normalising the trailing slash (a `Path` already has none; the probe only showed `agy` does not
normalise the entry); trusting the parent directory (does not work).

## 2. `src/quota.rs`: `model_group`

Lowercase the model text (`to_ascii_lowercase`) before the existing rules, which stay as they are, in this
order: `starts_with("claude")` or `starts_with("gpt")` gives `3p`; `contains("argon")` gives
`gemini-argon-wtu`; `starts_with("gemini")` gives `gemini`; otherwise `None`. Resulting groups for the probed
ids: `"Claude Opus 5.5 (Low)"` and `"GPT-OSS 120B (Medium)"` give `3p`; `"Gemini 4 Argon (Medium)"` gives
`gemini-argon-wtu`; `"Gemini 3.8 Flash (Medium)"` and `"gemini-3.8-flash-low"` give `gemini`. A new comment
states that `model.id` is a display name or a slug, which is why the match is case-insensitive.

## 3. `src/harness.rs`: `wire_agy_argv`

The `trustedWorkspaces` array receives the `cwd` string as now, and additionally the canonical path
(`std::fs::canonicalize(cwd)`) when canonicalisation succeeds and the string differs from `cwd`; each value is
added once (existing duplicate check). When canonicalisation fails (a path that does not exist, as in the unit
tests) only `cwd` is added; this is not a swallowed error because the literal entry is still injected and
`agy` itself reports a missing directory. A `shortcut:` comment states the ceiling: the entry is resolved once at wiring time, so a directory renamed or
re-linked between wiring and spawn, or a canonical path that differs from what `agy`'s own `getcwd` returns
(another mount namespace), would still show the dialog; upgrade by resolving at spawn if that is ever seen.

## 4. Tests

1. `tests/quota.rs`: one test per probed id (`"Claude Opus 5.5 (Low)"`, `"GPT-OSS 120B (Medium)"`,
   `"Gemini 4 Argon (Medium)"`, `"Gemini 3.8 Flash (Medium)"`) using the five-bucket payload of the existing
   `agy_status_selects_model_group_windows` test (`tests/quota.rs:1290`) unchanged, asserting the group's own
   windows with literal expected values (for that payload at `now = 1000`: `3p` gives five-hour 40% reset 1100
   and weekly 90% reset 1200; `gemini` gives 77% reset 1300 and 90% reset 1400; `gemini-argon-wtu` gives
   `five_hour: None` and weekly 80% reset 1500). Required property: in each test the expected window differs
   from the pessimistic minimum over all buckets in at least one of the two windows (on this payload the
   minimum is five-hour 40% and weekly 80%, so the `3p`, `gemini` and Argon cases all differ), so each fails
   on `7d7fa46`; do not change the fractions.
2. `tests/agy_wiring.rs`: a test that canonicalises a temp directory root first (`fs::canonicalize`), creates
   a sub-directory and a symlink to it, passes the symlink path as `cwd`, and asserts `trustedWorkspaces` is
   exactly the symlink path then the canonical path; a second test passes `<root>/sub/../sub` as `cwd` and
   asserts the canonical path is present. Both fail on `7d7fa46`. The existing
   `user_settings_keep_other_keys_and_override_queue_and_trust` (lines 48-85) asserts one entry for an existing
   `cwd` and is affected: canonicalise its `tempdir()` root before building `cwd` so it holds when `TMPDIR`
   is behind a symlink. `default_settings_are_injected_once` (nonexistent `/work/project`) stays unchanged.
3. Mutation check: the developer reverts the lowercase call and removes the canonical push in turn, names the
   failing test for each, and restores the files (`sha256sum` before and after); mutate in the checkout.

Assumption, not measured: `Orchestrator (Argon)` maps to `gemini-argon-wtu` and `Gemini 4 Hydrogen/Carbon` to
`gemini`; `GPT-OSS` to `3p` is the existing rule (the probes saw the same five buckets for every model).

## 5. Out of scope

`wired_command` in `src/main.rs`; any other harness; the `/model` persistence and `A2AMX_TOKEN`
visibility findings (documented by spec 4); the two surviving mutants of spec 3en (specs 2 and 3 cover
them); `src/confine.rs`.

## 6. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` all pass (the repo's
   documented commands).
2. `git status --short` lists exactly the four scoped files (the architect and tester judge).
3. The mutation check in 4.3 names a failing test for each mutant (the developer's report; the tester
   reruns it).

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `model_group` is case-sensitive prefix matching | `src/quota.rs` (`model_group`) | read |
| `wire_agy_argv` injects the given cwd only | `src/harness.rs` (`wire_agy_argv`) | read |
| Display-string ids and exact-path trust matching | tester report m_7452 (P5, P6) | tester measurement, not rerun by the architect |
| `default_settings_are_injected_once` uses a nonexistent cwd; `user_settings_keep_other_keys_...` uses an existing one | `tests/agy_wiring.rs:19-45`, `48-85` | read (consultant) |
| Quota windows of the five-bucket payload per group | `tests/quota.rs:1290-1316` | read; expected values derived by hand from `now = 1000` and the payload's `reset_in_seconds` (100, 200, 300, 400, 500) and fractions |
| `take_user_settings` already keeps only the last `--settings` | `src/harness.rs:589-615` | read (matches probe P4) |
