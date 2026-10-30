# Spec 3bd: the status line tag says `[confined]`, not `[sandbox]`

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `eacd6b6`; no consultant review: a one-word rename, see section 6). Owner request, 2026-10-30, context `a2amx-confined-tag-2026-10-30`.
Implementation goes to `a2amx-developer` only. No tester pass: a rename with one test.

**Baseline:** develop at `eacd6b6`; `cargo test --test status` passed 17 of 17 there (architect, 2026-10-30).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** `status::render` through `tests/status.rs`.

**Scope.** May edit exactly these files and no others: `src/status.rs`, `tests/status.rs`, `docs/cli.md` (line 77),
`docs/harnesses.md` (line 59). Does not touch: `CHANGELOG.md` (the architect edits the Unreleased entry), `docs/specs/*`
(spec 3ax stays as a historical record), `src/confine.rs`, `src/daemon.rs`, every other file.
No new dependency. Leave the diff uncommitted.

## 1. Why

The status bar tag for a confined session reads `[sandbox]`. The owner decided that is too strong a word: confinement
is a safety net, not a full sandbox (the threat model of spec 3ax section 1 and spec 3bb). The tag has not shipped in
a release (it is in the Unreleased changelog entry only), so renaming it breaks nobody. The wire field is already
`confined` and stays.

## 2. Changes

1. `src/status.rs:431`: the tag text `"[sandbox]"` becomes `"[confined]"`. Keep the dim attributes and the following
   space. For consistency rename the identifiers `show_sandbox` (lines 254, 263, 298-299, 307) and the `sandbox`
   parameter of `left_spans` (lines 418, 430) to `confined`. No other change; the drop order of the tag stays last.
   The tag is one character wider, so a width where the old tag just fit may now drop it. No existing test pins such a
   width (claims table); do not add one.
2. `tests/status.rs:47-53`: rename the test `confined_status_shows_dim_sandbox_tag` to
   `confined_status_shows_dim_confined_tag` and assert `"\x1b[2m[confined]\x1b[22m"`. Write this change first: the
   assertion fails on the unchanged `src/status.rs` (red phase), then passes after item 1.
3. `docs/cli.md:77`: `` `[sandbox]` `` becomes `` `[confined]` ``.
4. `docs/harnesses.md:59`: "an opt-in bubblewrap sandbox around the PTY child" becomes "an opt-in bubblewrap confinement
   of the PTY child". Leave every other use of the word "sandbox" in the docs: they name bubblewrap's wrapper or a
   harness's own nested sandbox, which are real sandboxes.

## 3. Out of scope

`CHANGELOG.md`, spec files, the launch notice text (it never says "sandbox"), the `confined` wire field, renames outside
`src/status.rs`, and any other wording of the docs.

## 4. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --test status
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the four files of section 0
rg -n -i 'sandbox' src/status.rs tests/status.rs docs/cli.md | rg -n '\[sandbox\]|show_sandbox|sandbox: bool|dim_sandbox'   # must print nothing
rg -n '\[confined\]' src/status.rs tests/status.rs docs/cli.md   # one hit in each of the three files
```

- The new assertion fails on the unchanged `src/status.rs`: the developer shows it (stash or scratch copy) and reports
  the failure line.
- In a confined session the exact `cargo test` fails on `real_bwrap_confines_the_session_process` (read-only HOME,
  backlog F40); use the writable-HOME form
  `HOME=<writable dir> CARGO_HOME=/home/zbalint/.cargo RUSTUP_HOME=/home/zbalint/.rustup` and report both results.
  Known flaky: backlog I9 and F37, rerun once and record as pre-existing.

## 5. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| The tag is rendered only at `src/status.rs:431` | `rg -n -i sandbox src --glob '!src/confine.rs'` printed only `src/status.rs` lines 254-431 | grep |
| `src/confine.rs` contains no "sandbox" | `rg -c -i sandbox src/confine.rs` printed nothing | grep |
| Only `tests/status.rs:52` asserts the tag; no other test mentions `[sandbox]` | `rg -n '\[sandbox\]' tests docs README.md CHANGELOG.md` | grep |
| No test pins a width at which the tag is the last item dropped | `rg -n 'confined' tests/status.rs tests/attach_cli.rs tests/daemon_cli.rs` shows only `confined: false` fixtures and the one test | grep |
| The wire field is `confined` | `src/status.rs:254` (`info.confined`), `tests/status.rs:49` | read |
| The tag has not shipped in a release | only `CHANGELOG.md:24` (Unreleased) names it; latest tag is `v0.1.2` | read, `git tag --list 'v*'` printed `v0.1.1`, `v0.1.1-rc.1`, `v0.1.2` |

## 6. Pre-lock record

Gate steps 1-9 run (claims table). Step 5 (content change): the old text `[sandbox]` was grepped across the whole tree,
including specs (3ax lines 234 and 325 are historical and stay). Steps 10-11 do not apply; step 12: baseline
`cargo test --test status` run (17 passed); step 13: no trust boundary. No consultant review: the change is one
literal, one test and two doc lines, and the claims are greps recorded above.

## Review record

Models: architect Claude Sonnet 5.5; no consultant review (a one-literal rename, see section 6); developer on the OMP
harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed); no tester pass.
BLOCKED/amend cycles: 0.

No problems found. The developer showed the renamed test failing on the unchanged source (`tests/status.rs:52`) before
the rename, then 697 passed, 0 failed, 3 ignored with a writable HOME; the exact environment shows only the known
read-only-HOME failure (backlog F40).
