# Spec 3bu: `a2amx confine suggest`

## 0. Status

**LOCKED** (2026-11-01; one consultant review `m_4980` applied, no second pass; context `a2amx-confine-friendly-2026-11-01`; backlog F62 step (a)). Architect-directed, owner-approved.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `7b3ca38`, 752 passed, 0 failed, 3 ignored; run as
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test` with a writable `HOME`.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `suggest::detect` and `suggest::render` (`tests/suggest.rs`), `team::parse` for the contract test, and the `a2amx confine suggest` binary.

**Scope.** May edit exactly these files and no others: `src/suggest.rs` (new), `src/lib.rs`, `src/cli.rs`, `src/main.rs`, `tests/suggest.rs` (new),
`docs/cli.md`, `docs/confinement.md`, `AGENTS.md` (one row in the Modules table). Does not touch: `a2amx.toml`, `CHANGELOG.md` (the architect edits it), `docs/backlog.md`,
`src/confine.rs`, `src/grant.rs`, `src/team.rs`, every other file. No new dependency. Leave the diff uncommitted.
Line numbers below are at `7b3ca38`; re-anchor them against the tree you are given.

## 1. Why

Confinement is a safety net against accidents, and a user turns it on to find out that `cargo build` or `npm install` fails on a read-only path. The fix is one `write` line,
and the recipes for it are in `docs/confinement.md` ("Recipes by ecosystem"), but a user has to read the page, find the right recipe and copy it. Backlog F62 sets the test: a new user
turns confinement on with defaults and needs no workaround for a Rust, npm or Python build. The cheapest step is a command that looks at the project and prints the fragment. It adds no mount mechanism, no grant and
no restriction; it only reads which build files exist.

## 2. The command

`a2amx confine suggest [DIR]` prints a TOML fragment for the build tools it finds in `DIR` (default: the current directory) and exits 0. Like `confine list`, it needs no daemon and no `HOME`.
It does not read file contents, does not write anything, does not recurse, and does not run any tool.

**`src/cli.rs`.** `ConfineAction` (`:176`) gains
```rust
/// Print a [session.confine] fragment for the build tools found in DIR.
Suggest {
    /// Project directory; defaults to the current directory.
    dir: Option<PathBuf>,
},
```
**`src/main.rs`.** The early match at `:80` becomes two arms: `ConfineAction::List` as today, and `ConfineAction::Suggest { dir }`, which resolves `dir` (default `std::env::current_dir()`),
and on a blocking thread (`spawn_blocking`; the checks are file syscalls) fails with `cannot read project directory <path>` unless it is a directory, then calls `suggest::detect` with a closure that tests whether `dir.join(name)` exists,
and prints `suggest::render(&found)` through `write_stdout`. The error goes through the existing error path (exit 1, message on stderr). The later `Command::Confine { .. }` arm (`:141`) stays.

## 3. The module (`src/suggest.rs`, pure)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecosystem { Rust, Node, Go, Maven, Gradle, Python }

/// Which ecosystems have a marker file, in the fixed order of `ECOSYSTEMS`; each with the first marker found.
pub fn detect(has_file: impl Fn(&str) -> bool) -> Vec<(Ecosystem, &'static str)>;

/// The fragment text, ending with a newline.
pub fn render(found: &[(Ecosystem, &'static str)]) -> String;
```

The table (fixed order is the listing order; markers are tried in the order shown, the first present one is reported):

| Ecosystem | Label | Markers | `write` paths | Note line (static text) |
| --- | --- | --- | --- | --- |
| `Rust` | Rust | `Cargo.toml` | `~/.cargo/registry` | `Rust: a Git dependency also needs ~/.cargo/git (untested).` |
| `Node` | Node | `package.json` | `~/.npm` | `Node: global installs and version-manager directories are not covered.` |
| `Go` | Go | `go.mod` | none | `Go: run go env GOMODCACHE GOCACHE and add both paths as write entries; do not open ~/go/bin.` |
| `Maven` | Maven | `pom.xml` | `~/.m2/repository` | `Maven: ~/.m2/settings.xml can hold passwords; add hide = ["~/.m2/settings.xml"] when the agent does not need it.` |
| `Gradle` | Gradle | `build.gradle`, `build.gradle.kts`, `settings.gradle`, `settings.gradle.kts` | `~/.gradle/caches`, `~/.gradle/wrapper` | `Gradle: gradle.properties and init.d stay closed; do not open all of ~/.gradle.` |
| `Python` | Python | `pyproject.toml`, `requirements.txt` | none | `Python: a virtual environment inside the workspace needs no write entry.` |

The paths and notes are the ones already in `docs/confinement.md` "Recipes by ecosystem"; the module restates them as data and must agree with that page (the docs section of this spec keeps them in step).

**`render` output, exactly.** With at least one ecosystem found:
1. `# Found: ` followed by `{marker} ({label})` for each found ecosystem joined with `, `, then `\n`.
2. Only when the union of the `write` paths (in table order) is non-empty: `# Add this to the session's table in your team file, or pass --confine-write for each path. Confinement must be enabled (enabled = true here or in the team's [confine] table).\n`, then `[session.confine]\n` and `write = [` + the paths each in double quotes joined with `, ` + `]\n`. When the union is empty, neither the instruction line nor the table is printed.
3. One `# ` + note line per found ecosystem, in table order.
4. `# More: docs/confinement.md, Recipes by ecosystem\n`.

With nothing found, exactly: `# No known build files here. The workspace itself needs no write entry.\n# Run the tool once, read the failing path, and see docs/confinement.md, Recipes by ecosystem.\n`.

The output carries no text taken from the project: the file names come from the static table, never from the directory listing or the path argument.

## 4. Docs

A ```toml block in `docs/confinement.md` must start with the line `# fragment`, or `tests/docs_examples.rs` parses it as a team file; the developer adds prose only, no sample-output block.
`docs/cli.md` (next to the `confine list` paragraph, `:66`): one paragraph for `a2amx confine suggest [DIR]`: what it reads (existence of the listed build files, no contents), what it prints, that it needs no daemon or `HOME`.
`docs/confinement.md` ("How to set it up", before "One command", `:16-18`): two sentences naming `a2amx confine suggest` as the quickest way to get the `write` line for a project. The recipes section is unchanged.
`AGENTS.md` Modules table: a row `suggest` after `grant`: `Build-file detection and the \`confine suggest\` fragment text (pure)`.

## 5. Tests (`tests/suggest.rs`)

(1) `detect`: a closure true only for `Cargo.toml` gives `[(Rust, "Cargo.toml")]`; only `build.gradle.kts` gives Gradle with that marker; both `build.gradle` and `build.gradle.kts` present reports `build.gradle`; no files gives empty; `Cargo.toml`, `package.json` and `pom.xml` together give Rust, Node, Maven in that order.
(2) `render` for Rust and Node equals a literal: the header `# Found: Cargo.toml (Rust), package.json (Node)`, the instruction line, `[session.confine]`, `write = ["~/.cargo/registry", "~/.npm"]`, the two note lines, the `More:` line.
(3) `render` for Go and Python only has no `[session.confine]` line, no `write =` line and no `Add this to` line, and contains the Go and Python notes.
(4) `render` of empty equals the two-line literal of section 3.
(5) Contract test (passes against any implementation of section 3): the `render` output for all six ecosystems, appended to `"team = \"x\"\n[confine]\nenabled = true\n[[session]]\nname = \"x\"\ncommand = [\"cat\"]\n"`, parses with `team::parse` (a session needs a name or a role: `src/team.rs:744`).
(6) Binary test (pattern of `tests/grant.rs:195`): `env_clear`, `confine suggest <tempdir>` with `Cargo.toml` and `pom.xml` created gives exit 0, empty stderr and stdout equal to `suggest::render` of Rust and Maven; a nonexistent directory gives a nonzero exit and a message containing `cannot read project directory`; a file given as `DIR` does the same.
(6b) Default directory: `confine suggest` with no argument and `.current_dir(tempdir containing package.json)`, `env_clear`, prints the Node fragment.
(7) Abuse case (trust boundary), a binary test: a directory whose name contains a newline and `# injected` (created in a tempdir) holding a `Cargo.toml`; `confine suggest <that dir>` succeeds and its stdout equals `suggest::render` of Rust and does not contain `injected`. (`render` takes no directory, so a test against `render` alone proves nothing.)

## 6. Out of scope

Presets as flags (F62 step b), a launch notice for read-only paths (step d), the own harness directory by default (step c), walking up or down the tree, reading file contents (for example `go env`, `package.json` fields), writing the fragment into a team file,
new grants, and any change to `src/confine.rs`.

## 7. Trust boundaries

The only input is the operator's command line (`DIR`) and the existence of fixed file names under it; the caller owns the process. Constrained: no file is read or written, nothing is executed, no text from the project reaches the output (test 7), and the path is only joined with static names. A symlinked marker counts as present; that is harmless because only existence is tested.

## 8. Placement

Detection data and the fragment text belong together and are pure, so they get their own small module like `grant`, not a block in `main.rs` (3,429 lines) or `confine.rs`. `main.rs` does only the filesystem check and the print, as it does for `confine list`.

## 9. Acceptance

```sh
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only the files in section 0
```

Known flaky: backlog I9 and F37, and `tests/team.rs::team_up_forwards_reset_configuration_and_consent`: rerun once, else record as pre-existing. Test 5 is a contract test. Mutants for the tester (on a scratch copy; its tree matches its diff afterward):
(1) the write union drops an ecosystem's second path (Gradle `~/.gradle/wrapper`; test 1 or 2 of the Gradle case: add a Gradle case to test 2's literal or to a test 2b); (2) the Gradle marker list drops `build.gradle.kts` (test 1); (3) Python gets a `write` path (test 3); (4) the order of ecosystems changes (tests 1, 2);
(5) `main` prints the directory name into the output (test 7, binary); (6) the empty case prints the fragment header (test 4); (7) the Rust path becomes `~/.cargo` (test 2); (8) the missing-directory check is removed (test 6); (9) `Suggest` reads `HOME` or contacts the daemon (test 6, `env_clear`).

## 10. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `ConfineAction` has only `List`; the `Command::Confine` early arm is at `main.rs:80` and the later arm `Command::Confine { .. }` at `:141` | `src/cli.rs:176`, `src/main.rs:80`, `:141` | read 2026-11-01 |
| `confine list` runs before `resolve_home`, so it needs no `HOME` or daemon | `src/main.rs:80-85` | read |
| The binary test pattern with `env_clear` exists | `tests/grant.rs:195` | read |
| `toml` is a dependency and `tempfile` a dev-dependency | `Cargo.toml` | read |
| No `suggest` symbol or file exists | `rg -n 'suggest' src tests docs` (only the grant lookup test names) | probed |
| The recipe paths and notes come from the docs page | `docs/confinement.md:120-227` | read |
| `team::parse(&str)` exists and takes the team file text | `src/team.rs:425` | read |
| A `[session.confine]` `write` list is a valid per-session key | `a2amx.toml.example`, `docs/teams.md` | read |

## Amendment log

None yet.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (review `m_4980`, exact model id not re-derived); developer on the OMP harness, `openai-codex/gpt-5.6-luna` as it reported it (advisor model not exposed). No tester pass: the owner asked to finish quickly, the module is small and pure, and the developer ran and reported all nine mutants killed. BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`m_4980`) | Test 5's prefix had no name or role, so `team::parse` would fail under any implementation | Prefix gets `name = "x"` |
| Draft review | Consultant | Test 7 and mutant 5 could pass trivially against `render`, which takes no directory | Test 7 is a binary test with a marker file; mutant 5 targets `main` |
| Draft review | Consultant | `Ecosystem` derives unspecified; the instruction line printed with no table; default-DIR branch untested; dead dedup; docs `# fragment` trap; anchor off by one | Derives named, instruction line only with a table, test 6b added, dedup dropped, docs sentence added, anchor fixed |
| Architect review | Architect (Claude Sonnet 5.5) | `unwrap_or(std::env::current_dir()?)` read the current directory even when `DIR` was given, so an explicit `DIR` failed in a deleted working directory | One-line `match` in `src/main.rs`; full suite (762 passed, 0 failed, 3 ignored), clippy and fmt rerun green |
