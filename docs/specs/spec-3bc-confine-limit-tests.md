# Spec 3bc: tests for the entry-count limit of `confine::plan`

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run; consultant review `m_4556`, findings 1-7 adopted, finding 8 recorded as a backlog row). Architect-directed, backlog F35, context
`a2amx-confine-limit-tests-2026-10-30`. Implementation goes to `a2amx-developer` only; the tester pass goes to
`a2amx-tester`.

**Baseline:** develop at the commit after spec 3bb is accepted (docs only, so the suite is unchanged from `9780cbb`);
run the full suite once before editing and record the counts.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** `confine::plan` through `tests/confine.rs`. No production code changes.

**Scope.** May edit exactly one file: `tests/confine.rs`. Does not touch: every other file, including `src/`.
No new dependency. Leave the diff uncommitted.

## 1. Why

The architect's mutants during the spec 3az review found two behaviors of the request-limit checks that no test pins:

| # | Mutant (in the committed `src/confine.rs`) | Behavior to pin |
| --- | --- | --- |
| 1 | `check_entry_count` (line 187): `values.len() > MAX_LIST_ENTRIES` changed to `>=` | A list of exactly 64 entries is accepted; 65 is refused with the message `<label> has more than 64 entries`. |
| 2 | `check_request_limits` (lines 681-683): swap the `confine_hide` and `confine_unhide` calls | With both lists over the limit, the error names `confine_hide` (hide is checked before unhide). |

The existing test `path_lists_and_argv_sizes_are_capped` (line 1182) only asserts `is_err()` on 65 entries; it does
not read the message, so it cannot tell the entry limit from any other refusal, and it never tries 64. Spec 3az
already pinned that `confine_write` precedes `confine_hide`.

## 2. Rule: tests only

Add tests. Do not change `src/`, existing tests or fixtures. Expected values are literals or worked examples written in
the test, never recomputed with the logic under test. A contract test passes on the unchanged code and shows no red
phase; its value is shown by the mutants of section 1.

## 3. Tests to add (names are suggestions; read the fixture helpers in `tests/confine.rs` first)

1. `a_list_of_exactly_64_entries_is_accepted`: for each of the three lists `write`, `hide` and `unhide`, a request
   holding 64 distinct missing absolute paths, `fixture.cwd.join(format!("missing-{index}"))` (hermetic: AGENTS.md
   forbids fixed paths), yields `Ok` from `plan` with Harness::Generic; the same list with 65 entries yields an error
   that contains, as a literal, `confine_write has more than 64 entries`, `confine_hide has more than 64 entries`
   and `confine_unhide has more than 64 entries` respectively. Missing paths are skipped before any guard runs for all
   three lists (claims table), so no directories need to exist. The `Ok` assertion prints the error on failure
   (`unwrap_or_else(|error| panic!("64 entries must be accepted: {error:#}"))`) and additionally asserts that each of
   the 64 paths is in `plan.skipped` (a public field; do not assert a count, `skipped` also holds missing default,
   container and mandatory paths). Only the `Ok` half kills the `>=` mutant: on the mutated code 65 entries still fail
   with the same message.
2. `hide_limit_error_precedes_unhide_limit_error`: a request with 65 `hide` and 65 `unhide` entries fails with an
   error that contains `confine_hide has more than 64 entries` and does not contain `confine_unhide`. Keep each entry
   well under 4096 bytes, keep `write` empty, and use the same missing-path style for the entries.

Use `MAX_LIST_ENTRIES` from `confine` for the counts only if it is public (it is, `src/confine.rs:19`); the expected
message text is a literal.

## 4. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test confine
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only tests/confine.rs
```

- The tester applies each mutant of section 1 to a scratch copy of the final snapshot (`git archive`, one shared
  `CARGO_TARGET_DIR`, restore after each) and names the new test that fails on it: both mutants must be killed.
  Expected: mutant 1 is killed by the 64-entry `Ok` assertion of test 1 and mutant 2 by test 2. The tester also runs
  the `confine_write`/`confine_hide` swap that spec 3az already killed, as a check that nothing regressed.
- Known flaky: backlog I9 and F37 (`tests/daemon.rs::exited_claude_session_never_reports_needs_input`, others in F37):
  rerun once, record as pre-existing if it then passes.

## 5. Out of scope

Production code, existing tests, fixtures, the 4096-byte path limit (already tested), and F36.

## 6. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `MAX_LIST_ENTRIES` is 64, public; `check_entry_count` bails with `{label} has more than {MAX_LIST_ENTRIES} entries` when `len > 64` | `src/confine.rs:19`, `:186-189` | read |
| Order of checks is write, hide, unhide | `src/confine.rs:681-683` | read |
| The existing test asserts only `is_err()` on 65 entries and never uses 64 | `tests/confine.rs:1182-1222` | read |
| A missing hide path is skipped, not an error | spec 3az claims table (`add_optional_hide`) | read |
| A missing `unhide` path is skipped, not an error (`add_canonical_existing` pushes to `skipped`) | `src/confine.rs:907-918` | read |
| The spec 3az test for write-before-hide exists | `tests/confine.rs:296` | grep |
| A missing write path is skipped before any guard runs (`add_validated_write` returns `Ok(None)`) | `src/confine.rs:571-579`, `:778-786` | read (consultant `m_4556`) |
| `check_request_limits` is the first statement of `plan`; with Generic and short paths a 65-entry error can only come from `check_entry_count` | `src/confine.rs:662`, `:680-691` | read (consultant) |
| `skipped` is a public field on `Plan` | `src/confine.rs:132` | read (consultant) |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer and tester on
the OMP harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed to
either). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | The draft said `confine_write` needs existing directories; missing write paths are skipped before any guard runs, so all three lists can be tested with missing paths | Tests 1 covers write, hide and unhide with missing paths; the "only if cheap" branch removed |
| Draft review | Consultant | Only the `Ok` half of the 64-entry test kills the `>=` mutant; the draft did not say so, and the `Ok` assertion did not print the error | Both stated in section 3 |
| Draft review | Consultant | `/nonexistent-3bc-{i}` is a fixed path, against AGENTS.md | Paths rooted in the fixture's cwd |
| Draft review | Consultant | The acceptance did not say which test kills which mutant | Mapped in section 4 |
| Draft review | Consultant | The team-file validation has its own copy of the limit with no boundary test | Backlog F39 |
| Tester pass | Tester (`gpt-5.6-luna`) | Advisor asked for an independent full `cargo test` and clippy run against section 4 before the report | Done: 697 passed, 0 failed, 3 ignored; clippy and fmt clean |
| Architect review | Architect, own mutants | none: the `>=`, the hide/unhide swap and a limit of 65 were all killed; the tester's ten mutants had no survivors | no problems found at this stage |
