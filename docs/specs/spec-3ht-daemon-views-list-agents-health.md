# Spec 3ht: function-level code health of `list_agents` in `src/daemon/views.rs`

## 0. Status

LOCKED. Context `a2amx-3ht-daemon-views-list-agents-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`Request::ListAgents` as admin and as a session), the MCP `list_agents` tool, the CLI.

Scope (edit): `src/daemon/views.rs`, only `list_agents` (`:68`) and the new functions it uses; the two Step A pins (team, running activity) appended to existing test files with few functions (candidates `tests/teams/team_visibility.rs`, `tests/broker/sessions_and_naming.rs`; the developer names the file after `cs check`), never a new file. Does not touch: any other function in `views.rs`, any other source or test file, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same replies and texts, same snapshot read once, same calls.

## 1. Why

`cs check` reports `:68` `list_agents` "Large Method (LoC = 71 lines)": the role-to-viewer resolution, the owner-state computation and the per-session `AgentSummary` literal with a visibility filter all sit in one function. After 3hs this is the only function finding left on `views.rs` (`cs check` prints score 9.60 and no other warning).

Rules that bind (earlier refactors): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs; no suppression.

## 2. Step A: pins (tests, green on the current code)

Existing pins, confirmed by the consultant's read (the developer reruns each and names it in the REPORT):

| Behaviour | Pin | Kills |
| --- | --- | --- |
| admin sees every session; a session sees itself, its visible party and not a hidden private team; `you` true only for the caller | `tests/teams/team_visibility.rs::private_team_visibility_hides_other_private_teams_but_not_ungrouped`, `::mutual_allow_is_required_for_private_team_message_visibility` | (a), (b) |
| `owner` present / away | `tests/presence/owner_presence.rs::interactive_presence_refuses_human_send_then_allows_it_after_grace`, `::attached_owner_presence_is_scoped_to_the_sending_session` | (c) |
| `owner` none without remote messaging | `tests/presence/away_notice.rs::away_notice_is_fixed_and_sent_once_to_remote_sessions_only` | (d) |
| `state` running / exited, address | `tests/broker/sessions_and_naming.rs::naming_addresses_and_role_expires_after_exit` | (e) |
| `attached` / `viewer` | `tests/presence/viewers.rs::viewer_attach_is_marked_in_session_and_agent_lists` | (f) |
| `quota` | `tests/quota/daemon_limits.rs::list_and_list_agents_report_a_claude_session_quota`, `::list_and_list_agents_report_an_omp_limit`; `tests/quota/omp_usage_daemon.rs::list_and_list_agents_report_omp_usage_percentages` | (g) |
| `harness`, `cwd` | `tests/broker/sessions_and_naming.rs::list_agents_reports_harness_and_spawn_directory` | (h) |
| `confine` | the admin ListAgents asserts in `tests/daemon/confine.rs` (~:167-216) | (j) confine dropped |

Unpinned, so the developer adds literal-expectation tests green on the unchanged code, appended to an existing test file with few functions (run `cs check` per file before and after): (1) `AgentSummary.team` is the team name for a team member and none otherwise; (2) `AgentSummary.activity` is `Some(..)` for a running session (today only the exited `None` is asserted for `ListAgents`). New mutants: (k) team dropped, (l) activity always none.

Not pinnable, kept as is: the `sender session is unavailable` refusal (views.rs:82) is reachable only by a race, because `session_has_exited` (serve.rs:189-198, 222-223) answers and closes before dispatch for a session role whose session is gone or exited. Mutant (i) is an expected survivor. The `Role::Gateway` arm (views.rs:72) is unreachable too (trust.rs:24 gives `ListAgents` to `AdminOrSession`; gateway_capability.rs:43 pins that) but stays: the match stays exhaustive.

## 3. Step B: target shape

### `src/daemon/views.rs`

- `fn owner_state(runtime: &Runtime, viewer: &Session) -> Option<String>`: `None` unless `viewer.remote().is_some()`; else `"present"` when `runtime.owner_cause(&viewer.id().0, Instant::now()).is_some()`, else `"away"`. Called as `viewer.as_deref().and_then(|viewer| owner_state(runtime, viewer))` (a `&Session`, so the caller adds no arm).
- `fn agent_summary(host_name: &str, session: &Arc<Session>, you: bool, omp_usage: Option<&BTreeMap<String, QuotaInfo>>) -> AgentSummary`: one session as one viewer sees it; today's literal; the counterpart of the `SessionSummary` literal in `list_sessions`. Four parameters, it does not depend on the viewer. `you` is computed in the `map` closure (`viewer.as_ref().is_some_and(|viewer| session.id() == viewer.id())`).
- `list_agents` keeps the role-to-viewer resolution, the snapshot, the unavailable-sender refusal (kept, unpinned), the visibility filter (inline) and the collect over `agent_summary`; `owner` through `owner_state`. The `Role` match stays exhaustive; if `cs` then reports Complex Method on `list_agents`, merging `Role::Admin | Role::Gateway => None` is allowed (behaviour-preserving), and nothing else.

If a finding remains, the developer quotes the cs text, reports per-function cc and stops. Hand counts: `owner_state` 3, `agent_summary` about 2, `list_agents` about 7-8 (role match, find closure, refusal `if` +2, filter `||` +2), each below 70 lines.

## 4. Out of scope

Every other function in `views.rs`; the wire types; `messaging::visible`.

## 5. Trust boundary

No boundary moves. The visibility filter (a session sees only its own party and visible parties) stays in `list_agents`, one place every `ListAgents` request passes through; the team_visibility pins show that a session does not see an invisible peer.

## 6. Acceptance

Baseline: after 3hs.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/views.rs
git status --short
```

- All pass; `cs check` lists no function finding on `views.rs` (the file-level finding may remain); whole-tree count below the baseline; new tests add no cs warning.
- `git status --short` lists only `src/daemon/views.rs` and the test file(s) named for the two new pins. No suppression.
- Mutation check by the tester only (scratch `target/scratch/3ht/`, own `CARGO_TARGET_DIR`, deleted before its REPORT): (a) the filter shows all sessions to a session; (b) `you` always false and always true; (c) `owner` always `away`; (d) `owner` set without remote; (e) the state inverted; (f) `viewer` taken from `attached`; (g) quota dropped; (h) cwd dropped; (j) `confine` dropped; (k) `team` dropped; (l) `activity` always none. (i), the unavailable-sender refusal dropped, is an expected survivor (not reachable through any seam, section 2). The tester names the killing test per mutant.

## 7. Claims

| claim | where | verified how |
| --- | --- | --- |
| `list_agents` at views.rs:68, cs "Large Method (LoC = 71 lines)" | `cs check src/daemon/views.rs` (tree with 3hs) | consultant ran it; score 9.60 |
| single caller | serve.rs:144 (import :17); MCP seam mcp.rs:325-328 | consultant grep |
| refusal unreachable except by race | serve.rs:189-198, 222-223 `session_has_exited` | consultant read; pinned guard in serve_protocol.rs `serve_exited_and_removed_sessions_close_before_parsing_requests` |
| gateway never reaches `list_agents` | trust.rs:24, tests/human_inbound/gateway_capability.rs:43 | consultant read |
| the pins in section 2 | named there | consultant read; developer reruns |
| no test asserts `team` or running `activity` via ListAgents | grep over tests/ | consultant grep (wire/messaging.rs is serde only; status_reporting.rs:107 uses `List`) |

## Review record

Models: architect Claude Sonnet 5.5 (Claude Code); consultant on the Claude Code harness (model not reported to the architect); developer `a2amx-junior-developer` on the omp harness, `openai-codex/gpt-5.6-luna`; tester `a2amx-tester` on the omp harness, `opencode-go/deepseek-v4.1-flash`.

BLOCKED/amend cycles: none.

| Found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | The "sender session is unavailable" refusal (views.rs:82) cannot be reached through any seam (`session_has_exited` answers and closes first), so Step A item 1 and mutant (i) were unpinnable; the `Role::Gateway` arm is unreachable but must stay | Section 2 narrowed; (i) an expected survivor; the match stays exhaustive |
| Draft review | consultant | The candidate pin list was unconfirmed; `AgentSummary.team` and a running session's `activity` had no pin over `ListAgents` | Confirmed pins listed by name; two new pins (team, activity) and mutants (k), (l) added |
| Draft review | consultant | `agent_summary(runtime, viewer, ..)` made the summary depend on the viewer; the hand count of `list_agents` (6) was low (7-8); a Complex Method fallback was undecided | `agent_summary(host_name, session, you, omp_usage)`; count 7-8; the `Admin \| Gateway` merge allowed if Complex Method appears; scope names the two test files |
| Implementation | developer | The first activity pin expected `Busy`; a fresh Generic session reports `Working` | Literal corrected; the pin passed on the old code first |
| Implementation | developer | clippy `question_mark` on `owner_state` | `viewer.remote()?` early return |
| Design read | consultant | `agent_summary` read `session_activity` last, the old code first; the `Working` assertion needed a why-comment (the helper's shell never enables bracketed paste, so activity is deterministic) | Activity read first again; why-comment added; blank lines restored (architect edits) |
| Tester pass | tester | Mutant (i) survived as expected; 11 of 12 killed; new pin 10/10 on the new source, 3/3 on HEAD | Recorded |

Tester mutants (12 plus (b) in two variants): all killed except (i). Fresh checks by the architect: `views.rs` 10.00 with no finding, whole-tree cs warnings 845 to 844, cargo test 1542 pass.
