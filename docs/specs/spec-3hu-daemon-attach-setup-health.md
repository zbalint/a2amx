# Spec 3hu: function-level code health of the attach setup phase in `src/daemon/attach.rs`

## 0. Status

LOCKED. Context `a2amx-3hu-daemon-attach-setup-health`. Location: main checkout, branch `develop`. Behaviour-preserving refactor of the first half of `attach` (everything before the `loop`), test-first where unpinned. Public seams: the stream protocol over a real daemon (`tests/daemon/attach_streams.rs`), the CLI (`tests/attach_cli/*`).

Scope (edit): `src/daemon/attach.rs` (`attach` up to the loop, `AttachmentGuard` untouched), `src/daemon/serve.rs` (the one `Request::Attach` arm at `:96`, for the new options parameter), tests only for Step A gaps in existing files. Does not touch: the loop of `attach` (spec 3hv), `attachment_status`, `send_data`, any other file.

No behaviour change: same replies, texts, frames, lock scopes and order.

## 1. Why

`cs check` reports on `attach.rs`: `:114` `attach` "Excess Number of Function Arguments (Arguments = 8)" (held back by `#[allow(clippy::too_many_arguments)]`, which the repository's rules do not permit), "Complex Method (cc = 37)", "Large Method (236 lines)", "Bumpy Road Ahead (bumps = 5)"; `:138` "Complex Conditional". This spec takes the setup phase (top-rows check, slot reservation, takeover wait, snapshot and install, presence, attached reply, initial exit and status frame); the loop follows in 3hv, and the `#[allow]` goes away here with the options struct.

## 2. Step A: pins

Existing pins (consultant-confirmed; the developer reruns each and names it in the REPORT):

| Behaviour | Pin | Kills |
| --- | --- | --- |
| 1. 9 top rows replies an `Error` | `tests/daemon/attach_streams.rs:4` (matches `Error{..}` only; optionally assert the text `top rows exceed 8` and that nothing attached) | (a), (b) |
| 2. no-force attach refused (`:145`); forced takeover, the old holder gets `Detached` (`:164`, `:447`, `:465`); the new one waits for the old to close (`:551`, the backpressure gate: the forced attach must not complete within 200 ms while the holder's input is blocked); CLI texts `tests/attach_cli/attach_session.rs:506,514` | | (c), (d), (e) |
| 3. `Attached` vs `AttachedTop`, reply then Data (`attach_streams.rs:39,41`), row origin (`:49-53`), late attach then `Exit` (`:285`); presence: a live attach counts (`owner_presence.rs:4`, `:99`) | | (h), (i), (j), (k) |
| 4. first status frame | `tests/daemon/status_reporting.rs:169` | not (m), see below |

`tests/attach_cli/*` (viewer.rs, status_and_team_line.rs) talk to a fake daemon and pin nothing in `daemon/attach.rs`: not candidates.

Unpinned, so the developer adds literal-expectation tests green on the unchanged code, appended to existing files with few functions (`cs check` per file before and after):

- (A) "attachment was taken over" (mutant (f)): reuse the gate of `attach_streams.rs:551`: forced attach A, wait 200 ms (the pattern at `:578-584`), forced attach B, then release; A gets `Error "attachment was taken over"` and B gets Data. Under mutant (f) A also installs and replies `Attached`.
- (B) an exited session's attach with status: `attach_status` yields Data then `Exit`, with no `Status` frame first (kills (l)).

Expected survivors, reported with their reason (no timing pin): (g) `dirty` not cleared (the loop's first `render_update` then sends no bytes, nothing observable); (i) presence counted for an exited attach (unobservable through any public seam: an exited session's token is refused before dispatch, `owner_state` reads only the caller's own entry and the heartbeat prunes exited ids). (m) is killed by `tests/presence/remote_status.rs:24` (a 1 s lease makes the first status frame the only `Active` one), not by `status_reporting.rs:169`.

## 3. Step B: target shape

- `pub(super) struct AttachOptions { pub force: bool, pub size: Size, pub top_rows: u16, pub viewer: bool, pub status: bool }`, passed by `serve.rs` (`attach(connection, session, options, runtime)`, 4 parameters, no `#[allow]`).
- `fn reserve_attachment(session: &Session, force: bool) -> Option<Reservation>` with `struct Reservation { generation: u64, previous: Option<PreviousAttachment> }` with `struct PreviousAttachment { takeover: watch::Sender<bool>, closed: watch::Receiver<bool> }` (named fields, no tuple): today's reservation block (generation bumped and `pending_attachment` set under one lock, previous slot's channels cloned).
- `async fn wait_closed(previous_closed: watch::Receiver<bool>)`: the `while !*previous_closed.borrow() { changed() ... }` wait (the takeover signal is sent by the caller before).
- `fn install_attachment(session: &Arc<Session>, slot: AttachmentSlot, options: &AttachOptions) -> anyhow::Result<(Vec<u8>, Option<i32>)>`: today's snapshot block (pending generation check, row origin, resize, full render, `dirty` cleared, slot installed, pending cleared), in that order inside one lock. The caller maps `Err` to `Response::Error { error.to_string() }` and `Ok(false)` exactly as today (`:201-213`), never `?` (a `?` would drop the connection instead of replying). The slot the caller builds carries the reserved generation.
- `fn count_presence(runtime: &Runtime, session: &Session, viewer: bool)`: the `attach_started` block, called after the session lock is released and only inside `if exited.is_none()` in `attach`; `guard.counted = true` stays tied to it.
- `async fn close_with_exit(connection: &mut Framed, code: i32) -> anyhow::Result<()>`: sends `Exit(code)` and shuts the stream down (this three-line sequence repeats three times in `attach`; the setup phase uses it once; 3hv replaces the other two).
- `attach` keeps the sequence and the early `response(.. Error ..)` returns. Order and lifetime rules a split must keep: the channels are created before the reservation; the `AttachmentGuard` is built right after a successful reservation and before the wait (a guard built later leaks `pending_attachment` when the future is dropped or the install fails, blocking every later non-force attach); the previous holder is signalled before the wait; the loop reads `guard.viewer` and `options.status`. The Complex Conditional at `:138` (`(a || b) && !force`) moves with the reservation block, so it is named there (`let busy = attachment.is_some() || pending.is_some();`, then `busy && !force`) so that `cs` does not report it in `reserve_attachment`.
- After the split `attach` keeps the loop (cc about 23) and stays Complex and Large until 3hv; hand counts for the new functions: `reserve_attachment` about 4, `wait_closed` 3, `install_attachment` about 5, `count_presence` 2, `close_with_exit` 1. New functions have at most four parameters.

## 4. Out of scope

The loop (3hv), `AttachmentGuard`, `attachment_status`, `send_data`, the client, the wire types.

## 5. Trust boundary

No boundary moves. `Attach` stays admin-only and the generation handshake (reserve, install, drop) is preserved exactly; Step A items 2 and 5 pin it.

## 6. Acceptance

Baseline: after 3hs (whole-tree cs count recorded then).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/attach.rs
git status --short
rg -n "allow\(clippy::too_many_arguments" src/daemon/attach.rs
```

- All pass; the last command matches nothing; on `attach` `cs check` may report only Complex Method, Large Method and Bumpy Road (the loop, until 3hv): the Excess Arguments finding and the `:138` Complex Conditional are gone, and no new function has a finding; whole-tree count below the baseline; new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression.
- Mutation check by the tester only (scratch `target/scratch/3hu/`, own `CARGO_TARGET_DIR`, deleted before its REPORT; targets `daemon`, `attach_cli`, `presence`, `broker`, `status`, `bridge`, `delivery`): (a) the top-rows bound is 9; (b) the bound check is dropped; (c) `force` ignored; (d) the previous attachment is not signalled; (e) the takeover wait is skipped; (f) the pending-generation check dropped; (g) `dirty` is not cleared at install; (h) the row origin is not set; (i) presence counted for an exited session; (j) presence not counted for a live session; (k) `AttachedTop` sent for top rows 0; (l) the exit frame skipped for an exited session; (m) the first status frame skipped (expected survivor), and (g) (expected survivor). The tester names the killing test per mutant; an expected non-kill is reported with its reason.

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| the findings and lines | `attach.rs:114,138` | `cs check` run, output read |
| `attach` has one caller | `serve.rs:96` | grep over `src/` before locking |
| the existing pins named in section 2 exist and cover what is claimed | `tests/daemon/attach_streams.rs`, `tests/presence/owner_presence.rs`, `tests/daemon/status_reporting.rs` | consultant read |
| no test asserts `attachment was taken over` | grep over `tests/` | consultant grep (pin (A) added) |
| `serve.rs:96` is the only caller; `bridge_attach` (`:119`) is another function | `serve.rs` | consultant read |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (consultant session, model not exposed to the architect); developer Gemini 4 Hydrogen (Medium) on antigravity; tester opencode-go/deepseek-v4.1-flash on OMP (no separate planning model). BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| draft review | consultant | "attachment was taken over" had no pin anywhere; an exited attach with status had none either | pins (A) and (B) added to Step A before locking |
| draft review | consultant | the tests/attach_cli files use a fake daemon and pin nothing in `daemon/attach.rs` | dropped from the pin table |
| tester pass | tester (deepseek-v4.1-flash) | the spec claimed `owner_presence.rs:347` pins mutant (i); it queries the sender's owner state, so (i) survives | consultant showed (i) is unobservable through any public seam; spec row corrected, (i) listed as an expected survivor |
| tester pass | tester (deepseek-v4.1-flash) | mutant (m) was listed as an expected survivor but `remote_status.rs:24` kills it | spec corrected |
| tester pass | tester (deepseek-v4.1-flash) | the guard-built-after-wait mutant survives; the spec's "future is dropped" rationale is unreachable because the attach future is awaited inline and aborted only at shutdown | recorded as equivalent; the guard order stays as the spec states it |
| tester pass | tester (deepseek-v4.1-flash) | first report named the wrong hanging tests under mutant (d) | corrected by the tester in a follow-up; kill outcome unchanged |
| architect review | consultant | nits: `install_attachment` reads only `top_rows` and `size` of `AttachOptions`; `viewer` lives in three places | accepted as is; 3hv must read `guard.viewer`, not `options.viewer` |
