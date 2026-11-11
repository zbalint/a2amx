# Spec 3dz: make `messages_cli_prints_submitted_and_undeliverable_rows` independent of the delivery cooldown

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-flaky-messages-cli-2026-11-11`. Governing contract: this spec plus the owner's request of 2026-11-11 ("can we fix that flaky test?"); backlog row F88 names the test class. No grilling record beyond that.

Scope (files that may be edited): `tests/attach_cli.rs`, only the test `messages_cli_prints_submitted_and_undeliverable_rows` (currently lines 1377-1481) and, if one is needed, a helper next to it. Also `docs/backlog.md`: the F88 row (the architect edits it in the accepting commit, not the developer).

Does not touch: any file under `src/`, any other test, `tests/common`, `CHANGELOG.md` (tests only, nothing user-visible), `AGENTS.md`.

Public test seams: the daemon's control API through `Client`, the `a2amx messages` binary through `run_binary`, the `a2amx new` command through `run_cli`. No private functions.

## 1. Why

The test failed once in CI on the 0.3.0 release commit 6240772, at the assertion that the `a2amx messages` output starts with the fixed header line (`tests/attach_cli.rs:1470`), passed on one rerun and on 40 solo local runs. The failed log is no longer retrievable, so the cause below is a diagnosis from the code, not an observed failure; section 5 proves that the old test depends on the clock, it does not reproduce the header mismatch itself.

Diagnosis. The test sends m_1 to a recipient with `--deliver auto`, waits until m_1 is `submitted`, sends m_2 and m_3, then kills the recipient and expects m_2 and m_3 to become `undeliverable` with detail `recipient_exited`. After a submit the delivery loop waits `COOLDOWN` (1 second, `src/messaging.rs:50`, checked in `src/delivery.rs:58-59,136,152`) before it delivers the next message, and a delivery then pastes, sleeps `PASTE_GAP` (400 ms, `src/messaging.rs:49`) and presses Enter (`src/session.rs:1372-1394`). While the cooldown runs, `begin()` returns nothing and the loop wakes only on a new message, an output wake or its 1-second interval tick (`src/delivery.rs:136-137,399,465-469`); the child prints nothing, so the next delivery try is the first tick after the cooldown, between 1 and 2 seconds after m_1's submit. The test is correct only when the kill lands before that. On a loaded runner the polling and sends take longer, the daemon starts delivering m_2, and the kill arrives during that delivery: the attempt then finishes as undeliverable with another detail (`recipient exited during delivery`, `src/session.rs:1400-1402`, or `session input is closed`, `src/session.rs:1375,1386`), or m_2 is submitted. The `messages` table sizes each column to its longest cell (`format_table`, `src/main.rs:3554`, called by `format_messages_table` at 3663; `format_row` at 3666), so a longer detail text widens the DETAIL column and the fixed header string no longer matches. This explains a failure at the header assertion and a pass in 40 solo runs.

Rejected: lengthening or tuning timeouts (the race stays); accepting either detail text (it weakens the test and still leaves the `submitted` outcome for m_2); changing `COOLDOWN` or the delivery code (not a product defect, the behaviour is right, the test depends on the clock).

## 2. Mechanical changes

2.1. In `tests/attach_cli.rs`, rewrite the body of `messages_cli_prints_submitted_and_undeliverable_rows` so that no assertion depends on how long the steps take. The shape:
- m_1 goes to a recipient named `agent-review` started with `--deliver auto` as today; the test waits for m_1 `submitted` as today.
- m_2 and m_3 go to a second recipient that never delivers: a session started with `--deliver hold` so they stay pending until the kill. The `SendMessage` replies to a hold recipient carry `recipient_hold: Some("deliver_hold")` (`src/daemon.rs:1400-1402`); the test asserts that literal.
- the second recipient is started with `common::new_agent(&mut admin, dir.path(), Some("agent-hold"), Harness::Generic, Deliver::Hold)` (tests/common/mod.rs:392-399), which returns its session id (the child there is `sleep 30`, long enough); the test asserts the id is `s3` (sender `s1`, `agent-review` `s2`) and kills it with `Request::Kill { now: true }`; then the test waits for m_2 and m_3 `undeliverable` as today.
- the expected `messages` output keeps the header and the three rows, with the TO cell of m_2 and m_3 being `agent-hold@host-a`, padded to the TO column width (19, from `agent-review@host-a`), and DETAIL `recipient_exited` for both. The expected lines, as literals (17 characters plus 2 pad plus the 2-space separator, so four spaces before `undeliverable`): `"m_2  agent-plan@host-a  agent-hold@host-a    undeliverable  recipient_exited  Second\n"` and the same for m_3 ending `"Trailing \n"`. The header and the m_1 line stay byte-identical to today's.
Everything else in the test (the sender, the `Subject` strings including the trailing space of `Trailing `, the `messages` binary call) stays.

2.2. Nothing else changes. If the developer finds that the second recipient cannot be built as described (for example the `hold` recipient does not reach `undeliverable` on kill), stop and reply `BLOCKED — SPEC ADJUDICATION REQUIRED` with the evidence; do not fall back to a sleep or a retry.

## 3. Out of scope

The other F88 tests (`team_line...`, `status_toggle_reattaches_to_resize_the_session`), `picker_groups_sessions_into_team_sections`, any change to delivery timing, and a general "wait for settled output" helper.

## 4. Trust-boundary note

Test-only change; no input path.

## 5. Acceptance

Baseline at draft time on the unchanged tree: the old test passes when run alone. Run commands as an ordinary user, one test at a time, no `cargo test` of the full suite in a loop (it has frozen this machine): `cargo test --test attach_cli messages_cli_prints_submitted_and_undeliverable_rows`.

1. Clock dependence of the old test (tester, architect): on a scratch change in the checkout (restored by checksum afterwards, no second copy), add `tokio::time::sleep(std::time::Duration::from_millis(3000)).await;` immediately before the `assert_eq!` that contains `Request::Kill` in the OLD test (3000 ms, because delivery of m_2 can start up to about 2 seconds after m_1's submit). Expected: the test fails deterministically, with m_2 `submitted` and the wait for `undeliverable` timing out after `WAIT`. Record the output. This is the red phase. Optionally, record what a few runs with a 1100 to 1300 ms sleep show (header mismatch or timeout); that is observation, not a gate.
2. The same 3000 ms sleep injected before the kill in the NEW test: it passes. This proves the new test is independent of the cooldown.
3. The new test passes ten times in a row without the injected sleep (`( for i in $(seq 10); do cargo test --test attach_cli messages_cli_prints_submitted_and_undeliverable_rows -- --exact || { echo "failed on run $i"; exit 1; }; done )`; the first run builds once).
4. `cargo test --test attach_cli`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass with no warnings (the whole attach_cli suite once, not in a loop).
5. `git diff --stat` shows only `tests/attach_cli.rs`; the diff is limited to the one test and a helper if added; no deleted assertion without an equivalent (the header, the three rows, `submitted`, `undeliverable`, `recipient_exited`).
6. Scratch: any scratch file goes under `target/scratch/3dz/` and is deleted before reporting; no extra `CARGO_TARGET_DIR`.

## Review record

Models: architect Claude Sonnet 5.5 (claude-sonnet-5-5); consultant on the claude harness (model not recorded); developer on OMP, working slot openai-codex/gpt-5.6-luna (advisor slot not exposed); no tester pass (a small test-only change; the architect reran the green check with the injected sleep and the single test, clippy and fmt). BLOCKED/amend cycles: 0.

| Where found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The red check's 1500 ms sleep is not deterministic: delivery of m_2 starts at the first 1 s interval tick after the cooldown, 1 to 2 s after the submit | Sleep raised to 3000 ms; section 1 states the window |
| Draft review | consultant | Section 1 said the check reproduces the header mismatch; it proves clock dependence | Reworded |
| Draft review | consultant | A `break` in the acceptance loop exits 0 on a failed run; the session id choice was left open; wrong citations for `format_table`; literals not given | Loop exits 1, `new_agent` and `s3`, citations fixed, literals written |
| Implementation | developer | The red run showed a timeout, not the intermediate m_2 state | Accepted; the spec's expected result was a timeout |
