# Message delivery

Status: proposed semantics, with two implemented slices: single-machine PTY delivery
(see [Implemented: PTY delivery](#implemented-pty-delivery)) and, for Claude Code only,
the prompt-submit hook with submission receipts and the corrupted-submission verdict
(see [Implemented: Claude Code hook](#implemented-claude-code-hook)). Everything else
in this document, including hooks for other harnesses, defines the contract to build
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
  Block it, or replace it where the harness allows. Record a known rejection for
  the attempt and queue the envelope again as a new attempt while this message has
  fewer than two rejected attempts. On its second rejection, set the message to
  `undeliverable` with detail `unmatchable_submission`; do not retry it.
  Each rejection still counts toward the session's corrupted-submission count, so
  repeated bad messages can still reach the three-in-a-row session backstop.
  The supervisor keeps the full submitted text, so it restores the human's draft to
  the composer later, pasted without Enter, once the session is idle.

The model never sees the corrupted prompt, and no human text is lost. What each hook
can do, from the harness documentation or source at the time of writing:

| Harness | Rewrite the prompt | Block the prompt |
| --- | --- | --- |
| Claude Code | No; it can only add context | Yes; the prompt is erased from the input box (verified on 2.1.285 and 2.1.286) |
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
for that harness. The channel is now a trait (`Channel` in `src/delivery.rs`, spec 2d);
the PTY channel serves every harness and the native channel serves `omp` (spec 2e).

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

## Implemented: PTY delivery

Built from [spec 2](specs/spec-2-messaging.md); the spec is the contract, this
section is the summary. Without a hook a message never goes beyond "submitted, outcome
unknown"; the [Claude Code hook](#implemented-claude-code-hook) adds the receipt.

**Message states.** `pending`, `delivering`, `submitted`, `unsubmitted`, `cancelled`,
`undeliverable`. `pending` and `delivering` are open. Nothing expires automatically;
the human lists, cancels a pending message, or releases a hold. Each intentional
attempt is a row in `attempts`, recorded as `started` before any PTY byte is written.
A crash in that window recovers as `unknown`.

**Delivery transaction.** One task per session takes the lowest pending message and,
when no hold applies, runs this on the PTY writer with the input gate held:

1. Check that the session is alive, not held, and that the harness profile says the
   composer is ready.
2. Record the attempt, then write the envelope as one bracketed paste.
3. Wait a fixed 400 ms. A paste followed by `CR` in one write does not submit in
   Claude Code; a `CR` sent separately 300 ms or more later does, including
   mid-turn, where Claude Code queues the message.
4. Check readiness again. If a human typed or the screen changed, set a hold, send no
   `CR`, and record `unsubmitted` with the reason (`human_input` or
   `screen_not_ready`). Otherwise send `CR` as its own write and record `submitted`.

Human keystrokes wait behind the transaction on the bounded input queue and are never
dropped. Terminal query replies bypass the gate so they stay serviceable. A 1 second
cooldown follows each submission. An earlier unresolved message blocks later ones, so
order is acceptance order.

**Readiness.** `--harness claude` inspects the screen: bracketed paste on, not
scrolled back, cursor visible, `❯` followed by a space or a non-breaking space (Claude
Code 2.1.286 draws U+00A0) at the cursor row's start between two full-width rules above
and below, and no row containing `Enter to confirm` or `Esc to cancel`.
Bracketed paste alone is not enough because Claude Code keeps it enabled while a
dialog is open. `--harness generic` is ready when bracketed paste is on and the
session is not scrolled back. The default for `claude` is `--deliver auto` and for
`generic` is `--deliver hold`, which never injects.

**Holds and pending reasons.** The first that applies is reported as `hold_reason`:

| Reason | Meaning |
| --- | --- |
| `deliver_hold` | the session was started with `--deliver hold` |
| `unsubmitted_envelope` | an envelope was pasted but not submitted; it may still be in the composer |
| `corrupted_submissions` | three corrupted submissions in a row; only an explicit release clears it |
| `human_draft` | a human typed or pasted since the last release, or a blocked draft was restored |
| `queued` | an earlier message for this recipient is still open |
| `not_ready` | the harness profile says the composer is not ready |
| `cooldown` | a message was submitted less than a second ago |

The `corrupted_submissions` hold remains a session-level backstop: a session is
held after three corrupted submissions in a row, even when those submissions came
from different messages. A single message contributes at most two rejected
attempts; its second rejection makes it `undeliverable` with detail
`unmatchable_submission`, which is not an open state and therefore does not block
later messages by itself.

The dirty flag is set by human typing or pasting; focus reports and mouse reports do
not count. An explicit release clears it: the prefix then `r` while attached. Where a
hook exists (Claude Code), a hook-observed human submit clears it too. Detach does not
clear it.

**Not implemented.** Hooks for harnesses other than Claude Code, native in-harness
channels, cross-host routing, and Codex and OMP profiles.

## Implemented: Claude Code hook

Built from [spec 2b](specs/spec-2b-hooks-receipts.md), extended by
[spec 2c](specs/spec-2c-delivery-robustness.md) (wrapper-tag escaping, the per-message
rejection limit, sender visibility); the specs are the contract, this section is the
summary.

**Install and failure.** `a2amx new --harness claude` appends `--settings <json>`
holding one `UserPromptSubmit` command hook that runs `a2amx hook`. Nothing is written
to disk, and the hook reaches the daemon with the session's `A2AMX_ADDR` and
`A2AMX_TOKEN`, which the harness passes to it. The hook fails open: an unreachable
daemon, a bad payload, a prompt above 512 KiB, or a 4 second timeout all allow the
prompt, exit 0, and write one line to stderr. Its stdout is empty unless it blocks,
because hook stdout becomes conversation context on this event.

**Matching.** The daemon removes paste wrappers from the submitted text, finds
`<a2amx-message id="m_N">` tags, and compares the whole text with the envelope it would
render for that message. A message id counts only when the message is addressed to the
calling session, belongs to the current daemon run, is `delivering`, `submitted`, or
`unsubmitted`, and has no receipt yet; anything else is treated as a human prompt, so a
stale or foreign id never blocks typing.

| Submitted text | Verdict | Effect |
| --- | --- | --- |
| Exactly one known envelope | allow | Receipt recorded; the `human_draft` and `unsubmitted_envelope` holds clear; the corrupted-submission count resets |
| No known envelope | allow | The composer was emptied by a human submit: the `human_draft` and `unsubmitted_envelope` holds clear |
| A known envelope plus other text, or characters interleaved into it | block | Attempt recorded as `rejected`; the message goes back to `pending` for a new attempt until its second rejection, then becomes `undeliverable` with detail `unmatchable_submission`; the human's own text is restored as a paste without Enter once the session is idle, under a `human_draft` hold; interleaved text is not restored and stays in the transcript |

Three corrupted submissions in a row on one session stop the cycle with the
`corrupted_submissions` hold, cleared only by an explicit release. A block shows
`UserPromptSubmit operation blocked by hook` and the original prompt in the harness
transcript, and the model never sees the prompt.

**Evidence.** A receipt is stored on the attempt (`attempts.receipt_at`; schema version
2). `message_status`, `a2amx messages`, and the wire `MessageInfo` report `evidence`:
`write_complete` for a `submitted` message without a receipt and `submission_observed`
for one with a receipt. A missing receipt is never a failure. A receipt can arrive
before the writer records its own outcome; whichever lands first wins, and a late write
outcome never downgrades a receipt or overwrites a rejection.
A native channel reports `native_receipt` for a `submitted` message whose harness
reported it entering the conversation.

An accepted send may include `recipient_hold`: `deliver_hold` for a recipient
started with hold delivery, or the session-level hold reason when delivery is held.
`message_status` reports `hold_explanation` alongside a hold reason when one is
present. It also reports `accepted_at` and `updated_at` as Unix seconds; absent
optional values are `null`.

**Observed Claude Code behavior** (2.1.285 and 2.1.286, through a PTY with a logging
hook):

- The hook payload is a JSON object with `session_id`, `transcript_path`, `cwd`,
  `prompt_id`, `permission_mode`, `hook_event_name`, and `prompt`.
- A multi-line paste (see the threshold note below), or a single line of roughly 1000
  characters or more, shows in the composer as `[Pasted text #N]` and reaches the hook
  as a blank line, an opening
  `<pasted_content id="ID">` line, the text, and a closing `</pasted_content id="ID">`
  line. The id is random. When the paste is the whole prompt and Claude Code queues it
  mid-turn, the prompt's ends are trimmed: the blank line before the opening tag and the
  newline after the closing tag are missing, so the matcher accepts a wrapper at the
  start or end of the prompt without them. Single lines up to 686 characters arrive unwrapped; the exact collapse
  threshold is not pinned. Every delivered envelope has newlines, so the matcher
  unwraps first.
- Claude Code escapes its own wrapper tag when it appears in pasted text: the text `<`
  followed by `pasted_content` reaches the hook with a backslash after the `<`
  (`<\pasted_content`), and the closing form `</pasted_content` with the backslash
  between `<` and `/` (`<\/pasted_content`). This holds with or without attributes, in
  the middle of a line, and for any letter case; text that already has the backslash
  is not escaped again, and the bare word without a `<` is left alone. Other markup,
  including `<a2amx-message`, is not touched. The matcher applies this same
  wrapper escaping for Claude Code 2.1.286, so a message whose body contains
  wrapper-tag text remains matchable; senders may still avoid writing that text
  when they can. This behavior was observed with a blocking hook and bracketed
  pastes on Claude Code 2.1.286.
- A short paste with two newlines reached the hook unwrapped, and a paste with three
  newlines arrived wrapped, in the same probe; the collapse threshold is not measured.
  Delivered envelopes are long enough to be wrapped.
- Each tab in pasted text reaches the hook as four spaces (a fixed four, not aligned to tab
  stops). The Claude Code profile therefore compares the prompt with the envelope after
  the same expansion; without it a message containing a tab can never match and is
  blocked as corrupted on every attempt. Blank lines, indentation, trailing spaces,
  Unicode, and shell-looking text arrive unchanged.
- Input sent as raw keystrokes in small paced chunks arrives unwrapped, but one fast raw
  write is collapsed too, so typing an envelope is not a reliable way around the
  wrapper.
- A message submitted mid-turn is queued by Claude Code and the hook fires at submit
  time with the active turn's `prompt_id`, which is why matching uses the A2AMX id.
- A hook that exceeds its timeout is cut off, the prompt proceeds, and a notice is shown.

**Not implemented.** Hooks for Codex and OMP, cross-host receipts and replay, a
timeout or expiry for messages with no receipt, and restoring a pending draft across a
daemon restart.

## Implemented: native channel (OMP)

An `omp` session stays PTY-hosted for human use, but message delivery uses only the
native channel, never a silent PTY fallback. The extension and its launch wiring
ship in spec 2f; until then, an `omp` session's messages wait with `channel_down`.
Do not release spec 2e alone.

Three links carry the protocol. The OMP extension talks newline-delimited JSON
on the stdin and stdout of `a2amx omp-bridge`. The bridge authenticates with the
session's environment token and switches a daemon connection to length-prefixed
JSON bridge frames after `bridge_attach`. Tool requests ride the stdio link as
`mcp` lines, then use a second ordinary daemon connection through the existing
MCP server; responses return as `mcp` lines, and notifications get no response.
Frames and lines have the existing 1 MiB payload limit.

The extension first sends `hello`, with the bridge protocol integer, its OMP
version and any missing APIs. The daemon answers `ready`, or `refused` for a
protocol mismatch or missing APIs and closes the link. OMP's version is logged,
not enforced. `state` reports idle, queued-message and human-draft state.
`deliver` carries the message id and the same rendered envelope as the PTY
channel. `ack` means OMP queued it, giving `write_complete`; `nack` makes it
`undeliverable` with the supplied reason, without retry. A later `receipt`
reports an agent-attributed message entering the conversation with byte-exact
envelope text and upgrades evidence to `native_receipt`. No receipt is not a failure.

Native sessions have no human-cleared delivery hold. Their self-clearing waiting
reasons, in precedence order, are:

- `channel_refused`: the last hello was refused; update A2AMX or OMP to resolve the
  protocol mismatch or missing feature. A later accepted hello clears it.
- `channel_down`: no accepted extension is connected; wait for it to connect.
- `draft_present`: a person has unsent prompt-box text; wait until it is sent or cleared.
- `in_flight`: an earlier message is queued but has not entered the conversation.

Idleness never blocks delivery: messages arriving mid-turn use `followUp`, with
one message in flight at a time.

Bridge death has three cases. When no accepted bridge is attached when the loop
considers a message, it stays `pending` with `channel_down` or `channel_refused`.
When the bridge vanishes after `begin` claimed the message and before
`native_send` queued the frame, the message ends `unsubmitted` with detail
`bridge_disconnected`: visible, terminal, never resent. When it vanishes after
the frame was queued for writing, written or not, and before acknowledgement, the
message is `submitted` with `write_complete`, never resent. After acknowledgement
it is unchanged. No path returns a started attempt to `pending`; reconnecting
never causes a resend.

The bridge exits successfully on stdin EOF or refusal, and with an error on a
lost daemon link or invalid input. It does not reconnect or keep a replay cache;
the extension respawns it after an error and sends `hello` and `state` again.
Only one bridge may attach to a session; a second is rejected without disturbing
the first.

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
