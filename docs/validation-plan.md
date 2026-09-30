# Validation plan

Status: checks to run while building the first slices. The terminal core, the single-machine messaging core, and the Claude Code hook are implemented (see architecture); [recorded results](#recorded-results) lists what has been observed so far.

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
| Large paste placeholder | A long envelope or draft shown as a placeholder still reaches the hook as full text on every harness; the size at which each harness collapses a paste is recorded in its adapter profile |
| Clear-draft candidate (later) | Many Delete keys then many Backspace keys empty the composer from any cursor position, including a multi-line draft and a collapsed paste, between turns and mid-turn, and do nothing on an empty composer; the key count comes from the characters A2AMX saw typed, counting each large paste once, and a draft above a cap is held instead of cleared |
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

These checks confirm the chosen emulator (`alacritty_terminal`). Replay captured
output from each target harness through it and compare the resulting screens with a
reference such as tmux's pane capture. Do not substitute screenshots alone for input
and protocol checks.

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

## Recorded results

Observations so far. Each names the harness version and how it was observed; none has
been run against a live pair of agents yet.

| Check | Harness | Result |
| --- | --- | --- |
| Paste and submit timing | Claude Code 2.1.285 | A `CR` in the same write as a bracketed paste does not submit; a `CR` written separately 300 ms or more later does, including mid-turn, where the message is queued |
| Readiness screens | Claude Code 2.1.285 | The trust dialog enables bracketed paste, so the screen check (composer rules, no dialog footer) is required; the composer stays visible while a turn runs |
| Corrupted submission: block behavior | Claude Code 2.1.285, 2.1.286 | A hook block stops the prompt, empties the composer, and shows the original prompt in the transcript |
| Large paste placeholder | Claude Code 2.1.285, 2.1.286 | The hook receives the full text wrapped in `<pasted_content id="...">` tags with a random id. Every multi-line paste and every single line from about 1000 characters is wrapped; single lines up to 686 characters are not. The exact single-line threshold is not pinned |
| Mid-turn delivery and receipts | Claude Code 2.1.285 | A message submitted during a turn is queued and the hook fires at submit time with the active turn's `prompt_id`, so matching uses the A2AMX id |
| Mid-turn wrapper shape | Claude Code 2.1.286 | A paste queued mid-turn reaches the hook with the prompt's ends trimmed: no blank line before the opening tag and no newline after the closing tag. An idle submit keeps both |
| Pasted tab | Claude Code 2.1.285, 2.1.286 | Each tab reaches the hook as four spaces; blank lines, indentation, trailing spaces, Unicode, and shell-looking text arrive unchanged |
| Composer glyph | Claude Code 2.1.286 | The composer is drawn as `❯` followed by U+00A0, not a plain space; a readiness check that requires a plain space never sees a ready composer |
| Two live agents, stress run | Claude Code 2.1.286 | 30 delivery attempts: 21 submitted, all with a hook receipt; the 9 rejections belong to three messages that contain a tab. Bursts, detach and attach while messaging, and awkward payloads (Unicode, shell-looking text, blank lines, indentation) delivered intact |
| Hook timeout | Claude Code 2.1.285 | A hook that exceeds its timeout is cut off, the prompt proceeds, and a notice is shown |
| Raw-typed envelope | Claude Code 2.1.286 | Small paced chunks arrive unwrapped; one fast raw write is collapsed like a paste |
| Duplicate and foreign receipts | automated tests | A repeated receipt is idempotent; a receipt from another session is ignored |

Not yet checked: any of the above on Codex or OMP (block behavior on Codex in
particular), a real run with two live agents messaging each other, and the cross-host
sections of this plan.

## Repository checks

The repository has a build and test suite. Before a change is done, `cargo test`,
`cargo clippy --all-targets -- -D warnings`, and `cargo fmt --check` must pass (see
[AGENTS.md](../AGENTS.md)). Documentation changes should also:

- Pass `git diff --check` and review of new files as well as tracked diffs.
- Have resolvable relative links and consistent requirement/proposal labels.
- Contain no personal paths, real infrastructure details, credentials, private
  transcripts, or memory exports.
- Avoid presenting planned commands or compatibility as implemented.
