# Spec 3gh: session start-up and message admission leave daemon.rs (daemon S6b)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing creation, confinement, team, watch, codex, messaging, rate-limit and human-inbound tests (`tests/daemon/session_creation.rs`, `tests/team*`, `tests/confine/`, `tests/codex/`, `tests/messaging/`, `tests/human_inbound/`, `tests/gateway.rs`, `tests/broker/`); the moved items are private to `daemon`.

Scope (edit): `src/daemon.rs`, new `src/daemon/launch.rs`, new `src/daemon/admission.rs`.

Does not touch: every other file under `src/`, every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md` and the backlog when committing.

## 1. Why

`src/daemon.rs` is 2,001 lines and `impl Runtime` is 1,184 of them. Two responsibilities in it are large, cohesive and independent of the rest of `Runtime`'s state:

- **launch** (`:208-681`, `:1358-1377`, `:1960-1975`): everything that turns a `NewSession` or `Confine` request into a running session and then starts its background tasks: splitting a confine request, planning the confinement, validating the watch lists, the team scope and the team/name uniqueness, preparing a Codex session, spawning (the creation gate is held across the spawn on purpose, see `:280`), `start_session_tasks` with the `sample_activity` task it spawns, `accept_channel_dialog`, and `kill_session` (the other end of a session's life).
- **admission** (`:699-1014`): everything that decides whether a message enters the store: finding the recipient, `send_message` with its remote and rate-limit checks, `insert_message_rate_limited` (the `rate` tokio mutex is held across `store.insert_message` on purpose), and the human side (`human_inbound`, `fail_human_bound`, `next_eligible_human`, `human_outbound_next`, `human_outbound_done`).

Each moves as one child module holding one decision: how a session comes to exist (`launch`), how a message is admitted (`admission`). `Runtime` keeps its state; `authenticate`, `omp_usage_command`, `owner_cause`, `reset_*`, the receipts and the views stay for now. After S6b `daemon.rs` is about 1,190 lines; S6c (receipts and `serve`) takes it under 1,000.

Rejected: one `runtime_methods.rs` for all moved `Runtime` methods (a grab bag); splitting `create_session` (172 lines, a `Complex Method` candidate) in this spec (the move first keeps the diff mechanical; the follow-up is recorded in section 4); moving `reset_target`, `reset_audit`, `reset_session` with admission (reset is a session-control feature with its own audit and is moved with the views in a later spec if at all); moving `owner_cause` (serves views and admission both; stays).

## 2. Mechanical changes

### 2.1 Two new files

Move, byte for byte except as 2.2 says, in source order within each file, whole items with their doc comments, attributes and `// shortcut:` comments. Ranges below run from an item's first doc-comment line to its closing brace, at HEAD 12178a1 (blank separator lines are not part of an item; the moved items keep one blank line between them). The first line of each range is the doc comment when there is one: `plan_confinement` starts at its `///` lines (`:222`), not at `async fn`; `accept_channel_dialog` ends at `:1377` and the `///` lines at `:1379-1380` belong to `pub fn current_exe` and stay:

- `src/daemon/launch.rs`: inside one `impl Runtime` block `split_confine` (`:208-220`), `plan_confinement` (`:222-278`), `create_session` (`:280-450`), `validate_watch_lists` (`:452-471`), `validate_team_scope` (`:473-514`), `check_team_and_name` (`:516-550`), `prepare_codex` (`:552-603`), `start_session_tasks` (`:605-649`), `sample_activity` (`:651-681`); the free functions `accept_channel_dialog` (`:1358-1377`) and `kill_session` (`:1960-1975`); and the constant `ACTIVITY_POLL` (`:49`, used only by `sample_activity`), which becomes a private constant of `launch.rs` (`ACTIVITY_QUIET` stays: `views.rs` uses it).
- `src/daemon/admission.rs`: inside one `impl Runtime` block `recipient` (`:699-706`), `send_message` (`:708-831`), `insert_message_rate_limited` (`:833-857`), `human_inbound` (`:859-911`), `fail_human_bound` (`:913-920`), `next_eligible_human` (`:922-951`), `human_outbound_next` (`:953-977`), `human_outbound_done` (`:979-1014`). None of these items has a doc comment or attribute except `plan_confinement`'s doc.

Stay in `daemon.rs`: `omp_usage_command`, `authenticate`, `owner_cause`, `reset_target`, `reset_audit`, `reset_session`, the receipts and prompt methods, `message_status`, `message_info`, `serve` and the rest, `Daemon` and everything after.

### 2.2 Visibility, imports, call sites

- The `pub(super)` set is closed: `create_session` and `kill_session` in `launch.rs`; `send_message`, `human_inbound`, `human_outbound_next` and `human_outbound_done` in `admission.rs` (their callers are the `serve` dispatch at `:1847-1848`, `:1857`, `:1901`, `:1914`, `:1916-1917`). Everything else is private to its new file (`sample_activity` is called only by `start_session_tasks`, `recipient` only by `send_message` and `human_inbound`). No sibling module and no `Daemon::start` calls a moved item (`rg` over `src/daemon/` found nothing); if the compiler asks for another `pub(super)`, stop and report.
- The moved code reaches the parent's private items (`Runtime` fields: `sessions`, `store`, `boot`, `limits`, `rate`, `host_name`, `layout`, `bwrap_path`, `env_path`, `exe`, `presence`, `leases`, `omp_usage`, `deliveries`, `stop_*`; free functions `lookup`, `failed`, `parse_message_id`, `random_hex`, the method `owner_cause`; the sibling modules' `pub(super)` items such as `Sessions`' methods, `trust::session_id`, `views::{session_quota, session_activity}`, `remote_ineligible` and the notices producers `observe_exit`, `heartbeat`, `activity`): a child sees the parent's private items, and `pub(super)` items of siblings; each new file has `use super::{...}` for what it uses and no visibility change in the parent. No sibling's private item is touched (the compiler would refuse).
- Imports move with the code; names that no remaining `daemon.rs` code uses are removed from its `use` lines (`-D warnings` says which). Expected, from the consultant's scan: `HashSet`, `crate::codex`, `confine::{Host, Plan}`, `delivery::RecheckFn`, `session::{SessionId, SessionSpec}`, `store::{InsertError, NewMessage}`, `wire::HumanOutboundMessage`, `views::{session_activity, session_quota}` leave the parent (the children import `super::views::{...}` themselves); `confine` itself, `Size`, `AnyChannel`, `Deliver`, `Instant`, `Harness`, `Message` stay. A moved-code import the parent no longer needs is removed there, not re-exported.
- `daemon.rs` declares `mod admission; mod launch;` (alphabetical with the others).

### 2.3 `src/daemon.rs`

Delete the moved items. Nothing else changes. `src/daemon.rs` shrinks by about 810 lines (expected 2,001 to about 1,180).

## 3. Behaviour

No observable change. The deliberate locks held across awaits (the creation gate across `plan_confinement`, `codex::start` and the spawn; the `rate` mutex across `insert_message`) and their `// shortcut:` and why-comments move unchanged. Tracing targets of moved log events change to the child's module path (`a2amx::daemon::launch`, `a2amx::daemon::admission`); nothing filters on them.

## 4. Out of scope

- Any behaviour change; splitting `create_session` (172 lines) and `send_message` (125 lines) (follow-up, after S7: `Complex Method` candidates); the duplicated name lookups at `check_team_and_name`, `live_watched_peers`, `recipient` and `reset_target` (backlog F94: one `by_name` helper once they are uniform).
- `reset_*`, receipts, views, `serve` (S6c and later).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at the commit before this spec: 66 suites, 1415 passed, 0 failed, 3 ignored (architect states the commit before assigning); after: the same (no test added, removed or edited).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'fn (split_confine|plan_confinement|create_session|validate_watch_lists|validate_team_scope|check_team_and_name|prepare_codex|start_session_tasks|sample_activity|accept_channel_dialog|kill_session|recipient|send_message|insert_message_rate_limited|human_inbound|fail_human_bound|next_eligible_human|human_outbound_next|human_outbound_done)\b' src/daemon.rs` prints nothing; `rg -n 'insert_message\(|ACTIVITY_POLL' src/daemon.rs` prints nothing; `rg -c '^\s*(pub\(super\) )?(async )?fn (split_confine|plan_confinement|create_session|validate_watch_lists|validate_team_scope|check_team_and_name|prepare_codex|start_session_tasks|sample_activity|accept_channel_dialog|kill_session)\b' src/daemon/launch.rs` prints 11 and the same for `recipient|send_message|insert_message_rate_limited|human_inbound|fail_human_bound|next_eligible_human|human_outbound_next|human_outbound_done` in `src/daemon/admission.rs` prints 8; `rg -n 'pub\(super\)' src/daemon/launch.rs src/daemon/admission.rs` prints exactly the six items of 2.2 plus nothing else.
- `git status --short` lists only `src/daemon.rs`, `src/daemon/launch.rs`, `src/daemon/admission.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- Diff check (tester, mechanical): each moved item's body is byte-identical to its body at the base commit (`git show HEAD:src/daemon.rs`) apart from visibility, imports and `super::` paths; nothing the spec keeps moved.
- Mutation check (tester, checkout, one change at a time, restoring by checksum), naming the repo test that fails on each mutant; survivors reported, not fixed: the creation gate removed (`session_creation.rs` concurrent same-name test); `validate_watch_lists` skips the control-from check (`session_creation.rs:66,:150`); `check_team_and_name` accepts a duplicate name (`broker/sessions_and_naming.rs`); `plan_confinement` drops the Generic `preflight_env` call or the `without_overlays` fallback (its killer likely needs real bwrap: say what the test does when it skips); `prepare_codex` returns `Ok(Err(..))` before `codex::start` (`tests/codex/app_server_launch.rs`) and passes `authorize = true` always (`no_authorize_peers_leaves_the_operator_line_out`); `start_session_tasks` does not spawn the exit watcher (`tests/daemon_cli/peer_exit_watch.rs`); `kill_session` inverts `now` (expected survivor: every protocol kill in the tests uses `now: true`; report it); `send_message` passes an empty windows slice to `insert_message_rate_limited` (`rate_limits.rs`); `insert_message_rate_limited` records no timestamp (`rate_limits.rs:46`); `human_inbound` skips the lease check (`human_inbound/inbound_admission.rs`); `human_outbound_done` loses the `sent` branch (`outbound.rs:146-161`, `gateway.rs`); `next_eligible_human` skips the recheck (`outbound.rs:343`). The tester names the test per mutant.
- Known flaky tests: those that wait on timers (rate limits, codex start, heartbeat); a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| item ranges listed | `awk` outline of `src/daemon.rs` at the base commit | probed |
| the moved items have call sites only inside `daemon.rs` (tests use the protocol; `tests/codex/app_server_launch.rs:28` and `tests/messaging/limits_and_envelopes.rs:19` name `send_message`/`human_inbound` as protocol terms) | `rg -n` per name over `src/` and `tests/` | probed: `split_confine :281`, `plan_confinement :392`, `validate_watch_lists :318`, `validate_team_scope :349`, `check_team_and_name :372`, `prepare_codex :405`, `start_session_tasks :443`, `sample_activity :630`, `accept_channel_dialog :640`, `insert_message_rate_limited :763, :807, :895`, `fail_human_bound :938`, `next_eligible_human :954`, `human_outbound_* :1916-1917`, `send_message :1901`, `kill_session :1960` |
| `recipient`'s callers are `send_message` (`:777`) and `human_inbound` (`:869`), both moving, so it is private to `admission.rs`; `owner_cause` (`:683`, stays) is called by `send_message` (`:731`) and is reached as a parent private method | `rg -n 'recipient\(' src`, `rg -n 'owner_cause' src/daemon.rs` | probed |
| the creation gate and the `rate` mutex are held across awaits on purpose | comments at the gate and `insert_message_rate_limited` | read |
| the moved code reaches only parent items and siblings' `pub(super)` items | read of the moved bodies | read by the architect and the consultant (m_8523) |
| ranges are doc-comment-to-closing-brace; `current_exe`'s doc was in the first draft's `accept_channel_dialog` range | consultant read of `:1352-1392` | corrected |
| `kill_session`'s `now` only matters for a graceful non-Generic kill; tests kill with `now: true` | `src/session/exit.rs:14-19`, test greps | consultant read; mutant listed as an expected survivor |
| the move cannot add a Sonar issue: `src/daemon.rs` has no open issue on `develop`, so `create_session` and `send_message` are under the complexity limit (30); Sonar may count the moved bodies as new code, but a rule that did not fire in `daemon.rs` does not fire in the new files | `search_sonar_issues_in_projects` for `zbalint_a2amx:src/daemon.rs`, branch `develop`: 0 open issues | probed; if the gate still fails on a moved function, the architect reads the issue and decides, it is not the developer's to fix |
| `serve`'s call sites of the six `pub(super)` items | `rg -n` at HEAD | probed |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (the consultant's report names claude-opus-5-5 for itself); developer OMP openai-codex/gpt-5.6-luna; tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | `accept_channel_dialog`'s range swallowed `current_exe`'s doc comment (`:1379-1380`); start-to-next-start ranges included trailing blanks and a neighbour's doc | ranges rewritten doc-comment-to-brace; boundary rule stated |
| draft review | consultant | the `pub(super)` list named `sample_activity` and hedged; the closed set is six items | list closed in 2.2 |
| draft review | consultant | four mutants not killable as written (`plan_confinement` "ignores a grant", `prepare_codex` "skips the start", `send_message` rate-limit overlap, `kill_session` `now`) and a half-vacuous grep (`SYSTEM_SESSION`) | concrete mutants named; grep replaced by counts; `ACTIVITY_POLL` moved with its only user |
| draft review | consultant | Sonar may count moved bodies as new code | probed: no open issue on `daemon.rs` at develop; recorded in the claims table |
| tester pass | tester | the spec called the `kill_session` `now` inversion an expected survivor; eight `kill_and_down` tests kill it | spec claim was false; recorded here (code unchanged) |
| tester pass | tester | `without_overlays` fallback of `plan_confinement` has no daemon-level test (mutant M4b survives) | backlog follow-up |
| tester pass | tester | the agent-branch empty-windows mutant is killed by `broker::limits_and_lifecycle::rate_limit_does_not_cross_sender_sessions`, not `rate_limits.rs` | recorded |
| implementation | developer | first `cargo check` failed on a missing `HashSet` import in `launch.rs` | explicit import added before the report |
