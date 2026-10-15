# Backlog

Status: the running list of open known items, issues, untested behavior, ideas, and
features. It is a tracker, not a design: a decision belongs in [architecture](architecture.md)
or [delivery](delivery.md), and an implementation slice gets a locked spec under
`docs/specs/`. Check an item against the code before acting on it, because entries can
lag the code.

## How to use this file

- One row per item, newest at the bottom of its section. Give each item a stable ID.
- Evidence column: say how the state was learned (`code`, `docs`, `manual`, `reported`).
  `reported` means the owner said so and it was not re-run.
- When an item is done, move it to [Closed](#closed) with the commit or spec that closed it.
- `// shortcut:` comments in the code are the ledger of deliberate corners; this file
  lists only the ones worth scheduling.

## Known gaps (documented as not built)

| ID | Item | Evidence |
| --- | --- | --- |
| G1 | Agent-initiated launch (an agent starting another session). Out of scope for now (owner, 2026-10-04); revisit later | docs |
| G3 | Cross-host support: central broker, remote launch, cross-host delivery. Deferred until the owner says it is needed | docs |
| G7 | Expose OMP native idle, pending, in-flight, and waiting flags in `list --details` | docs |
| G8 | Apply stale-human-draft clearing to OMP message delivery; reset-only clearing intentionally does not call `begin_delivery` | docs |
| G9 | Show OMP `human_draft` in `list`: `NativeChannel::hold_reason()` is always empty, so the current list and picker cannot expose that hold | code |

## Untested or unverified

| ID | Item | Evidence |
| --- | --- | --- |
| U4 | Real-Codex smoke test as an ignored test, like the OMP ones | docs |
| U6 | A human typing exactly inside the paste-to-Enter window to force a real corrupted submission | docs |
| U7 | Block behavior on Codex in the validation plan's recorded results | docs |
| U8 | User-supplied `--mcp-config`, `--allowedTools`, and `--append-system-prompt` duplicate-flag behavior is unprobed (`--append-system-prompt` probably last-wins) | code |

## Known issues

| ID | Item | Evidence |
| --- | --- | --- |
| I3 | A split escape sequence can hold a session; the paste-then-`CR` gap is one fixed constant; an unreadable PTY can hold the writer gate; message bodies are stored as plaintext | docs |
| I7 | Flaky test: `tests/attach_cli.rs::attach_does_not_capture_session_already_in_drag_mode` fails now and then with an unexpected `ESC[?1000h` in the attached output (line 254); passes 6 of 6 when run alone. Likely a race: the attach client enables wheel capture before it learns the session is already in drag mode | manual |
| I8 | The daemon sometimes does not deliver a message to an OMP session, and sometimes does not run the reset. Seen 2026-10-14: `reset_session` on the tester was rejected as `busy` (`idle=true in_flight=true`, idle for over two hours) and a HOLD notice sat `pending` ("an earlier message is queued in the recipient and has not entered its conversation yet") until the owner reset the session by hand; `message_status` then showed `native_receipt`. The owner says it is a real a2amx bug and has happened before with the OMP harness; a restart of the team usually postpones it. Investigated 2026-10-14: the daemon only mirrors the extension; OMP asides do not count as pending (so the first hypothesis was wrong); cause is either OMP's queued-message count staying nonzero or the extension going silent. Spec 3af adds `queue`/`host_pending` to the state frame, a 30 s keepalive and an `ext_queue`/`ext_pending` suffix on the busy reset rejection; the fix follows the next occurrence. Recurred 2026-10-15 on the tester with the 3af build: `reset_session` was rejected `idle=true in_flight=true waiting=false idle_for=1056s reported=4s ago ext_queue=0 ext_pending=true`, a queued `ASSIGN` stayed `pending/in_flight`, and `a2amx screen` showed an empty idle prompt. So the extension is alive (keepalive works), its own queue is empty, and OMP's `ctx.hasPendingMessages()` (`queuedMessageCount > 0`: steering queue + follow-up queue + next-turn messages, read from the OMP 18.10.4 binary) is true while the harness is idle. The owner confirms that submitting any text in the composer releases the message, so a new turn clears it. Cause found in the tester's OMP session log: the queued item was OMP's own `todo-error-reminder` custom message ("todo failed, so todo progress is not visible to the user. Failure: Invalid todo arguments..."), created at 23:50:21Z after a failed `todo` tool call, held in OMP's next-turn queue for about 1.5 hours, and delivered only when a new turn started at 01:23Z (the owner typed a `.`); the OMP advisor was ruled out. So `idle && hasPendingMessages()` means "waiting for the next turn", not "in flight", and a failed `todo` call by the model is enough to trigger it. Candidate fix: the extension reports `pending` from the host only while the harness is busy (`fifo.length > 0 || (hasPending && !idle)`), because an idle host's queued item does not start a turn on its own and delivery to an idle harness starts one | fixed by spec 3ai (the daemon treats `idle && queue == 0 && host_pending` as not in flight); real-OMP proof pending |
| I9 | Flaky test: `tests/daemon.rs::exited_claude_session_never_reports_needs_input` failed once in an invalidated full run (deadline timeout family) and passed its focused rerun and later full runs | manual |
| I11 | `validate_message` rejects a body containing the closing tag `</a2amx-message>` but not one containing an opening tag `<a2amx-message id="m_<k>"`. `envelope_ids` then returns a second id for such a prompt. Today that fails closed (the channel receipt path needs exactly one id and an exact envelope match), but the hook path's behavior with a forged id for another open message is unchecked. Decision needed: reject the opening tag too (as the terminator is), accepting that a message can no longer quote it, or leave it and document the reasoning. Surfaced by the consultant, 2026-10-15; spec 3ag pins today's behavior | code |
| I12 | The module doc at the top of `src/wire.rs` (lines 1-5) lists only `Data`/`Exit` server frames and `Input`/`Resize`/`Detach` client frames; the code also has `Detached`, `Status`, `Redraw`, `Scroll` and `Release`. Update the doc comment. Found while writing spec 3ag, 2026-10-15 | code |
| I13 | Tests inherit the ambient environment of the session that runs them. `tests/omp_wiring.rs::generic_sessions_get_no_a2amx_bin` fails when `A2AMX_BIN`, `A2AMX_ADDR`, `A2AMX_TOKEN` or `NO_COLOR` are set (as they are inside an OMP session), so `cargo test` passes for the architect and fails for the developer unless run with `env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR`. The test daemon and its launches should clear `A2AMX_*` and `NO_COLOR` in `tests/common`. Reported by the developer, 2026-10-15 (spec 3ag) | manual |
| I14 | Flaky test: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` failed once in a full run (assert at line 153, the scroll banner was not on screen) and passed a single rerun and the next full run; the same one-off was seen at the spec 3o wrap-up (2026-10-07). PTY timing, root cause unconfirmed. Reported by the tester, 2026-10-15 | manual |
| I15 | Coverage of subprocess tests that kill their child: `src/channel.rs` shows 0% and `src/mcp.rs` about 21% in the Sonar report although `tests/claude_channel.rs` and `tests/mcp.rs` exercise them, probably because a child ended by a kill signal writes no `.profraw`. `tests/hook.rs` had the same symptom from `env_clear()` dropping `LLVM_PROFILE_FILE` (fixed 2026-10-15, `src/hook.rs` 0% to 83%). Check how those tests end their children (graceful stdin close versus kill) before changing anything | manual |
| I16 | In an isolated temp `A2AMX_HOME` (installed binary, repo cwd) a text typed through the PTY into a `--harness claude` session (Claude Code 2.1.301) stayed in the composer after Enter, so the agent never sent a message and the README example could not show an exchange. The banner `server:a2amx · no MCP server configured with that name` was on the screen, but it is cosmetic (a live round trip proved the channel works, 2026-10-03), so it is not the cause. From an empty cwd the same launch stopped at Claude's workspace-trust dialog (F2). Not diagnosed: the driving script's Enter, an unanswered dialog, or an isolated-home effect. Reported by the developer while capturing the README example, 2026-10-16 | reported |

## Ideas and possible features

| ID | Item | Evidence |
| --- | --- | --- |
| F2 | Named launch templates in an operator config file, launch ownership (a launcher may kill only what it launched), trusted working directory so the Claude trust dialog cannot block unattended delivery. Out of scope for now with G1 (owner, 2026-10-04) | docs |
| F6 | Session activity feed for watchers, so an operator can build an advisor-style peer (as OMP's advisor does inside its harness). A mechanism, not a role: who watches and what a note means is the operator's policy, and what a feed carries is documented rather than gated. Shape: a richer `watch` (turn finished, activity digest) through the ordinary queue and hold gates; Claude Code hooks (`Stop` with its transcript path, `PostToolUse`) are the likely source, while OMP and Codex would need feeds from the extension and the app-server. Open: volume against the bounded queue, and file-change attribution when sessions share a directory (hooks give declared changes only; `Bash` is opaque). Parked by the owner, 2026-10-11; exploratory | reported |
| F10 | `proptest` properties for the pure modules (`wire` framing, `prefix`, `messaging` validation, `names`, `picker` model); `cargo-fuzz` on `wire` and `messaging` parsing as a follow-up | reported (owner, 2026-10-14) |
| F11 | `cargo-mutants` baseline on the pure modules, scoped per module because a full test run takes minutes. Used to find weak tests, not published | reported (owner, 2026-10-14) |
| F12 | Heartbeat that shows what a busy peer is doing: the 30-minute heartbeat says only "busy for 43m, last message 30m ago", so an architect must run `a2amx screen <peer>` to learn whether the developer is progressing. Include the last few lines of the peer's screen (or the harness's own todo/status line) in the heartbeat for watched busy peers. Reported by the architect, 2026-10-15 | reported |
| F13 | Record a real two-agent message exchange (Claude Code and OMP) for the README example: a `vhs` or asciinema script checked into the repo, so it can be re-recorded after a CLI change. The README example today shows only the real `a2amx list` output, because the exchange could not be captured unattended (I16). Raised during spec 3ak, 2026-10-16 | reported |

## Closed

| ID | Item | Closed by |
| --- | --- | --- |
| C1 | Claude Code, OMP and live multi-agent spec delivery across harnesses | reported: used live to deliver several specs through agents on different harnesses |
| C2 | Scroll in an attached OMP session | reported: wheel scrolling works after spec 2q (`0a4a9d9`) |
| C3 | Status text claiming Codex and OMP receipts or delivery profiles were not built | `94c0672` |
| C4 | Mid-turn message receiving on OMP (`aside`) and on Codex, with real harnesses | reported: tested by the owner |
| C5 | Configured session reset with consent, guarded PTY submission, CLI and MCP surfaces; completion markers remain per-harness work and Claude SessionStart after `/clear` is unverified | spec 2s |
| C6 | Agent activity (`idle`, `working`, `busy`) in `list` and `list_agents`, and activity-aware heartbeat busy detection | spec 2y |
| I1 | Exited sessions keep their name until `a2amx kill`, so a new session cannot reuse it | spec 2z |
| I2 | OMP install and extension cleanup/error handling nits | spec 3a |
| I4 | SonarQube Cloud S3776 (cognitive complexity over 30) on command logic: `src/team.rs` and `src/main.rs` split into helpers, behavior unchanged | specs 3x (`6672971`) and 3y (`9ac581a`) |
| I5 | Same S3776 finding on the large loops and state machines: `src/picker.rs`, `src/session.rs`, and `src/daemon.rs` `create_session`, `heartbeat`, `report_prompt`, `serve` split into helpers, behavior unchanged | specs 3z, 3aa (`9141e7c`), 3ab (`28cf1b8`), 3ac (`7ee110b`), 3ad (`288e15f`); Sonar scan of `develop` at `457a500` has no open S3776 issue and the gate is `OK` |
| I10 | SonarQube Cloud `rust:S1612`: all 26 `unwrap_or_else` closures that only call `into_inner` in `src/` now use `PoisonError::into_inner`, behavior unchanged | spec 3ae (`7913a91`); Sonar scan of `develop` at `fd5dca9` has no open issue and the gate is `OK` |
| F4 | Bulk cleanup of exited sessions, for example `a2amx kill --exited` | spec 2z |
| U5 | Limits at the edges: 50 open messages per recipient, 20 sends per minute per sender, bodies near 32 KiB, recipient exiting with an open message, daemon restart with open messages | spec 3b |
| C7 | Attach rendering no longer erases a glyph written in the terminal's last column | spec 3c |
| C8 | ACTIVITY column in the Ctrl-b w session picker, including running/exited cells and CWD clipping coverage | spec 3d |
| G2 | Prompt-submit hook safety net for Codex and OMP | dropped by the owner, 2026-10-04: receipts already come from the native channels |
| G4 | Codex: restart a dead app-server | by design, 2026-10-04: a session whose app-server dies is crashed, and probing showed the Codex screen exits and the session ends with `exited(1)` |
| G5 | Codex: resume a conversation by re-attaching to an existing thread | not needed, 2026-10-04: sessions start fresh and SALTMDB carries the memory across them |
| G6 | Codex: delivery through the PTY channel | won't do while native delivery works well (owner, 2026-10-04); reopen if a problem shows up |
| I5 | Claude readiness during the Press Ctrl-D again footer | not needed, 2026-10-04: spec 2u sends both keys without a gate and graceful exit works; reopen only if a gate is added |
| F5 | Delete `scripts/screen-probe.py` | `a2amx screen` confirmed working on a live OMP session, 2026-10-04; script removed |
| C9 | Richer attach status bar | spec 3e |
| C10 | Separator row above the attach status bar | spec 3f |
| C11 | Team-file name prefix and a guard for a one-string command | spec 3g |
| C12 | Optional role label shown to the session itself | spec 3h |
| C13 | UPTIME and IN-STATE columns in `a2amx list` | spec 3i |
| C14 | OMP quota percentages from `omp usage --json` | spec 3j |
| C15 | User `--settings` is merged into the Claude launch instead of overriding it | spec 3k |
| U1 | `developer_instructions` taking effect in Codex | verified by the tester on an isolated daemon, 2026-10-04: the default session's model states the peer-authorization instruction and a `--no-authorize-peers` session reports none (Codex 0.160.0, one model; not a version matrix) |
| U2 | Two simultaneous Codex sessions; `/new` thread switching | verified by the tester, 2026-10-04: a message reached only the receiver's own thread with a native receipt, and after `/new` the next message landed in the new thread only |
| C16 | Private teams, visibility-aware `list_agents`, team-aware list/picker/status, and `list --team`/`--as` filters | spec 3l |
| C17 | `team status`, `team up --dry-run`, and the lost-message restart note | spec 3m |
| C18 | Agent-level allow between private teams | spec 3n |
| C19 | Team-wide ordered context reset with attached-session safety and continuation on refusal | spec 3o |
| C20 | Needs-input label for stuck sessions (Claude screen heuristic, Codex protocol signal); OMP and generic sessions have no signal | spec 3p |
| C21 | A session with a role and no name uses the role as its name | spec 3q |
| C22 | `a2amx new` without `--name` generates a name | spec 3r |
| C23 | Build version stamped into the binary; the daemon reports it in daemon status | spec 3s |
| C24 | Session picker scrolls and takes page/home/end keys | spec 3t |
| C25 | Session picker groups sessions into foldable team sections and filters | spec 3u |
| F9 | Verify or drop the `rust-version` claim | checked 2026-10-14: the declared 1.85 did not build (`build.rs:73` uses a let-chain, stable from 1.88); `rust-version` is now 1.88 and CI job `msrv` checks it on every push |
| F7 | Pre-push gate for secrets, dependency policy and workflow mistakes | `scripts/prepush-fast.sh` (Gitleaks, `cargo-deny` with `deny.toml`, `actionlint`, `zizmor`) and CI job `hygiene` running the same script; full history scanned once on 2026-10-14, no findings |
| F8 | Repo basics for outside readers | `SECURITY.md` (GitHub private vulnerability reporting, enabled by the owner 2026-10-15), `CONTRIBUTING.md` (issues and third-party pull requests welcome, owner 2026-10-15) and `CHANGELOG.md` |
| I6 | Flaky `picker_switches_sessions_and_exit_status_is_reported`: the 80-column header clipped `QUOTA` when two generated names were long (3 of 20 failures before, 6 of 20 during spec 3y) | test now names its sessions `aa` and `bb`; 25 of 25 passes (test-only change, 2026-10-14) |
