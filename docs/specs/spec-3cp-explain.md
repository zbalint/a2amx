# Spec 3cp: `a2amx explain SESSION` says why a session is in the state it is in

## 0. Status

LOCKED. New CLI command and a new request and response on the control protocol; no existing
request, response or message changes.

Scope (files that may be edited): `src/wire.rs`, `src/daemon.rs`, `src/session.rs`,
`src/cli.rs`, `src/main.rs`, `docs/cli.md`, `tests/wire.rs`, and the new `tests/explain.rs`.
Does not touch: every other `src/` file, every other test file, `CHANGELOG.md`, `docs/backlog.md`,
`docs/roadmap.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base the commit that
contains this spec. Context id: `a2amx-explain-2026-11-04`. Public test seams: the `a2amx` binary
(`explain`) against a daemon in a temp state dir (the pattern of `tests/team.rs:1411`), the
library `Client` for `Request::Explain` (the pattern of `tests/broker.rs:1988`), and the JSON of
the new request and response (`tests/wire.rs`).

## 1. Why

Backlog F18 (roadmap 0.2.0): to learn why a peer is not receiving messages, or why a reset was
refused, an operator or architect reads `list`, `messages` and `screen` and guesses. The daemon
already holds the reasons: the activity classification (`session_activity`, `src/daemon.rs:2508`),
the hold reason and its sentence (`hold_explanation`, `src/messaging.rs:67`), the input-needed
signal (`Session::needs_input`, `src/session.rs:605`), the reset gate (`reset_gate_reason`,
`src/session.rs:945`) and readiness (`harness::ready`). None is available to a client together.

Owner decision (2026-11-04): a new read-only command `a2amx explain SESSION` printing the state
and readiness, why the activity is what it is, why messages are held, and why a reset would be
refused. It reuses the existing reasons and adds no state. The part of F18 that asks where the
last activity came from (native channel, prompt hook or screen) is answered by what the daemon
decides activity from today (the screen's last change, readiness, a channel hold, a running
reset). The daemon keeps no record of which source produced an event and this spec adds none.

Rejected: a verbose flag on `list` (the table would grow wide); adding the facts to the MCP
`list_agents` result (it is for peers); recording the source of each activity event (new state).

## 2. `src/wire.rs`

Add `Request::Explain { session: String }` next to `Request::Screen` (`:200`) and
`Response::Explain { info: ExplainInfo }` next to `Response::Screen` (`:282`). New types, with the
same serde derives as `SessionSummary` and `skip_serializing_if = "Option::is_none"` on every
`Option`:

```rust
pub struct ExplainInfo {
    pub id: String,
    pub address: String,
    pub harness: Harness,
    pub exit_code: Option<i32>,         // Some once the child exited; every field below is then None/false
    pub ready: Option<bool>,            // harness::ready now; None only for an exited session
    pub starting: bool,                 // as SessionSummary.starting
    pub activity: Option<Activity>,
    pub activity_cause: Option<String>, // one of the phrases in section 3
    pub needs_input: Option<String>,    // Session::needs_input signal
    pub hold: Option<HoldInfo>,
    pub pending: u32,
    pub reset: ResetVerdict,
}
pub struct HoldInfo { pub reason: String, pub explanation: Option<String> }
pub struct ResetVerdict { pub allowed: bool, pub code: Option<String>, pub reason: Option<String> }
```

An older daemon answers an unknown request with its usual error and the client prints it.

## 3. `src/session.rs` and `src/daemon.rs`

`src/session.rs`, two splits that change no behavior of reset:

1. `clear_stale_omp_draft` (`:925-943`) becomes a pure predicate `stale_omp_draft(&State) -> bool`
   (the test it already makes: OMP harness, connected, idle, no native draft, last human input
   older than the margin) plus the clearing, which calls the predicate.
2. `reset_gate_reason` (`:945`) becomes a pure `reset_gate_preview(&self) -> Option<(&'static str,
   String)>` that returns `(BUSY, "reset is already running")` when `resetting()`, treats a
   `HumanDraft` hold for which `stale_omp_draft` is true as no hold, and otherwise decides as
   `reset_gate_reason` does today (exited, hold, `harness::ready`, `reset_native_reason`);
   `reset_gate_reason` clears the stale draft and then calls the same decision without the busy
   test, so reset's own answers do not change (`begin_reset`, `:976-982`, still produces BUSY).
   `explain` calls the preview: it does not mutate, and it now agrees with what reset would
   decide at that instant.

`src/daemon.rs`: dispatch `Request::Explain` next to `Request::Screen` (`:2051`) with
`explain_of(runtime, id).await` beside `screen_of` (`:2297`); it is `async` because the pending
count comes from the store (`store.open_counts`, `:2166`), which is awaited before any session
lock is taken. An unknown id answers `unknown session {id}` as `screen_of` does. Lock discipline
(the spec 3co `starting` deadlock): every Session method that locks (`starting`, `needs_input`,
`exit_code`, the PTY channel's `hold_reason`, `session_activity`, `reset_gate_preview`) is called
before the single `session.lock()` of the handler or after it is released, never inside a live
guard; `exit_code` is read from that guard's `state.exit_code` if it is needed under it.

One helper computes the activity and its cause from one read, so the two cannot disagree:
`session_activity_cause(session) -> Option<(Activity, &'static str-or-String)>`; `session_activity`
(`:2520`) is rewritten to call it and return its first element. The causes, in the order the code
decides (`:2508-2531`): `held: <hold reason>` (a channel hold exists; busy), `reset running`
(`resetting()`; busy), `screen not ready` (working), `screen changed <n>s ago` (changed within
`ACTIVITY_QUIET`, 10 s; working), `screen quiet for <n>s` (idle). The code does not rank a hold
against a running reset (both are busy); this order is the display order only.

`hold` is chosen as `message_info` does (`:1462-1468`): `deliver_hold` when
`session.deliver() == Deliver::Hold`, otherwise the channel `hold_reason`, otherwise none;
`HoldInfo.explanation` is `messaging::hold_explanation(reason)`. The `needs_input` signal goes only
in `needs_input`, never in `hold` (note `list`'s HELD column counts it, `:2178-2180`, so the two
can differ). `activity`'s `held: <reason>` cause uses the channel hold only, as
`session_activity` does. `explain` reads readiness through `session_activity`, which calls
`observe_ready` and may latch `starting` to false, exactly as `list` does; the docs say so.

Trust boundary: `Request::Explain` is admin-only. The dispatcher allowlists the requests a session
token may send (`:2113-2125`); `Explain` is not on it, so a session token gets "not permitted for
a session token". If it is ever opened to peers it needs the visibility check `reset_session`
uses (`:1255`).

## 4. `src/cli.rs` and `src/main.rs`

`Command::Explain { session: String }` ("Explain why a session is in its current state", read
only). `run_explain` mirrors `run_screen` (`src/main.rs:1949`): resolve the reference
(`resolve_session`), request, print. Output, exactly, one `key: value` per line, no trailing
space. A sample for a Generic session in which a person has typed a draft:

```
s1 (worker@host-a): generic, running
activity: busy (held: human_draft)
ready: yes
needs input: no
hold: human_draft - A person typed in the recipient session. Delivery resumes when they submit or release it (prefix, then r).
queued: 2
reset: refused (draft_present): hold human_draft
```

- First line: `<id> (<address>): <harness>, running`; an exited session prints only
  `<id> (<address>): <harness>, exited (code <n>)`.
- `activity:` is `idle`, `working` or `busy` and the cause in parentheses.
- `ready:` is `yes` or `no`; ` (starting)` follows `no` when `starting`.
- `needs input:` is `no` or the signal text.
- `hold:` is `none`, or `<reason> - <explanation>` (just `<reason>` with no explanation).
- `queued:` is the open message count for the session.
- `reset:` is `gate open`, or `refused (<code>): <reason>`. "Gate open" is the answer at this
  instant; a reset can still fail at a later step.
Exit 0; 1 on an error response (an unknown name fails in `resolve_session` with its own error
text, which the test asserts).

## 5. `docs/cli.md`

Add `a2amx explain SESSION` to the command list with the sample of section 4 and one sentence per
line of what it reads. Say that it is read-only, admin-only (a session token is refused), that it
reports today's evidence and does not record where past activity came from, that looking can
latch `starting` as `list` does, and that `reset: gate open` is the gate's answer and not a
promise.

## 6. Tests

- `tests/wire.rs`: round trips of `Request::Explain`, of a `Response::Explain` with every field
  set, and of one with every optional field absent (the keys are omitted).
- `tests/explain.rs` (new), through the binary and a daemon in a temp home (copy the helper shape
  from `tests/team.rs`; a shared helper module is out of scope). A Generic session is ready only
  after it enables bracketed paste (`harness::ready`, `src/harness.rs:205-209`), so a plain `cat`
  is never ready: start the sessions with `sh -c "printf '\033[?2004h'; exec cat"` (the pattern at
  `tests/quota.rs:314`) and poll `explain` until the line `ready: yes` appears.
  1. A ready Generic session prints `ready: yes`, `needs input: no`, `hold: none`, `queued: 0`,
     `reset: gate open`; the `activity:` line starts with `activity: ` (the cause depends on the
     10 s window, do not assert it).
  2. The same command started with `--deliver hold` prints `hold: deliver_hold - ` followed by
     exactly `hold_explanation("deliver_hold")`, and `reset: gate open` (deliver mode does not
     gate reset: neither `reset_gate_reason` nor `begin_delivery` reads it).
  3. A plain `cat` session (never ready) prints `ready: no` and `reset: refused (not_ready):
     screen not ready`, and an `activity:` line with the cause `screen not ready`.
  4. An exited session (`sh -c "exit 0"`) prints exactly the exited line.
  5. An unknown name exits 1 and prints the error that `resolve_session` prints.
  6. `explain` is read only: `list` before and after shows the same `held`, `pending` and state
     (compare those fields; the uptime and activity-age columns change with time).
  7. A session token is refused: send `Request::Explain` through the library `Client` with a
     session token (the pattern of the existing "not permitted for a session token" test) and
     assert the error.
  8. The stale OMP draft case (kills the mutant that calls `reset_gate_reason`): copy the shape of
     `tests/broker.rs:1988` `reset_stale_omp_draft_hold_clears_after_margin` (it uses
     `omp_reset_target`); wait past the 2 s margin; send `Request::Explain` twice; both replies
     show `reset: gate open`, and `list` still shows the session held `human_draft` (explain did
     not clear it).

## 7. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| `Request::Screen`/`Response::Screen` and `screen_of` are the model | `src/wire.rs:200, 282`, `src/daemon.rs:2051, 2297-2311`, `src/main.rs:1949-1968` | read |
| A Generic session is ready only with bracketed paste on and not scrolled; `cat` never enables it | `src/harness.rs:205-209` | read (consultant) |
| `reset_gate_reason` clears a stale OMP draft first, then refuses on exited, a hold, an unready screen, the native bridge | `src/session.rs:925-962` | read (consultant, architect) |
| `begin_reset` produces the BUSY answer | `src/session.rs:976-982` | read (consultant) |
| `deliver_hold` comes from `session.deliver() == Hold`, not from a channel hold; `PtyChannel::hold_reason` is the PTY hold; the native channel's is none | `src/daemon.rs:1193, 1465`, `src/delivery.rs:142-144, 198-200` | read (consultant) |
| `session_activity` is busy for a channel hold or a reset, working when not ready or changed within 10 s, else idle | `src/daemon.rs:2508-2531` | read |
| The pending count is awaited from the store before the session lock | `src/daemon.rs:2166` | read (consultant) |
| The dispatcher allowlists session-token requests; `Explain` is not on it | `src/daemon.rs:2113-2125` | read (consultant) |
| `--deliver hold` is the clap value of `Deliver::Hold` | `src/harness.rs:35-38`, `src/cli.rs:55` | read (consultant) |
| The only exhaustive `match` on `Request` is `src/daemon.rs:2031`; `mcp.rs`, `hook.rs`, `daemon.rs:343` and the `main.rs` matches have wildcard arms | grep | consultant; the compiler catches the rest |
| `list_sessions` reads the locking methods before taking the lock | `src/daemon.rs:2165-2210` and spec 3co Amendment 1 | read |
| No existing `explain` command | `rg -n 'explain' src` finds only `suggest::explain` | grep |
| Neither reset gate nor `begin_delivery` reads the deliver mode | `src/session.rs:945-962, 1065-1077` | read (consultant) |

## 8. Out of scope

Recording where activity events came from; the MCP tools; the OMP extension; message-level
explanation (`messages` already shows per-message state); changing any hold reason or text; showing
screen lines (`screen` does that); `CHANGELOG.md` and the backlog row (the architect).

## 9. Acceptance

1. `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   pass. Judge: the architect reruns them.
2. Red phase: `tests/explain.rs` and the wire tests fail before the code (compile or assert). The
   developer names the tests that pass on the old code (none expected).
3. With the built binary and a temp home: `a2amx new --detach --name worker -- sh -c "printf
   '\033[?2004h'; exec cat"`, then `a2amx explain worker` prints the section 4 lines with
   `ready: yes` and `reset: gate open`; a `cat` session prints `reset: refused (not_ready)`.
   Judge: the tester.
4. Mutation check by the developer, restored by checksum: (a) `explain_of` calls
   `reset_gate_reason`: test 8 fails; (b) `hold` takes the channel hold only (drop
   `deliver_hold`): test 2 fails; (c) print `refused` when the verdict is allowed: tests 1 and 2
   fail; (d) drop the explanation text: test 2 fails; (e) allow `Explain` for a session token:
   test 7 fails; (f) show `ready: unknown` for Generic: tests 1 and 3 fail; (g) in the preview skip
   the `resetting()` busy answer: no test can run a reset step, so report it as a survivor unless
   a cheap fixture exists.
5. Public interface: a new CLI command and a new request/response pair; the changelog line is the
   architect's.
6. `git status --short` shows only files in scope. Scratch only under `target/scratch/3cp/`.
