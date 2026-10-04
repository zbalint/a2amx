# Spec 3b: messaging limit edges, audit and the missing tests (backlog U5)

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `af690a2`; gate notes in section 5).
Backlog item U5. Decisions D1 to D3 are the architect's, marked **(architect)**, settled on the
owner's instruction of 2026-10-04 to write a spec for every planned item that needs no owner
input. This spec adds tests only; it changes no production code.

**Baseline:** develop at `af690a2`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-limit-edges`.
Public test seams: the wire `Request::SendMessage` and message queries against a real daemon
(`tests/broker.rs`, `tests/common`).

**Scope.** May edit exactly these files and no others.

- `tests/broker.rs`, and `tests/common/mod.rs` only if a helper is genuinely needed
- `docs/backlog.md` (move or shrink U5)

Does not touch: anything under `src/`. If a test shows a real defect, do not fix it here: stop and
report `BLOCKED — SPEC ADJUDICATION REQUIRED` with the failing evidence. No new dependency. Do
not commit, stage or merge.

## 1. Why

Backlog U5 lists five limit edges as untested: 50 open messages per recipient, 20 sends per
minute per sender, bodies near 32 KiB, a recipient exiting with an open message, and a daemon
restart with open messages. A read of `tests/broker.rs` at lock time shows most are already
covered, so the entry lags the code:

- queue limit and rate limit with small custom `Limits`: `tests/broker.rs` lines 589 to 763
  (`queue_full`, `rate_limited`), and the defaults in `tests/messaging.rs:24` to `26`;
- recipient exiting with an open message: `tests/broker.rs:860` to `866` (`recipient_exited`);
- daemon restart: `graceful_restart_marks_exited_and_continues_ids` (line 1051) and
  `abrupt_binary_restart_recovers_pending_as_daemon_restarted` (line 1413).

What is not covered end to end is a body at the size limit: `tests/messaging.rs:48` and `:71`
check `validate_message` at `MAX_MESSAGE_BYTES` and one byte over, but nothing sends that body
through the daemon and a real session. Two edges are also checked only with small custom limits,
never with the default numbers.

## 2. Decisions

- **D1. (architect)** First step, before writing any test: the developer confirms the coverage
  map above against the tree (read the cited tests) and reports any entry that is wrong. A
  covered edge gets no new test.
- **D2. (architect)** Add these tests, and only these, in `tests/broker.rs`:
  1. A body of exactly `MAX_MESSAGE_BYTES` (32 KiB, 32768 bytes of printable ASCII, no control
     characters) is accepted by the daemon, delivered to a `Generic` session with
     `Deliver::Auto` whose child echoes its input to a file or screen the test can read, and the
     message ends `submitted`. Where a literal comparison is too heavy, compare the received
     byte length (32768) and the first and last 16 bytes of the body.
  2. A body of `MAX_MESSAGE_BYTES + 1` is refused with failure code `too_large` through the wire
     (not only through `validate_message`), and nothing is queued.
  3. With the default `Limits`, a recipient held with `Deliver::Hold` accepts exactly 50 open
     messages and refuses the 51st with `queue_full`. Use two or more senders so the 20 per
     minute per sender limit is not the one that fires first, and say why in a comment.
  4. With the default `Limits`, one sender gets 20 sends accepted in a minute and the 21st
     refused with `rate_limited`; use several recipients so `queue_full` does not fire first.
- **D3. (architect)** Expected values are literals (`50`, `20`, `32768`, `"queue_full"`,
  `"rate_limited"`, `"too_large"`), never `Limits::default()` fields or `MAX_MESSAGE_BYTES`
  recomputed in the assertion. A 20-message burst must not need a real minute of waiting: if the
  rate window is wall-clock and cannot be driven faster, test 4 stays as the burst (20 accepted,
  the 21st refused, all within the window) and does not wait for it to reset.

## 3. Tests (failing first)

These are tests of existing behavior, so they may pass on first run. Write each, run it, and if it
passes, break the assertion once on purpose (a wrong literal) to confirm the test can fail, then
restore it. Report any that fail for a real reason as in section 0.

## 4. Docs

`docs/backlog.md`: replace U5 with the one edge, if any, the audit in D1 shows is still untested,
or move U5 to Closed with this spec.

## 5. Acceptance and gate notes

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass, `git diff --stat` shows only `tests/` and `docs/backlog.md`. Gate notes at lock time
(baseline `af690a2`): `MAX_MESSAGE_BYTES` is `32 * 1024` (`src/messaging.rs:12`) and is enforced at
`src/messaging.rs:307`; the default limits are 50 pending per recipient and 20 per minute
(`src/messaging.rs:79` to `80`); `tests/broker.rs` has the `session(...)` and `common::new_agent`
helpers the new tests use. The daemon-side check that a subject over 200 bytes yields `too_large`
is at `tests/broker.rs:631`. Whether the body size is checked again on the daemon's receive path
(as opposed to `validate_message` alone) is not verified at lock time; test 2 settles it, and a
failure there is a `BLOCKED` report.
