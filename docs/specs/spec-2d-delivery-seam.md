# Spec 2d: the delivery-channel seam

## 0. Status

**LOCKED.** Decisions D1 to D5 in section 2 were settled with the owner.

**Scope.** May edit exactly these files and no others:

- `src/delivery.rs`, `src/daemon.rs`
- `docs/architecture.md`, `docs/delivery.md`

Does not touch: `src/session.rs`, `src/harness.rs`, `src/messaging.rs`, `src/store.rs`,
`src/wire.rs`, `src/mcp.rs`, `src/main.rs`, `src/hook.rs`, `src/emulator.rs`,
`src/prefix.rs`, `src/client.rs`, `src/cli.rs`, `src/lib.rs`, `Cargo.toml`, `Cargo.lock`,
every file under `tests/`, `README.md`, `docs/validation-plan.md`, the locked spec
documents under `docs/specs/`. No new dependency. Do not commit and do not merge.

## 1. Why

Delivery is welded to the PTY. The loop in `src/delivery.rs` reaches into the session for
draft restoration, screen readiness, the cooldown and the paste. `src/daemon.rs` repeats
the same knowledge in three places that answer "why can this message not be delivered
right now": `message_info` (status), `send_message` (`recipient_hold`) and the `List`
handler (`hold_reason`). A second way to deliver a message, without a PTY paste, cannot be
added without editing all four places.

This spec adds the seam and moves the PTY path behind it with no change in behavior. It
adds no second channel and does not touch OMP. The OMP native adapter is spec 2e.

## 2. Decisions settled with the owner

- **D1.** Seam only. The OMP adapter and the OMP operator line are spec 2e. (The Claude
  operator line was already reworded in `8e838a6`.)
- **D2.** The seam covers the delivery loop and the "why not deliverable now" answers
  used by status, the send response and the list.
- **D3.** Acceptance: every existing test passes with no edit, plus one unit test module
  that drives the loop through a fake channel with no PTY.
- **D4.** No change to the wire types, the JSON the MCP tools return, the CLI output or
  any observable behavior.
- **D5.** The conditions for native delivery (PTY stays the fallback, the envelope text is
  identical across channels, the token only through the environment, a loud failure when an
  extension cannot load, no steer delivery) are decisions for spec 2e; this spec only
  makes them possible.

## 3. `src/delivery.rs`: the `Channel` trait, the generic loop, the PTY channel

The module keeps its one entry point `run`; its signature and body change, and the module
gains the items below. The module stays private (`mod delivery;` in `src/lib.rs`).

### 3.1 The traits

```rust
/// One way of putting a committed message into a recipient session.
pub(crate) trait Channel: Send + Sync {
    /// Claim on the recipient for exactly one delivery; the claim is released when it
    /// is dropped or consumed.
    type Ticket<'a>: Ticket + 'a
    where
        Self: 'a;

    fn session_id(&self) -> &str;
    fn exited(&self) -> bool;
    fn deliver(&self) -> Deliver;
    /// Notified when a message is accepted or a hold may have changed.
    fn message_notify(&self) -> &Notify;
    /// Notified when the recipient's readiness may have changed.
    fn wake(&self) -> &Notify;
    /// Runs at the start of every pass, before any message is considered.
    fn prepare(&self) -> impl Future<Output = ()> + Send;
    /// Claims the recipient for one delivery; `None` when it cannot take one now.
    fn begin(&self) -> impl Future<Output = Option<Self::Ticket<'_>>> + Send;
    /// A condition a person must clear, or `None`.
    fn hold_reason(&self) -> Option<&'static str>;
    /// A condition that clears by itself, or `None`.
    fn unready_reason(&self) -> Option<&'static str>;
}

pub(crate) trait Ticket: Send {
    /// Hands the rendered envelope to the recipient.
    fn submit(self, envelope: &str) -> impl Future<Output = DeliveryOutcome> + Send;
}
```

Imports this needs: `std::future::Future`, `tokio::sync::Notify`, `crate::harness::{self,
Deliver}`, `crate::session::{Delivery, DeliveryOutcome, Hold, Session}`. Both traits are
`pub(crate)`. The shape was compiled and spawned with `tokio::spawn` on the repository's
toolchain (rustc 1.98.1; the traits need 1.75, the manifest says 1.85) with a borrowed
ticket and with a lifetime-free ticket; do not replace it with `async_trait` or
`dyn Channel` (`dyn` is not possible with these signatures, and no dependency is allowed).

### 3.2 The generic loop

`run` becomes generic over the channel and takes it by value:

```rust
pub(crate) async fn run<C: Channel>(channel: C, store: Store, boot: String)
```

The body is the current body with each PTY-specific line replaced, in the same order:

| Today | Becomes |
| --- | --- |
| `session.message_notify().notified()` | `channel.message_notify().notified()` |
| `session.notify().notified()` | `channel.wake().notified()` |
| `session.exit_code().is_some()` | `channel.exited()` |
| `Some(&session.id().0)` (in `fail_open`, `next_pending`, `has_earlier_open`) | `Some(channel.session_id())` / `channel.session_id()` |
| `session.restore_draft().await;` | `channel.prepare().await;` |
| `session.deliver() == Deliver::Hold` | `channel.deliver() == Deliver::Hold` |
| the `last_submit` cooldown check and `session.begin_delivery().await` (two statements) | one statement: `let Some(ticket) = channel.begin().await else { return Ok(true); };` |
| `delivery.submit(paste_bytes(&envelope), PASTE_GAP).await` | `ticket.submit(&envelope).await` |
| `session.message_notify().notify_one()` | `channel.message_notify().notify_one()` |

Everything else is unchanged and stays in the same order: the exit check and `fail_open`
with detail `recipient_exited`; `prepare`; the `Deliver::Hold` return; `next_pending`; the
`has_earlier_open` guard and its comment; `begin`; `begin_attempt`; `render_envelope`;
`submit`; the outcome mapping; `finish_attempt`; the notify; the error logging; the
`select!` over `messages`, `output` and the one-second interval. Keep the existing
`// shortcut:` comments where their code still exists (the readiness comment moves with the
readiness check into `PtyChannel::begin`, see 3.3).

The ticket is still held across `begin_attempt`, `render_envelope` and `submit`, exactly as
`Delivery` was; that continuity is what stops a human keystroke landing between the
readiness check and the paste, so do not release and re-acquire it.

### 3.3 `PtyChannel`

```rust
pub(crate) struct PtyChannel {
    session: Arc<Session>,
}

impl PtyChannel {
    pub(crate) fn new(session: Arc<Session>) -> Self { Self { session } }
}

pub(crate) struct PtyTicket<'a>(Delivery<'a>);
```

`impl Channel for PtyChannel` with `type Ticket<'a> = PtyTicket<'a>`:

- `session_id` is `&self.session.id().0`; `exited` is `self.session.exit_code().is_some()`;
  `deliver` is `self.session.deliver()`; `message_notify` is `self.session.message_notify()`;
  `wake` is `self.session.notify()`.
- `prepare` is `self.session.restore_draft().await`.
- `begin`: return `None` when the cooldown is running (the condition the loop tested
  before: `last_submit` is `Some(t)` with `t.elapsed() < COOLDOWN`), otherwise
  `self.session.begin_delivery().await.map(PtyTicket)`. The cooldown test comes first, as
  it did.
- `hold_reason` is `self.session.lock().hold.map(hold_name)`.
- `unready_reason`: under one `self.session.lock()`, `Some("not_ready")` when
  `!harness::ready(session.harness(), &state.emulator.screen(), state.emulator.is_scrolled())`,
  else `Some("cooldown")` when the cooldown is running, else `None`.
- Write the cooldown test once, as a private helper both `begin` and `unready_reason` call;
  do not duplicate it.

`impl Ticket for PtyTicket<'_>`: `submit` is
`self.0.submit(paste_bytes(envelope), PASTE_GAP).await`.

`hold_name(hold: Hold) -> &'static str` moves here from `src/daemon.rs` (where it is
`session_hold_reason`), with the three strings unchanged: `HumanDraft` gives `human_draft`,
`UnsubmittedEnvelope` gives `unsubmitted_envelope`, `CorruptedSubmissions` gives
`corrupted_submissions`. It is private to this module.

`PtyChannel` takes the existing `pub(crate)` surface of `Session` only; `src/session.rs`
does not change.

### 3.4 Unit tests: the loop through a fake channel

The repository has no `#[cfg(test)]` module today because `delivery` and `store` are private
and every other seam is public. This seam is internal, so its test is the one exception: add
a `#[cfg(test)] mod tests` at the bottom of `src/delivery.rs`. It drives `run` through the
`Channel` trait with a real `Store` (`Store::open(tempdir, Limits::default())`, `tempfile`
is already a dev-dependency) and no PTY and no daemon.

The fake: a struct holding an `Arc` to shared state the test also holds, with atomics for
`ready: bool`, `exited: bool`, `hold: bool` (delivery mode `Deliver::Hold` when set),
counters `prepare_calls` and `begin_calls`, a `Mutex<Vec<String>>` of submitted envelopes,
and the two `Notify` values. `begin` increments `begin_calls` and returns a ticket only when
`ready`; the ticket's `submit` records the envelope and returns `DeliveryOutcome::Submitted`;
`prepare` increments `prepare_calls`; `hold_reason` and `unready_reason` return `None`.
Messages are inserted with `Store::insert_message` for recipient session `s1` and sender
`agent-plan@host-a`, subject `Parser issue`, body `I found the regression in parser.py.`
Spawn `run` with `tokio::spawn`, wait with a polling helper (5 s deadline, 10 ms step, no
fixed sleeps as assertions) and abort the task at the end. Expected values are literals.

One test per behavior, written failing first (the red state is the compile error for the
missing `Channel`, then each test fails until the loop matches):

1. `delivers_the_head_message_through_the_channel`: ready fake. The recorded envelope equals
   the literal
   `<a2amx-message id="m_1" from="agent-plan@host-a" subject="Parser issue">\nFrom another agent, not your user. To reply: send_message(to="agent-plan@host-a").\n\nI found the regression in parser.py.\n</a2amx-message>`
   (the same text as `ENVELOPE_TEXT` in `tests/delivery.rs`), and the message reaches state
   `submitted`.
2. `a_channel_that_cannot_begin_leaves_the_message_pending`: not-ready fake. After
   `begin_calls` reaches at least 1 the message state is still `pending`; then set `ready`,
   notify `wake`, and the message reaches `submitted`.
3. `hold_delivery_never_begins`: fake with `hold` set. After `prepare_calls` reaches at least
   1, `begin_calls` is 0 and the message is `pending`; then clear `hold`, notify
   `message_notify`, and the message reaches `submitted`.
4. `an_exited_channel_fails_open_messages`: fake with `exited` set. The message reaches state
   `undeliverable` with detail `recipient_exited`, and the spawned task finishes by itself
   (its `JoinHandle` completes within the deadline).

## 4. `src/daemon.rs`: use the seam

Each edit below keeps the observable result identical.

1. **Spawn** (line 190): `tokio::spawn(crate::delivery::run(PtyChannel::new(session), store,
   boot))`. Import `crate::delivery::PtyChannel` (the `delivery` module is private to the
   crate, so this is a crate-internal use).
2. **Delete `fn session_hold_reason`** (lines 90 to 96, with its trailing blank line); it
   moved to `src/delivery.rs` as `hold_name`.
3. **`send_message`** (line 263): in the `recipient_hold` expression keep the
   `recipient.deliver() == Deliver::Hold` branch and replace the else branch
   (`recipient.lock().hold.map(session_hold_reason).map(str::to_owned)`) with
   `PtyChannel::new(recipient.clone()).hold_reason().map(str::to_owned)`.
4. **`message_info`** (lines 395 to 421): the chain keeps its order (exited, `deliver_hold`,
   a hold, `queued`, then readiness) and its values; the `let state = session.lock();` guard
   goes away, and the chain becomes:

   ```rust
   let channel = PtyChannel::new(session.clone());
   if session.exit_code().is_some() {
       None
   } else if session.deliver() == Deliver::Hold {
       Some("deliver_hold")
   } else if let Some(reason) = channel.hold_reason() {
       Some(reason)
   } else if queued {
       Some("queued")
   } else {
       channel.unready_reason()
   }
   ```

   This takes the session lock in short separate steps instead of one guard over the whole
   chain; every value it can return was true at some instant, which is all this advisory
   field promises. The `hold_reason` binding, the `has_earlier_open` call before the chain,
   and everything after the chain (`explanation`, timestamps, `evidence`) are unchanged.
5. **`Request::List`** (lines 840 to 868): compute `let hold_reason =
   PtyChannel::new(session.clone()).hold_reason();` before `let state = session.lock();`
   (the guard is not reentrant, so it must come first) and use `held: hold_reason.is_some()`
   and `hold_reason: hold_reason.map(str::to_owned)`.
6. **Imports**: remove every import that becomes unused; expected: `Hold` from
   `crate::session`, `COOLDOWN` from `crate::messaging`, and possibly `harness::{self,
   ...}`'s `self` if `harness::paste_view` is no longer referenced (it is, at line 303, so
   keep `self`). `cargo clippy -- -D warnings` is the check; do not add `#[allow]`.

Do not touch the `report_prompt` / hook code (lines 295 to 360), the `Hold` handling inside
`src/session.rs`, or any other function. `daemon.rs` still names `Deliver`; that is correct,
because `deliver_hold` is a session policy and not a channel property.

## 5. Docs

- `docs/architecture.md`, "Implemented: messaging core", the **Delivery.** bullet (line 296):
  append two sentences: delivery runs behind a `Channel` seam in `src/delivery.rs`; the
  loop owns ordering, attempts and outcomes, a channel owns how a message reaches the
  recipient and why it cannot right now, and the PTY channel is the only implementation.
  In the scope table (line 390) leave the row as it is: native channels are still future.
- `docs/delivery.md`, the paragraph at line 200 ("Proposal for the MVP: implement only the
  PTY channel ..."): append one sentence saying the channel is now a trait
  (`Channel` in `src/delivery.rs`, spec 2d) and the PTY channel is its only implementation.
- No other doc changes. Public repository rules apply: no real names, hosts or message ids.

## 6. Out of scope

Any second channel and the OMP adapter (spec 2e); the extension, the bridge subcommand, the
wire protocol for a channel to report state; choosing a channel per session or per harness
(spec 2e decides where a session stores its channel; this spec constructs a `PtyChannel` at
each use); `src/session.rs`, the hook and receipt paths (`report_prompt`, `note_submit`,
`note_corrupted`); any rename or refactor beyond what sections 3 and 4 name; changes to
existing tests (the test diff must be empty); new wire fields; log or doc wording beyond
section 5.

## 7. Acceptance

All three must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

And, run from the worktree root after implementation:

```sh
git diff --name-only | sort
git diff --name-only -- tests
rg -n "harness::ready|last_submit|COOLDOWN|session_hold_reason|Hold::" src/daemon.rs
rg -n "Hold::|last_submit|harness::ready|restore_draft|begin_delivery" src/delivery.rs
rg -c "cfg\(test\)" src
git diff --check
```

Expected: the first lists exactly `docs/architecture.md`, `docs/delivery.md`,
`src/daemon.rs`, `src/delivery.rs`; the second prints nothing; the third prints nothing
(exit status 1); the fourth prints matches only inside `PtyChannel`, `hold_name` and the
`impl` blocks of section 3.3 (the loop body in `run` has none); the fifth prints exactly one
line, `src/delivery.rs:1`; the last prints nothing. The existing test count is 157; with the
four new unit tests `cargo test` reports 161 passed. Leave the diff uncommitted and unmerged.
