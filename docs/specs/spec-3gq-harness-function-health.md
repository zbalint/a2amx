# Spec 3gq: `src/harness.rs` function-level code health

## 0. Status

LOCKED after consultant review m_8671 (seven findings applied); assign only after spec 3gp is accepted (one active writer). Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; HEAD at drafting dae5b21; the governing version is the commit that adds this file. Context `a2amx-main-code-health-2026-11-22`.

Scope (edit): `src/harness.rs`; tests: `tests/messaging/claude_readiness.rs`, `tests/messaging/agy_readiness.rs`, `tests/messaging/harness_wiring.rs`, `tests/agy_wiring.rs`, `tests/omp_wiring.rs` (and `tests/confine/notices_and_preflight.rs` for `validate_prompt_options`), for characterization tests only. Does not touch: any other file, `Cargo.toml`, `CHANGELOG.md`.

Owner rules (memories d295229d, 828f8118, fb283e4c): goal is a CodeScene-clean repo; no `@codescene` or other suppression comments; an agent never dismisses a finding: whatever a step does not fix is reported. Lessons that bind (memory df233241): list checks in order and split only into contiguous runs; one precedence test per adjacent pair of guards with different results, exact texts asserted; mutate every condition of every new function.

## 1. Why

`cs check src/harness.rs` (cs 1.0.58, 2026-11-22; match by function name) scores the file 6.31 with 19 findings. The function-level ones:

| Function | Finding | Verdict |
|---|---|---|
| `agy_composer_idle` | Complex Conditional (`bottom != top + 2 \|\| cursor.row != top + 1 \|\| cursor.col != 2`) | real: one named predicate |
| `claude_ready` | Complex Method cc 10, Complex Conditional | real: mode test, glyph test, row bounds and rule rows in one function |
| `claude_composer_empty` | Complex Method cc 13, Bumpy Road | real: the "dim placeholder suggestion" analysis is a self-contained second half |
| `paste_view` | Deep Nested Complexity (depth 4) | real: the Claude escaping loop sits inside a `match` arm |
| `build_agy_status_line` | Complex Method cc 9 | real: two branches build the identical `--report` object (merging them into one `\|\|` may not lower cc; the split in step 5 is the guard extraction) |
| `take_user_settings` | Complex Method cc 14, Bumpy Road 3 | real: scan, value parse and argument stripping in one function |
| `validate_prompt_options` | Complex Method cc 10, Bumpy Road 2 | real: the OMP prompt-value-is-a-file test is inline |
| `append_system_prompt` | Complex Method cc 9, Bumpy Road 2 | real, same family of loops as the previous two |
| `claude_argv` (private) | 7 arguments | real: a private struct changes no public path |
| `wire_agy_argv` | Bumpy Road 2 | to be re-measured after the others |

## 2. Changes (one step per function; suite green after each; no behavior change, error texts byte for byte)

0. Characterization first, per function: name the existing test that pins each branch and each error text (`claude_readiness.rs`, `agy_readiness.rs`, `harness_wiring.rs`, `agy_wiring.rs`, `omp_wiring.rs`, `notices_and_preflight.rs`) and add a passing test with literal expectations for any branch or text not pinned, with exact `assert_eq!` (not `contains`). Screen-heuristic functions (`claude_ready`, `claude_composer_empty`, `agy_composer_idle`) are tested through `a2amx::harness::ready`, `claude_composer_empty` and `agy_composer_idle` with hand-built screens. Inputs to add where no test pins them: Claude: cursor and `❯` at row 0 (must return false, not panic), `❯` followed by a non-space and by NBSP, a rule only above, a dialog marker; composer-empty: cursor column 3, first cell inverse with a non-dim rest, inverse with no dim cell, `[Image #` and `[Pasted text #`; agy: each of the three cursor comparisons failing alone (bottom = top+3, cursor row top+2, column 3) and each placeholder followed by non-blank text; `paste_view`: `<pasted_content` at index 0 (the `start == 0` sentinel). `take_user_settings` (exact `assert_eq!` texts; today only `contains("--settings")` at `harness_wiring.rs:417-450`): `["claude","--settings"]`, `[..,"--settings="]`, `[..,"--settings","  "]`, a missing file (the text includes the value), `{` inline, `[1]` in a file, precedence `["--settings=","--settings"]` and `["--settings","{}","--settings="]` (a later bad value wins over an earlier good one), `["claude","--","--settings"]` untouched, and inline detection of `{x` and `x}` (read as files). `validate_prompt_options` and `append_system_prompt` (today `is_err` only at `notices_and_preflight.rs:146-216`): Claude `["claude","--append-system-prompt"]` (missing value), Claude with `-file` as the separate value, Claude with `-file` after a missing value (the `-file` error wins), Omp with a file value then a missing value (the file error wins), Omp with `--append-system-prompt=` empty (not a file), Codex and Agy with a file value (Ok), Generic with a malformed value (argv unchanged, no validation). `build_agy_status_line`: `{"command":"a\u0000b","enabled":false}` and `{"command":5,"enabled":false}` return the user's object unchanged (the NUL and non-object guards win over `enabled = false`; a merged condition stays after the NUL check).
1. `agy_composer_idle`: one private predicate for "cursor is on the composer's input row" (the three comparisons); the order of the early returns (`ready`, `agy_composer`, cursor test, blank or placeholder row) stays. The `let Some(..) = agy_composer(screen) else` branch is unreachable (`ready(Agy, ..)` already requires it); it is not removed here (section 4).
2. `claude_ready`: all exits return false and every check is pure, so order matters only for panics: the `row == 0` test must stay before `row - 1`. Split into contiguous steps (mode and cursor, glyph, row bounds, rules, dialog marker): extract the glyph test and the bounds-and-rules test as private functions, bounds before rules.
3. `claude_composer_empty`: extract the part after the blank-row early return (inverse first cell, dim-only rest, not a `[Pasted text #`/`[Image #` marker) as a private function; the early checks (`claude_ready`, column 2, cell slice) and the blank-row `true` return stay in `claude_composer_empty` in the same order: the `true` must come before the inverse-first-cell test, and the empty-cells test must come before `cells[1..]` (a panic otherwise).
4. `paste_view`: move the Claude escaping into its own private function taking and returning the text; the `match` keeps one line per harness.
5. `build_agy_status_line`: construct the `--report` object once for the empty-command and `enabled = false` branches (same JSON for every input), and extract the guard run (non-object, no string command, NUL byte: each returns the user's object unchanged) as one private function returning the command or the early value; the NUL and non-object guards stay before the `enabled = false` test.
6. `take_user_settings`: three private functions in the same order as today's phases: find the value (`--settings`, `--settings=`, with the two missing-value errors), parse it (inline JSON or file, the empty and invalid-JSON and not-object errors), strip the flags (the `--` boundary stays).
7. `validate_prompt_options` and `append_system_prompt`: the two loops iterate the same range with the same two forms (the consultant verified this against HEAD): extract the "find the `--append-system-prompt` values (separate form skips its value, `=` form, missing-value error)" iteration into one private function used by both. `validate_prompt_options` keeps its own first-`--` slice, its Claude `-file` pre-scan over the whole slice before the loop (so `-file` wins over a missing value, and fires even when `-file` is the value of a separate `--append-system-prompt`), and its signature (callers: `src/team/validate.rs:165`, `tests/confine/notices_and_preflight.rs:200,213`). `append_system_prompt` keeps the Generic early return before any validation, then `validate_prompt_options` first, then its last-occurrence loop. The missing-value error inside `append_system_prompt` is unreachable (validate runs first on the identical slice) and disappears with the duplicate; the developer says so in the report. Extract the OMP file test (`Path::new(value)`, absolute or joined with the base directory, `is_file`) as `omp_value_is_file`.
8. `claude_argv` (private, 7 arguments): take one private struct built by the two public wrappers (`wire_claude_argv`, `wire_claude_channel_argv`) whose own signatures do not change.
9. Run `cs check src/harness.rs`; report each remaining finding with function and line. Do not touch the signatures of `wire_claude_argv`, `wire_claude_channel_argv` or `wire_omp_argv` (section 4).

## 3. Acceptance

- `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; counts before and after.
- `git status --short` lists only files in section 0.
- `CS_DISABLE_VERSION_CHECK=1 cs check src/harness.rs` before and after lines pasted; none of the section 1 findings remains (except `wire_agy_argv` if it persists: reported) and none is new.
- Tester mutation pass: every condition of every new function, naming the killing test. Reordering the false returns of `claude_ready` and `agy_composer_idle` is an equivalent mutant except where an earlier check guards a panic (`row == 0` before `row - 1`); do not demand killers for the others. Required: the `row == 0` guard moved after the rules (panic); in `claude_composer_empty` the blank-row `true` moved after the inverse test and `cells[1..]` before the first-cell check; the three comparisons of the cursor predicate separately; each branch of the `--settings` scan, `starts_with`/`ends_with` of the inline-JSON test, and `trim()` dropped in `take_user_settings` and in the empty-command check; the Claude `-file` pre-scan moved into the loop; `append_system_prompt`'s last-occurrence turned into first; the NUL guard moved after the `enabled = false` test; each `validate_prompt_options` rejection.

## 4. Flagged for the owner (not fixed here)

| Finding | Why flagged |
|---|---|
| `wire_claude_argv` (6 arguments), `wire_claude_channel_argv` (6) | A parameter struct is a public-signature change with 31 call sites (`src/commands/launch.rs` 2, `tests/messaging/harness_wiring.rs` 22, `tests/statusline.rs` 5, `tests/claude_channel.rs` 2). A design choice with a very large mechanical diff; proposal: a `ClaudeWiring` struct as its own spec if the owner wants it. (`claude_argv`, 7 arguments, is private and is fixed in step 8.) |
| `wire_omp_argv` (5 arguments) | Public; 5 call sites (`src/commands/launch.rs` 1, `tests/omp_wiring.rs` 4). Same proposal. |
| Unreachable or misleading code found while reviewing, deliberately left alone: the `agy_composer` else-branch in `agy_composer_idle`; the missing-value error in `append_system_prompt` (goes with the shared iterator, step 7); the non-object tests in `wire_agy_argv` and `merge_settings` (an object is guaranteed); the short-cells branch in `claude_composer_empty` (only if the emulator builds short cell vectors: unchecked); and the wrong wording "permissions.allow must be an array" when `permissions` itself is not an object in `wire_agy_argv` (pinned by `tests/agy_wiring.rs`; a text fix is a behavior change). | Each is a code or behavior change the owner has not approved. |
| file-level Overall Code Complexity | Re-measured after this spec. The file holds readiness for five harnesses, argv wiring for four and the prompt-option rules; splitting it moves public paths. |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (strongest slot, review m_8671); developer antigravity-cli Gemini 4 Hydrogen (Medium); tester omp default slot (deepseek-v4.1-flash), automatic advisor. BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | seven findings: guard-order panics (`row == 0`), duplicate validation making the append missing-value error unreachable, call counts of the argv wrappers | applied before locking |
| Architect review | architect | `step_settings_value` introduced a new Bumpy Road 2 | developer split it into `separate_settings_value` and `inline_settings_value`; cs 9.38 to 9.53 |
| Tester pass | tester | survivor: `--append-system-prompt` last occurrence turned into first was not killed (spec-required kill) | architect added `append_system_prompt_extends_the_last_occurrence`; mutant re-run killed |
| Tester pass | tester | survivor: an inverse NBSP first cell with a dim rest was accepted as a suggestion | architect added `inverse_nbsp_with_dim` case to the claude readiness edge test; mutant re-run killed |

Equivalent or unreachable mutants (no killers): rules bound `>=` versus `>`, glyph/rules order swap, dim scan before the first-cell guard (needs an empty cell slice the emulator cannot produce). Narrow survivors left unpinned and reported: re-scan of a separate `--settings` value that is itself `--settings`, parse object check precedence, `rposition` with two `--` separators, re-scan of a prompt value that is itself `--append-system-prompt=`.
