# Spec 3hf: the Sonar pre-push gate starts from an empty coverage directory and prints its stage times

## 0. Status

LOCKED after consultant review m_9297 (five findings applied) and its addendum m_9298 (one sentence in acceptance 4); no second review sent because no rule changed. Context `a2amx-3hf-prepush-sonar-speed`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Implemented by the architect under the tiny-change exception (one shell script, about ten lines; the owner chose to have this specified first and said to put it ahead of spec 3he); independent review by the consultant of this spec and of the diff, no developer or tester pass. Public seam: the script's own log output and exit code.

Scope (edit): `scripts/prepush-sonar.sh` only. Does not touch: `.github/workflows/ci.yml` (a clean checkout has no old coverage files; its steps stay identical to the script's), `scripts/prepush-fast.sh`, `Cargo.toml`, any test, any `src/` file, `docs/how-this-is-built.md`.

The spec records the owner's decisions of 2026-11-26 (direct, in the session): the Sonar gate is too slow (48 and about 50 minutes on the two runs of 2026-11-26), optimizing it goes ahead of spec 3he, and an automatic clearing step before each run is wanted ("yes!" to adding it). Addition the owner did not raise: the printed stage times (section 2, step 2), added so the next gate number is a measurement; it changes no behavior of the gate.

## 1. Why

`scripts/prepush-sonar.sh` runs the whole suite as a coverage run and then reads every raw coverage file (`.profraw`) the run left. `cargo llvm-cov --no-report` does not remove the files of earlier runs, so the directory grows without bound: on 2026-11-26 `target/llvm-cov-target` held 51,515 `.profraw` files (13.5 GB; by day 11-16 5,382, 11-17 1,369, 11-18 5,436, 11-20 4,003, 11-21 12,156, 11-22 20,180, 11-26 2,989 for the two runs of that day) in a 38 GB directory. Each run starts one `llvm-profdata merge` process per file and `cargo llvm-cov report` reads them all again. Stale files come from older binaries, so they also do not belong in the coverage Sonar sees. CI starts from a clean checkout and pays none of this.

File timestamps already show where the time went (consultant m_9297, re-checked by the architect with `find -printf` and `ls --full-iso`): run 2 of 2026-11-26 wrote its coverage files from 01:19 to 01:32, wrote `lcov.info` at 02:05:28 and ended its scan at 02:08, so the merge loop and the report took about 33 of its roughly 50 minutes; run 1 (files 00:15 to 00:29, scan upload at 01:02) shows the same shape. The build (5 minutes 37 seconds in run 1), the tests (about 13 minutes) and the scan (under 2 minutes) account for the rest. The expected effect of the fix is a total near 20 minutes; this is an inference from those timestamps, not a measurement of the new script. The spec therefore does two things: removes the cause, and makes the script print how long each stage takes so the next number is a measurement.

Alternatives rejected: deleting the whole `target/llvm-cov-target` (it holds the 26 GB instrumented build; a full rebuild per run costs the 5m37s and more, and the incremental cache is what makes later runs cheaper); `cargo llvm-cov clean --workspace` (it removes the workspace package's artifacts and forces that rebuild); rewriting the merge loop into one `llvm-profdata merge` over many files (with the old files gone the loop handles one run's files, about 1,500 each run on 2026-11-26; the stage time printed by this spec says whether it matters); changing CI (it has no stale files).

## 2. `scripts/prepush-sonar.sh`

Step 1. Before the test run (the line `A2AMX_REQUIRE_NODE=1 A2AMX_JS_LCOV=js.lcov cargo llvm-cov --locked --no-fail-fast --no-report || tests=$?`, currently `:32`), after `tests=0`, add with a why-comment:

```
# Raw coverage files of earlier runs come from older binaries and are never wanted; they made each
# run read tens of thousands of stale files. This keeps the instrumented build.
cargo llvm-cov clean --locked --profraw-only
```

Step 2. Print the stage times. After the `trap` line add a small function used at each stage end:

```
stage_start=$SECONDS
stage_done() { echo "stage $1: $((SECONDS - stage_start))s"; stage_start=$SECONDS; }
```

and call `stage_done build+tests` right after the test run line, `stage_done merge` after the `for` loop's `done`, `stage_done report` after the `cargo llvm-cov report` line. The label says `build+tests` because the stage also counts the scratch clone, the clean and the instrumented build, so it is not comparable with the 13-minute plain suite. The scan stage is the one already printed by the scanner (`Total time:` in its own output). Extend the existing EXIT trap so the total prints on every exit: `trap 'rm -rf "$scratch"; clean_profraw; echo "gate total: ${SECONDS}s"' EXIT`.

The test run's own exit code handling (`tests=$?` returned last) stays exactly as is.

## 3. Claims about the current tree

| Claim | Where | Verified how |
| --- | --- | --- |
| `cargo llvm-cov clean --profraw-only` exists in the installed version and the CI-pinned version | `cargo llvm-cov --version` prints 0.9.1; `.github/workflows/ci.yml:126` installs `--version 0.9.1`; `clean --help` lists `--profraw-only` | read and ran `--help` |
| It removes only `.profraw` files under `<target>/llvm-cov-target` and keeps other files | probe in `target/scratch/3hf` with `CARGO_TARGET_DIR`: two `.profraw` files gone, `keep.txt` and `debug/deps/lib.rlib` kept, exit 0 | probed (scratch deleted) |
| The merge loop and the report read every profraw file | `scripts/prepush-sonar.sh:36-39` | read |
| The directory held 51,515 `.profraw` files and 38 GB on 2026-11-26 | `find target/llvm-cov-target -maxdepth 1 -name '*.profraw'` and `du` | ran; the by-day counts come from `find -printf '%TY-%Tm-%Td'` |
| Root `./*.profraw` files are already removed by `clean_profraw` on entry and exit | `scripts/prepush-sonar.sh:16-18` | read |
| `lcov.info`, `js.lcov` and `target` are gitignored | `.gitignore:5,8,1` | `git check-ignore -v` |
| CI has the same loop and no clearing step | `.github/workflows/ci.yml:174-177` | read |
| No other file names `llvm-cov-target` or the merge loop | `rg llvm-cov-target` over the tree outside `target/` and `docs/specs/`: only the script | ran |
| `shellcheck` is installed | `/usr/bin/shellcheck` | ran `command -v` |

## 4. Out of scope

CI changes; the merge loop's per-file process model (upgrade trigger: a printed `stage merge:` above 2 minutes on a clean directory); the stale test executables in `target/llvm-cov-target/debug/deps` (133 executables, 695 entries) that `cargo llvm-cov report` may pass to `llvm-cov export` as objects, a suspect for a slow `stage report:` (unverified: the 0.9.1 source was not readable; trigger: a printed `stage report:` above 5 minutes); the test-suite duration (about 13 minutes without coverage) and the instrumented build; sccache, linker or profile changes; deleting `target/llvm-cov-target` itself; the other leftover logs in `target/` (`pf.*`, `ps.*`).

Trust boundary: none. A local script run by the owner's session; it takes no input from a less trusted source and creates no new attack path.

## 5. Acceptance

1. `bash -n scripts/prepush-sonar.sh` exits 0 and `shellcheck scripts/prepush-sonar.sh` has no new finding against the pre-change script (judge: the architect's run of both on the old and new file).
2. `rg -n 'clean --locked --profraw-only' scripts/prepush-sonar.sh` has one match, on a line before the line containing `cargo llvm-cov --locked --no-fail-fast --no-report` (judge: `rg -n` line numbers).
3. `git diff --name-only` lists `scripts/prepush-sonar.sh` and the spec only.
4. One complete gate run by the architect (the session with the `sonar` grant; the developer roles have none) on the committed change: the log contains the lines `stage build+tests:`, `stage merge:`, `stage report:` and `gate total:`; `find target/llvm-cov-target -maxdepth 1 -name '*.profraw' | wc -l` after the run is below 5,000 (it was 51,515); the Sonar gate result is the scanner's (PASSED or the real findings). The review record states `gate total` and each stage time against the earlier 48 and about 50 minutes. The spec makes no promise about the new total (about 20 minutes is the inference in section 1); if it is still above 40 minutes the numbers decide the next spec. The scan's coverage may come out lower than in the two earlier runs: old `.profraw` files whose function hashes still matched the current binaries may have added counts from other runs (inference, not verified). A coverage condition that fails on this first run is read against that before it is read as a regression.
5. No test is added or changed: the suite is not affected by this script. Contract: the full gate run in 4 is the only run.

## 6. Review

Consultant review of this draft before the lock; consultant review of the diff before the commit. Reviewer and implementer have different addresses.

## Amendment 1

The architect adopted the consultant's finding 5 (`--locked` on the clean command) after the review, so section 2 step 1 reads `cargo llvm-cov clean --locked --profraw-only`, but acceptance 2 still searched for the old text and could not match. Acceptance 2's pattern is now `clean --locked --profraw-only`. Nothing else changes. Found by the architect while implementing; checked with `rg -n 'profraw-only' docs/specs/spec-3hf-prepush-sonar-profraw.md` (every mention agrees).

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect). No developer or tester pass: the architect implemented the ten-line script change under the tiny-change exception and the consultant reviewed the spec and the diff.

BLOCKED/amend cycles: 0; one amendment (architect, found while implementing).

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The draft said the time split of the gate was unknown; file timestamps already showed merge and report took about 33 of 50 minutes (run 2: coverage files 01:19 to 01:32, `lcov.info` at 02:05:28, scan end 02:08) | Why section rewritten with the measurement and an expected total near 20 minutes |
| Draft review | consultant | The by-day counts in the Why section summed to 28,551, not 51,515 (four days missing) | Full by-day list put in |
| Draft review | consultant | The label `stage tests` also counted the clone, the clean and the instrumented build | Renamed `build+tests` and the reason stated |
| Draft review | consultant | Stale test executables in `debug/deps` may slow the report stage; `--locked` missing on the clean command | Named as a suspect with a trigger in Out of scope; `--locked` adopted |
| Draft review | consultant | The first Sonar scan after the clean may show lower coverage than before (stale counts from old runs) | One sentence added to acceptance 4 |
| Implementation | architect | Acceptance 2 still searched for `clean --profraw-only` after `--locked` was adopted, so it could not match | Amendment 1 (bae5f35) |
| After the push | architect | Memory 74ef6fa3 (2026-11-10) blamed the slow gate on stray profraw files in the repo root; they were in `target/llvm-cov-target` | Memory revised to its successor 1998ed81 |

Result of the acceptance run (2026-11-26, commit b5fc8d9, load average 3 to 4 on 4 cores because a second team was working on the machine): `stage build+tests: 1107s`, `stage merge: 41s`, `stage report: 48s`, scan 1 minute 44 seconds, `gate total: 1303s` (about 22 minutes) against 48 and about 50 minutes on the two earlier runs; raw coverage files after the run 1,474 (before 51,515); one truncated file dropped by the merge loop; the Sonar gate passed (bugs 0, smells 0, vulnerabilities 0, duplication 0.0; no coverage condition shown because the four new commits hold no coverable lines). The two suspects in section 4 are not triggered: `stage merge` 41s is under 2 minutes and `stage report` 48s is under 5 minutes.
