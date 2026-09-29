# Message delivery

Status: proposed semantics. This document defines the delivery contract to build
and check, not implemented behavior.

## Guarantees and limits

The desired contract is durable central acceptance, explicit recipient identity,
noninterleaving input, and evidence of submission where a supported hook exists.
Unfinished human input must remain intact. When safe delivery cannot be established,
retain the message and expose why it is pending.

PTY input is a byte stream. A successful write does not establish that input
reached the chat composer. A submission hook can observe a prompt before other
hooks or harness logic reject it. Neither proves model processing or compliance.

There is no exactly-once processing guarantee. A screen that appears ready can
change before injection; an observation-only adapter cannot eliminate that race.
If testing cannot enforce the safety contract, restrict automatic delivery or
require cooperative harness support rather than claiming universal safety.

## Message and attempt identity

Keep an immutable message ID and a separate ID for each intentional delivery
attempt. Bind attempts to the recipient session and process incarnation. Display
aliases are not routing authority. Derive the sender through authenticated MCP
session binding.

The broker creates a provenance envelope containing the message identifier, sender,
subject, and body. Only the message identifier is visible in the envelope; the
supervisor matches the whole envelope text against the recorded attempt. Proposal:

```text
<a2amx-message id="m_01J..." from="agent-plan@host-a" subject="Parser issue">
From another agent, not your user. To reply: send_message(to="agent-plan@host-a").

I found the regression in parser.py.
</a2amx-message>
```

The `from` value is directly usable as the `to` argument. The envelope explains
itself because tool descriptions may not be loaded in the recipient when a message
arrives. Escape metadata and
validate the body so control characters, embedded paste terminators, or crafted
delimiters cannot become terminal commands or spoof envelope structure.

This validation applies to agent payloads, not transparent human keystrokes. An
attribution label helps the receiving model interpret origin but does not create
a separate model-role security boundary.

## Acceptance and delivery path

1. Authenticate the sender; resolve and authorize the immutable recipient.
2. Validate size/content limits and commit the message centrally.
3. Return an ID and acceptance status to the sender without waiting for delivery.
4. Route the pending message to the recipient's host supervisor.
5. Check ownership and harness readiness; hold when either is uncertain.
6. Record an attempt locally before starting the PTY side effect.
7. Serialize the supported paste/submission sequence and record write progress.
8. Validate a matching hook receipt, persist it locally, and report it centrally.
9. Reconcile retained evidence after disconnects until centrally acknowledged.

Do not hold a database transaction across PTY I/O or a hook callback. The broker
must service callbacks while delivery is in progress; otherwise a synchronous
hook and a waiting sender can deadlock.

## Evidence model

These are semantic observations, not a final database schema or one linear enum.

| Evidence | Meaning |
| --- | --- |
| Accepted | Central commit succeeded |
| Pending | No permitted delivery attempt is currently proceeding |
| Attempt started | Input may have reached the recipient |
| Write complete | All intended bytes were accepted by the PTY transport |
| Submission observed | An authenticated hook reported the matching submitted envelope |
| Known rejection | A specific failure is established; retain its reason and any partial progress |
| Unknown outcome | Available evidence cannot establish what reached the harness |

Observed in a separate project: a prompt-submit hook or extension sees prompts typed
mid-turn on all three target harnesses. Claude Code and Codex report the active
turn's existing identifier rather than a new one; OMP has no message identifier and
may join queued messages into one prompt. Adapters must therefore match on A2AMX
identifiers, and paste wrapping and joined messages are handled per adapter.

Write progress and hook receipts arrive independently. A receipt may precede the
writer's completion event. Merge evidence without downgrading a confirmed receipt
when a late write event arrives. A timeout alone is not a known rejection.

Receipts must validate session incarnation, attempt ID, and submitted envelope
content. Match the whole supported message structure, not an ID substring in an
otherwise mixed prompt. Account only for tested harness transformations such as
paste wrappers. Use A2AMX identifiers rather than assuming one hook event per
harness turn ID.

Hooks should be bounded and idempotent, preserve the host harness's required exit
and output contract, and avoid printing receipt data into the conversation. Hook
failure must be visible without hanging normal interaction. Exact failure policy
and timing are adapter-specific decisions.

## Input arbitration

One logical writer owns each PTY. Proposed input categories are human bytes/paste,
agent message transactions, and terminal protocol replies. Mux commands are
consumed by A2AMX rather than forwarded as ordinary input.

Human composition ownership and harness readiness are separate gates. A human can
own a draft while the harness is idle, and an unattended harness can be displaying
an approval dialog.

Proposal: a session is marked dirty when a human types or pastes into it. Incoming
messages queue while it is dirty, and the client shows a pending count. The flag
clears when a hook observes a human prompt being submitted, which empties the
composer, or when the human uses an explicit release key after deleting or
abandoning the text. Idle time, detach, and controller loss are not proof of an
empty composer, so a dirty session stays held across detach and network reconnects.
The operator is responsible for not leaving unsent text behind.

A2AMX never sends Ctrl-C to clear a draft. Its meaning depends on the harness and
the turn state: it interrupts a running turn in Claude Code even when text is in the
composer, opens a cancel/background dialog in Codex mid-turn with an empty
composer, and does nothing mid-turn in OMP. A per-harness clear-draft action, gated
on knowing the harness is idle, is a later optimization.

### Corrupted submissions

The dirty flag can be wrong, for example when text is typed just as a message is
pasted. For harnesses with a prompt-submit hook, the hook is a safety net. It sends
the submitted text to the supervisor and obeys the verdict:

- The prompt is exactly a recorded envelope, or contains no envelope: allow it.
- The prompt contains an envelope's identifier but is not exactly that envelope
  (a human draft was merged in, or characters were interleaved): it is corrupted.
  Block it, or replace it where the harness allows. Record a known rejection for the
  attempt and queue the envelope again as a new attempt. The supervisor keeps the
  full submitted text, so it restores the human's draft to the composer later,
  pasted without Enter, once the session is idle.

The model never sees the corrupted prompt, and no human text is lost. What each hook
can do, from the harness documentation or source at the time of writing:

| Harness | Rewrite the prompt | Block the prompt |
| --- | --- | --- |
| Claude Code | No; it can only add context | Yes; the prompt is erased from the input box |
| Codex | No; it can only add context | Yes; the documentation does not say what happens to the text |
| OMP | Yes; the extension `input` handler can return replacement text | Yes |

Limits: this does not help with approval dialogs, because no prompt is submitted.
The hook fails open if the supervisor is unreachable. A harness without a working
hook depends on the dirty-flag hold alone. The hook is synchronous, so it must answer
quickly.

The adapter determines supported submission behavior and evidence of readiness.
Unknown state holds delivery. Bracketed-paste support is useful framing, not a
readiness signal or a submission acknowledgment. Avoid universal clear-line,
Ctrl+C, or submit sequences whose meaning changes between harness screens.

An agent transaction is atomic relative to competing application input, not to
kernel writes or harness interpretation. Partial writes and interruption require
progress accounting. Human input arriving during a transaction must be visibly
buffered under a bounded policy; overflow must not silently lose keystrokes.
Emergency interruption can abort delivery but leaves partial effects uncertain.
Do not resume an abandoned transaction's remaining bytes automatically.

Terminal replies must remain serviceable without corrupting paste framing or
deadlocking a harness waiting for a reply. The exact scheduling policy is part of
the implementation; a FIFO of arbitrary byte chunks is insufficient.

## Mid-turn delivery

Requirement: a message accepted for a busy recipient must be able to reach it during
its active turn. A planner may need to correct an implementer that is inside one
long turn.

Proposal: a harness adapter may offer more than one delivery channel:

- **PTY composer input.** Works with any harness. The risks in
  [input arbitration](#input-arbitration) apply, and what the harness does with
  input during a turn (steer, queue, interrupt, ignore) is adapter-specific.
- **Native in-harness channel.** A hook or extension asks the local supervisor for
  pending messages at a boundary the harness exposes, such as after a tool call, and
  returns them as context. It writes nothing to the terminal, so it cannot corrupt a
  draft or answer a dialog. Availability and semantics per harness are unverified.

A native channel is bounded by the next boundary. File writes are tool calls in the
target harnesses, but a long single tool call, a wait on a subagent, or a stretch of
generation without tools may produce no boundary. Whether a subagent's boundaries
reach the parent's hooks is open. The adapter declares which channels it supports,
and the broker shows the wait reason for a pending message.

Proposal for the MVP: implement only the PTY channel, since it is the one path that
works for any harness, including one running in a container (delivery needs only the
PTY; a container needs a route to the host supervisor only for sending and for
receipts). Keep the delivery channel an adapter concept so native channels can be
added later without a redesign. Submission receipts are optional per harness
profile: with a hook or extension the message reaches "submission observed", and
without one it stays "write complete, outcome unknown". Native channels, such as an
extension pushing messages into a live OMP session (its extension API appears to
allow this; source read, not run), are post-MVP optimizations that remove PTY risk
for that harness.

Evidence differs by channel. A native-channel handoff shows the harness asked for and
received the text, not that it was submitted through the composer or processed.
Name each receipt for what it proves.

## Ordering and backpressure

Proposal: one in-flight attempt per recipient, with pending messages ordered by
central acceptance sequence rather than host wall clocks. A blocked or unknown
head message pauses automatic delivery to that recipient until resolved. Other
recipients continue independently. Define explicit expiry/cancel/retry behavior
before relaxing this ordering.

Bound payload sizes, per-recipient queues, and global retained data. Reject new
acceptance explicitly when limits are reached. Rate limits should contain message
storms and agents repeatedly replying to each other. The broker does not decide
whether an agent's task is complete or generate replies itself.

## Recovery and retry

| Event | Proposed response |
| --- | --- |
| Target host unreachable before an attempt | Keep centrally accepted message pending |
| Network drops after local submission | Persist receipt locally and reconcile; avoid a new injection |
| Hook missing or times out | Expose uncertainty; do not interpret silence as nondelivery |
| Crash after attempt record but before recorded completion | Recover as unknown unless stronger evidence exists |
| Receipt committed centrally, reply lost | Replay receipt idempotently |
| Human attach connection lost | Revoke its control authority; preserve draft hold; do not replay uncertain keyboard bytes |
| New process reuses an alias | Keep old messages tied to their original target until explicit disposition |

The critical window is submission occurring before durable evidence is recorded.
A local journal reduces uncertainty across network outages but cannot atomically
commit a PTY side effect and a database transaction. Unknown attempts require
inspection or an explicit retry that warns of possible duplication.

Reconnections need epochs or equivalent fencing so an old supervisor connection
or controller cannot compete with its replacement. Reconcile message IDs, attempt
IDs, incarnation, and evidence before resuming delivery. The protocol is open.

An acceptance-idempotency key would deduplicate a retried send after a lost central
response. It cannot deduplicate model actions. Without such a mechanism or a status
lookup, report ambiguous acceptance instead of encouraging blind resend.

## Harness adapter responsibilities

| Generic runtime | Harness-specific adapter |
| --- | --- |
| PTY lifecycle, output draining, writer ownership | Readiness evidence and unsupported screen states |
| Message persistence, authentication, routing | Paste and submit behavior, including active turns |
| Attempt IDs and evidence reconciliation | Hook setup, parsing, timing, and failure contract |
| Human ownership and control fencing | Tested transformations of submitted text |
| Bounded queues and visible pending reasons | Version/capability compatibility profile |

Support is a tested combination of harness version, configuration, terminal
capabilities, and wrapper arrangement. A hook installed successfully is not proof
that active-turn delivery is safe. See the [validation plan](validation-plan.md).
