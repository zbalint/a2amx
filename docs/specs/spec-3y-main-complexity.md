# Spec 3y: split `run_team_up`, `handle_input` and `handle_picker_input` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14; pre-lock gate run against develop at `96d066d`; consultant review `m_1634`, findings 1-7 adopted). Architect-directed, backlog I4 part 2, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `96d066d` (spec 3x and its two follow-up tests committed); `cargo test --test attach_cli` 26 passed, `cargo test --test team` 74 passed.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the `a2amx` binary only (`tests/team.rs` dry-run output, `tests/attach_cli.rs` PTY attach,
mouse, picker). All functions here are private to `src/main.rs`; no direct test of a helper.

**Scope.** May edit exactly these files and no others.

- `src/main.rs`: `run_team_up` (line 889), `AttachmentState::handle_input` (1519), `handle_picker_input` (1751), and
  `restore_current` (1717) only because it shares a duplicated expression with `handle_picker_input`
- `tests/team.rs`: only the two dry-run tests of section 4

Does not touch: every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

`rust:S3776` flags `run_team_up` (46), `handle_input` (41) and `handle_picker_input` (33) against a limit of 30
(backlog I4). `run_team_up` mixes planning, the dry-run report and the real spawn loop. `handle_input` mixes mouse
parsing, picker input, scroll input and prefix batching in one loop. `handle_picker_input` repeats the
connect-and-attach expression that `restore_current` also contains.

## 2. Rule: behavior is unchanged

Same output bytes, same order of daemon requests, same errors. No new `pub` item, no signature change to any
function other than those named below. No change to `shortcut:` comments (keep them with the code they annotate).

## 3. Changes in `src/main.rs`

### 3.1 `run_team_up`

Keep the function and signature. Extract:

1. `async fn dry_run_report(home, cwd, file_dir, actions) -> anyhow::Result<String>`: the `if dry_run` loop that builds
   `output`. `run_team_up` keeps `write_stdout`, `note_lost_messages` and the early `return Ok(())`, in that order.
   Inside it, a `describe_start(...)` helper for one `Action::Start` (the harness, cwd, team, role, attach, watch,
   control_from, heartbeat, command lines) is allowed.
2. `async fn start_planned_session(home, client, session, cwd, file_dir, cols, rows) -> anyhow::Result<(String, String)>`:
   the `Action::Start` arm of the real loop (heartbeat parse, `create_session`, the `started {name} {id}` line). The
   `failed {name}` context wraps only the `create_session` result; the heartbeat parse error is NOT wrapped in it and
   keeps propagating with `?` before `create_session`. The `AlreadyRunning` arm and the `attached_id` update stay in `run_team_up`. The
   `// shortcut:` comment stays directly above the loop in `run_team_up`.

### 3.2 `handle_input`

Keep the function and signature. Extract:

1. `parse_decimal(bytes: &[u8]) -> u16`: takes the LEADING ASCII digits of `bytes` (`take_while(is_ascii_digit)`) and folds them
   with `fold(0u16, |v, b| v.saturating_mul(10).saturating_add(u16::from(b - b'0')))`. `handle_input` passes
   `&bytes[offset + 3..]` and `scan_session_mouse` passes `&bytes[first..end]` (already all digits, so the
   `take_while` is a no-op there). The `take_while` inside is required: without it `;` and later bytes underflow `b - b'0'`. This is the one reuse change outside the
   named functions and is limited to those two call sites.
2. `async fn feed_scroll_input(&mut self, byte: &[u8]) -> anyhow::Result<()>`: the scroll-mode branch.
3. `prefix_batch_len(&mut self, bytes, offset, segment_end) -> usize` (synchronous; the caller keeps `prefix_machine.feed`,
   `handle_prefix_actions` and `offset += count`): the `count` computation including the
   `after_prefix` updates; the comment about ending a batch at the prefix moves with it.

### 3.3 `handle_picker_input` and `restore_current`

1. `async fn attach_session(&self, session: &str) -> anyhow::Result<Attachment>`: `Client::connect` then `attach_status` with
   `force = false`, `self.cols` and `pty_rows(self.rows, self.status_visible)`. Both `restore_current` and the
   `Outcome::Attach` arm use it.
2. `async fn switch_session(&mut self, selected: String) -> anyhow::Result<bool>`: the `Outcome::Attach` body after the
   same-session check. Returns `true` when the new attachment is installed (the caller returns `Ok(None)`), `false`
   after showing the error footer with `state.refresh().await?`. `restore_current` keeps its own
   `let _ = state.refresh().await` (it ignores the refresh error; `switch_session` propagates it). Do not unify these.

## 4. Safety net

Existing coverage, checked against the tree at develop `6672971` (find by name, line numbers drift):
`tests/team.rs::dry_run_prints_wired_commands_without_starting_or_installing` asserts, with `contains`, every dry-run
line except `  team: `; the real-path `already running`, `started` and `failed` lines are asserted exactly (`team.rs` real
path tests); the dry-run `AlreadyRunning` arm has NO test. `tests/attach_cli.rs` covers scroll, wheel, click drop,
session mouse reporting, picker open and scroll, teams, and
`picker_switches_sessions_and_exit_status_is_reported`, which reaches the switch error arm ("attached elsewhere", the
picker stays open). No test reaches the error arm of `restore_current`; that extraction is a pure move verified by
reading the diff.

Before the first edit run `cargo test --test attach_cli --test team` and record counts. Then add, passing on the
unchanged code, to `tests/team.rs`:

1. A dry-run test for a file with a top-level `team = "t"` and one session, asserting the exact stdout (so line order
   is pinned) including `  team: t, private, allow: -\n`.
2. A dry-run test with one session already running, asserting the exact line `already running {name} {id}\n`.

Expected values are literals from the unchanged output.

## 5. Out of scope

Everything in `main.rs` not named in section 3, `team.rs` (spec 3x), and the other Sonar findings.

## 6. Acceptance

```sh
cargo test --test attach_cli --test team
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/main.rs and, if added, tests/team.rs
```

Final proof of the complexity fix is the Sonar rescan after the owner pushes. Known flaky:
`tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live`: rerun once, record as pre-existing if it
then passes.
