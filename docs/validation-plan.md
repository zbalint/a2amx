# Validation plan

Status: checks to run while building the first slices. No implementation exists yet.

PTY hosting, typing into a harness, and prompt-submit hooks are established practice
and are not re-proven here. The checks below cover what is specific to A2AMX: safe
injection, faithful attach and detach, and honest outcomes across hosts and failures.
Run them against the three target harnesses (Claude Code, Codex, OMP) as each slice
lands, and record the result in the design documents.

## Test hygiene

Use disposable Linux environments, fictional identities, synthetic prompts, and
explicitly enrolled test hosts. Avoid production systems and real credentials in
fixtures. Keep raw transcripts and runtime databases outside the repository. Record
the harness version, terminal, and wrapper with each result. Use event ordering and
explicit synchronization for race tests, not sleeps. Published artifacts contain only
synthetic data.

## Delivery safety

| Check | Required observation |
| --- | --- |
| Human partial draft | Incoming message stays pending; the draft is unchanged in the editor |
| Multiline, history recall, pasted draft | Ownership is not released by an Enter or idle heuristic |
| Approval, authentication, menu, or editor screen | No automated confirmation; unknown state holds delivery |
| Ready-to-dialog race | A forced transition between the readiness check and submission is prevented, or automatic delivery is marked unsupported in that state |
| Human input or interrupt during injection | Defined arbitration; no silent keystroke loss; partial outcomes preserved |
| Harness exit during write | Partial progress is visible; trailing bytes are not replayed |
| Envelope matching | Each envelope is matched, including several joined into one prompt and large pastes shown as placeholders; ID substrings in mixed prompts do not match |
| Corrupted submission | A prompt combining an envelope with a human draft is blocked (or replaced on OMP) before the model sees it; the envelope is queued again and the draft is restored to the composer; block behavior is verified per harness, including what Codex does with the blocked text |
| Control characters and forged framing | Supported text round-trips; forbidden controls, paste terminators, and forged envelopes are rejected |
| Duplicate, late, missing, or reordered receipts | Receipts merge idempotently; a missing receipt stays uncertain |
| Mid-turn delivery | A message sent during a long tool call, a subagent wait, and generation without tools is delivered and reported with its actual delay |

Any unpreventable approval or draft-corruption race blocks a claim of safe automatic
delivery in that state. Narrow support for that harness or change the mechanism.

## Terminal fidelity

| Check | Required observation |
| --- | --- |
| Alternate screen and reattach | Reattachment restores the screen without replaying historical effects |
| Output during snapshot | Snapshot plus updates has no missing or duplicated transitions |
| Resize while active or detached | Defined dimensions and a correct redraw |
| Detached terminal queries | The harness gets exactly one correct reply with no client attached |
| Keyboard modes and prefix | Ctrl/Alt, Shift+Tab, arrows, function keys, UTF-8, and supported extended modes pass through; fragmented escape and paste sequences are preserved |
| Session switching | Cursor, paste, mouse, focus, and keyboard modes do not leak between sessions |
| Slow viewer and output flood | PTY draining continues; memory is bounded; the viewer can resynchronize |
| Client exit or error | The client terminal is restored where recoverable; the hosted process keeps running |

Select the terminal emulator from this evidence. Do not substitute screenshots alone
for input and protocol checks.

## Cross-host routing and recovery

| Check | Required observation |
| --- | --- |
| Remote launch | A central request starts a session on a chosen host; failures surface with a reason |
| Bidirectional messages | Correct sender and recipient, ordered acceptance, matching receipts |
| Sender loses commit response | Acceptance is reported as ambiguous or reconciled idempotently; no blind duplicate |
| Target disconnected before delivery | The message stays pending and visible |
| Partition after submission, before central receipt | Local evidence reconciles without a fresh injection |
| Central restart | Local PTYs continue; directory and message evidence reconcile |
| Supervisor crash at each attempt boundary | Recovery distinguishes known evidence from unknown side effects |
| Harness restart or alias reuse | Old messages cannot silently target a new incarnation |
| Stale connection or two human controllers | Fencing rejects stale authority; exactly one controller has input; a disconnected draft stays protected |
| Terminal flood during messaging | Control traffic and receipts keep making progress |
| Disk full, queue full, SQLite busy | Explicit failure; no false durable acceptance or unbounded buffering |
| Forged sender, receipt, or control request | Rejected |
| Containerized harness and nested PTY wrapper | The MCP bridge and hook reach the host supervisor from inside the container; credentials stay out of arguments and logs; resize, signal, and disconnect behavior is recorded |

An accepted message stays discoverable with an honest outcome in every tested failure
window. Missing evidence is never treated as proof that a message was not submitted.
No exactly-once processing claim is made.

## Repository checks at the documentation stage

There is no application build or test suite yet. Documentation changes should:

- Pass `git diff --check` and review of new files as well as tracked diffs.
- Have resolvable relative links and consistent requirement/proposal labels.
- Contain no personal paths, real infrastructure details, credentials, private
  transcripts, or memory exports.
- Avoid presenting planned commands or compatibility as implemented.

Once implementation starts, add the actual build and test commands to the project
documentation and replace this list with the appropriate checks.
