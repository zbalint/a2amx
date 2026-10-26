# Spec 3aq: A blocking Semgrep gate with the repository's own rules

## 0. Status

**LOCKED** (2026-10-26; pre-lock gate run against develop at `2b06e86`; consultant review `m_3960`, findings adopted).
Architect-directed, backlog F15, context `a2amx-semgrep-gate-2026-10-26`. Implementation goes to `a2amx-developer` only.

**Baseline (develop at `2b06e86`, run 2026-10-26):** `scripts/prepush-fast.sh` exits 0 in about 2 s;
`cargo clippy --all-targets -- -D clippy::undocumented_unsafe_blocks` exits 0 with no output. Semgrep 1.181.0 is
installed at `~/.local/bin/semgrep`. The full suite is run as `env -u A2AMX_BIN -u NO_COLOR cargo test`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** the Semgrep fixture suite (`semgrep scan --test`, section 3.3) for each rule, and the exit code
of `scripts/prepush-fast.sh` for the whole gate. There is no Rust test: the rules are not Rust code.

**Scope.** May edit exactly these paths and no others.

- new: `.semgrep/rules/*.yml` (the four rule files of section 3.1) and `.semgrep/tests/*.rs` (one fixture per rule)
- `scripts/prepush-fast.sh`
- `.github/workflows/ci.yml`: the `hygiene` job only (its comment, `env`, and one install step)
- `sonar-project.properties`: one new `sonar.exclusions` line
- `Cargo.toml`: one new line in `[lints.clippy]`
- `AGENTS.md`: one new short subsection, section 3.6

Does not touch: every file under `src/` and `tests/`, `Cargo.lock`, `deny.toml`, `docs/`, `CHANGELOG.md`, `README.md`,
`CONTRIBUTING.md`, the other workflows (`release.yml`, `scorecard.yml`), the other jobs of `ci.yml`. No new Rust
dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

`AGENTS.md` states conventions that only review enforces today: terminal-emulation types are named in
`src/emulator.rs` only and the PTY module in `src/session.rs` only, no shell-interpolated commands, never send Ctrl-C
to a PTY, and a `// SAFETY:` comment on every `unsafe`. Reviewers have caught drift on these by luck. SALTMDB solved
the same problem with repository-local Semgrep rules that run in its pre-push script and in the CI `hygiene` job
(`SEMGREP_VERSION` pinned, `pipx install`, no login, one rule file per invariant with a fixture, a guard against a
zero-test result). a2amx has no Semgrep at all, so a pull request cannot fail on a rule, while Sonar already runs on
every same-repository pull request. The owner wants the gap closed because pull requests are open.

Only invariants a pattern can decide exactly are rules. A broad registry scan (`--config auto`) is a separate,
non-blocking job (backlog F16) and is not part of this spec: on 2026-10-16 its Rust findings were five false
positives from a web-framework rule, so a blocking gate on it would not hold.

**Rejected, with the evidence.** "No `std::fs` in an `async fn` outside `spawn_blocking`" is not a rule. A probe
`pattern-inside: async fn` with `pattern-not-inside: spawn_blocking(...)` (a) did not exempt the closure form until
it was written `tokio::task::spawn_blocking(...)`, and (b) still flagged `src/codex.rs:367-368`, which are correct (not violations):
the calls sit in a closure bound to `prepare` and passed to `spawn_blocking(prepare)` on line 374. It also sees only
the fully qualified `std::fs::` spelling, not `fs::read` after `use std::fs`, so it would be noisy and incomplete at
once. "No `unwrap()` or `expect()`" is already enforced by clippy (`Cargo.toml` `[lints.clippy]`). A bare `0x03`
integer is not a rule target: it is the `Detach` frame tag in `src/wire.rs` (lines 499, 528, 556, 587).

## 2. Rule: the gate must not change behavior

The changes below add checks and move no product code. `src/` and `tests/` are untouched, so the existing suite
result is the baseline and must not change. If a rule flags `src/` on the current tree, that is a BLOCKED, not a
reason to edit `src/` or to weaken the rule (section 3.1 states the probed result: no finding on `2b06e86`).

## 3. Changes

### 3.1 `.semgrep/rules/`: four rule files

Every rule: `languages: [rust]`, `severity: ERROR`, a `message` that names the convention and where the code belongs,
and `metadata` carrying `source: AGENTS.md`. The patterns below were probed on 2026-10-26 with throwaway rules in the
architect's scratchpad against `src/` at `2b06e86` and gave no finding; the exact text is the target.

1. `no-shell-command.yml`, id `no-shell-command`:

   ```yaml
   patterns:
   - pattern: $P::new($X)
   - metavariable-pattern:
       metavariable: $P
       pattern-either:
       - pattern: Command
       - pattern: std::process::Command
       - pattern: tokio::process::Command
   - metavariable-regex:
       metavariable: $X
       regex: '^"(?:/(?:usr/)?bin/)?(?:sh|bash|dash|zsh|ksh|fish|cmd|cmd\.exe|powershell|pwsh)"$'
   ```

   It flags a literal shell program. A `Command::new(program)` with a variable is not decidable by a pattern and is
   not flagged (a gap, listed in 3.6). `-c` alone is not a rule target: `src/codex.rs:335`
   passes `-c` as a Codex config flag.
2. `no-ctrl-c-literal.yml`, id `no-ctrl-c-literal`:

   ```yaml
   pattern-regex: '(?:b?"[^"\n]*\\(?:x0?3|u\{0*3\})[^"\n]*"|b?''\\(?:x0?3|u\{0*3\})'')'
   ```

   It flags a string, byte-string, or char literal containing the Ctrl-C byte (`\x03`, `\x3`, `\u{3}`, `\u{03}`).
   An integer `0x03` is not flagged (see section 1).
3. `terminal-types-outside-emulator.yml`, id `terminal-types-outside-emulator`:

   ```yaml
   pattern-regex: '\balacritty_terminal\b(?!::tty\b)'
   paths:
     exclude: ["src/emulator.rs"]
   ```

   Any use of the emulation crate other than its `tty` path, anywhere but `src/emulator.rs`. A grouped import
   `use alacritty_terminal::{tty, ...}` is flagged (the `{` follows the crate name), which forces the one-path-per-line
   style.
4. `tty-outside-session.yml`, id `tty-outside-session`:

   ```yaml
   pattern-regex: '\balacritty_terminal::tty\b'
   paths:
     exclude: ["src/session.rs"]
   ```

   Together with rule 3 this keeps `alacritty_terminal::tty` in `src/session.rs` and everything else in
   `src/emulator.rs`. Known gap: a grouped import that names `tty` inside `src/emulator.rs`
   (`use alacritty_terminal::{tty, ...}`) is not flagged by either rule. Rule 3 excludes the file and rule 4 matches only
   the `::tty` spelling.

Further known gaps, listed in 3.6: rule 3 misses a `pub use alacritty_terminal::...` re-export from
`src/emulator.rs` that lets other files name the types without the crate name; rule 2 also flags `"\\x03"` (an escaped
backslash, not Ctrl-C) and misses `\x03` after an escaped quote in the same string; rule 1 misses
`Command::new("env").arg("sh")`.

`pattern-regex` does not distinguish comments from code; a comment naming one of these crates outside the allowed
files is a finding, which is acceptable (`src/emulator.rs` is the only file that mentions the crate in prose).

### 3.2 `Cargo.toml`: the `// SAFETY:` convention

Add `undocumented_unsafe_blocks = "deny"` to `[lints.clippy]`, after `unimplemented = "deny"` (line 32). This is
clippy's own check for the `AGENTS.md` rule, so no Semgrep rule is written for it: Semgrep's Rust parser drops
comments and cannot see a `// SAFETY:` line. The baseline probe above shows it passes on the current tree, which has
two `unsafe` blocks (`src/main.rs:231`, `tests/daemon.rs:1748`). Known gap: it covers blocks and `unsafe impl`, not
`unsafe fn` declarations.

### 3.3 `.semgrep/tests/`: one fixture per rule

One file per rule, named `<rule id>.rs`, so a rule is tested alone (two rules annotated in one file report each
other's matches as incorrect lines). Annotate with `// ruleid: <id>` and `// ok: <id>` on the line above the code, as
Semgrep requires. Cases each fixture must hold (at least):

- `no-shell-command`: `ruleid` for `Command::new("sh")`, `std::process::Command::new("/bin/bash")`,
  `tokio::process::Command::new("cmd.exe")`; `ok` for `Command::new("git")`, `Command::new("shell-tool")`,
  `Command::new(program)`.
- `no-ctrl-c-literal`: `ruleid` for `b"\x03"`, `"\u{3}"`, `'\x03'`, `b"ab\x03cd"`; `ok` for `b"\x1b[A"`,
  `vec![0x03]`, `"\x1b03"`.
- `terminal-types-outside-emulator`: `ruleid` for `use alacritty_terminal::term::Term;`,
  `use alacritty_terminal::{tty, grid};`, `use alacritty_terminal::vte::ansi::Color;`; `ok` for
  `use alacritty_terminal::tty::{self, ChildEvent};`.
- `tty-outside-session`: `ruleid` for `use alacritty_terminal::tty::{self, ChildEvent};`; `ok` for
  `use alacritty_terminal::term::Term;`.

Fixtures are not Rust crate members: `.semgrep/` is not under `src/`, `tests/` or any `[[test]]`, so `cargo` never
compiles them. Fixtures need not compile; they only need to parse.

### 3.4 `scripts/prepush-fast.sh`

- Add `semgrep` to the tool list on line 9 and to the header comment (lines 2 to 5): the script now also runs the
  repository's Semgrep rules, and needs `semgrep` on PATH (a missing tool is an error, as for the others).
- After the `zizmor` line, run the rules, then the fixture suite. Both use `--metrics=off --disable-version-check
  --oss-only`, so there is no login, no telemetry and no registry rules:

  ```sh
  semgrep scan --config .semgrep/rules --metrics=off --disable-version-check --oss-only --error --strict --quiet \
    --include '*.rs' src
  ```

  ```sh
  test_output="$(semgrep scan --test --config .semgrep/rules .semgrep/tests \
    --metrics=off --disable-version-check --oss-only --include '*.rs' 2>&1)" || {
    printf '%s\n' "$test_output" >&2
    exit 1
  }
  ```

  then `printf '%s\n' "$test_output"` on success (so the count is visible), and a guard. With an empty fixture
  directory `semgrep scan --test` prints "No unit tests found." and exits 0, with no `N/M:` line at all, so the guard
  takes `N` from the `N/M:` line and counts a missing line as 0, and exits 1 when `N` is 0 or when `N` differs from
  the number of `*.yml` files in `.semgrep/rules` (a rule whose fixture was deleted would otherwise still pass).
  The developer reads SALTMDB's `scripts/prepush-fast.sh:115-123` and mirrors the guard; the rest of that script
  (per-scope table, parallel batches) is not needed for four rules scanning one tree. `--strict` on the `src` scan
  makes a file Semgrep cannot parse fail the gate instead of being skipped silently; `src/` passes with it.

The scan targets `src` only, not `tests/` or `build.rs`: the conventions in `AGENTS.md` govern product code, and
`tests/` legitimately holds `0x03`-style literals and unsafe blocks.

### 3.5 `.github/workflows/ci.yml` and `sonar-project.properties`

- `hygiene` job (comment at line 35, job lines 37 to 74 at `2b06e86`): add `SEMGREP_VERSION: 1.181.0` to its `env`, an `Install semgrep` step after
  `Install zizmor` modeled on SALTMDB's (`pipx install "semgrep==${SEMGREP_VERSION}"`, with `SEMGREP_ENABLE_VERSION_CHECK:
  "0"` and `SEMGREP_SEND_METRICS: "off"` in the step's `env`, and the same one-line comment that pipx pins Semgrep but not
  its transitive dependencies), and update the comment above the job to say it also runs the repository's Semgrep
  rules. The job already runs `scripts/prepush-fast.sh` and already runs on every `pull_request` and `push`, with
  no secrets, so Dependabot and fork pull requests get the gate with no further change.
- `sonar-project.properties`: add `sonar.exclusions=.semgrep/**` after line 24. Sonar's default source set is the
  whole repository, so without it the fixtures are analyzed as Rust source and their lines count against new-code
  coverage and the issue gates.

### 3.6 `AGENTS.md`

Add a `## Semgrep rules` section after `## Security defaults`: the rules live in `.semgrep/rules/`, one file per
invariant with a fixture of the same name in `.semgrep/tests/`; `scripts/prepush-fast.sh` runs them and CI runs that
script; a new rule ships with `ruleid:` and `ok:` samples and is first run against the current code; no
`# nosemgrep`-style suppression comment without the owner's approval; and the gaps of 3.1 and 3.2 (variable program names, `env sh`, re-exports, `unsafe fn`,
a grouped `tty` import in `src/emulator.rs`, the two Ctrl-C literal edge cases) so a reviewer knows what is still checked by reading. Five
sentences at most.

## 4. Out of scope

- A "no `std::fs` in async" rule (section 1 states why), a rule on bare `0x03`, and a rule on `unwrap`/`expect`.
- The weekly broad Semgrep scan (backlog F16) and any `--config auto` or registry rule.
- Pull request or branch rules, Semgrep App, Semgrep Pro, or any use of a token.
- Changing any file under `src/` or `tests/`, the other workflows, the Dependabot config, `deny.toml`.
- `CHANGELOG.md`, `docs/backlog.md`, and `docs/how-this-is-built.md` (lines 19 and 124 to 125 list the static-analysis
  tools): the architect updates them when committing.
- A `.semgrepignore`. Backlog F15 expected one; none is needed because the scan targets are explicit (`src`, and
  `.semgrep/tests` for the fixture suite) and the default ignore list did not drop `.semgrep/tests` (probed).
- Pinning Semgrep's transitive dependencies (a `pipx install` limit, as in the other pinned tools).

## 5. Acceptance

Post-implementation unless noted; the baseline commands in section 0 are already run. Judge: the command output,
re-run by the tester and by the architect at final review.

1. `scripts/prepush-fast.sh` exits 0, and its output shows the Semgrep fixture suite with a nonzero test count.
2. `semgrep scan --test --config .semgrep/rules .semgrep/tests --metrics=off --disable-version-check --oss-only
   --include '*.rs'` reports every test passing.
3. `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`
   pass with no warnings. `git status --short` shows only the paths of section 0 (this spec file is committed before the assignment).
4. Mutation check, on a scratch copy (`git archive HEAD` into the scratchpad, then the developer's changed files over it;
   the assigned checkout is never mutated). Run `git init -q && git add -A && git commit -qm scratch` in the copy (Gitleaks scans history, which must not be empty) before anything else: a rule's
   `paths.exclude: ["src/emulator.rs"]` matches only inside a git work tree, and in a bare `git archive` copy the same
   rule fires on the excluded file (probed by the consultant, rc=1 on the unmutated tree).
   The first row is the control and must pass, so each mutant fails for its own reason. Each mutant must make `scripts/prepush-fast.sh`, or for the SAFETY row the
   clippy command, fail; the developer names the failing rule or step per mutant, then discards the copy:

   | Mutant | Failing check |
   | --- | --- |
   | none (control: the unmutated copy) | `scripts/prepush-fast.sh` and the clippy command both exit 0 |
   | add `use alacritty_terminal::term::Term;` to `src/main.rs` | `terminal-types-outside-emulator` |
   | add `use alacritty_terminal::tty::EventedPty;` to `src/main.rs` | `tty-outside-session` |
   | add `let _ = std::process::Command::new("sh");` to `src/main.rs` | `no-shell-command` |
   | add `let _ = b"\x03";` to `src/session.rs` | `no-ctrl-c-literal` |
   | delete the `// SAFETY:` comment above the `unsafe` at `src/main.rs:231` | `cargo clippy --all-targets -- -D warnings` |
   | remove every `.rs` file from the scratch copy's `.semgrep/tests` | the script exits non-zero (zero-test guard, or Semgrep's own error) |
   | delete `bash` from the shell alternation in the scratch copy's `no-shell-command.yml` | the `/bin/bash` fixture case |

5. `actionlint` and `zizmor --no-progress .github` (both run by the script) report nothing new: `zizmor` still
   shows "No findings to report".
6. Abuse cases for the trust boundary this spec does not cross: the gate reads only repository files and the Semgrep
   package; it adds no secret, no network call beyond `pipx install`, and no token. The tester confirms by reading the
   `ci.yml` diff that no `secrets.*` or `permissions:` change appears.
7. Known flaky tests: none reported for this spec. A failure in an unrelated `tests/` file is recorded as
   pre-existing only if it also fails on a clean `git stash` of the diff.
8. After the push (architect, not the developer): the `hygiene` job is green on the pushed commit and on a rebased open
   Dependabot pull request, and the Sonar gate stays `OK` (the exclusion keeps the fixtures out of it).

## 6. Claims checked

| Claim | File:line or test | Verified how |
| --- | --- | --- |
| `prepush-fast.sh` runs in CI via the `hygiene` job, on push and pull_request, no secrets | `ci.yml` lines 3 to 5, 35 to 74 | read |
| `hygiene` installs zizmor with `pipx` and the script's tool list is `gitleaks cargo-deny actionlint zizmor` | `ci.yml` install steps; `scripts/prepush-fast.sh:9` | read |
| `src/` has no finding for the four rules | `src/` at `2b06e86` | probed: throwaway rules via `semgrep scan --json`, zero results; a mutant with `use alacritty_terminal::term::Term;` appended to `src/main.rs` exits 1 |
| the only `unsafe` in `src/` and `tests/` is `src/main.rs:231` and `tests/daemon.rs:1748`, both documented | `rg -n "unsafe" src tests build.rs` | probed; clippy `-D clippy::undocumented_unsafe_blocks` exits 0 |
| `0x03` is the `Detach` tag, not a PTY input | `src/wire.rs:499,528,556,587`; `tests/prefix.rs:25` | read |
| `-c` in `src/codex.rs:335` is a config flag, `Command::new(program)` is a variable | `src/codex.rs:335,375`; `src/main.rs:215`; `src/daemon.rs:1418` | read |
| Semgrep 1.181.0 parses Rust, supports `paths.exclude` in rules (inside a git work tree only), and `--test` accepts `.rs` fixtures with `// ruleid:` annotations | local `semgrep --version`, probes in the scratchpad | probed: fixture suite passes for `no-shell-command` and `no-ctrl-c-literal` (after correcting one fixture line that was a true Ctrl-C) and for the fs-in-async probe |
| two rules annotated in one fixture file report each other's matches as incorrect lines | probe `t4.rs` | probed (both rules reported "incorrect lines") |
| `.semgrep/` is not compiled by cargo and is not covered by `sonar.coverage.exclusions=tests/**`; Sonar sources default to the repository | `Cargo.toml`; `sonar-project.properties:24`; no `sonar.sources` set | read |
| SALTMDB's scope table and parallel batches are unnecessary here | SALTMDB `scripts/prepush-fast.sh:19-105` | read |
| no other file in the repository mentions Semgrep except docs | `rg -n -i semgrep` over `.github`, `scripts`, `AGENTS.md`, `docs` | probed: only `docs/roadmap.md:97` and `docs/backlog.md` rows F15 to F17 |

## Review record

**Models.** Architect: Claude Sonnet 5.5. Consultant: a Claude Code session, model not exposed to the architect.
Developer and tester: OMP sessions reporting `openai-codex/gpt-6.1-terra`; their planning, working and advisor
slot models are not exposed to the architect. **BLOCKED/amend cycles:** 0.

| Found in | Found by | Problem | Resolution |
| --- | --- | --- | --- |
| Draft | Architect probe | The planned `std::fs`-in-async rule gave a false positive at `src/codex.rs:367-368` and missed `fs::` spellings | Rule dropped; reason recorded in section 1 |
| Draft | Architect probe | Two rules annotated in one fixture file reported each other's matches as incorrect lines | One fixture per rule |
| Draft review | Consultant | `paths.exclude: ["src/emulator.rs"]` matches only in a git work tree, so the mutation method (a bare `git archive` copy) failed on the unmutated tree | Scratch copy is `git init`ed and committed first, with a control row |
| Draft review | Consultant | Zero-test guard: an empty fixture directory prints no `N/M:` line and exits 0; a deleted fixture gave 3/3 passing | A missing line counts as 0; `N` must equal the rule-file count |
| Draft review | Consultant | The fixture count was printed only on failure although acceptance 1 expects it on success | Output printed on success |
| Draft review | Consultant | Acceptance 3 expected a clean `git status` while the spec file was untracked | Spec committed before the assignment |
| Draft review | Consultant | Missing gaps: re-export from the emulator, two Ctrl-C literal edge cases, `env sh`; `--strict` suggested | Gaps listed in 3.1 and `AGENTS.md`; `--strict` adopted |
| Tester pass | Tester | Survivors: the `tokio::process::Command` alternative (the plain `Command` pattern already matches it), `--strict`, and the `semgrep` prerequisite entry | No change: the first is a redundant alternative, the other two are error branches that need a broken environment to exercise |
| Architect review | Architect | `AGENTS.md` section has six sentences against the stated limit of five | Accepted as is |

