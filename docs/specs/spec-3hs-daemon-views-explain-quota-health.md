# Spec 3hs: function-level code health of `explain_of` and `session_quota` in `src/daemon/views.rs`

## 0. Status

LOCKED. Context `a2amx-3hs-daemon-views-explain-quota-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD at drafting `27d3485`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`Request::Explain`, `Request::ListAgents`), the CLI over it (`tests/explain.rs`), the quota tests under `tests/quota/`.

Scope (edit): `src/daemon/views.rs`, only `explain_of` (`:156`), `session_quota` (`:275`) and the new functions they use; tests only where Step A finds an unpinned behaviour, appended to existing files named there, never a new file. Does not touch: `list_agents` (`:68`, a later spec), `list_sessions`, the activity helpers, any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same replies, same texts, same lock scopes and calls.

## 1. Why

`cs check` reports on `views.rs`: `:156` `explain_of` "Large Method (LoC = 97 lines)" (a seven-element tuple built by an exited branch and a running branch, with the hold chain and the reset verdict inline); `:275` `session_quota` "Complex Method (cc = 9)" (the Claude/Agy report merge and the OMP usage merge in one function). The file-level "Overall Code Complexity" and `:68` `list_agents` remain after this spec.

Rules that bind (earlier refactors 3hk-3hr): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs so order stays; no suppression.

## 2. Step A: pin what is unpinned (tests, green on the current code)

The developer first lists, per behaviour below, the test that already pins it and runs it. Candidates by grep: `tests/explain.rs` (`ready_generic_explain_reports_state_and_open_reset_gate`, `deliver_hold_explain_reports_hold_explanation_without_gating_reset`, `non_ready_generic_explain_reports_screen_readiness_gate`, `non_ready_claude_explain_reports_starting_latch`, `claude_explain_reports_needs_input_reason`, `exited_explain_reports_only_the_exited_state_line`, `unknown_explain_reference_is_rejected_by_the_daemon`, `claude_native_explain_reports_hold_and_stale_reset_gate`, `claude_native_explain_keeps_nonempty_composer_held`, `agy_background_job_explain_reports_waiting_and_cause`, `stale_omp_draft_preview_does_not_clear_the_hold`), `tests/quota/quota_info.rs`, `tests/quota/daemon_limits.rs`, `tests/quota/omp_usage_daemon.rs`, `tests/status.rs`, `tests/statusline.rs`. A behaviour with no pin gets a new small test with literal expectations (whole field values), green on the unchanged code, appended to an existing file with few functions; check `cs check` on that file before and after (a long test is a new Large Method).

1. `Explain` of an unknown session reference replies `Error "unknown session <id>"`; a store failure reading the open counts replies `Error` with the error text, before the lookup.
2. An exited session: `pending` 0, `exit_code` set, `ready` none, `starting` false, no activity, cause, needs-input or hold, and a closed reset verdict with no code or reason.
3. A running session: `pending` is the session's open count; `activity` and `activity_cause` from `session_activity_cause`; `needs_input`; the hold in this priority: `deliver: hold` gives `deliver_hold` with its explanation; else the channel's hold reason with its explanation; else the session's own hold with `hold_name` and no explanation; none otherwise; `ready` from the harness readiness of the screen; `starting`; the reset verdict open when `reset_gate_preview` is none, else closed with its code and reason.
4. `session_quota`: none for an exited session or a harness other than Claude, Codex, OMP, Agy; for Claude and Agy the screen quota merged with the reported windows, none when neither window is known; for Codex the screen quota; for OMP the provider usage with `limit_reached` OR-ed from the screen, `limit_reached` alone when there is no provider usage, none when neither.

Gaps the consultant found; each gets a pin unless noted:

5. A PTY channel hold with its explanation, and the `deliver_hold` priority over it (kills mutants (b) and (d)): the PTY channel hold arm is reachable only for Generic sessions (native and Codex `hold_reason` return none, `delivery.rs:144,200,272`), so every Claude and OMP explain test reaches the session-hold arm. New test in `tests/explain.rs` or `tests/daemon/status_reporting.rs` (the recipe is at `status_reporting.rs:542-567`: a Generic session with `Deliver::Auto`, attach, send input `x`, wait for `human_draft`): `Request::Explain` gives `hold == HoldInfo { reason: "human_draft", explanation: Some(<the human_draft text of messaging.rs>) }`; a second Generic session with `Deliver::Hold` and the same input gives reason `deliver_hold`.
6. The exited session at the wire level (kills mutants (g) and (h) for the reset and the closed fields; the CLI prints only the exited line): `Request::Explain` on an exited Generic session asserts the whole `ExplainInfo` field values of item 2 (`exit_code` `Some(0)`, `ready` none, `starting` false, no activity, cause, needs-input or hold, `pending` 0, `reset` closed with no code or reason). New test appended to a file with few functions.
7. `session_quota` of an exited Claude session is none (kills (n)): a Claude session running `printf '\033[24;1H5h 0%% left 7d 80%% left'; exit 0`; wait for it to exit; `list` shows `quota` none. New test in `tests/quota/daemon_limits.rs` (270 lines, 7 functions, reuses `status_line_session`).
8. OMP provider usage with the screen's `limit_reached` (kills (l)): the developer checks whether `direct_omp_session` in `tests/quota/omp_usage_daemon.rs` can take a screen script; if a pin fits in a small test, add it; if not, report mutant (l) as a coverage gap (no new test hook, no large fixture).

The Codex arm of `session_quota` has no daemon-level pin (a Codex session needs an app-server): it stays a structurally identical `_ => screen` arm and is not pinned. A store failure reading the open counts is unreachable at the public seam. An unreachable branch is reported, not pinned; no test hook is added.

## 3. Step B: target shape

The design is the instruction; the `cs` result is the check.

### `src/daemon/views.rs`

- `fn explain_hold(session: &Arc<Session>) -> Option<HoldInfo>`: today's three-way hold chain of the running branch (`deliver_hold`, the channel hold, the session hold), same reason strings and explanations, same order.
- `fn reset_verdict(session: &Session) -> ResetVerdict`: (one caller; it names the gate-preview to wire-verdict job and is 12 lines; it is not needed for the line count) today's `reset_gate_preview` mapping (open when none, closed with code and reason).
- `explain_of` keeps the counts read (`Error` before the lookup), the lookup and the unknown-session error, and builds the reply in one pass without the tuple or a new struct: first an `ExplainInfo` literal holding the exited values (id, address, harness, `exit_code` from the session; `ready` none, `starting` false, no activity, cause, needs-input or hold, `pending` 0, `reset` closed with no code or reason; `ExplainInfo` has no `Default` and adding one is out of scope), then one `if exit_code.is_none() { .. }` that overwrites `pending` (from the counts), `activity` and `activity_cause` (from `session_activity_cause`), `needs_input`, `hold` (`explain_hold`), `starting`, `ready` (one `session.lock()` for the readiness read, as today) and `reset` (`reset_verdict`), in that order, each field assigned once. Moving `id`, `address` and `harness` ahead of the running reads is not observable: they are plain field reads. A per-field-group `if exit_code.is_some()` variant is not allowed (it raises cc to about 9).
- `fn reported_quota(session: &Session, screen: Option<QuotaInfo>) -> Option<QuotaInfo>`: the Claude/Agy merge (screen quota or default, the reported five-hour and weekly windows overlaid, none unless a window is known), with today's `store::now()` call.
- `fn omp_quota(session: &Session, omp_usage: Option<&BTreeMap<String, QuotaInfo>>, screen: Option<QuotaInfo>) -> Option<QuotaInfo>`: the OMP provider usage merge with `limit_reached` (today's `match (omp_quota, limit_reached)`).
- `session_quota` keeps the exit and harness guard before the screen read (other harnesses never lock or parse), then `match harness { Claude | Agy => reported_quota(..), Omp => omp_quota(..), _ => screen }` (Codex returns the screen quota as today). In `reported_quota`, `store::now()` and `quota_report` stay after the screen lock has dropped. `explain_hold` keeps the session-hold arm's separate `session.lock().hold` read; it is not folded into the channel read.

If a finding remains, the developer quotes the cs text, reports the per-function cc and stops; no extra split without the architect. Hand counts (cc, +1 per `if`/`if let`/`let else`/`&&`/`||`/`match`, an operator inside a closure counts, `?` free): `explain_hold` 3, `reset_verdict` 1 (a `map_or_else`), `reported_quota` 4, `omp_quota` 2, `session_quota` 4, `explain_of` about 4 and about 53 lines.

## 4. Out of scope

`list_agents`, `list_sessions`, `session_working`, `session_activity`, the activity helpers, `message_status` and `message_info`; the wire types; the quota parser; the CLI.

## 5. Trust boundary

No boundary is created or moved. `Explain` is admin-only (`daemon/trust.rs`; `session_token_cannot_explain` pins it) and stays so; the refactor reads the same session state under the same locks.

## 6. Acceptance

Baseline (`27d3485`): `cargo test` passes (1537 passed, 3 ignored); whole-tree cs count 848.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/views.rs
git status --short
```

- All pass; `cs check` lists no finding on `explain_of` and `session_quota` (the `:68` and file-level findings remain, out of scope); the whole-tree count is below 848 (judge: the architect's `cs` run and `scripts/prepush-fast.sh`'s `cs delta`); new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression, no `#[allow]`.
- Step A tests pass on the unchanged source and on the diff (the tester reruns them on `git show HEAD:` sources).
- Mutation check, by the tester only, in a scratch snapshot with its own `CARGO_TARGET_DIR` under `target/scratch/3hs/` (deleted before its REPORT); per mutant run `cargo test --lib` and the targets `explain`, `quota`, `status`, `statusline`, `broker`, `delivery`, `bridge`, `claude_channel`, `codex`, `human_inbound`, `wire`, `daemon`, `table`, `omp_smoke`. Mutants, one change each: (a) `explain_hold` drops the `deliver_hold` arm; (b) swaps `deliver_hold` and the channel hold; (c) drops the session-hold arm; (d) drops the explanation of the channel hold; (e) `reset_verdict` inverts allowed; (f) drops the reason; (g) the exited branch reports a pending count (expected non-kill: an exited session's open messages are failed on exit, so its count is already 0); (h) the exited branch reports a reset verdict allowed; (i) `reported_quota` drops the weekly overlay; (j) drops the five-hour overlay; (k) returns the quota even with no known window; (l) `omp_quota` drops the OR of `limit_reached` (killed only if item 8's pin fits, else a reported coverage gap); (m) returns none for `(None, true)`; (n) `session_quota` returns quota for an exited session. The tester names the killing test per mutant; an expected non-kill is reported with its reason; any other survivor in moved code is a fix request.

## 7. Claims (architect, drafting)

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `views.rs:156,275` | `cs check` run, output read |
| `explain_of` has one caller (`serve.rs:143`); `session_quota` has four callers (`views.rs:30,127`, `attach.rs:105`, `admission.rs:129`), its signature stays | grep over `src/` | run, output read |
| the hold chain and the quota rules above are today's behaviour | `views.rs:156-252,275-319` | read |
| `Explain` is `Audience::AdminOnly` | `daemon/trust.rs:37-42` | read |
