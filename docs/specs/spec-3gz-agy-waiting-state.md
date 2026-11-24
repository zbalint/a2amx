# Spec 3gz: a `waiting` activity for an agy session with a running background job

## 0. Status

LOCKED after consultant review m_8838 (ten findings; nine applied, finding 2 rejected with evidence, see section 4). Amendment 1 (2026-11-24, after tester pass m_8918, developer follow-up 1 m_8926 and tester BLOCKED m_8929) at the commit that adds it: see section 6; it supersedes the text it names. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; HEAD at drafting 060b445; the governing version is the commit that adds this file. Context `a2amx-main-code-health-2026-11-22`. Owner priority: top (2026-11-23).

Owner decision (2026-11-23): a session whose turn ended while a background job runs is neither working nor idle; it shows as its own state, because the job's completion wakes the agent. agy only; other harnesses stay as they are (backlog item G17, their background-job signals are unread).

Scope (edit): `src/harness.rs`, `src/wire.rs`, `src/daemon/views.rs`, `src/status.rs`, `src/attach/picker.rs`; tests: `tests/messaging/agy_readiness.rs` (the matcher), one new test in `tests/broker/agy.rs` (the daemon), `tests/status.rs` (the status line), the wire test that serializes `Activity` (`tests/wire/visibility.rs:195` builds one; find or add the pin there); docs: `docs/cli.md` (`:55` the `explain` activity sentence, `:239`), `docs/architecture.md` (`:434`, `:535`, `:601`), `docs/harnesses.md` (the agy section), `docs/backlog.md` (item G17). Architect in the accepting commit: `CHANGELOG.md`, not in the developer's scope. Does not touch: `src/picker.rs` (spec 3gw owns it; the picker mapping below needs no change there), `src/daemon/notices.rs`, `src/table.rs` (both read the activity through `as_str` or `!= Idle` and need no edit), `Cargo.toml`.

This is a public-interface change: the control protocol's `Activity` gains `waiting`, shown by `a2amx list` (ACTIVITY), `a2amx explain`, the MCP `list_agents` result (`AgentSummary.activity`), the attach status line and the team line. A client and a daemon of different versions can fail to decode a status frame or list reply that holds `waiting` (`Activity` has no unknown-value fallback), so the changelog says to upgrade both together; a minor bump at release while the major version is 0.

Owner rules (memories d295229d, 828f8118, 7baeb937): no suppression comments, an agent never dismisses a finding.

## 1. Why

`session_activity_cause` (`src/daemon/views.rs:332`) reports `idle` when the screen is ready and unchanged for ten seconds (`ACTIVITY_QUIET`, `src/daemon.rs:40`), and `session_working` (`views.rs:320`) says "not working" on the same test. An agy session that ended its turn with a background job still running looks exactly like that. Measured on the owner's captures (2026-11-23, single job, two jobs, a 336-character command): the composer is `>` between two rule rows, the row above the top rule is blank, and below the bottom rule there is one row per job followed by one closing rule row:

    ● [21:48:54] sleep 60 && echo "60s task done" running
    ● [21:48:47] sleep 30 && echo "30s task done" running

(each row: two spaces, `● `, `[HH:MM:SS]` start time, the command, a space, `running`). The newest job is the top row, so rows are not in start order. A command longer than the screen is cut and ends with `...`, then ` running`; the row never wraps. When a job ends its row disappears and nothing is left; the woken agent's reply changes the screen. Observed 2026-11-23: the daemon sent "senior-developer: idle" while the developer's test run was still going.

The two classifiers are independent: `session_working` (activity notices, `notices.rs:334`) computes `!ready || unchanged < ACTIVITY_QUIET` from the screen and never reads `Activity`; `session_activity` (lists, status frames, heartbeat, `notices.rs:45`) comes from `session_activity_cause`. Both must learn the waiting state.

## 2. Changes (one step at a time; suite green after each)

0. Baseline: record `cargo test --no-fail-fast` counts, and `rg -n "Activity::" src tests` and `rg -n "idle.*working.*busy|working.*idle|\`idle\`" docs README.md` before editing.
1. `src/harness.rs`: `pub fn agy_background_job_running(screen: &Screen) -> bool`. Pure over the screen, next to `agy_turn_in_flight`. True when `agy_composer(screen)` finds the composer `(top, bottom)` and any row from `bottom + 1` to the last row of the screen matches, by cells (`cell_char`): columns 0 and 1 are `' '`, column 2 is `'●'` (U+25CF), column 3 `' '`, column 4 `'['`, columns 5 and 6 digits, column 7 `':'`, columns 8 and 9 digits, column 10 `':'`, columns 11 and 12 digits, column 13 `']'`, column 14 `' '`, and the last non-blank cells of the row spell ` running` (a blank before `running`). The suffix is read from the last non-blank cell backwards, so wide characters in the command text do not matter. The closing rule row cannot match. A short helper per concern is allowed (prefix, suffix); at most four parameters on every function. Add a `// shortcut:` comment: narrow and scrolled screens are not measured; a miss reads as `idle` (today's behavior), a false positive reads as `waiting` and never causes input; the trigger to revisit is a capture where ` running` is cut off or the row wraps.
2. `src/wire.rs`: `Activity::Waiting`, `as_str` gives `"waiting"`; the serde form is `waiting` (the enum is `snake_case`).
3. `src/daemon/views.rs`: one private predicate `fn background_job_waiting(session: &Arc<Session>) -> bool`: true only when the session's harness is agy and `harness::agy_background_job_running` holds on the session's current screen. It takes the state lock itself and is called only after any other state lock of the caller is released (the lock is not reentrant). Then: (a) `session_activity_cause`'s final branch becomes: unchanged less than `ACTIVITY_QUIET` gives `Working`; otherwise `background_job_waiting` gives `Waiting` with the cause text `background job running`; otherwise `Idle`. The order of the earlier branches (held, resetting, not ready, screen changed) does not change, so an unready screen or a screen that changed within ten seconds with a job row present stays `working`. (b) `session_working` returns `true` when `background_job_waiting` holds in addition to its present test (`!ready || unchanged < ACTIVITY_QUIET`), so the activity notices send no "idle" while a job runs and send it only after the row is gone and the debounce has passed. Do NOT derive `session_working` from `session_activity_cause` (that would count held and resetting sessions as working in the notices). Notice words stay `working` and `idle`; there is no third word.
4. `src/status.rs`: a new `DotState::Waiting`: `dot_state` maps `Activity::Waiting` to it, `activity_word` gives `"waiting"`, `activity_color` gives `"\x1b[38;5;141m"` (a violet not used by any other state). The held, starting and exited precedence is unchanged. `src/attach/picker.rs:174`: `Some(Activity::Waiting) => EntryState::Working` (deliberate: the picker's summary counts `working`; adding a picker state touches `src/picker.rs`, which spec 3gw owns; a `// shortcut:` comment names the upgrade trigger: a picker state of its own once 3gw is accepted).
5. Docs: add the word `waiting` where the three words are listed (the lines in the scope list), the colour 141 in `docs/architecture.md:434`, a sentence in `docs/architecture.md` (the heartbeat paragraph near `:535`) that a waiting peer counts as busy for the heartbeat digest (`!= Idle`) (amendment 1: nothing is said about a waiting recipient; `watcher_may_receive` gates on ready, not resetting, no hold and the quiet interval, not on activity, so a quiet waiting watcher still gets heartbeats), a sentence in `docs/cli.md:239` that IN-STATE restarts when the activity changes between working, waiting and idle, a `docs/harnesses.md` note under agy (what the row looks like and that a2amx reads it), and close backlog item G17 for agy (keep the other harnesses open in it).
6. Run `cs check` on the changed `.rs` files; report each new finding.

## 3. Acceptance

- `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; counts before and after.
- `git status --short` lists only the files of the scope list.
- Matcher tests in `tests/messaging/agy_readiness.rs` (the `agy_screen_from_size` seam; fixtures built from the captured rows, exact `assert_eq!` on the bool): one job row; two rows (newest on top); a long-command row ending `...` then ` running`; a row whose command text contains ` running` and ends `...` then ` running` (a match); no composer (false); the row ABOVE the composer (false: a transcript row `  ● [..] ... running` above the composer); `○` instead of `●` (false); missing brackets, `[12:34]` without seconds, `[12:34:5x]` (false); a row that ends `done` or `exit` (false); the closing rule row alone (false); the row on the last screen row below the composer when there is no closing rule row (true); a composer with nothing below (false).
- Daemon test in `tests/broker/agy.rs` (a `sh -c` session printing the captured screen, harness agy, as `agy_reset_target` does): with the job row present the list reports `Waiting` after waiting past `ACTIVITY_QUIET` (about eleven seconds, one test); a session with the same screen but no job row reports `Idle`; a job row on a screen that changed within the last seconds reports `Working` (print again, check at once). The activity-notice behavior ("no idle while the row is present") is covered by this daemon test through `session_working` only if a watcher can be set up with the existing helpers; otherwise say so in the report and cover `session_working` through the tester's mutants.
- `tests/status.rs`: the status line for a member with `Activity::Waiting` shows the dot in `38;5;141` and the word `waiting` (exact bytes, like the existing dot tests at `:233`, `:348`, `:377`); the wire test that serializes `Activity` pins `"waiting"`.
- Tester pass: mutants of every condition of the matcher (each column test, the suffix, the digits, the row range, the composer), of `background_job_waiting` (harness test, the screen test), of the final branch of `session_activity_cause` and of `session_working`'s new term, naming the killing test; equivalent mutants listed with a reason.

## 4. Review findings and dispositions (consultant m_8838)

1. Applied: `session_working` is a separate classifier (step 3b, shared predicate, no third notice word).
2. Rejected, settled: the consultant said `list_agents` carries no activity. `AgentSummary.activity: Option<Activity>` exists (`src/wire.rs:564`, set at `src/daemon/views.rs:111`) and the `list_agents` rows show `"activity":"idle"`; the spec keeps `list_agents` as a surface.
3. Applied: DotState and picker mapping decided (step 4); `tests/status.rs` added.
4. Applied: seams named in section 3; the eleven-second daemon test is accepted.
5. Applied: status updated; narrow and scrolled screens are closed by decision plus the shortcut comment.
6. Applied: exact columns and near misses in step 1 and section 3.
7. Applied: step 3 states the ready and quiet requirement.
8. Applied: wire note in section 0; the changelog line (architect) says client and daemon upgrade together.
9. Applied: step 5 documents the heartbeat and IN-STATE effects.
10. Applied: all the doc lines named in the scope list.

## 5. Flagged for the owner (not fixed here)

Other harnesses (Claude Code, OMP, Codex) have the same pattern of background work; this spec covers agy only. A dedicated picker state and the notice text (`waiting` as a third notice word) are left out on purpose.

## 6. Amendment 1 (2026-11-24)

Cause: the tester pass (m_8918) and the tester's BLOCKED (m_8929) found that section 2 step 5 demanded a false sentence, and that three acceptance texts of section 3 could not be met as written. The architect verified each against the code and the developer's diff.

1. Step 5 above: the clause "a waiting recipient receives no heartbeat" is deleted. It was false (`src/daemon/notices.rs:401` `watcher_may_receive` does not read the activity). The architect and the consultant (m_8838 finding 9) both wrote it; the developer's docs (`docs/architecture.md`) correctly omit it.
2. Section 0 scope list gains `tests/table.rs` and `tests/explain.rs` (a `Waiting` fixture each, which pins `as_str` and the cause text) and, the activity-notice test of the `session_working` term lives in `tests/broker/agy.rs` (already in scope), not in `tests/daemon_cli/`.
3. Section 3, matcher tests: `assert!` and `assert!(!..)` replace "exact `assert_eq!` on the bool" (clippy `bool_assert_comparison` rejects `assert_eq!(.., true)` and no suppression is allowed). Added cases: a non-digit in each of the columns 5, 6, 8, 9, 11 and 12; an empty command (`  ● [12:34:56] running`, the suffix starts at column 14: false) and a command of one space (`  ● [12:34:56]  running`, suffix at column 15: true).
4. Section 3, daemon test: the `Working` case may use a fresh session printing the row, checked at once, instead of printing again on the quiet session; both test the same classifier branch. Added: a generic-harness session with the same screen reports `Idle`; a watcher test (`working_after` and `idle_after` one second) in which an agy peer with the job row gets no `idle` notice while a control peer that redraws without the row does.
5. Section 3, status test: the `Waiting` case compares the full visible line exactly like the neighbouring cases.
6. The picker mapping stays without a test: `picker_entries` is private and no public seam reaches it; recorded as unpinned, not as a gap to close in this spec.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (strongest slot); developer antigravity-cli Gemini 4 Hydrogen (Medium); tester omp default slot (deepseek-v4.1-flash). BLOCKED/amend cycles: 1 (Amendment 1). Verification at acceptance: cargo fmt --check, clippy -D warnings and the full suite (1464 passed, 0 failed, 3 ignored) run fresh by the architect and the tester; `cs check` warning counts of the five production files equal to HEAD. Architect mutants (all killed, files restored by checksum): `.any` to `.all` in the matcher, `is_ascii_digit` to `is_alphanumeric`, status colour 141 to 75.

| Found at | By | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | ten findings (nine applied, finding 2 rejected with evidence: `list_agents` does carry the activity) | applied before locking; rejection settled |
| Draft review | consultant and architect | both wrote that a waiting recipient receives no heartbeat; `watcher_may_receive` does not read the activity | found by the tester (m_8918); Amendment 1 deleted the clause |
| Implementation | developer | the spec's `assert_eq!` on a bool is rejected by clippy `bool_assert_comparison`; the demanded "print again" Working case was replaced by a fresh session | spec defects; Amendment 1 changed both texts |
| Tester pass | tester | 38 mutants, 26 killed, 2 equivalent, 10 survivors: digit columns 5, 6, 8, 9, 11, the suffix start bound, the harness guard, the cause text, the `session_working` term, `Activity::as_str` | developer follow-up 1 (m_8926): near misses, boundary rows, generic-harness case, table and explain fixtures, watcher test; the recheck killed all but the picker mapping |
| Tester pass | tester | the picker mapping `Waiting` to `Working` has no public seam | recorded as unpinned in Amendment 1 section 6.6 |
| Tester pass | tester | the first tester assignment's mutant run aborted before restore under `errexit` | restored by checksum; lesson stored |
| Tester recheck | tester | BLOCKED m_8929: the locked step 5 demanded the false heartbeat clause the follow-up removed | Amendment 1 (0762def) |
| Tester recheck | tester | `docs/architecture.md:551` still said working means only not-ready or changed within ten seconds | architect added the agy clause at acceptance |
| Amendment 1 | tester | amendment named `tests/daemon_cli/` for the watcher test, which lives in `tests/broker/agy.rs` | architect corrected the text at acceptance |
