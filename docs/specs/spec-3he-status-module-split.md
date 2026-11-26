# Spec 3he: split `src/status.rs` into the team line, the status bar and the drawing helpers they share (move only)

## 0. Status

LOCKED after consultant review m_9346 (six findings applied; none changed a rule, so no second review). Context `a2amx-3he-status-split`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, base the commit that adds this spec. Public seams: the existing `a2amx::status` paths (`render`, `StatusBar`, `clear_status`, `render_team_line`, `render_team_rule`, `TeamLine`, `TeamRegion`, `TeamTarget`), driven by `tests/status.rs`, `tests/status_layout.rs`, `tests/attach_model.rs` and `tests/daemon/status_reporting.rs`; no test changes.

Scope (edit): `src/status.rs`; new `src/status/spans.rs`, `src/status/dot.rs`, `src/status/team_line.rs`, `src/status/bar.rs`. Does not touch: any test file, `src/lib.rs` (`pub mod status;` stays), `src/attach/model.rs` or any other caller, `docs/` (the `status` row of the module table in `AGENTS.md` stays true), `CHANGELOG.md` (no user-visible change).

The spec follows spec 3hd's amendment 1: after the function-health work `cs check src/status.rs` reports one finding, file-level Low Cohesion, because the file draws two screen elements with separate callers (the team line and rule at `src/attach/model.rs:389-401`, `:582-591`; the bar and its clear at `:405`, `:589`). Owner decision (2026-11-26, direct): the goal is no `cs` finding left or each one marked a proved false positive; enforcement follows. Addition the owner did not raise: two shared helper modules (`spans`, `dot`) instead of leaving the shared code in the root (section 1).

## 1. Why

`src/status.rs` (738 lines) holds three kinds of code: the team line (row 1) and team rule (row 2), the status bar (last row) and its clear, and the small drawing kit both use (`Span`, the styled-text length and render functions, the colour constants, the activity dot). Its callers use the first two kinds separately, and the old file had no Low Cohesion finding only while two giant functions tied everything together.

Target shape, one responsibility each:

| File | Owns | Public or shared items |
| --- | --- | --- |
| `src/status.rs` (root, 7 lines) | declares the four modules and re-exports the public paths | `pub use bar::{StatusBar, clear_status, render};` and `pub use team_line::{TeamLine, TeamRegion, TeamTarget, render_team_line, render_team_rule};` |
| `src/status/team_line.rs` | the team member line on row 1 and the separator on row 2 | `TeamRegion`, `TeamTarget`, `TeamLine`, `render_team_line`, `render_team_rule` (all `pub`); everything else private |
| `src/status/bar.rs` | the status bar on the last row and its clear | `StatusBar`, `render`, `clear_status` (`pub`); everything else private |
| `src/status/spans.rs` | styled text: `Span`, its length and render functions, the colour constants, the separator row and the detail prefix | the 14 `pub(super)` items of section 2 |
| `src/status/dot.rs` | the activity dot: `DotState`, how a session's facts map to it, its word and colour | `DotState`, `dot_state`, `activity_word`, `activity_color` (`pub(super)`) |

Every item in `spans.rs` and `dot.rs` either has callers in both consumers (the team code and the bar code; verified by `rg`, section 6), or reads `Span`'s private fields (`render_span`), or belongs with its type (`activity_word` with `DotState`); so none belongs in either consumer. A move-only split is the point: no behaviour, wording, colour, width or priority changes (the earlier specs on this file and on `attach/model.rs` hold those).

Alternatives rejected: splitting only into `team_line.rs` and `bar.rs` and keeping the drawing kit in the root (the root would hold the kit plus the activity dot, two unrelated data shapes, the same cohesion problem one level up); a single `shared.rs` (same reason); making `Span` fields `pub(super)` (nothing outside `spans.rs` reads them).

## 2. The moves

Line numbers are those of `src/status.rs` at the base commit. Each new file is the listed ranges in the listed order, one blank line between ranges, moved byte-for-byte (comments and the `// shortcut:` comment included), plus its own `use` lines. A range starts at its first attribute, doc-comment or code line.

| File | Old lines |
| --- | --- |
| `src/status/spans.rs` | 5-9, 12-39, 490-497, 516-528, 633-638, 709-738 |
| `src/status/dot.rs` | 640-689 |
| `src/status/team_line.rs` | 41-250, 499-514 |
| `src/status/bar.rs` | 10, 252-488, 530-631, 691-707 |

Lines 1-3 (the `use` lines), 4, 11, 40, 251, 489, 498, 515, 529, 632, 639, 690 and 708 (blank) are not moved: the `use` lines are redistributed by the compiler's need (rule 3), the blanks are separators. Together with the root's new lines the ranges account for every old line once.

Edits, and the only edits besides the moves:

1. **Declarations in `src/status.rs`.** Replace the whole old content with `mod bar;`, `mod dot;`, `mod spans;`, `mod team_line;`, a blank line, then the two `pub use` lines of section 1. `src/lib.rs` is unchanged.
2. **Visibility.** Exactly 18 private items become `pub(super)`, because a sibling module calls them: in `spans.rs` the constants `BAR_FOREGROUND`, `BAR_BACKGROUND`, `NAME_OPEN` and `NAME_CLOSE` (`SEPARATOR_OPEN` stays private there; `ALERT_OPEN` moved to `bar.rs` and stays private), the struct `Span` and its methods `plain` and `styled` (`Span::len` and the three fields stay private), and the functions `render_separator`, `sanitize`, `char_len`, `spans_len`, `detail_prefix`, `render_spans`, `render_span`; in `dot.rs` the enum `DotState` and the functions `dot_state`, `activity_word`, `activity_color`. The signature text is otherwise unchanged; `cargo fmt` wraps `dot_state`'s signature into six lines (section 7, acceptance 2). No other item changes visibility.
3. **Imports.** Each file keeps exactly the `use` lines it needs (the compiler and clippy's `unused_imports` are the judge); a module names a sibling's items through `use super::spans::{...}` and `use super::dot::{...}`, never `crate::status::...` (the uses check of acceptance 1 would match it). A `use` list over 100 columns wraps into a multi-line `use super::...::{ ... };` block, which the `sed` of acceptance 2 removes (its closing `};` is at column 0). The root keeps no `use` line.
4. **Everything else** is moved byte-for-byte, with the relative order inside each file unchanged. `cargo fmt` runs once after assembly.

Placement check (spec-writing step 14): `team_line.rs` and `bar.rs` each own one screen element; `spans.rs` and `dot.rs` hold what two callers share. No file ends over 400 lines (the ranges give `bar.rs` 360, `team_line.rs` 227, `spans.rs` 95, `dot.rs` 55 and root 7 before each file's `use` lines).

## 3. Tests

No test is added, changed or moved: the split changes no behaviour and no public path, and the four test files above already pin the rendered bytes. The developer runs them unchanged.

## 4. Out of scope

- Any behaviour, wording, colour, width or priority change; the `shortcut:` comment's East Asian Ambiguous width ceiling.
- `src/attach/model.rs` and every other caller of `status::`.
- The `cs` findings of `tests/status.rs` (5.09), `tests/attach_model.rs` and `tests/daemon/status_reporting.rs`: the later test-file plan.
- Moving `render_team_line`'s callers, renaming items, or merging `fit_members` and `fit_bar`.
- A typed split of `StatusBar`/`Parts`; `Parts::new` stays as it is.
- Updating the `AGENTS.md` module table (the `status` row stays true).
- `docs/backlog.md:107` (F72) names `status.rs` `left_spans`, which moves to `src/status/bar.rs`: the architect updates that reference when committing; it is not in the developer's scope.

## 5. Trust boundary

None crossed: the module renders bytes from values the daemon already validated and sanitizes names and reasons (`sanitize`, `clipped_name`); the split moves that code unchanged and adds no input, path, network or process use.

## 6. Claims (verified by the architect on `3cc1605` and `a119e8e`)

| Claim | Where | Verified how |
| --- | --- | --- |
| `src/status.rs` is 738 lines; no `#[cfg(test)]` block; no `//!` header | `wc -l`; `rg -c 'cfg\(test\)'` returns 0 matches | probed |
| Callers use only `render`, `StatusBar`, `clear_status`, `render_team_line`, `render_team_rule`, `TeamRegion`, `TeamTarget` by name (`TeamLine` is the return type of `render_team_line`) | `rg -o 'status::[A-Za-z_]+' src tests`: 59 `render`, 59 `StatusBar`, 33 `TeamTarget`, 30 `TeamRegion`, 20 `render_team_line`, 5 `render_team_rule`, 1 `clear_status`; `use a2amx::status::{...}` at `tests/attach_model.rs:7` | probed |
| Items used by both team code and bar code: the four constants, `Span`, `sanitize`, `char_len`, `spans_len`, `render_spans`, `render_separator`, `detail_prefix`, `dot_state`, `activity_color` | line-by-line `rg -n` of each symbol in the old file (team lines 41-250 and 499-514, bar lines 252-488 and 530-707) | probed |
| `ALERT_OPEN` is used only at old line 431 (bar); `SEPARATOR_OPEN` only at 491 and 635 (both `spans.rs` after the move); `activity_word` only at 610 (bar) | `rg -n` | probed |
| `Span`'s fields are read only in `render_spans` and `render_span`; `Span::len` only in `spans_len` | read of old lines 12-39, 709-738 | read |
| The ranges plus the unmoved lines cover old lines 1-738 once each | script over the ranges | probed |
| Concatenating the section 2 ranges, applying the 18 visibility edits and running the normalized sort-and-diff of section 7 gives exactly the expected block there; rustfmt (edition 2024, `max_width = 100`) changes only `dot.rs` | scripted rebuild in a scratch directory, `rustfmt` | probed |
| `pub(super)` makes an item visible to the parent `status` and all its child modules, so a sibling can use it | Rust visibility rules | recall; the compiler settles it in the developer's first build |

## 7. Acceptance

1. **Baseline.** Before editing: `cargo test -- --list 2>/dev/null | rg ': test$' | LC_ALL=C sort > target/scratch/3he/before.txt` (not empty), the totals of a full `cargo test --no-fail-fast` (architect's baseline at `11ed84d`: 65 suites, 1488 passed, 0 failed, 3 ignored; the developer records its own), `rg -n 'status::' src tests | LC_ALL=C sort > target/scratch/3he/uses-before.txt`, `git show HEAD:src/status.rs > target/scratch/3he/status.rs`, and `CS_DISABLE_VERSION_CHECK=1 cs check src/status.rs` (expected: one finding, Low Cohesion). After the split: the sorted test list is identical to `before.txt`, the totals are equal, and `rg -n 'status::' src tests | LC_ALL=C sort` diffs empty against `uses-before.txt`.
2. **Same code.** `old` = `git show HEAD:src/status.rs`, `new` = `cat src/status/spans.rs src/status/dot.rs src/status/team_line.rs src/status/bar.rs`. After `sed '/^\(pub \|pub(crate) \|pub(super) \)\?use .*{$/,/^};$/d; s/^[[:space:]]*//; /^$/d; /^\(use \|mod \|pub use \|pub(crate) use \|pub(super) use \)/d'` and `LC_ALL=C sort`, `diff old new | rg '^[<>]' | LC_ALL=C sort` (after `cargo fmt`; the raw `diff` also prints hunk headers and lists lines in position order, so the pipeline is the check) prints exactly the lines of the block below and nothing else (the architect ran this on a scripted rebuild, formatted with the repository's `rustfmt.toml`, before locking): the 18 old signature lines removed, their `pub(super)` forms added, with `dot_state` wrapped by `cargo fmt` into the six added lines shown.

```text
< const BAR_BACKGROUND: &str = "\x1b[38;5;252;48;5;236m";
< const BAR_FOREGROUND: &str = "\x1b[38;5;252m";
< const NAME_CLOSE: &str = "\x1b[22m";
< const NAME_OPEN: &str = "\x1b[1m";
< enum DotState {
< fn activity_color(state: DotState) -> &'static str {
< fn activity_word(state: DotState) -> &'static str {
< fn char_len(text: &str) -> usize {
< fn detail_prefix() -> [Span; 2] {
< fn dot_state(activity: Option<Activity>, held: bool, starting: bool, exited: bool) -> DotState {
< fn plain(text: impl Into<String>) -> Self {
< fn render_separator(row: u16, width: usize) -> Vec<u8> {
< fn render_span(span: &Span, limit: usize) -> String {
< fn render_spans(spans: &[Span], limit: usize) -> String {
< fn sanitize(text: &str) -> String {
< fn spans_len(spans: &[Span]) -> usize {
< fn styled(open: &'static str, text: impl Into<String>, close: &'static str) -> Self {
< struct Span {
> ) -> DotState {
> activity: Option<Activity>,
> exited: bool,
> held: bool,
> pub(super) const BAR_BACKGROUND: &str = "\x1b[38;5;252;48;5;236m";
> pub(super) const BAR_FOREGROUND: &str = "\x1b[38;5;252m";
> pub(super) const NAME_CLOSE: &str = "\x1b[22m";
> pub(super) const NAME_OPEN: &str = "\x1b[1m";
> pub(super) enum DotState {
> pub(super) fn activity_color(state: DotState) -> &'static str {
> pub(super) fn activity_word(state: DotState) -> &'static str {
> pub(super) fn char_len(text: &str) -> usize {
> pub(super) fn detail_prefix() -> [Span; 2] {
> pub(super) fn dot_state(
> pub(super) fn plain(text: impl Into<String>) -> Self {
> pub(super) fn render_separator(row: u16, width: usize) -> Vec<u8> {
> pub(super) fn render_span(span: &Span, limit: usize) -> String {
> pub(super) fn render_spans(spans: &[Span], limit: usize) -> String {
> pub(super) fn sanitize(text: &str) -> String {
> pub(super) fn spans_len(spans: &[Span]) -> usize {
> pub(super) fn styled(open: &'static str, text: impl Into<String>, close: &'static str) -> Self {
> pub(super) struct Span {
> starting: bool,
```

   The comparison is of whole lines; `mod` and `pub use` lines are deleted by the `sed`.
3. **Order inside each file.** The architect rebuilds the four files from the section 2 ranges and compares in order with the developer's; the tester repeats this independently.
4. `wc -l src/status.rs src/status/*.rs`: every file under 1,000. The ranges give 7 (root), 55 (`dot.rs`), 95 (`spans.rs`), 227 (`team_line.rs`) and 360 (`bar.rs`) lines before the `use` lines each file adds, so expect about 57, 231-235 and 368-372 for the last three; the count is information, only the 1,000 limit is checked.
5. `CS_DISABLE_VERSION_CHECK=1 cs check` on each of the five files: no finding on any of them; report the scores. The architect's scratch rebuild (the four files formatted, without `use` lines) scored 10.00 on `bar.rs`, `dot.rs`, `spans.rs` and `team_line.rs`; the 7-line root has no score (`N/A`), which counts as no finding. A remaining finding is reported to the architect, not worked around (no reshuffling to satisfy the metric, and no suppression comment).
6. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass; `git diff --stat` and `git status --short` list only `src/status.rs` and the four new files.
7. Mutation pass: none, since no test and no behaviour changes; the tester's pass is the order check (3) and the full-suite run.

After the REPORT the developer stays idle and starts nothing until the architect assigns again.
