# Spec 3cn: A message body may not contain an opening envelope tag; SECURITY.md says how an escape is reported

## 0. Status

LOCKED. Code, tests and docs.

Scope (files that may be edited): `src/messaging.rs`, `tests/messaging.rs`, `tests/prop_messaging.rs`,
`docs/architecture.md`, `SECURITY.md`.
Does not touch: every other `src/` file, `CHANGELOG.md`, `docs/backlog.md`, `docs/roadmap.md`,
`docs/philosophy.md`, any other spec, any other test file.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base the commit that
contains this spec. Context id: `a2amx-envelope-tag-2026-11-04`. Public test seam:
`a2amx::messaging::{validate_message, render_envelope, envelope_ids}` (the MCP `send_message`
reaches it through `src/daemon.rs:1136`; no MCP test is added).

## 1. Why

`validate_message` rejects a body that contains the closing tag `</a2amx-message>`
(`src/messaging.rs:404`) but not one that contains an opening tag. `envelope_ids`
(`src/messaging.rs:529`) then returns a second id for a prompt whose body quotes
`<a2amx-message id="m_<k>"`. Today this fails closed on the channel path
(`channel_prompt_verdict`, `src/daemon.rs:1344-1356`), which needs exactly one id and an exact
envelope match. On the hook path a second id cannot forge a receipt: a receipt is recorded only
when exactly one id is known and the whole text equals that envelope (`src/daemon.rs:1319-1327`);
with two or more known ids the prompt is blocked and a rejection is charged to every one of them
(`block_corrupted_submission`, `:1383-1418`), and a message becomes undeliverable on its second
rejection (`docs/delivery.md:407`). So the harm is different and realistic under philosophy
rule 1: an honest agent that quotes the header of another open message makes its own message
and that unrelated message undeliverable, and the human's typed text in the composer is lost.
Rejecting the opening tag at send time removes that path before it reaches the recipient. Backlog I11 asked for a decision. The
owner decided on 2026-11-04: reject the opening tag as the closing tag is rejected; a message can
no longer quote a raw opening tag. This removes the path instead of constraining it.

Pinned today by spec 3ag: the property test `injected_body_id_cannot_displace_the_envelopes_own_id`
(`tests/prop_messaging.rs`, around `:186-212`) injects an opening tag into a body, requires
`validate_message` to accept it (`prop_assume!`), and checks the ids. That test describes the old
behavior and changes.

Rejected: rejecting only the exact id-bearing prefix `<a2amx-message id="m_` (it is what
`envelope_ids` reads today, but a parser change would reopen the path; the owner asked for "the
opening tag"); escaping the tag in the body (it changes what the recipient reads).

SECURITY.md (owner decision, same day): a confinement escape is reportable as a bug, not as a
vulnerability, because confinement is a safety net and not a sandbox (`docs/philosophy.md` rule 1).

## 2. `src/messaging.rs`

In `validate_message`, directly after the closing-tag check (`:404-409`), add a second check
that rejects a message that contains the text `<a2amx-message`, compared ASCII case-insensitively
(any opening of the element, with or without attributes; `<A2AMX-MESSAGE` too, so a model cannot
be shown a nested envelope that differs only in case; `harness.rs:247` already compares the
paste-wrapper tag with `eq_ignore_ascii_case`), with `code::INVALID_CONTENT` and the text
`message contains an envelope opening tag (write it without the leading <)`. The text goes only
to the sender, so quoting the tag in it is safe. The closing-tag check and its text stay as they
are (case-sensitive; a case variant of the closing tag is left, see the attack-path table). The subject is not checked: it reaches the
envelope only through `push_escaped_attribute`, which turns `<` into `&lt;` (the property test
`a_subject_cannot_add_a_tag_or_an_id` pins this and stays unchanged).

Update the module comment at `:1` only if it names the terminator as the sole rejected tag.

## 3. Attack paths (trust boundary: a peer agent's message body reaches another agent's prompt)

| Path | Disposition |
|---|---|
| A body quotes `<a2amx-message id="m_<k>"` so `envelope_ids` returns a second id | Removed at send time by section 2 |
| The same with another case (`<A2AMX-MESSAGE`) | Removed by the case-insensitive check; the daemon's parser matches only the lowercase prefix, so this one only mattered to a model reading the prompt |
| The opening tag split across a fake paste wrapper (`<a2amx-` , blank line, a wrapper whose text is `message id="m_5"`) that `unwrap_pastes` could join | Constrained: validation accepts it, and it fails only because Claude Code escapes the wrapper tag in pasted text and `paste_view` mirrors that (`harness.rs:232-256`, whose shortcut comment says only 2.1.286 was probed). The new property (section 4) asserts no valid body adds an id, so a parser change that joined fragments would fail a test |
| The subject carries a tag | Already constrained: `push_escaped_attribute` turns `<` into `&lt;` |
| `from` carries a tag | Not possible: it comes from the token and is `[a-z0-9-]` plus the host name |
| A body contains a case variant or lookalike of the closing tag (`</A2AMX-MESSAGE>`) | Left: it is not an id and not a parser input; the envelope's own "From another agent" line covers the model. `SECURITY.md`'s in-scope item about content that forges the envelope means the daemon's parsing and receipts |

## 4. Tests

- `tests/messaging.rs`: add to the rejected-input table (`:64` loop) a body that contains
  `<a2amx-message id="m_5"` and one that contains `before <a2amx-message after` (no attributes),
  both expecting `code::INVALID_CONTENT` (the table's tuples are `(subject, message, code)`, so
  these are message bodies); check the new text `message contains an envelope opening tag` in one
  case and that the closing-tag case keeps `message contains the envelope terminator`. Also assert
  that `validate_message("subject", "the a2amx-message element")` is accepted (the check needs the
  leading `<`), and that a body with `<a2amx-messages-list` is rejected: the check is a plain
  substring match, so a longer element name starting with the same text is rejected too; the test
  says so in a comment, and the docs sentence says "a body may not contain `<a2amx-message`".
- `tests/prop_messaging.rs`: replace `injected_body_id_cannot_displace_the_envelopes_own_id` by a
  property that injects `<a2amx-message id="m_{k}"` at a random position and asserts
  `validate_message` rejects it with `invalid_content`. No `prop_assume!` on acceptance. Keep body
  plus injection within `MAX_MESSAGE_BYTES` (use the existing `safe_bodies(1..=32700)`: the
  injection is at most 31 bytes, so the total stays under 32768 and the size check at `:386`
  cannot fire first) and keep control characters out (they are checked first, `:395`). Add one
  property: a body built from fragments such as `<a2amx-`, `a2amx-message id="m_`, `<`, `"` and
  newlines that passes `validate_message` renders an envelope with `envelope_ids(render) == [n]`
  (`safe_bodies` never produces `<`, so nothing tests this today). Add a case-variant injection
  (`<A2AMX-Message id="m_{k}"`) to the rejecting property.
- Existing tests that render a hand-built envelope (`tests/bridge.rs`, `tests/delivery.rs`,
  `tests/claude_channel.rs`, `tests/hook.rs`) call `render_envelope` or write literals directly;
  they do not go through `validate_message` and must not change. The developer greps and lists
  them.

## 5. Docs

- `docs/architecture.md:426`: the limits sentence says control characters are rejected "(the
  envelope terminator too)". Make it say the envelope terminator and the text `<a2amx-message`
  (an opening envelope tag) are rejected. No other doc states the limits (`docs/mcp.md` does not
  exist; `rg -n 'invalid_content' docs --glob '!docs/specs/*'` finds only `architecture.md:429`).
- `SECURITY.md`: after the "What is in scope" list add a short section with its own heading
  ("Not handled as a vulnerability"): confinement is a safety net against honest mistakes, not a
  sandbox or a security boundary (link `docs/philosophy.md`); an escape from it is welcome as a
  normal bug report (a public issue is fine) and is fixed on a best-effort basis; an escape that
  also reaches an item above (tokens, another user's state files, acting as another session) is
  still reported privately. Keep the in-scope item about content that forges the envelope. Read
  the whole file first and keep its structure.

## 6. Claims about existing code

| Claim | Where | Verified how |
|---|---|---|
| `validate_message` rejects `</a2amx-message>` only | `src/messaging.rs:404-409` | read |
| `envelope_ids` reads `<a2amx-message id="m_<digits>"` | `src/messaging.rs:529-546` | read |
| The subject is escaped (`<` to `&lt;`) in the envelope | `src/messaging.rs:413-422, 430-436` | read |
| The channel path needs exactly one id and an exact envelope match | `src/daemon.rs:1344-1356` | read |
| On the hook path a receipt needs exactly one known id and an exact text match; two or more known ids block the prompt and charge a rejection to each; a second rejection makes a message undeliverable | `src/daemon.rs:1319-1327, 1383-1418`, `docs/delivery.md:407` | read (consultant, then architect); `store::reject_attempt` internals not read |
| The daemon's id parser matches only the lowercase prefix `<a2amx-message id="m_` | `src/messaging.rs:530` | read |
| The text `<a2amx-message` appears in agent-facing prose that agents copy: the peer-authorization prompt, the server instructions, `docs/architecture.md`, `docs/delivery.md`, the owner's shared agent instructions | `src/harness.rs:76`, `src/mcp.rs:457`, docs | grep (consultant); a message that quotes it now fails with the actionable text above (the owner's decision) |
| The daemon validates every agent send with `validate_message` | `src/daemon.rs:1136` | read |
| The only other text containing `<a2amx-message` in `src/` is the envelope renderer, the id parser, two system-prompt constants (`src/mcp.rs:457`, `src/harness.rs:76`) and delivery test strings; none goes through `validate_message` | `src` | grep |
| A test pins today's acceptance of an injected opening tag | `tests/prop_messaging.rs` `injected_body_id_cannot_displace_the_envelopes_own_id` | read |
| No daemon-generated body contains the opening text | `src/daemon.rs` | grep: no `a2amx-message` in `src/daemon.rs`; system notices carry names, not tags |

## 7. Out of scope

Subject handling; `envelope_ids`; the hook and channel paths; the closing-tag check; message size limits; the `send_message` tool description in `src/mcp.rs` (it names neither tag; a backlog note); the shared agent instructions that warn about the paste-wrapper tag (the owner's); `CHANGELOG.md`
(the architect adds the public-interface entry, because the MCP `send_message` now rejects
bodies it accepted), the backlog rows and the roadmap line.

## 8. Acceptance

1. `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`
   pass. Judge: the architect reruns them.
2. Red phase: the new rejecting cases in `tests/messaging.rs` and the replaced property fail on the
   old code. The case that `validate_message("subject", "the a2amx-message element")` is accepted,
   the fragments property, and the closing-tag case pass on the old code (contract tests); the
   developer names them.
3. `rg -n 'terminator too' docs/architecture.md` finds nothing; the new sentence names both tags.
4. Mutation check by the developer, in the checkout, restored by checksum: (a) remove the new
   check: the new unit cases and the property fail; (b) narrow it to `<a2amx-message id=`: the
   no-attribute case fails; (c) make it case-sensitive: the case-variant injection fails; (d) apply
   it to the subject too: `a_subject_cannot_add_a_tag_or_an_id` fails.
5. Public interface: the MCP `send_message` error text for the new case is
   `message contains an envelope opening tag (write it without the leading <)` with code
   `invalid_content`; the changelog line is the architect's.
6. `git status --short` shows only files in scope. Scratch only under `target/scratch/3cn/`.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5 (fresh); developer and tester OMP
with `openai-codex/gpt-5.6-luna` for planning and working (advisor model not exposed by the
harness). BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| Draft review | consultant | The spec said a second id on the hook path could forge a receipt; it cannot: a receipt needs exactly one known id and an exact text match, and two ids block the prompt and charge a rejection to each (the architect had not read past `daemon.rs:1376`) | Why and claims table rewritten around the real harm |
| Draft review | consultant | "Removes the path" overstated: case variants reach a model, and a split tag across a paste wrapper depends on Claude Code escaping the wrapper; no attack-path table; the error text gave the sender no hint; acceptance wording (an accept case cannot be red; the property's size bound; a docs grep matching specs) | Case-insensitive check, attack-path table, fragments property, actionable error text, acceptance reworded, SECURITY.md heading and private-report carve-out |
| Tester pass | tester | One mutant survived (the needle cut to its first 13 bytes); the Generic harness joins a split wrapper (outside the hook path, documented in the spec) | Architect added a near-prefix acceptance assertion (tiny change); the wrapper case is the spec's recorded constraint |
| Tester pass | tester | Four timing-sensitive tests failed once in a full run that overlapped the architect's own test runs; each passed alone | Classified as load contention; backlog I18 |
