# Spec 3az: tests for four behaviors of `confine::plan` that no test pins

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `b2582e3`; consultant review `m_4466`, findings A-D adopted, E needed no change). Architect-directed, backlog F34, context `a2amx-sonar-3ax-2026-10-30`.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `b2582e3`; the full suite passes there (685 passed, 0 failed, 3 ignored).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** `confine::plan` through `tests/confine.rs`. No production code changes.

**Scope.** May edit exactly one file: `tests/confine.rs`. Does not touch: every other file, including `src/`.
No new dependency. Leave the diff uncommitted.

## 1. Why

The spec 3ay mutation pass found four behaviors of `plan` that no test pins; the tester ran them on the code after
spec 3ay. The code before spec 3ay has the same four lines, so they are gaps in the older tests, not regressions of
that refactor (read, not run on the old code). A later change to `plan` could break any of them unnoticed.

| # | Mutant (in the committed `src/confine.rs`) | Behavior to pin |
| --- | --- | --- |
| 1 | `finish`: remove `self.skipped.sort();` | `Plan::skipped_paths()` is sorted. The dry run prints it (`src/main.rs`, `plan.skipped_paths()` loop), so its order is visible to the operator. |
| 2 | `hide_or_skip`: when `kind` is `HideKind::Container` and the path is missing, do not record it in `skipped` | A missing container-socket path is reported in `skipped`. |
| 3 | `check_request_limits`: swap the `confine_write` and `confine_hide` `check_entry_count` calls | With both lists over the limit, the error names `confine_write`. |
| 4 | `unhide`: remove `self.default_hidden.retain(..)` | An unhidden default-hidden path leaves `default_hidden` as well as `hidden`. |

## 2. Rule: tests only

Add tests. Do not change `src/`, existing tests or fixtures. Expected values are literals or worked examples written in
the test, never recomputed with the logic under test. Each new test passes on the unchanged code (contract tests: they
cannot show a red phase on production code); their value is shown by the mutants of section 4.

## 3. Tests to add (names are suggestions; the developer reads the fixture helpers in `tests/confine.rs`)

1. `skipped_paths_are_sorted`: plan with `hide` of two missing absolute paths given in descending order
   (`/nonexistent-3az-zz`, then `/nonexistent-3az-aa`). Assert both are in `skipped_paths()` and, by index
   (`position`), that `-aa` comes before `-zz`. Other entries may sit between them; the test checks only their
   relative order.
2. `missing_container_socket_paths_are_reported_as_skipped`: with the fixture home, which has no `.docker` directory,
   a default request puts `<home>/.docker/desktop/docker.sock` and `<home>/.docker/run/docker.sock` in
   `skipped_paths()` (compare with `ends_with`, the other container entries depend on the host); with
   `allow_containers: true` neither is in `skipped_paths()`.
3. `write_limit_error_precedes_hide_limit_error`: a request with 65 `write` and 65 `hide` entries (the limit
   `MAX_LIST_ENTRIES` is 64) fails with an error that contains `confine_write has more than 64 entries`. Keep each
   entry well under 4096 bytes (the existing `path_lists_and_argv_sizes_are_capped` uses `format!("/tmp/path-{index}")`;
   reuse that style), so the count check is the first failure.
4. `unhide_removes_default_hidden_paths`: the fixture home has `.aws` (the existing unhide test uses it). Write the
   expected path as `std::fs::canonicalize(fixture.home.join(".aws"))`, because `plan` returns canonical paths
   (the tests at lines 625-735 do the same; the older test at 139 compares a raw path and only passes because the
   tempdir is already canonical here). A plan with no unhide has that path in both `hidden` and `default_hidden` (a
   sanity assertion, so the test would not pass for the wrong reason); a plan with `unhide: ["~/.aws"]` has it in
   neither.

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
  `CARGO_TARGET_DIR`, restore after each) and names the new test that fails on it: all four mutants must be killed.
  A test that does not kill its mutant is a fix request.
- The tester also reruns the three mutants the spec 3ay pass already killed in `release_operator_writes`, as a check
  that the new tests did not weaken the suite.
- Known flaky: `tests/daemon.rs::exited_claude_session_never_reports_needs_input` (backlog I9): rerun once, record
  as pre-existing if it then passes. The confine file passed 12 of 12 reruns on the architect's machine.

## 5. Out of scope

Production code, the two other tests of spec 3ay that already exist, renames, fixtures, and the equivalent mutant
(the `container_hidden.retain` in `unhide`, which cannot be observed while the refusal before it stays).

## 6. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `MAX_LIST_ENTRIES` is 64 and the error text is `{label} has more than {MAX_LIST_ENTRIES} entries` | `src/confine.rs:19`, `:185-187` | read |
| The two home-relative container entries are `~/.docker/desktop/docker.sock` and `~/.docker/run/docker.sock` | `src/confine.rs:59-60` | read (line corrected by the consultant, `m_4466`) |
| `add_optional_hide` records a missing path in `skipped` | `src/confine.rs:585-589` | read |
| The dry run prints `plan.skipped_paths()` | `src/main.rs:1106` | grep |
| The four mutants survived the spec 3ay pass: sorts, container skipped, entry-limit order, unhide default_hidden | tester report `m_4457` (mutants 8, 10, 12, 14) | tester's account; not re-run by the architect |
| The existing unhide test uses `~/.aws` | `tests/confine.rs`, `plan_skips_missing_and_unhides_existing_home_secrets_read_only` | read |
| The fixture home has `.aws` and `.ssh/config`, and `plan.default_hidden` and `plan.hidden` are public fields | `Fixture::new` (`tests/confine.rs:36-38`), `Plan` (`src/confine.rs:121-136`) | read |
| `~/.aws` is a default hide | `DEFAULT_HIDES` (`src/confine.rs:62-87`) | read |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness (model name not exposed to the architect);
developer and tester on the OMP harness, model `openai-codex/gpt-5.6-luna` (the advisor model was not exposed to
either). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Architect (Claude Sonnet 5.5) | The draft claimed the four mutants "survive on the code before spec 3ay too"; the tester ran them only after spec 3ay, so the claim was reasoned from identical lines, never run. A claims row also said "not yet read" for the fixture | Wording changed to say read, not run; the fixture and `DEFAULT_HIDES` were read before locking |
| Draft review | Consultant | Line anchor for the two home-relative container entries was 58-59, the lines are 59-60 | Corrected |
| Draft review | Consultant | Test 4 compared a raw `fixture.home.join(".aws")` path; `plan` returns canonical paths | Spec asks for `std::fs::canonicalize` |
| Draft review | Consultant | Test 1 should assert by position and test 3 should keep entries short | Both added to section 3 |
| Architect review | Architect | The tester's report called a 3.7G cargo `target` directory in `/tmp` "unrelated prior data" and left it; `/tmp` (a 3.9G tmpfs) was at 95% | Architect checked it and deleted it; tester assignments now name a cleanup list and require `df -h /tmp` and `ls -d /tmp/a2amx-*` in the report |
| Architect review | Architect, own mutants | Two mutants outside this spec's four behaviors survive: the list limit at exactly 64 (`>` to `>=`) and the order of the `hide` and `unhide` limit checks | Backlog F35, tests only; not a blocker |
