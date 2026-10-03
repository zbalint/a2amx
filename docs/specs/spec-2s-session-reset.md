# Spec 2s: reset a session's context with a configured command sequence

## 0. Status

**LOCKED** (2026-10-03, pre-lock gate run against the tree at the baseline below). Decisions D1 to
D11 are the owner's answers of 2026-10-03 (reset sequences are configurable per session; the
target names who may trigger it; fail fast; OMP users may add `/prewalk restart`, a builtin OMP
command), plus the architect's design, grounded by a probe of real claude, codex and omp sessions
(section 3). Native message delivery is unchanged.

**Baseline:** develop at the commit that adds this spec (on top of `9771c4f`). **Location and
branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `a2amx-session-reset`. Public test seams: the `a2amx` CLI against a real daemon with
fake sessions (`tests/common`, as `tests/daemon_cli.rs` does), the broker path with a session token
(`tests/broker.rs`), the wire round trip (`tests/wire.rs`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `src/session.rs`, `src/daemon.rs`,
  `src/mcp.rs`, `src/messaging.rs` (error codes only)
- tests: `tests/daemon_cli.rs`, `tests/broker.rs`, `tests/wire.rs`, `tests/mcp.rs` (the exact
  tool list and schema assertions gain `reset_session`), `tests/team.rs` (the `TeamSession`
  literals gain the two new fields), and the mechanical
  `reset: vec![], control_from: vec![]` additions to every existing `NewSession` literal in
  `tests/attach_cli.rs`, `tests/claude_channel.rs`, `tests/codex.rs`, `tests/common/mod.rs`,
  `tests/daemon.rs`, `tests/delivery.rs`, `tests/hook.rs`, `tests/quota.rs`
- docs: `README.md`, `docs/architecture.md`, `docs/delivery.md`, `docs/backlog.md`,
  `a2amx.toml.example`

Does not touch: `src/harness.rs`, `src/delivery.rs`, `src/quota.rs`, `src/emulator.rs`,
`extension/omp.ts`, `scripts/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the other specs. No new
dependency. Do not commit, stage or merge: leave the diff uncommitted for review.

## 1. Why

During a long run with peer sessions the architect's context stays small, but the developer's
context grows with every spec it implements. The architect should be able to reset the developer's
context between specs. Native delivery cannot do it: slash commands such as `/clear` are executed by
the harness's own composer, so they must be typed into the PTY. Typing into a composer is exactly
what a2amx must do carefully (never destroy a human draft, never send Ctrl-C, never act
mid-turn), so the feature is a small, gated control action and not a general "type this" tool.

## 2. Decisions

- **D1.** The action is **reset**: the daemon types the target session's configured reset sequence,
  one step at a time. A caller never supplies command text; callers only name the session.
- **D2.** The sequence is per session: a list of command strings. Default for every harness:
  `["/clear"]`. A user who runs OMP with prewalk configures `["/clear", "/prewalk restart"]`. The
  default never assumes prewalk.
- **D3.** Configuration: the team file entry (`reset = ["/clear", "/prewalk restart"]`), and
  `a2amx new --reset <command>` (repeatable, in order). Both reach the daemon in
  `NewSession.reset: Vec<String>` (empty means the default of D2; `serde(default,
  skip_serializing_if = "Vec::is_empty")`). Validation, applied by one function used by the CLI,
  the team parser and the daemon: at most 8 steps; every step starts with `/`, is at most 200
  bytes, and has no control characters (so no `\r`, `\n`, ESC). Steps are slash commands only: a
  step that is not a slash command would send a prompt to the model, which is not a reset.
  An explicit empty list in the team file is an error (omit the field for the default).
- **D4.** Who may trigger it: the owner at the CLI (`a2amx reset <session>`, admin token, always
  allowed), and agents through `Request::Reset { session }` (and the MCP tool `reset_session`)
  only if the sender's session name is in the **target's** `control_from`
  (`NewSession.control_from: Vec<String>`, same serde attributes; team file
  `control_from = ["architect"]`; `a2amx new --control-from <name>`, repeatable; each name checked
  with `messaging::validate_name`). Default: nobody. The sender is derived from the authenticated
  token (`Role::Session`), never from an argument. A sender not listed, and a sender whose session
  has no name, gets code `not_permitted` and the target is not touched. A session cannot reset
  itself (`not_permitted`).
- **D5.** Gates, checked before every step and never queued: the session exists and is running;
  `Session::begin_delivery` accepts (no hold, harness `ready`); a human draft is detected by the
  hold (`Hold::HumanDraft` maps to `draft_present`, every other hold to `held`). For OMP the daemon
  also requires the bridge connected and the extension's last reported state `idle && !draft &&
  !pending` (add `idle` to `NativeSlot`, stored in `native_state`; today `src/daemon.rs` drops it);
  not connected maps to `not_ready`, `draft` to `draft_present`, `!idle` or `pending` or an
  in-flight native delivery to `busy`. A failed gate fails the whole action with a reason; nothing
  is queued for later.
- **D6.** Typing a step: reuse `Session::begin_delivery` and `Delivery::submit` with
  `messaging::paste_bytes(step)` (bracketed paste, a gap, a re-check, then one CR), for every
  harness. Probe result: bracketed paste plus one CR executes the command in claude, codex and
  omp, while raw typing plus one CR does not execute in codex (its autocomplete consumes the first
  Enter). Never send Ctrl-C or any control byte other than the final CR. Outcome mapping:
  `Submitted` continues; `Unsubmitted` fails with `held` (the existing re-check set the
  `UnsubmittedEnvelope` hold and the pasted step remains in the composer; the human must clear it,
  as with a message) and names the step; `Failed` fails with `write_failed`.
- **D7.** Step completion: after the CR the daemon waits a fixed settle time of 1500 ms, then until
  the session is again `ready` (poll every 100 ms), with a per-step timeout of 30 s measured from
  the CR (`step_timeout`). The settle time exists because the probe saw transient screens right
  after the command (Claude shows a SessionEnd hook marker for about 0.15 s; Codex shows
  `Starting` for about 1 s) while all three harnesses were ready again within about 1 s.
  `// shortcut:` comment at the constant: fixed settle, replace by a per-harness completion marker
  if a harness is found that needs longer or reports readiness early. The next step then goes
  through the gate of D5 again. Fail fast: the first failed gate, write failure or timeout stops the
  sequence; steps already typed are not undone.
- **D8.** `Request::Reset { session: String }` and `Response::Reset { steps: u32 }` in
  `src/wire.rs`, exactly `{"type":"reset","session":"s2"}` and `{"type":"reset","steps":2}`. Failures
  are `Response::Failed { code, message }` with codes (add to `messaging::code`) `not_permitted`,
  `unknown_session`, `exited`, `held`, `not_ready`, `draft_present`, `busy`, `step_timeout`,
  `write_failed` (configuration is validated at session creation, so there is no config code).
  The message names the step as `step N of M`. `Request::Reset` is added to the allow-list of
  requests a session token may send; the daemon authorizes it by D4.
- **D9.** CLI: `a2amx reset <id|name>` (resolved with `resolve_reference` like `kill`) prints
  `reset s2: 2 steps` to stdout on success and `a2amx: <message>` to stderr with exit status 1 on
  failure. MCP tool `reset_session` with the single argument `to` (a name or address, resolved like
  `send_message`'s `to`), exact-keys validated like the other tools, returning the same text; its
  schema description says it runs the target's configured reset and needs the target's consent.
- **D10.** Audit: every attempt (allowed or refused) writes one `eprintln`-style daemon-log line
  (use the daemon's existing logging path) with the sender (`admin` or the sending session's
  address), the target address, the outcome code and the failed step if any. The command text is
  not logged; no tokens anywhere.
- **D11.** One reset per session at a time: a second request while one runs fails with `busy`
  (an `AtomicBool` or equivalent in `Session`, cleared on every exit path). While a reset runs,
  delivery to that session cannot interleave because `begin_delivery` holds the input gate.

## 3. Probe findings (2026-10-03, throwaway sessions of an isolated daemon)

- **Claude Code 2.1.288:** `/clear` executes with raw typing plus CR or bracketed paste plus CR; no
  confirmation. The screen is the normal ready prompt again after about 0.5 s (a SessionEnd hook
  marker is visible for about 0.15 s). Empty composer (placeholder) and a typed draft look
  different on screen. A fresh working directory shows the workspace trust dialog, so the probe ran
  in an already trusted directory; reset targets are long-lived sessions in trusted directories.
  Whether the user's SessionStart hook ran after `/clear` could not be seen on screen.
- **Codex 0.160.0:** bracketed paste plus one CR executes `/clear`, ready again in about 1 s. Raw
  typing plus one CR does not: the first Enter only accepts the autocomplete entry, a second Enter
  runs it. Empty and draft composers look different.
- **OMP 18.5.0:** `/clear` and `/prewalk restart` both execute with one CR (raw or paste), each
  showing a confirmation line within about 0.15 s; no model turn started. The extension reports
  `ctx.isIdle()` (polled every 500 ms); `idle` does not change across slash commands, so it is
  only a "not mid-turn" gate, not a completion signal.
- **Daemon:** there is no way to type into a detached session except attaching to it; `Request`
  has no input request. Reset therefore adds the daemon-side typing path of D6.

## 4. Changes

1. `src/wire.rs`: `NewSession.reset`, `NewSession.control_from` (serde as in D3/D4),
   `Request::Reset`, `Response::Reset`.
2. `src/messaging.rs`: the error codes of D8 and the shared sequence validator of D3
   (`validate_reset_steps`) with `validate_name` reuse for `control_from`.
3. `src/team.rs`: `TeamSession.reset: Option<Vec<String>>` and `control_from: Vec<String>`
   (`deny_unknown_fields` stays), validated with the shared validator; an explicit empty `reset` is
   an error; flag-form sessions use the defaults.
4. `src/cli.rs`, `src/main.rs`: `new --reset`, `new --control-from`, the `reset` subcommand (D9);
   `NewOptions`/`create_session` carry both lists and send them in `NewSession`; `team up`
   forwards the team entry's lists.
5. `src/session.rs`: store `reset` and `control_from` on `Session` (from `SessionSpec`), `idle` in
   `NativeSlot`/`native_state`, the one-at-a-time flag, and the sequence runner (gate, type via
   `begin_delivery`/`Delivery::submit`, settle, wait ready, per-step timeout).
6. `src/daemon.rs`: store the config at session creation (validating again, since the wire is the
   trust boundary), pass `idle` into `native_state` (the `BridgeUp::State` arm), handle
   `Request::Reset` with the D4 authorization and the D10 audit line, add `Request::Reset` to the
   session-token allow-list.
7. `src/mcp.rs`: the `reset_session` tool (D9), mirroring `message_status` for validation and
   result shape.
8. Mechanical: add `reset: vec![], control_from: vec![]` to every existing `NewSession` literal
   (`src/daemon.rs`, `src/main.rs` and the test files of section 0); `SessionSpec` literals in
   `src/daemon.rs` and `src/session.rs`, the four `NewOptions` literals in `src/main.rs` and the
   `TeamSession` literals in `src/team.rs` and `tests/team.rs` get the new fields. The OMP
   extension needs no change: it embeds `tool_schemas()` and runs tool calls through the same
   `a2amx mcp` path, so `reset_session` appears there by itself (`tests/omp_wiring.rs` only checks
   that the three existing tool names are present and stays green).
9. Docs: `README.md` (command list, the team file fields), `docs/architecture.md` (request list,
   the reset flow), `docs/delivery.md` (typing a step reuses the PTY delivery gates, never
   Ctrl-C), `a2amx.toml.example` (a commented `reset` and `control_from` example), `docs/backlog.md`
   (note: completion marker per harness; Claude SessionStart hook after `/clear` unverified).

## 5. Tests (failing first, one behavior at a time, public seams only)

A fake composer fixture in `tests/daemon_cli.rs`: a `sh -c` Generic session that turns bracketed
paste on (`printf '\033[?2004h'`), reads one line at a time, and on the line `/clear` prints a
marker line, on any other line prints `got:<line>`; a second fixture variant that prints
`busy` and withholds the paste-mode sequence for a while to exercise timeouts. Cases:

1. Default reset: `a2amx new --name worker -- <fixture>`, `a2amx reset worker` prints
   `reset s1: 1 step` and the screen (read with `a2amx screen`) shows the `/clear` marker.
2. Configured sequence: `new --reset /clear --reset '/prewalk restart'`; both steps typed in
   order; `reset s1: 2 steps`.
3. Validation: a step without a leading `/`, with a control byte, over 200 bytes, more than 8 steps,
   and an invalid `control_from` name are rejected at `new`; an empty `reset` list in a team file is
   an error with a clear message.
4. Draft refusal: a human draft (typing through an attached pty, so the hold is set) makes
   `a2amx reset` fail with `draft_present` and type nothing.
5. Not ready: a session not in bracketed paste mode fails with `not_ready` (or `held` after an
   unsubmitted step) and names the step.
6. Step timeout: the second step never becomes ready again: fails with `step_timeout`, message
   `step 2 of 2`, the first step stays typed.
7. Exited and unknown session: `exited` and `unknown_session`.
8. Authorization (broker path, session tokens): a sender listed in `control_from` can reset; an
   unlisted sender, an unnamed sender and the session itself get `not_permitted` and the target
   shows no new screen output; the sender cannot be forged by an argument.
9. `busy`: a second reset while one runs fails with `busy`.
10. Wire literals (D8) and the MCP `reset_session` exact-keys validation and result.
11. Team file: `reset` and `control_from` parse, are forwarded by `team up`, and an unknown field
    is still rejected.
Report per test whether a genuine RED run was captured.

## 6. Out of scope

Arbitrary text typing for agents, queueing a reset for later, resetting while a human has a draft
(even with a force option), changing native delivery, `list`/`list_agents` columns, automatic reset
policies, re-sending any bootstrap after a reset (the architect's next ASSIGN does that), raw typed
input, per-harness completion markers, Windows, and any change to `src/harness.rs`.

## 7. Acceptance

Fresh, on the final tree, no warnings: `env -u A2AMX_BIN cargo test`,
`env -u A2AMX_BIN cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`,
`git diff --check`; diagnostics clean on changed files; `git diff --stat` shows only the files of
section 0. Do not touch `~/.a2amx/bin` or the live daemon. A manual check against real claude,
codex and omp sessions is done afterwards by the architect with the owner (needs an installed
binary), not by the developer.
