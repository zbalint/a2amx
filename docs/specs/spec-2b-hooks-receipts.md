# Spec 2b: Claude Code prompt-submit hook and submission receipts

## 0. Status

**LOCKED** for implementation.

**Scope.** Add `src/hook.rs` and `tests/hook.rs`. Edit `src/messaging.rs`,
`src/harness.rs`, `src/store.rs`, `src/wire.rs`, `src/session.rs`, `src/daemon.rs`,
`src/delivery.rs`, `src/mcp.rs`, `src/cli.rs`, `src/main.rs`, and `src/lib.rs` (which may
only gain `pub mod hook;`). Edit `tests/messaging.rs`, `tests/wire.rs`,
`tests/delivery.rs`, `tests/broker.rs`, `tests/mcp.rs`, `tests/attach_cli.rs`, and replace
`tests/fixtures/fake_claude.sh` (§13).

**Does not touch:** anything under `docs/` (this spec included), `README.md`,
`AGENTS.md`, `LICENSE`, `rust-toolchain.toml`, `rustfmt.toml`, `clippy.toml`,
`.gitignore`, `Cargo.toml`, `Cargo.lock`, `src/emulator.rs`, `src/client.rs`,
`src/prefix.rs`, `tests/common/`, and the other files under `tests/` and
`tests/fixtures/`. It adds no crate.

Read first: `docs/delivery.md` ("Evidence model", "Corrupted submissions"),
`docs/architecture.md` ("Implemented: messaging core"), `AGENTS.md`,
`docs/specs/spec-2-messaging.md`, and the code under `src/`. Spec 2's behavior stays in
force except where a section below changes it.

## 1. Why

Spec 2 delivers a message by pasting an envelope into a Claude Code composer and sending
`CR` as its own write. It ends at "submitted, outcome unknown": nothing confirms the
harness accepted the prompt, and nothing protects a human's half-typed draft when the
dirty flag is wrong (for example text typed just as a paste lands). The human hold also
clears only on an explicit `prefix r`.

Claude Code's `UserPromptSubmit` hook fixes all three: it runs a command with the
submitted prompt before the model sees it, and the command can block the prompt. This
spec adds that hook for Claude Code only. A hook event that matches a delivered envelope
is a **submission receipt**. An event that contains an envelope mixed with other text is
a **corrupted submission** and is blocked. Any other event is a human prompt, which
empties the composer and therefore clears the human hold.

Facts from probes of the real harness (Claude Code 2.1.285 and 2.1.286; `--settings`
with a logging hook, driven through a PTY) shaped this design:

1. `--settings` accepts inline JSON, and a hook defined only there fires. The hook
   process inherits the harness environment (`A2AMX_ADDR`, `A2AMX_TOKEN` reach it).
2. The hook's stdin is a JSON object with the keys `session_id`, `transcript_path`,
   `cwd`, `prompt_id`, `permission_mode`, `hook_event_name` (`"UserPromptSubmit"`), and
   `prompt`. `prompt` is the raw submitted text with `\n` newlines.
3. Printing `{"decision":"block","reason":R}` and exiting 0 stops the prompt: the model
   never sees it, the composer is emptied, and the transcript shows `UserPromptSubmit
   operation blocked by hook: R` followed by `Original prompt: <text>`.
4. A hook that exceeds its `timeout` is cut off, the prompt proceeds, and a visible
   "timed out" notice is shown. A hook that prints nothing and exits 0 allows the prompt.
5. **Paste wrapping.** Every bracketed paste that contains a newline, and every single
   line of roughly 1000 characters or more, is shown in the composer as `[Pasted text #N]`
   and reaches the hook wrapped as `\n\n<pasted_content id="ID">\n` + text +
   `\n</pasted_content id="ID">\n`, where `ID` is a short alphanumeric string that
   changes every time. Single lines of 686 characters or fewer arrive unwrapped and
   byte-identical. A paste wrapper can also appear in the middle of a prompt when a
   human typed text around a paste. Every delivered envelope has newlines, so every
   delivered envelope arrives wrapped and the matcher must unwrap first.
6. A message submitted while a turn is running is queued by Claude Code and the hook
   fires at submit time with the active turn's `prompt_id` (not a new one). Matching
   therefore uses the A2AMX message id inside the envelope, never `prompt_id`.

Envelope wording, attachments for long messages, other harnesses, and an
unattended-only delivery mode are **not** in this spec (§14).

## 2. Design (read before any section below)

**D1. One hook subcommand, fail open.** `a2amx hook` reads the harness payload on
stdin, asks the daemon for a verdict over the same TCP + session-token channel the MCP
server uses, and prints nothing (allow) or the block JSON. Every failure (daemon
unreachable, bad payload, timeout, oversized prompt) allows the prompt, exits 0, and
writes one diagnostic line to stderr. stdout is reserved for the block JSON because
hook stdout is added to the conversation on this event.

**D2. Install inline.** `a2amx new --harness claude` appends `--settings <json>` whose
only content is one `UserPromptSubmit` command hook. No settings file is written, so
there is nothing to clean up and the user's own settings are untouched. The JSON holds no
secret: identity reaches the hook through the inherited `A2AMX_*` environment.

**D3. Match on the A2AMX message id, whole text.** The daemon unwraps paste wrappers,
finds envelope message ids, and compares the whole text to the envelope it would render
for that message (`render_envelope`). The exact envelope is a receipt; a known envelope
inside other text is a corrupted submission; text with no known envelope is a human
prompt. A message id counts as **known** only when it belongs to this session, to the
current boot, is in state `delivering`, `submitted`, or `unsubmitted`, and has no receipt
yet. Everything else is treated as not an envelope, so a stale or foreign id never blocks
a human prompt.

**D4. Evidence, not a new state.** Message states are unchanged. A receipt is stored on
the attempt row (`attempts.receipt_at`) and surfaces as `evidence`:
`write_complete` for a `submitted` message without a receipt, `submission_observed` for
one with a receipt, `None` for every other state. A missing receipt is never a failure.

**D5. A corrupted submission is blocked, re-queued, and its draft restored.** The
attempt is recorded as `rejected` with detail `corrupted_submission`, the message goes
back to `pending` (a new attempt will be made), and the human's own text, with the
envelope removed, is pasted back into the composer without `CR` once the session is idle.
The session holds (`human_draft`) while that draft is in the composer, so the envelope
waits behind the human. Three corrupted submissions in a row on one session stop the
cycle: the session holds with `corrupted_submissions` until an explicit release.

**D6. A human submit clears the human hold.** Any allowed prompt empties the composer, so
it clears the `human_draft` and `unsubmitted_envelope` holds. It never clears
`corrupted_submissions`; only `prefix r` does.

**D7. Receipts and write progress race.** A hook event can arrive before the delivery
task records its own outcome. Whichever lands first wins and a late write outcome never
downgrades a receipt or overwrites a rejection (§5).

**D8. No new trust.** The hook authenticates with the session token like the MCP server.
A receipt is only accepted for a message addressed to the calling session. Prompt text is
kept in memory only (for the draft restore), is never logged, and never stored.

## 3. `src/messaging.rs`

Add these items (pure; no I/O):

```rust
pub const MAX_CORRUPTED_SUBMISSIONS: u32 = 3;
pub const MAX_RESTORE_BYTES: usize = 64 * 1024;
pub const CORRUPTED_SUBMISSION_REASON: &str =
    "A2AMX blocked this prompt because it mixed your text with a peer message. The message will be delivered again.";

/// Replace every complete paste wrapper with its inner text.
pub fn unwrap_pastes(prompt: &str) -> String;
/// Message sequence numbers of every `<a2amx-message id="m_N"` in `text`.
pub fn envelope_ids(text: &str) -> Vec<i64>;
/// Drop control characters other than `\n` and `\t`.
pub fn sanitize_draft(text: &str) -> String;
```

**`unwrap_pastes`.** The opening marker is `"\n\n<pasted_content id=\""`. Scan left to
right. At an opening marker at byte `p`: read the id, a run of one or more ASCII
alphanumeric characters; require the next three bytes to be `">\n` (the inner text starts
after them); the closing marker is `"\n</pasted_content id=\"" + id + "\">\n"`; find its
first occurrence at or after the inner start. When all of that matches, output the text
before `p` (since the last cursor) followed by the inner text, and continue after the
closing marker. When any part fails to match, output the text up to and including the
single byte at `p` and continue scanning from `p + 1`, so a malformed or unclosed wrapper
is copied unchanged. Worked examples (literal, `ESC`-free):

| Input | Output |
| --- | --- |
| `"\n\n<pasted_content id=\"458d\">\nalpha\nbeta\n</pasted_content id=\"458d\">\n"` | `"alpha\nbeta"` |
| `"hello \n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"458d\">\n"` | `"hello alpha"` |
| `"a\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\nb\n\n<pasted_content id=\"2\">\nY\n</pasted_content id=\"2\">\n"` | `"aXbY"` |
| `"\n\n<pasted_content id=\"458d\">\nalpha\n"` (unclosed) | unchanged |
| `"\n\n<pasted_content id=\"458d\">\nalpha\n</pasted_content id=\"zzzz\">\n"` (ids differ) | unchanged |
| `"plain text"` | unchanged |

**`envelope_ids`.** Find each occurrence of the literal `<a2amx-message id="m_`; it counts
when it is followed by one to eighteen ASCII digits, then `"`, and the number is greater
than zero. Return the numbers in order of first appearance, without duplicates. Examples:
`<a2amx-message id="m_12" from="x">` → `[12]`; ids `m_3`, `m_3`, `m_7` → `[3, 7]`;
`id="m_"`, `id="m_x1"`, `id="m_0"`, and bare `m_12` without the tag → `[]`.

**`sanitize_draft`.** Remove every character for which `char::is_control()` is true except
`'\n'` and `'\t'` (this removes `\x1b`, `\r`, `\0`, `\x7f`, and U+0085, the same set
`validate_message` rejects). It never truncates.

## 4. `src/harness.rs`

`wire_claude_argv` (signature unchanged) appends one more pair **last** among its extras,
after the optional `--append-system-prompt` pair and still inserted before a literal `--`
exactly as today: `--settings` and the compact JSON string

```json
{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command","command":"<Q> hook","timeout":5}]}]}}
```

where `<Q>` is the executable path quoted for a POSIX shell: wrap it in single quotes and
replace each `'` inside it with `'\''`. Add a private `shell_quote(&str) -> String` for
that (the `command` field is one shell command line; the hook subcommand takes no
arguments). For the path `/tmp/a2amx binary` the command is `'/tmp/a2amx binary' hook`;
for `/tmp/it's/a2amx` it is `'/tmp/it'\''s/a2amx' hook`. The extras vector capacity
becomes 10 when `authorize_peers` is true and 8 when it is false. `Generic` is unchanged:
no hook.

## 5. `src/store.rs`

**Schema v2.** `user_version` becomes 2. `attempts` gains `receipt_at INTEGER`
(nullable).

```rust
match version {
    0 => { /* the v1 DDL with `receipt_at INTEGER` added to attempts, then PRAGMA user_version = 2 */ }
    1 => { /* ALTER TABLE attempts ADD COLUMN receipt_at INTEGER; PRAGMA user_version = 2 */ }
    2 => Ok(()),
    other => bail!("unsupported messages.db schema version {other}"),
}
```

Both branches run in one transaction, as today. Attempt `outcome` gains the value
`rejected` (existing: `started`, `submitted`, `unsubmitted`, `failed`, `unknown`).

**`Message`** gains `pub(crate) observed: bool`, true when any attempt of the message has
`receipt_at IS NOT NULL`. Add, as the eleventh selected column of each of the four
`SELECT`s that feed `message_from_row` (`next_pending`, `get`, and both `list`
statements), exactly:

```sql
EXISTS (SELECT 1 FROM attempts WHERE attempts.message_seq = messages.seq
                                 AND attempts.receipt_at IS NOT NULL)
```

and read it as the last column in `message_from_row`.

**`finish_attempt`** (signature unchanged) now runs in this order inside its transaction:
update the attempt with `WHERE id = ?attempt AND message_seq = ?seq AND outcome =
'started'`; if no row changed, look the attempt up: if it does not exist, bail `unknown
attempt <id>` as today; if it exists, commit and return `Ok(())` **without touching the
message** (a receipt or rejection already decided it, D7). Only when the attempt row
changed, update the message exactly as today (`unknown message sequence` bail if no row).

**New methods** (each one transaction, each through `Store::run`):

```rust
/// True when a receipt was newly recorded.
pub(crate) async fn record_receipt(&self, seq: i64) -> Result<bool>;
/// True when an attempt was newly rejected.
pub(crate) async fn reject_attempt(&self, seq: i64) -> Result<bool>;
```

Both select the target attempt with

```sql
SELECT id FROM attempts
 WHERE message_seq = ?1 AND outcome IN ('started','submitted','unsubmitted')
   AND receipt_at IS NULL
 ORDER BY id DESC LIMIT 1
```

and return `Ok(false)` when there is none.

- `record_receipt`: set that attempt `outcome = 'submitted'`, `detail = NULL`,
  `receipt_at = <now>`, `finished_at = COALESCE(finished_at, <now>)`; set the message
  `state = 'submitted'`, `detail = NULL`, `updated_at = <now>` only `WHERE state IN
  ('delivering','submitted','unsubmitted')` (a cancelled or undeliverable message keeps
  its state). Return `Ok(true)`.
- `reject_attempt`: set that attempt `outcome = 'rejected'`, `detail =
  'corrupted_submission'`, `finished_at = COALESCE(finished_at, <now>)`; set the message
  `state = 'pending'`, `detail = NULL`, `updated_at = <now>` only `WHERE state IN
  ('delivering','submitted','unsubmitted')`. Return `Ok(true)`.

`recover`, `purge`, `has_earlier_open`, `open_counts`, `cancel`, and `insert_message` are
unchanged (a re-queued `pending` message is open again and counts toward the recipient
limit like any other).

## 6. `src/wire.rs`

Add to `Request` (session role only, §8.2):

```rust
ReportPrompt { prompt: String },
```

Add to `Response`:

```rust
PromptVerdict {
    verdict: String,                                   // "allow" | "block"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<String>,                            // only with "block"
},
```

`MessageInfo` gains `#[serde(default, skip_serializing_if = "Option::is_none")] pub
evidence: Option<String>`. JSON literals: `{"type":"report_prompt","prompt":"x"}`;
`{"type":"prompt_verdict","verdict":"allow"}`;
`{"type":"prompt_verdict","verdict":"block","reason":"r"}`; a `MessageInfo` with
`evidence: None` serializes exactly as today, and with `Some("submission_observed")` it
gains `"evidence":"submission_observed"` after `hold_reason`.

## 7. `src/session.rs`

`Hold` gains `CorruptedSubmissions`. `State` gains `pub restore: Option<String>` and
`pub corrupted: u32` (both initialized `None` / `0`). **Invariant:** `restore.is_some()`
implies `hold.is_some()`.

```rust
pub(crate) fn note_submit(&self, clean_envelope: bool);
pub(crate) fn note_corrupted(&self, draft: Option<String>) -> bool;
pub(crate) async fn restore_draft(&self);
```

- `note_human_input(bytes)`: when `input_is_typing(bytes)` it additionally sets `restore =
  None` (the human started a new draft; the old text stays in the transcript), then sets
  `hold` to `HumanDraft` only when `hold` is `None`, as today.
- `release()`: additionally sets `restore = None` and `corrupted = 0`.
- `note_submit(clean_envelope)`: under the lock, when `hold` is `HumanDraft` or
  `UnsubmittedEnvelope`, set `hold = None` and `restore = None`; when `clean_envelope` is
  true set `corrupted = 0`. Then `message_notify().notify_one()`. It never clears
  `CorruptedSubmissions`.
- `note_corrupted(draft)`: under the lock, increment `corrupted`; `capped = corrupted >=
  MAX_CORRUPTED_SUBMISSIONS`; `restore = draft` when it is `Some` and non-empty and its
  byte length is at most `MAX_RESTORE_BYTES`, else `restore = None`; when `capped` set
  `hold = Some(CorruptedSubmissions)`, else when `restore` is `Some` set `hold =
  Some(HumanDraft)` (replacing any existing hold), else leave `hold` unchanged. Return
  `capped`. The caller passes an already sanitized draft.
- `restore_draft()`: return at once when `restore` is `None`. Otherwise acquire
  `input_gate`, then under the session lock: if the session exited, set `restore = None`
  and return; if `restore` is now `None`, return; if `harness::ready(...)` is false,
  return leaving `restore` set (the delivery task retries on its next wake-up); otherwise
  take the text, reserve an input slot, and enqueue `messaging::paste_bytes(&text)` as
  **one** item with no `CR` and no gap. `hold` is left as is. A `// shortcut:` comment
  records that a queue that never drains can hold the gate here (same ceiling as
  `Delivery::submit`).

## 8. `src/daemon.rs`

### 8.1 Roles

`Request::ReportPrompt { .. }` joins `SendMessage`, `ListAgents`, and `MessageStatus` in
the list a session token may send. The `match request` in `serve` (the only exhaustive
match over `Request` in the tree) gains the arm `Request::ReportPrompt { prompt }`: for
the admin role it replies `Response::Error { message: "report_prompt needs a session
token" }`, for a session role `runtime.report_prompt(id, prompt).await?`.

### 8.2 `Runtime::report_prompt(session_id, prompt) -> anyhow::Result<Response>`

Never hold the session mutex across an `.await`. Steps, in order:

1. `text = messaging::unwrap_pastes(&prompt)`. Collect the **known** messages: for each
   `seq` in `envelope_ids(&text)`, `store.get(seq)`; keep it only when `boot` equals the
   current boot, `recipient_session` equals `session_id`, `state` is `Delivering`,
   `Submitted`, or `Unsubmitted`, and `observed` is false (D3).
2. **No known message → human prompt.** `session.note_submit(false)`; reply
   `PromptVerdict { verdict: "allow", reason: None }`.
3. **Exactly one known message and `text` equals `render_envelope("m_<seq>",
   sender_address, subject, body)` for it → receipt.** `store.record_receipt(seq)`;
   `session.note_submit(true)`; reply allow. (A result of `false` is not an error.)
4. **Otherwise → corrupted submission.**
   - Build the draft: start from `text`; for each known message remove the first
     occurrence of its rendered envelope. If every known envelope was found, the draft is
     the remainder trimmed of surrounding whitespace then passed through
     `sanitize_draft`; if any known envelope was not found verbatim (characters were
     interleaved into it), there is no draft.
   - `session.note_corrupted(draft)` **first**, then `store.reject_attempt(seq)` for each
     known message, then `session.message_notify().notify_one()`. The order matters: the
     hold must exist before the message becomes `pending` again, or the delivery task
     could re-paste over the human's draft.
   - Reply `PromptVerdict { verdict: "block", reason: Some(CORRUPTED_SUBMISSION_REASON) }`.

Store errors propagate with `?` (the connection closes; the hook then fails open).

### 8.3 Status fields

`message_info`: `evidence` is `Some("submission_observed")` when `state` is `Submitted`
and `observed`, `Some("write_complete")` when `Submitted` and not `observed`, else `None`.
`hold_reason` gains `"corrupted_submissions"` for `Hold::CorruptedSubmissions`; the list
in spec 2 §8.5 becomes: `deliver_hold`, `unsubmitted_envelope`, `corrupted_submissions`,
`human_draft`, `queued`, `not_ready`, `cooldown` (first that applies).

## 9. `src/delivery.rs`

In the loop body, immediately after the "session has exited" check and before the
`Deliver::Hold` check, add `session.restore_draft().await;`. Nothing else changes: a
re-queued message is delivered by the existing head-of-queue logic once no hold applies.

## 10. `src/hook.rs` (new) and `a2amx hook`

```rust
pub async fn run() -> anyhow::Result<()>   // always Ok; see below
const HOOK_TIMEOUT: Duration = Duration::from_secs(4);   // below the 5 s in the settings
const MAX_STDIN_BYTES: usize = 2 * 1024 * 1024;
const MAX_HOOK_PROMPT_BYTES: usize = 512 * 1024;
```

Steps; any failure calls `note(reason)` and returns `Ok(())` with nothing on stdout:

1. Read stdin on a blocking thread, at most `MAX_STDIN_BYTES`; more than that is a
   failure.
2. Parse it as a JSON object. When `hook_event_name` is present and is not
   `"UserPromptSubmit"`, return `Ok(())` silently. `prompt` must be a string of at most
   `MAX_HOOK_PROMPT_BYTES` (`// shortcut:` a larger prompt is allowed unexamined; an
   envelope is at most about 33 KiB, so this only skips the safety net for huge human
   prompts).
3. Read the address and token with `mcp::configuration()` (made `pub(crate)`, §11).
4. Within one `tokio::time::timeout(HOOK_TIMEOUT, ..)`: `Client::connect_addr`, then
   `request(Request::ReportPrompt { prompt })`.
5. `Response::PromptVerdict { verdict: "block", reason }` → print one line
   `{"decision":"block","reason":<reason or "">}` to stdout. `"allow"` → print nothing.
   Any other response or error → `note`.

`note(reason)` writes `a2amx hook: <reason>` and a newline to stderr, best effort (write
errors are ignored with a comment saying why), where `<reason>` is a fixed phrase, never
the prompt, the payload, the token, or an error chain that could contain them. The
process always exits with status 0.

## 11. `src/mcp.rs`

`configuration()` becomes `pub(crate)`. The `message_status` result gains `"evidence":
message.evidence` after `"hold_reason"`: `{"id":…,"to":…,"state":…,"detail":…|null,
"hold_reason":…|null,"evidence":…|null}`.

## 12. `src/cli.rs`, `src/main.rs`, `src/lib.rs`

`cli.rs`: add `Hook` to `Command` with the doc comment `/// Run the prompt-submit hook
adapter (reads the harness payload on stdin).`

`main.rs`: in `dispatch`, before resolving the home directory, `Command::Mcp` returns
`mcp::run().await` as today and `Command::Hook` returns `hook::run().await`; the later
`Command::Mcp => unreachable!(..)` arm becomes `Command::Mcp | Command::Hook =>
unreachable!(..)`. In `message_value_rows` the DETAIL cell is `detail`, else
`hold_reason`, else `"submission_observed"` when `evidence` is exactly that, else `"-"`
(a `write_complete` row still prints `-`, so spec 2's worked example is unchanged).

`lib.rs`: add `pub mod hook;` between `pub mod harness;` and `pub mod mcp;`.

## 13. Tests (write each behavior's test first)

Seams: **S6** pure functions in `messaging` and `harness`; **S7** the daemon through
`Client` with a session token; **S8** delivery into the fake harness under a real daemon;
**S10** the `a2amx hook` binary over pipes against a real daemon. Test only through public
interfaces. Expected values are literals or the worked examples in this spec. Do not
sleep: wait on states, files, or `eventually`. No fixed ports or paths. Tests never run
the real `claude`.

**Existing tests stay green unchanged in meaning.** These literal edits are required and
are the only permitted ones:

- `tests/wire.rs`: the `MessageInfo` literal gains `evidence: None`.
- `tests/mcp.rs`: the expected `message_status` JSON gains `"evidence": null`.
- `tests/messaging.rs` (`harness_defaults_and_claude_argv_wiring_are_exact`): with
  `authorize_peers` true, `argv[9]` is `--settings` and `argv[10]` parses as the §4 JSON
  for `/tmp/a2amx binary` (command `'/tmp/a2amx binary' hook`, timeout 5); in the
  `--` case (`false`), `before_separator` is `claude`, `--mcp-config`, JSON,
  `--allowedTools`, value, `--settings`, JSON, `--`, `positional`, so the assertion
  `before_separator[5] == "--"` becomes `before_separator[5] == "--settings"` and a new
  one checks `before_separator[7] == "--"`.
- `tests/attach_cli.rs` (`claude_new_wires_mcp_and_optional_peer_authorization`): the first
  session's argument file has 8 lines (`wait_for_file(.., 8)`, `assert_eq!(len, 8)`), with
  `first_args[6] == "--settings"` and `first_args[7]` parsing as the §4 JSON whose
  `hooks.UserPromptSubmit[0].hooks[0].command` equals `format!("'{expected_exe}' hook")`
  (`expected_exe` is the canonicalized path already computed there); the second
  (`--no-authorize-peers`) has 6 lines, `wait_for_file(.., 6)`, and its expected vector
  gains `"--settings", first_args[7]` at the end.

**`tests/fixtures/fake_claude.sh`** is replaced by exactly this content (it now records
the session credentials and, after the submit, appends every further byte to `$OUT.rest`):

```sh
#!/bin/sh
# Test double for the Claude Code composer. Env: OUT (output file prefix),
# PASTE_LEN (byte length of the paste to expect), MODE (ready | dialog_after_paste).
# After the first submit every further byte is appended to $OUT.rest.
size=$(stty size)
cols=${size#* }
rule=
i=0
while [ "$i" -lt "$cols" ]; do rule="$rule─"; i=$((i + 1)); done
printf '\033[2J\033[?2004h\033[2;1H%s\033[3;1H❯ \033[4;1H%s\033[3;3H\033[?25h' "$rule" "$rule"
printf '%s\n%s\n' "$A2AMX_TOKEN" "$A2AMX_ADDR" > "$OUT.creds.tmp"
mv "$OUT.creds.tmp" "$OUT.creds"
stty raw -echo
dd bs=1 count="$PASTE_LEN" of="$OUT.paste.tmp" 2>/dev/null
mv "$OUT.paste.tmp" "$OUT.paste"
t1=$(date +%s%N)
if [ "$MODE" = dialog_after_paste ]; then
  printf '\033[2J\033[H Enter to confirm · Esc to cancel'
fi
stty min 0 time 20
dd bs=1 count=1 of="$OUT.cr" 2>/dev/null
t2=$(date +%s%N)
echo $(((t2 - t1) / 1000000)) > "$OUT.gap_ms"
stty min 1 time 0
exec cat >> "$OUT.rest"
```

(`$OUT.creds` holds the token on line 1 and the address on line 2; it lives in the test's
temp dir. `stty min 1 time 0` matters: without it `cat` sees the earlier read timeout as
EOF and exits. This exact script was run under a PTY at lock time: paste, `CR`, a second
bracketed paste, and later single bytes all landed in `$OUT.rest`, and `$OUT.creds` held
both lines.)

In `tests/delivery.rs`, `Case` gains a `hook` client (a `Client::connect_addr` built from
the recipient's `$OUT.creds`, awaited with `eventually`) and two consts:
`ENVELOPE_TEXT` (the worked-example envelope of spec 2 §3 without the paste markers: the
existing `ENVELOPE` const minus `\x1b[200~` and `\x1b[201~`) and `WRAPPED` (a function
returning `"\n\n<pasted_content id=\"458d\">\n" + text + "\n</pasted_content
id=\"458d\">\n"`). Let `E` be `ENVELOPE.len() + 1` (paste plus `CR`).

**`tests/messaging.rs` (S6).** `unwrap_pastes`: the six table rows of §3.
`envelope_ids`: the examples of §3. `sanitize_draft`: `"a\u{1b}b\rc\0d\u{7f}e\u{85}f"` →
`"abcdef"`; `"x\ny\tz é"` unchanged. The two `shell_quote` cases of §4 through
`wire_claude_argv` (`/tmp/a2amx binary`, `/tmp/it's/a2amx`).

**`tests/wire.rs`.** Round trips and literal JSON for `ReportPrompt`, both
`PromptVerdict` shapes, and `MessageInfo` with and without `evidence` (§6).

**`tests/delivery.rs` (S7/S8)**, each with the fake in `ready` mode and an attached client
for readiness unless noted:

1. *Receipt.* Send; wait `submitted`; status `evidence` is `write_complete`. The hook
   client reports `WRAPPED(ENVELOPE_TEXT)`: verdict `allow`; status is `submitted` with
   `evidence` `submission_observed`. Reporting it again: `allow`, status unchanged.
2. *Foreign session cannot forge a receipt.* The sender's client (`case.sender`) reports
   `WRAPPED(ENVELOPE_TEXT)` for a message addressed to the recipient: `allow`, and the
   message's `evidence` stays `write_complete`.
3. *Human prompt clears the human hold.* Attach, send `Input(b"x")` (a `Redraw` round
   trip), `List` shows `held: true`; the hook client reports `"hello"`: `allow`; `List`
   shows `held: false`.
4. *Unknown id.* The hook client reports text containing an envelope for `m_99`: `allow`,
   nothing changes.
5. *Corrupted with a draft.* After `submitted`, report `"my draft " + WRAPPED(ENVELOPE_TEXT)`
   (note `WRAPPED` starts with `\n\n`, so the joined text unwraps to `my draft ` +
   `ENVELOPE_TEXT`): verdict `block` with reason `CORRUPTED_SUBMISSION_REASON`. The
   message becomes `pending` with `hold_reason` `human_draft`; `$OUT.rest` becomes exactly
   `\x1b[200~my draft\x1b[201~` (eventually); `List` shows `held: true`. Then a
   `ClientFrame::Release`: the message is delivered again and `$OUT.rest` becomes the draft
   paste followed by `ENVELOPE` followed by `\r`, and the message reaches `submitted`.
6. *Corrupted without a draft (interleaved).* Report `ENVELOPE_TEXT` with the character
   `X` inserted right after `"agent-plan@host-a"` in the `send_message(to=` line: `block`;
   no draft is restored; the message is delivered again without a hold: `$OUT.rest`
   becomes exactly `ENVELOPE` followed by `\r` and the message reaches `submitted`.
7. *Three strikes.* Repeat case 6's report three times, each time waiting for the new
   delivery (`$OUT.rest` length `n * E` after the first two) and `submitted`. After the
   third: the message is `pending` with `hold_reason` `corrupted_submissions`, `List`
   shows `held: true`, and `$OUT.rest` does not grow. A human prompt report (`"hello"`)
   replies `allow` and the session stays held. `Release`: the message is delivered again
   (`$OUT.rest` length `3 * E`) and reaches `submitted`.
8. *Unsubmitted envelope, human presses Enter.* `MODE=dialog_after_paste`: the message is
   `unsubmitted` and the session held (`unsubmitted_envelope`); the hook client reports
   `WRAPPED(ENVELOPE_TEXT)`: `allow`; the message is `submitted` with `evidence`
   `submission_observed` and `List` shows `held: false`.
9. *Permissions.* The admin client's `ReportPrompt` gets `Response::Error` with message
   `report_prompt needs a session token`.

**`tests/broker.rs`.** A v1 database file (created in the test with `rusqlite` and the
v1 DDL literal from spec 2 §5, `PRAGMA user_version = 1`, one `submitted` message and one
`submitted` attempt) is opened by a daemon started on that state dir: `ListMessages` lists
the message with `evidence` `write_complete`; after `daemon.shutdown()`, reading
`PRAGMA user_version` with `rusqlite` gives 2.

**`tests/hook.rs` (S10).** Start a daemon and a fake recipient the way `Case` in
`tests/delivery.rs` does (use `mod common;` for the shared helpers; repeat the small
setup here rather than importing from another test file). Run `env!("CARGO_BIN_EXE_a2amx") hook`
with `A2AMX_ADDR` and `A2AMX_TOKEN` from `$OUT.creds` (and nothing else from the
environment), stdin the JSON object below with `prompt` set as stated:

```json
{"session_id":"s-example","transcript_path":"/tmp/example.jsonl","cwd":"/tmp/example","prompt_id":"p-1","permission_mode":"auto","hook_event_name":"UserPromptSubmit","prompt":"<PROMPT>"}
```

- exact wrapped envelope (after a delivered message): exit 0, stdout empty, stderr empty;
  then status `evidence` is `submission_observed` and `a2amx messages` (the binary, same
  state dir) prints a row whose DETAIL cell is `submission_observed`.
- `"my draft " + WRAPPED(ENVELOPE_TEXT)`: exit 0 and stdout is exactly one line that parses
  to `{"decision":"block","reason":CORRUPTED_SUBMISSION_REASON}`.
- missing `A2AMX_ADDR`/`A2AMX_TOKEN`: exit 0, stdout empty, stderr starts `a2amx hook:`.
- address of a closed port: exit 0, stdout empty, stderr starts `a2amx hook:`.
- stdin not JSON: exit 0, stdout empty, stderr starts `a2amx hook:`.
- `hook_event_name` `"SessionStart"`: exit 0, stdout and stderr empty, status of the
  message unchanged (no receipt).
- a `prompt` of `MAX_HOOK_PROMPT_BYTES + 1` bytes: exit 0, stdout empty, stderr starts
  `a2amx hook:`.
- a TCP listener that accepts and never replies: exit 0, stdout empty, and the process ends
  within 8 seconds.

Behaviors with no test, by design (each is a few lines of the code in §7/§5 that the final
review reads instead): a receipt or rejection landing before the delivery task's own
`finish_attempt` (D7; it needs a sub-millisecond race); `note_human_input` cancelling a
pending restore; `restore_draft` returning with `restore` kept while the screen is not
ready; a corrupted prompt with two known envelopes; the 64 KiB restore limit.

## 14. Out of scope

Codex and OMP hooks and profiles; any change to the envelope text or the peer
authorization wording; attachments for long messages and a shorter inline size cap; typing
an envelope as keystrokes instead of pasting; a `--deliver unattended` mode; a native
in-harness delivery channel; hook handling of approval dialogs (no prompt is submitted
there); cross-host receipts and receipt replay; a timeout or expiry for messages with no
receipt; persistence of the pending draft across a daemon restart; a `release` CLI
command; changes to `src/emulator.rs`, `src/client.rs`, or `src/prefix.rs`; edits to
`docs/`, `README.md`, or `AGENTS.md` (the reviewer updates docs); renames of existing
public items other than as this spec names them.

Known gaps to record as `// shortcut:` comments where the code lives, not to be "fixed"
here: a prompt above `MAX_HOOK_PROMPT_BYTES` is allowed unexamined (§10); the paste
wrapper shape and its thresholds were observed on Claude Code 2.1.285/2.1.286 only and the
exact single-line collapse threshold (between 686 and 1186 characters) is not pinned; a
human who types again between submitting and the hook's report has that typing's hold
cleared by the report (the corrupted-submission check catches a resulting merge); a draft
with interleaved characters is not restored (the text remains in the transcript); any
holder of a session token can report a forged prompt for that session (same-user posture,
as with the MCP server); a queue that never drains can hold the gate in `restore_draft`.

## 15. Acceptance

Run from the worktree root with `PATH=$HOME/.cargo/bin:$PATH`. All must hold:

```sh
cargo test                                   # 0 failed
cargo clippy --all-targets -- -D warnings    # no warnings
cargo fmt --check                            # no diff
rg -n 'todo!\(|unimplemented!\(' src tests   # no output
rg -n '#\[ignore' src tests                  # no output
rg -l 'alacritty_terminal' src               # exactly: src/emulator.rs, src/session.rs
rg -l 'rusqlite' src                         # exactly: src/store.rs
deps() { awk '/^\[/{s=$0} s ~ /dependencies\]$/ && /^[a-z_-]+ =/{print s, $1}' | sort; }
diff <(git show HEAD:Cargo.toml | deps) <(deps < Cargo.toml)   # no output
git diff --check                             # no output
git status --porcelain                       # only files named in §0 Scope
```

and every behavior listed in §3–§12 has at least one test in §13, except the ones §13
names as deliberately untested. Leave the diff **uncommitted** and **unmerged** in the
worktree.

## Amendment 1 (adjudicating OMP question: future-schema test vs schema v2)

§5 makes `user_version` 2 the current schema, so the existing test
`daemon_refuses_a_future_database_schema` in `tests/broker.rs`, which used version 2 as its
"future" fixture, would now start successfully. §13's list of permitted edits to existing
tests gains one entry: in that test, `pragma_update(None, "user_version", 2)` becomes
`pragma_update(None, "user_version", 3)` and the expected error string becomes
`"unsupported messages.db schema version 3"`. Nothing else in the test changes; it still
asserts that startup fails and leaves no `admin.token`. The fix is in the fixture value, not
in §5: version 3 is the smallest version this spec does not define. A tree-wide search for
`user_version` and `schema version` in `src/` and `tests/` found no other fixture that
depends on version 2 being unsupported. Scope and acceptance are unchanged.
