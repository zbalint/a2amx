# Feasibility and validation plan

Status: planned experiments. **No experiment below has been run for A2AMX.**

The first milestone is evidence that terminal delivery can meet the intended
safety contract across two harnesses and two Linux hosts. A successful happy-path
message exchange alone is insufficient.

## Test environment and evidence

Use disposable Linux environments, fictional identities, synthetic prompts, and
explicitly enrolled test hosts. Use the three target harnesses (Claude Code,
Codex, OMP); their versions remain open. Avoid production systems and real credentials in
fixtures. Keep raw transcripts and runtime databases outside the repository.

For each experiment record:

- Exact harness version/configuration, Linux environment, outer terminal, and wrapper.
- Setup, injected event sequence, expected behavior, and observed behavior.
- Message, attempt, and incarnation correlations using synthetic identifiers.
- Whether evidence proves transport progress, submission observation, or more.
- A reproducible sanitized artifact and a pass/fail/unsupported conclusion.

Use event ordering and explicit synchronization for race tests; sleeps alone are
not evidence of correct ordering. Published artifacts must contain only synthetic
data and be reviewed for secrets and identifying details.

## Stage 1: local delivery and arbitration

Build only enough disposable instrumentation to own a PTY, send a synthetic
message, observe input/output, and receive a hook callback. Validate each harness
separately before combining them. Prototype interfaces are not production contracts.

| Case | Required observation |
| --- | --- |
| Idle prompt | Exact message submitted once; receipt matches the attempt |
| Active turn | Determine whether input steers, queues, interrupts, or is ignored; receipt timing is recorded |
| Long single tool call, subagent wait, generation without tools | Record when a message sent during each reaches the recipient, per channel |
| Native-channel boundary probe (informs post-MVP adapters) | Per harness, determine whether a hook or extension can return pending text as context at a tool boundary, and which activities produce one |
| Human partial draft | Incoming message stays pending; draft remains byte/content equivalent in the editor |
| Multiline, history recall, pasted draft | Ownership cannot be released by a simplistic Enter/idle heuristic |
| Approval/authentication/menu/editor screen | No automated confirmation or destructive input; unknown state holds delivery |
| Ready-to-dialog race | Force a transition between readiness check and submission; prove prevention or mark automatic delivery unsupported |
| Paste and submit timing | Split writes and fast bursts do not produce accidental submissions or dropped text |
| Unicode and control-bearing payload | Supported text round-trips; forbidden controls and forged framing are rejected |
| Duplicate, late, missing, reordered hooks | Receipts merge idempotently; missing receipt remains uncertain |
| Other hook blocks processing | Submission observation is not reported as model processing |
| Human input/interrupt during injection | Defined arbitration, no silent keystroke loss, partial outcomes preserved |
| Harness exit during write | Partial progress is visible; no automatic trailing-byte replay |

Completion: each target profile has documented submission semantics and a demonstrated
ownership policy. Any unpreventable approval or draft-corruption race blocks a
claim of safe automatic delivery in that state. Narrow support or revise the
mechanism before moving to a production implementation.

## Stage 2: terminal compatibility

Exercise attachment and detachment independently of messaging.

| Case | Required observation |
| --- | --- |
| Alternate screen and redraw | Reattachment reconstructs the correct screen without replaying historical effects |
| Output during snapshot | Snapshot plus updates has no missing or duplicated state transitions |
| Resize while active/detached | Defined dimensions, correct redraw, and no stale coordinate assumptions |
| Detached terminal queries | Harness receives one correct reply without an attached outer terminal |
| Keyboard modes and prefixes | Ctrl/Alt, Shift+Tab, arrows, function keys, UTF-8 and supported extended modes behave as advertised |
| Fragmented escape/paste sequences | Prefix recognition preserves payload and protocol framing |
| Session switching | Cursor, paste, mouse, focus, and keyboard modes do not leak between sessions |
| Slow/disconnected viewer and output flood | PTY draining continues; bounded memory; viewer can resynchronize |
| Client exit/error | Client terminal is restored where recoverable; hosted process remains running |

Completion: publish a bounded terminal capability profile and identify unsupported
features. Select the emulator from this evidence. Do not substitute screenshots
alone for input and protocol correctness checks.

## Stage 3: cross-host routing and recovery

Use one central daemon and two supervisors, with one harness on each host. Prove
bidirectional messaging and remote human attachment, then inject failures.

| Case | Required observation |
| --- | --- |
| Bidirectional active-turn messages | Correct sender/recipient, ordered acceptance, matching receipts |
| Sender loses commit response | Ambiguous acceptance or idempotent reconciliation; no blind duplicate send |
| Target disconnects before delivery | Central message remains pending and visible |
| Partition after submission, before central receipt | Local evidence reconciles without a fresh injection |
| Central restart | Local PTYs continue; directory and message evidence reconcile |
| Supervisor crash at each attempt boundary | Recovery distinguishes known evidence from unknown side effects |
| Harness restart / alias reuse | Old messages cannot silently target a new incarnation |
| Old connection survives reconnect | Fencing rejects stale control and delivery authority |
| Two competing human controllers | Exactly one has input authority; a disconnected draft stays protected |
| Terminal flood during messaging | Control and receipts make progress under defined bounds |
| Disk full, queue full, SQLite busy | Explicit failure; no false durable acceptance or unbounded buffering |
| Forged sender/receipt/control request | Unauthorized identities and operations are rejected |
| Remote launch | A central request starts a session on a chosen host; failures surface with a reason |
| Containerized harness | Its MCP bridge and hook reach the host supervisor from inside the container; credentials stay out of arguments and logs |
| Nested PTY wrapper | Record resize, signal, disconnect, and local MCP/hook connectivity behavior |

Completion: an accepted message remains discoverable with an honest outcome across
every tested failure window. Recovery never treats missing evidence as proof that
the message was not submitted. No exactly-once processing claim is inferred.

## Design decisions after the experiments

Review findings against [architecture](architecture.md) and [delivery](delivery.md).
Resolve the following before implementing the production protocol:

1. Supported harness profiles and permitted automatic-delivery states.
2. Human ownership transfer and pending/unknown message UX.
3. Hook validation, timeout, and receipt persistence behavior.
4. Enrollment, authorization, transport, reconnect fencing, and flow control.
5. Retry/idempotency rules and session-incarnation lifecycle.
6. Terminal capability contract and crate selection.

Record accepted choices and their evidence in the design documents. Leave failed
or unsupported cases visible rather than removing them from the matrix. Introduce
separate decision records only when alternatives and rationale warrant them.

## Repository checks at the documentation stage

There is no application build or test suite yet. Documentation changes should:

- Pass `git diff --check` and review of new files as well as tracked diffs.
- Have resolvable relative links and consistent requirement/proposal labels.
- Contain no personal paths, real infrastructure details, credentials, private
  transcripts, or memory exports.
- Avoid presenting planned commands, experiments, or compatibility as implemented.

Once implementation starts, add the actual build and test commands to the project
documentation and replace this limited check list with the appropriate checks.
