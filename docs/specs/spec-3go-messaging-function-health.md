# Spec 3go: `src/messaging.rs` function-level code health (seven findings)

## 0. Status

LOCKED after consultant review m_8641 (ten findings applied); assign only after spec 3gn is accepted, because both edit `src/messaging.rs`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the commit that adds this file. Context `a2amx-main-code-health-2026-11-22`.

Scope (edit): `src/messaging.rs`, `tests/messaging/limits_and_envelopes.rs`, `tests/messaging/paste_and_teams.rs`. Does not touch: any other file, `Cargo.toml`, `CHANGELOG.md`.

Owner rules that bind this spec (memories d295229d, 828f8118, fb283e4c): the goal is a CodeScene-clean repo so `cs` can become a mandatory gate; no `@codescene` comments; no finding is dismissed without the owner's approval. A finding this spec does not fix is listed in section 4 as flagged.

## 1. Why

`cs check src/messaging.rs` (cs 1.0.58, 2026-11-22; line numbers before spec 3gn lands, so match by function name) reports, after 3gn removes `sgr_mouse_report_len`:

| Function | Finding | Verdict |
|---|---|---|
| `visible` | Complex Method cc 9 | real: one nested `match` plus a second `match` in one boolean |
| `default_host_name` | Complex Conditional (character test) | real: the identical predicate (`is_ascii_lowercase() \|\| is_ascii_digit() \|\| == '-'`, on `char`) is the last line of `valid_dns_label` (`src/messaging.rs:346` at HEAD b9271c5); the first-character rule at `:343` (no hyphen) is a different predicate and stays separate |
| `parse_interval` | Complex Method cc 9 | real: an `if let` ladder over three suffixes |
| `validate_message` | Complex Method cc 11 | real: eight guards in one function mixing subject and body rules |
| `validate_message` | Complex Conditional (control character but not `\n`/`\t`) | real: a named predicate reads better and `sanitize_draft` uses the same `\n \| \t` set |
| `unwrap_pastes` | Complex Method cc 10 | real: one loop does the search, the prefix test, the id parse and the closing-tag search |
| `envelope_ids` | Complex Conditional (four clauses in one `if`) | real: parse and accept rule in one expression |

## 2. Changes (one commit-sized step each; the whole suite must stay green after every step)

Before each step the developer confirms the behavior is pinned: names the existing test(s) that cover each branch of the function (`tests/messaging/*`, `tests/prop_messaging.rs`, the unit tests at the bottom of `src/messaging.rs`) and adds a characterization test, passing before and after, for any branch no test pins. Expected values are literals, never recomputed with the function under test. No behavior change anywhere; error texts stay byte for byte.

1. `visible`: extract the team-level rule and the agents-list rule into two private functions (names the developer chooses); `visible` combines them with `||`. Same truth table for every pair of parties.
2. Character predicate: one private function (`is_label_char` or similar) used by `default_host_name` and by `valid_dns_label` (not the `valid_name` in `src/relay_files.rs`, which is out of scope); the first-character rule stays separate. `default_host_name` reads `/proc` and has no characterization seam; any mutant of the shared predicate is killed through `validate_name`/`validate_host` (`tests/messaging/limits_and_envelopes.rs:168`). The branch at the end of `default_host_name` that returns a fallback when the host is empty after truncation is dead code (flagged, section 4, left untouched).
3. `parse_interval`: replace the `if let` ladder with a table of `(suffix, multiplier)` searched with `strip_suffix` (no `split_at`: it panics on `""` and on a multi-byte last character), keeping the exact error text and the `1..=HEARTBEAT_MAX` range. Add the characterization case `"1é"` with its exact error text. `interval_from_seconds` repeats the error text; it may share the message construction if that does not change its signature, otherwise it is flagged (section 4).
4. `validate_message`: the eight guards alternate between subject and message (subject empty, message empty, subject size, message size, subject control, message control, terminator, opening tag), so a subject-check-then-body-check split changes which guard fires first (for `(201 x "s", "")` today `invalid_content`, after such a split `too_large`; for `("line\nsubject", 32769 x "m")` today `too_large`, after it `invalid_content`). Split along a line that keeps the order: guards 1 to 5 stay in the public function; guards 6 to 8 (message control character, terminator, opening tag) move into a private body-content check called last. First write one precedence test per adjacent guard pair whose results differ (1|2, 2|3, 3|4, 4|5, 5|6, 6|7, 7|8), asserting the code and the exact text, and assert every one of the eight error texts literally (today only the terminator and opening-tag texts are pinned, `tests/messaging/limits_and_envelopes.rs:66-77`). The disallowed-control test becomes a private predicate named for the rejected set (`is_disallowed_control`: a control character other than `\n` and `\t`); `sanitize_draft` keeps `!is_disallowed_control(c)`, which is its exact current negation (verified by the consultant against `:536` and `:705`).
5. `unwrap_pastes`: first add the characterization test for the `start < cursor` branch (nothing pins it): input `"\n\n<pasted_content id=\"1\">\nX\n</pasted_content id=\"1\">\n\n<pasted_content id=\"2\">\nY\n</pasted_content id=\"2\">\n"` is expected to give `"X\n<pasted_content id=\"2\">\nY\n</pasted_content id=\"2\">\n"`; the developer confirms the value against HEAD before relying on it and reports if it differs. Then extract the per-candidate parse (prefix test, id parse, closing search) into a private function returning the candidate's `(start, inner_start, inner_end, after)` or `None`; `search = tag + 1` stays before the first `continue`, the `start < cursor` test stays in the caller, and the `// shortcut:` comment stays with the matcher it describes.
6. `envelope_ids`: extract the digit-string-to-sequence rule (1..=18 digits, closing quote, parse, `> 0`) into a private function returning `Option<i64>`, leaving `if let Some(seq) = f(digits) && !ids.contains(&seq)` in the loop; first-appearance order and de-duplication unchanged (pinned by `tests/messaging/paste_and_teams.rs:69-94` and `tests/prop_messaging.rs:319-322`). If `cs check` still reports a Complex Conditional on that line, the developer says so and proposes the next split instead of dismissing it.

## 3. Acceptance

- `cargo test --no-fail-fast`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; test count reported before and after (only characterization tests added).
- `git status --short` lists only the files named in section 0.
- `CS_DISABLE_VERSION_CHECK=1 cs check src/messaging.rs` (reported): none of the seven findings in section 1 remains and no new finding appears; the developer pastes the before and after warn lines.
- Tester mutation pass, naming the killing test: `&&` to `||` in each mutual-consent test of `visible`'s two rules, `||` to `&&` in their combiner, and delete each match guard; swap each adjacent guard pair of `validate_message`; drop the `\n` exemption and separately the `\t` exemption of `is_disallowed_control`; drop `-` from the label predicate and make its digit test false; delete one row of the interval table, drop the all-digits check, change a multiplier; the `start < cursor` test of `unwrap_pastes`; `seq > 0` to `>= 0` and the 1..=18 bound to 1..=19 in `envelope_ids`. The tester mutates every condition of every new function (lesson of spec 3et).

## 4. Flagged for the owner (not fixed here)

| Finding | Why flagged |
|---|---|
| `render_envelope` Excess Number of Function Arguments (5 string arguments) and the file-level String Heavy Function Arguments | A fix is an `Envelope` parameter struct, a public-signature change touching 13 call sites (one production caller, `src/store.rs`; two unit tests in `src/messaging.rs`; ten in `tests/prop_messaging.rs` and `tests/messaging/limits_and_envelopes.rs`). It would not be a false positive; it is a design choice with a large diff. Proposal: do it as its own spec if the owner agrees. |
| file-level Low Cohesion | `messaging.rs` holds names, intervals, envelopes, paste handling, team visibility and input classification. Fixing it means splitting the module and moving public paths used by tests and other modules. Proposal: a separate spec after this one, once the remaining content is known. |
