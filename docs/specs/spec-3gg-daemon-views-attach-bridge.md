# Spec 3gg: session views, the attach stream and the bridge leave daemon.rs (daemon S6a)

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the daemon protocol, through the existing list, agents, explain, screen, quota, attach and bridge tests (`tests/refs_and_list.rs`, `tests/explain.rs`, `tests/quota/`, `tests/attach_*`, `tests/bridge.rs`, `tests/broker/`, `tests/daemon/`); the moved items are private to `daemon`.

Scope (edit): `src/daemon.rs`, `src/daemon/notices.rs` (one `use` line), new `src/daemon/views.rs`, `src/daemon/attach.rs`, `src/daemon/bridge.rs`.

Does not touch: every other file under `src/`, every test, `Cargo.toml`, `Cargo.lock`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates `AGENTS.md` and the backlog when committing.

## 1. Why

`src/daemon.rs` is 2,812 lines, and the owner's goal is no `.rs` file over 1,000 lines through a real design, not a split by size. About 830 lines of it are request handlers with no mutable daemon state of their own, in three families that each hold one responsibility and are currently interleaved with the daemon's lifecycle code:

- **views** (`:1953-2207`, `:2235-2273`, `:2366-2459`): how a session or a team is presented to a caller: `list_sessions`, `list_agents`, `screen_of`, `explain_of`, `list_messages`, and the derived per-session facts every view needs (`session_quota`, `session_working`, `session_activity_cause`, `session_activity`). The derived facts are also used by `Runtime` methods that stay for now (`sample_activity`, `send_message` and, in `notices.rs`, the activity code).
- **attach** (`:2469-2812`): the interactive stream of one session over a control connection: `attach`, its `AttachmentGuard`, `attachment_status` and `send_data`.
- **bridge** (`:2208-2234`, `:2274-2365`): the OMP bridge relay endpoint: `bridge_attach` and `bridge`.

Each family is a free-function group over `&Runtime` or `&Arc<Runtime>` that reaches `Runtime`'s private fields only through methods and fields a child module can already see. They move as three child modules; `Runtime` keeps its state and its lifecycle, which are the subject of S6b, S6c and S7. The module names state the responsibility: `views` (read-only presentation and the derived facts), `attach` (the attach stream), `bridge` (the bridge relay).

Rejected: one `handlers.rs` for everything (a grab bag of unrelated responsibilities, the opposite of what the owner asked for); moving `kill_session` and `cancel_message` into `views` (they mutate; they go with the launch/session code (S6b) and the message code (S6b/S6c)); splitting `attach` (241 lines, CodeScene "Complex Method") in this spec: moving it first keeps the diff mechanical and gives the later split a file of its own; the follow-up is recorded in section 4; moving `lookup` (used by every family and by `Runtime` methods; it stays with `Runtime`, whose session map it reads).

## 2. Mechanical changes

### 2.1 Three new files

Move, byte for byte except as 2.2 says, in source order within each file:

- `src/daemon/views.rs`: `list_sessions` (`:1953-2003`), `list_agents` (`:2004-2075`), `screen_of` (`:2093-2109`), `explain_of` (`:2110-2207`), `list_messages` (`:2235-2256`), the doc comment and `session_quota` (`:2366-2412`), `session_working` (`:2413-2424`), `session_activity_cause` (`:2425-2455`), `session_activity` (`:2456-2459`).
- `src/daemon/attach.rs`: `AttachmentGuard` and its `Drop` (`:2469-2505`), `attachment_status` (`:2506-2564`), `attach` (`:2565-2806`, from its `#[allow(clippy::too_many_arguments)]` line), `send_data` (`:2807-2812`).
- `src/daemon/bridge.rs`: `bridge_attach` (`:2208-2234`) and `bridge` (`:2274-2365`).

`kill_session` (`:2076-2092`) and `cancel_message` (`:2257-2273`) stay where they are; so do `lookup` (`:2460-2468`), `response` (`:1767`), `serve` and everything else.

### 2.2 Visibility, imports, call sites

- Items that code outside the new file still calls become `pub(super)`: from `views.rs` `list_sessions`, `list_agents`, `screen_of`, `explain_of`, `list_messages` (called by `serve` at `:1829, :1845, :1846, :1882, :1914`), `session_quota` (`:813, :1966, :2063, :2557`), `session_working` (`notices.rs:333`), `session_activity_cause` (`:2146`, moved with `explain_of` into the same file: stays private), `session_activity` (`:674`, `:1960`, `:2047`, `:2524`, `:2546`, `notices.rs:44`); from `attach.rs`: `attach` (`:1861`); from `bridge.rs`: `bridge_attach` (`:1900`). Everything else stays private to its new file (`AttachmentGuard`, `attachment_status`, `send_data`, `bridge`).
- Calls between the families (definite): `attach.rs` calls `views::session_activity` (`:2524`, `:2546`) and `views::session_quota` (`:2557`) and so needs `use super::views::{session_activity, session_quota};`; `bridge.rs` calls no other family (it uses the parent's `lookup`, `response`, `trust::session_id` and `record_receipt`); `views.rs` calls neither `attach` nor `bridge`.
- The moved code calls items of the parent that stay (`Runtime` and its fields and methods, `Role`, `lookup`, `response`, `session_id` via `trust`, `record_receipt`, `owner_cause`, `message_info`, `ACTIVITY_QUIET` and others): a child module sees the parent's private items, so each new file has `use super::{...}` for what it uses and no visibility change in the parent. Imports move with the code.
- `daemon.rs` declares `mod attach; mod bridge; mod views;` (alphabetical with the others) and `use` lines for the `pub(super)` items it still calls; the `use` names that no remaining code uses are removed (`-D warnings` says which; the consultant counted `BTreeMap`, `tokio::io::AsyncWriteExt`, `NativeGuard`, `AttachmentSlot`, `BRIDGE_PROTOCOL`, `BridgeDown`, `BridgeUp`, `ClientFrame`, `ExplainInfo`, `HoldInfo`, `ResetVerdict`, `ServerFrame`, `SessionSummary`, `StatusInfo`, `TeamMember`, `AgentSummary`, `QuotaInfo`, `Activity` and `crate::quota` as the likely ones; `attach.rs` needs `AsyncWriteExt` for `stream.shutdown()`).
- `src/daemon/notices.rs:14`: `use super::{Runtime, session_activity, session_working};` becomes `use super::views::{session_activity, session_working}; use super::Runtime;` (one `use` change, as `cargo fmt` orders it; nothing else in the file changes).

### 2.3 `src/daemon.rs`

Delete the moved items. Nothing else changes. `src/daemon.rs` shrinks by about 830 lines (expected 2,812 to about 1,980).

## 3. Behaviour

No observable change. Tracing targets of moved log events change to the child's module path (`a2amx::daemon::attach`, `a2amx::daemon::bridge`, `a2amx::daemon::views`) by default; nothing filters on them (spec 3gc and 3ga made the same change). The `// shortcut:` comments and the lock/await structure of `attach`, `bridge` and the views move unchanged, including the lock behaviour: no moved code holds a session lock and the presence lock together, or any std lock across an await.

## 4. Out of scope

- Any behaviour change, and splitting `attach` (follow-up: its 241 lines, six parameters and `Complex Method` score; a later spec after S7), `explain_of`'s 98 lines, `list_agents`'s 72.
- `kill_session`, `cancel_message`, `lookup`, `serve` (S6b, S6c).
- The typed `SessionId`, the `Role` mapping helper (backlog F94).

## 5. Acceptance

- `cargo test --no-fail-fast`: all pass. Baseline at the commit before this spec: 66 suites, 1415 passed, 0 failed, 3 ignored (architect states the commit before assigning); after: the same (no test added, removed or edited).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass.
- `rg -n 'fn (list_sessions|list_agents|screen_of|explain_of|list_messages|session_quota|session_working|session_activity_cause|session_activity|attachment_status|attach|send_data|bridge_attach|bridge)\b' src/daemon.rs` prints nothing; `rg -n 'struct AttachmentGuard' src/daemon.rs` prints nothing; `rg -c '^(async )?fn |^pub\(super\) (async )?fn ' src/daemon/views.rs src/daemon/attach.rs src/daemon/bridge.rs` prints views 9, attach 3 (`AttachmentGuard::drop` is indented and not counted) and bridge 2.
- `git status --short` lists only `src/daemon.rs`, `src/daemon/notices.rs`, `src/daemon/views.rs`, `src/daemon/attach.rs`, `src/daemon/bridge.rs`; `git diff --stat -- Cargo.toml Cargo.lock` is empty; `git diff -- src/daemon/notices.rs` shows only the one `use` change.
- Diff check (tester, mechanical): each moved item's body is byte-identical to its body at the base commit (`git show HEAD:src/daemon.rs`), apart from visibility, imports and `super::` paths.
- Mutation check (tester, in the checkout, one change at a time, restoring by checksum; the code is moved, so the mutants show the tests still pin it), naming the repo test that fails on each: `list_sessions` drops the quota; `list_agents` ignores the viewer's visibility filter (`tests/teams/team_visibility.rs`, tester names it); `screen_of` returns an empty screen; `explain_of` loses `activity_cause`; `session_quota` returns `None` for a Claude or OMP session (`tests/quota/daemon_limits.rs`; Codex has no daemon-level quota test, do not use it); `attachment_status` sets `remote` to `None` (`tests/presence/remote_status.rs`, tester names it); `attach` ignores `force` (`attach_force_takes_the_session_over_from_another_attachment`, `tests/attach_cli/attach_session.rs:494`); `AttachmentGuard::drop` does not clear `state.attachment` (a later non-force attach fails with "attached elsewhere"; tester names the test); `bridge_attach` skips the OMP-session check (`tests/bridge.rs:358`, `tests/claude_channel.rs:342`); `send_data` chunk size raised above 64 KiB (`large_final_snapshot_is_split_at_the_data_limit_and_precedes_exit`, `tests/daemon/attach_streams.rs:488`, assertion `:534`); shrinking the chunk survives (the test only asserts at most 64 KiB) and is not to be mutated. Survivors are reported, not fixed.
- Known flaky tests: attach and bridge tests wait on timers and PTY output; a failure is rerun once alone and recorded as pre-existing if it passes.

## 6. Claims (claim | source | verified how)

| Claim | Source | Verified how |
|---|---|---|
| ranges of the thirteen free functions, the guard and the doc comment | `awk` outline of `src/daemon.rs` at the base commit | probed |
| call sites of the moved functions outside their own range | `rg -n` per name over `src/` | probed: `:1829, :1845, :1846, :1882, :1914, :1861, :1900` (serve), `:674, :813, :1960, :1966, :2047, :2063, :2146, :2524, :2557`, `notices.rs:44, :333` |
| `session_activity_cause` is used by `explain_of` and `session_activity` only | `:2146`, `:2457` | probed |
| `attach` is called once, by `serve` | `:1861` | probed |
| nothing in the three families is called from a file other than `daemon.rs` and `notices.rs` | `rg -n` over `src/`, `tests/` for the names (tests use the protocol only) | to confirm before lock |
| the moved code reaches only parent items a child can see; no sibling's private item | consultant read of `registry`, `omp_usage`, `remote`, `trust` visibility and the moved bodies | read (consultant m_8505) |
| `session_working` is used by `notices.rs` only; `ACTIVITY_QUIET` is used only by `session_working` and `session_activity_cause` | `:2422`, `:2442`, `notices.rs:333` | read (consultant); `session_working` moves with the other derived facts into `views` as 2.1 says, and `ACTIVITY_QUIET` stays in the parent (it is declared at `:46` with its `// shortcut:` comment; `views` reaches it as `super::ACTIVITY_QUIET`); revisit the placement of the derived facts when S6b/S7 split `Runtime` |
