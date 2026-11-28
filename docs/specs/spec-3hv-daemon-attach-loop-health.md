# Spec 3hv: function-level code health of the attach loop in `src/daemon/attach.rs`

## 0. Status

LOCKED. Context `a2amx-3hv-daemon-attach-loop-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Behaviour-preserving refactor of the second half of `attach` (the loop after the setup phase of spec 3hu), test-first where unpinned. Public seams: the stream protocol over a real daemon (`tests/daemon/attach_streams.rs`, `status_reporting.rs`, `serve_dispatch_pins.rs`), presence over the control protocol (`tests/presence/*`).

Scope (edit): `src/daemon/attach.rs` (the loop and what it calls, `AttachmentGuard`, `close_with_exit`, `send_data`), tests only for Step A gaps appended to existing files named there. Does not touch: the setup phase functions of 3hu (`reserve_attachment`, `wait_closed`, `install_attachment`, `count_presence`), `attachment_status`, `serve.rs`, any other file.

No behaviour change: same frames in the same order, same lock scopes (the session lock is never held across an await; the presence mutex is a leaf and is never held with the session lock), same error replies, same exit and shutdown sequence.

## 1. Why

After 3hu `cs check src/daemon/attach.rs` reports on `attach` (`:192`): "Complex Method (cc = 31)", "Large Method (197 lines)", "Bumpy Road Ahead (bumps = 4)", and the file "Overall Code Complexity" (8.27). The cause is the loop: one `select!` with four arms, a frame `match` of the seven `ClientFrame` kinds nested in the receive arm, two inline copies of the send-`Exit`-then-shutdown sequence in the loop (the setup already uses `close_with_exit`) and one `Detached` variant of it.

Rules that bind (earlier refactors): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs; no suppression.

## 2. Step A: pins (tests, green on the current code)

Existing pins (consultant-confirmed; the developer reruns each and names it in the REPORT; the tests marked (c) use `start_confined_daemon_with_limits` and return early without `bwrap`, so the tester shows they ran):

| Loop behaviour | Pin |
| --- | --- |
| taken over: `Detached` then the stream closes | `attach_streams.rs:164` (+`:165` closed), `:447`, `:465` |
| output arm: `Data`, then `Exit` after the process exits | `live_exit` (`attach_streams.rs:258-266`), `large_final_snapshot` (`:488`, `:538`); NOT `:285` or `detached_queries`, which are the setup path (session already exited) |
| status frame only on change | `status_reporting.rs:508` (changed at `:591`, no repeat for 2.5 s at `:607-619`) |
| `Detach` ends; disconnect ends | `attach_streams.rs:183-184`, `:302-303`; `:185-192` |
| `Input` reaches the session; a viewer's `Input` is ignored; after `Mode{viewer:false}` it reaches | `attach_streams.rs:131-134`; `viewers.rs:76-91` (c); `viewers.rs:92-117` (c) |
| `Release`, ignored for a viewer | `viewers.rs:208` (c) (`:240`, `:269`) |
| `Mode` updates `slot.viewer` | `viewers.rs:131-192` (c) |
| `Mode` updates the presence mode | `tests/presence/away_notice.rs:103-120`, `owner_presence.rs:220-226` |
| `Resize` resizes and redraws; `Scroll` and `Redraw` send a full render | `attach_streams.rs:167-182`, `viewers.rs:51-67`; `attach_streams.rs:322-344` |

Unpinned, so the developer adds literal-expectation tests green on the unchanged code, appended to existing files with few functions (`cs check` per file before and after; the placement nit of `serve_dispatch_pins.rs` stands):

- (G1) a frame on an exited session while scrolled back: while the session is scrolled back, output keeps `dirty` set, so the exit filter yields nothing and the `Exit` is withheld until the next client frame. Test: `sh -c 'i=1; while [ $i -le 30 ]; do printf "\r\nline%02d" $i; i=$((i+1)); done; read l; printf bye; exit 5'`, attach 40x5, wait for `line30`, `Scroll` to the top and wait for `line01`, send `Input "go\n"` (`note_human_input` does not unscroll), poll `Request::List` until `exit_code == Some(5)`, drain about 300 ms asserting no `Exit`, send `Resize{50,7}`, expect `Data` then `Exit(5)`, and assert `List` still shows 40x5 (a `Resize` on an exited session does not resize). Kills (l), (n), (p). The developer confirms in the red run that the scrolled offset survives the final `bye` (no newline); if not, they adapt the script and say so.
- (G2) `Mode` with an unchanged viewer does not touch presence: interactive attach to a remote session as in `owner_presence.rs`, `Mode{viewer:false}`, `Redraw`, receive the data, `Detach`, then the owner state leaves "present" as in `owner_presence.rs` (without the `guard.viewer != viewer` check the interactive count goes to 2 and `attach_ended` only brings it to 1, so the owner stays present). Kills (r).

Expected survivors, reported with their reason: (m) `dirty` not cleared after `redraw` (at most one extra harmless `Data`, which the tests tolerate); (q) notifier registered after `output_pending` (a lost-wakeup race, no deterministic test); any mutant on `guard.counted` inside the loop (equivalent: the loop is reached only when `exited.is_none()`).

## 3. Step B: target shape

After setup, `attach` builds `struct AttachLoop { session: Arc<Session>, runtime: Arc<Runtime>, guard: AttachmentGuard, taken_over: watch::Receiver<bool>, status: bool, last: Option<StatusInfo> }` and returns `AttachLoop{..}.run(connection).await`, so `attach` is the setup sequence plus one call. The `ticker` stays a local of `run`. Methods (at most three parameters counting `self`):

- `async fn run(&mut self, connection: &mut Framed) -> anyhow::Result<bool>`: first `self.report_status(connection).await?` (with `last` `None` it always sends, as the first status frame does today; it comes after the setup's exited return, which `serve_dispatch_pins.rs:369` pins), then the ticker (one period after that, `Delay` missed-tick behaviour), then `loop { if self.serve_next(connection, &mut ticker).await? { return Ok(true) } }`.
- `async fn serve_next(&mut self, connection, ticker: &mut Interval) -> anyhow::Result<bool>`: one iteration, true when the attach is over. It starts with the taken-over check (`*self.taken_over.borrow()`: `close_with(connection, ServerFrame::Detached(..))`, return true), clones `let session = Arc::clone(&self.session);` (so the pinned notifier borrows the local, not `self`: `session.notify().notified()` held across the `select!` would otherwise make every `&mut self` handler call E0502), registers the notifier (`enable()`) BEFORE computing `output_pending`, then `select!` with the arms yielding `done: bool`: `taken_over.changed()` => false (the check stays at the top, not in the arm), tick => `{ self.report_status(connection).await?; false }`, output => `self.send_pending_output(connection).await?`, `connection.recv()` => `match payload? { Some(p) => self.on_frame(connection, p).await?, None => true }`.
- `fn output_pending(session: &Session) -> bool` (free or associated, one job: "is there a screen update or an exit to send"): the existing expression under one short session lock, called after `enable()`.
- `async fn report_status(&mut self, connection) -> anyhow::Result<()>`: when `status`, compute `attachment_status`, send `Status` only when it differs from `last`, update `last`.
- `async fn send_pending_output(&mut self, connection) -> anyhow::Result<bool>`: `render_update` under the lock, `dirty` cleared unless scrolled, the exit filter, send `Data` outside the lock, and on exit `close_with(Exit)`; returns whether the attach is over.
- `async fn on_frame(&mut self, connection, payload: Vec<u8>) -> anyhow::Result<bool>`: decode (`ClientFrame` has seven variants) and dispatch: `Detach` returns true; `Input` and `Release` stay one-statement arms guarded by `!self.guard.viewer`; `Mode` calls `change_mode`; `Resize`, `Scroll` and `Redraw` call `redraw` (bound with `frame @ (..)` if the pattern allows, else the same `_ => unreachable!()` fallback inside `redraw`).
- `fn change_mode(&mut self, viewer: bool)`: the `Mode` arm: when `guard.counted && guard.viewer != viewer` the presence mode change and `guard.viewer = viewer`, then `slot.viewer` under the session lock. The presence `MutexGuard` stays a temporary or sits in its own block, never held across the session lock (the leaf rule).
- `async fn redraw(&mut self, connection, frame: ClientFrame) -> anyhow::Result<bool>`: under the lock apply the frame (`Resize` only when the session has not exited), clear `dirty`, `render_full`; send `Data` outside the lock; on exit `close_with(Exit)`; returns whether the attach is over.
- `close_with_exit` (3hu) becomes `close_with(connection, frame: ServerFrame)` (frame by value): sends the frame and shuts the stream down. Four sites use it: the setup `Exit`, the loop's `Detached`, and the two loop `Exit` sites. A signature change to the existing helper, not a second helper.

Order and lifetime rules a split must keep: the taken-over check runs at the top of every iteration before the `select!`; the notifier is enabled before `output_pending` is read, per iteration; `output_pending` is computed under one short lock released before the `select!`; `self.guard.viewer`, not the setup options, is the viewer flag; `AttachLoop` is a local of `attach`, built after the setup's exited return, and dropped (so the guard drops) when `attach` returns after the close's shutdown, as today; `send_data` is called outside the session lock.

If a finding remains, the developer quotes the cs text, reports per-function cc and stops. Hand counts (consultant, token model calibrated to today's 31): `run` 2, `serve_next` about 6 to 8 (the risk: if cs still reports Complex Method or Bumpy Road on it, the developer quotes the finding and stops), `on_frame` 5, `redraw` 4, `change_mode` 5, `send_pending_output` 3, `report_status` 3, `output_pending` 3.

## 4. Out of scope

The setup phase, `attachment_status`, `AttachmentGuard`'s `Drop`, the client, the wire types, `serve.rs`.

## 5. Trust boundary

No boundary moves. `Attach` stays admin-only and the viewer flag keeps denying `Input` and `Release` (Step A item 4 pins it).

## 6. Acceptance

Baseline: whole-tree cs count 843 (after 3hu).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/attach.rs
git status --short
```

- All pass; `cs check` lists no function finding on `attach.rs` and the file score is at least 9.0; whole-tree count below the baseline; new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression.
- Mutation check by the tester only (scratch `target/scratch/3hv/`, own `CARGO_TARGET_DIR`, deleted before its REPORT): (a) taken-over check dropped at loop top; (b) `Detached` text changed; (c) status sent every tick; (d) status never sent on tick; (e) `Detach` frame ignored; (f) viewer `Input` reaches the session; (g) non-viewer `Input` dropped; (h) `Release` ignored for non-viewers; (i) `Mode` does not update `guard.viewer`; (j) `Mode` does not update `slot.viewer`; (k) presence mode not changed on `Mode`; (l) `Resize` resizes an exited session; (m) `dirty` not cleared after `redraw` (expected survivor); (n) `Exit` not sent after `redraw` of an exited session (killed by G1); (o) `Exit` not sent after output of an exited session; (p) scrolled output clears `dirty` (killed by G1); (q) notifier registered after `output_pending` (expected survivor); (r) the `guard.viewer != viewer` check dropped (killed by G2). The killers: (a) `attach_streams.rs:164`, `:447`; (b) `:164`, `:447`, `:465`; (c) and (d) `status_reporting.rs` around `:591` and `:617`; (e) `attach_streams.rs:184`; (f) `viewers.rs:91`; (g) `attach_streams.rs:134`; (h) `viewers.rs:240`; (i) `viewers.rs:117`; (j) `viewers.rs:184`; (k) `away_notice.rs:110-120`, `owner_presence.rs:225`; (o) `live_exit`. The confined tests are shown to have run. The tester names the killing test per mutant and reports survivors with their reason.

## 7. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `attach` at `:192`, findings Complex Method 31, Large Method 197, Bumpy Road 4 | `attach.rs` | `cs check` after 3hu |
| `ClientFrame` has seven variants | `wire` | consultant read |
| four `close_with` sites | `attach.rs` `:272`, `:288-291`, `:327-328`, `:383-384` | consultant read |
| `guard.counted` is always true inside the loop | `attach.rs:258-260, 271-273` | consultant read |
| a pinned `session.notify().notified()` borrowed from `self.session` makes `&mut self` handlers E0502; a cloned local avoids it | `attach.rs:296-298` | consultant inference from the compiling code; the developer confirms with the compiler |
| the presence guard is a temporary of the `if let` scrutinee and gone before `session.lock()` | `attach.rs:347-357` | consultant read |
| the existing pins of section 2 | tests | consultant read |
| G1 and G2 are unpinned | grep over `tests/` | consultant grep |
