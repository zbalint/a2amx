# Spec 2k: infer the harness from the command

## 0. Status

**LOCKED.** Decisions D1 to D5 in section 2 were settled with the owner. Builds on spec 2j
(committed `66b95ee`). The work is done directly on `develop` in the main checkout, no
worktree. Shared task `context_id`: `a2amx-harness-inference`. Public test seams:
`Harness::infer` (a pure function) and the `a2amx` binary driven as a subprocess.

**Scope.** May edit exactly these files and no others. New files are marked.

- `src/harness.rs`, `src/cli.rs`, `src/main.rs`
- `tests/harness_infer.rs` (new), `tests/daemon_cli.rs`
- `README.md`

Does not touch: every other file under `src/` and `tests/` (`tests/common` included),
`docs/`, `scripts/`, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, the locked spec documents. No
new dependency. Do not commit and do not merge: leave the diff uncommitted in the working
tree.

In `tests/daemon_cli.rs` the only permitted change is adding the two tests of section 5 (and
helpers they need, in that file). Do not change the existing seven tests.

## 1. Why

`a2amx new --name x --harness claude -- claude` repeats the harness in the command. Without
`--harness` the session gets the generic profile, whose messages wait for a person
(`Deliver::Hold`), so forgetting the flag silently produces a session that never receives
messages. The executable name already says which harness is meant.

## 2. Decisions settled with the owner

- **D1.** When `--harness` is omitted, the CLI infers the harness from the command. The
  daemon, the wire protocol and `Request::NewSession` do not change.
- **D2.** Inference looks only at the file name of the first command word: exactly `claude`,
  `codex` or `omp`, case-sensitive, after stripping any directory (`/usr/bin/claude`,
  `./omp`). Anything else is `Harness::Generic`.
- **D3.** An explicit `--harness` always wins, `--harness generic` included. That is how a
  user opts out.
- **D4.** No wrapper parsing: `env X=1 claude`, `npx claude`, `sh -c "claude"` stay generic.
  The user passes `--harness` for those.
- **D5.** The change is deliberate and silent: `a2amx new -- claude` becomes a Claude
  session, which delivers messages, where it was a generic one. No message is printed.

## 3. `src/harness.rs`

Add to `impl Harness` (the block that holds `default_deliver`, `src/harness.rs:38`):

```rust
/// The harness named by the command's executable, or `Generic` when it names none.
pub fn infer(command: &[String]) -> Self {
    match command
        .first()
        .and_then(|word| Path::new(word).file_name())
        .and_then(|name| name.to_str())
    {
        Some("claude") => Self::Claude,
        Some("codex") => Self::Codex,
        Some("omp") => Self::Omp,
        _ => Self::Generic,
    }
}
```

`Path` is already imported (`src/harness.rs:4`). Worked examples:

| `command` | result |
|---|---|
| `["claude"]` | `Claude` |
| `["/usr/bin/claude", "--resume"]` | `Claude` |
| `["./codex"]` | `Codex` |
| `["omp"]` | `Omp` |
| `["sh"]` | `Generic` |
| `["Claude"]` | `Generic` |
| `["claude-code"]` | `Generic` |
| `["env", "FOO=1", "claude"]` | `Generic` |
| `["/"]` | `Generic` (no file name) |
| `[]` | `Generic` |

## 4. `src/cli.rs` and `src/main.rs`

`src/cli.rs`, `Command::New` (lines 50 to 52): the field becomes optional with no default.

```rust
/// Harness profile. Inferred from the command when omitted: claude, codex and omp are recognised by executable name, anything else is generic. omp delivers through an OMP extension; without it, messages wait (channel_down). codex hosts a private app-server and delivers through it.
#[arg(long, value_enum)]
harness: Option<Harness>,
```

`src/main.rs`, `run_new`: the destructured `harness` is now `Option<Harness>`. Directly after
the `let Command::New { .. } = options else { .. };` block and before `let command = match
harness`, add

```rust
let harness = harness.unwrap_or_else(|| Harness::infer(&command));
```

Everything after it is unchanged, since it already uses a concrete `Harness`. No other call
site reads `harness` from `Command::New` (check with `rg "Command::New" src tests`).

## 5. Tests

Write the failing test first, one behavior at a time.

**`tests/harness_infer.rs` (new).** One test per row of the section 3 table, or a table-driven
test with the expected values written out as literals. It calls `Harness::infer` directly.
No process is started.

**`tests/daemon_cli.rs`.** Two tests that drive the binary, using the file's existing helpers
and a `Cleanup` guard. Each writes an executable file named `claude` into its own temp
directory with the content `#!/bin/sh` then `exec sleep 30`, mode 0755, starts a daemon with
`daemon --background`, and runs `new --detach` against that path. A Claude-profile session's
`list` row contains the wired `--mcp-config` argument; a generic one does not.

| # | Behavior | Check |
|---|---|---|
| 8 | inferred | `new --detach -- <dir>/claude`, then `list` stdout contains `--mcp-config` |
| 9 | explicit generic wins | `new --detach --harness generic -- <dir>/claude`, then `list` stdout does not contain `--mcp-config` |

Codex and OMP are covered by the pure test only: a fake `codex` would make the daemon start a
real `codex app-server`, and an `omp` session installs an extension into the state dir.

## 6. `README.md`

In the command block, the line `a2amx new --name agent-plan --harness claude -- claude` keeps
working; add a comment line directly above it: `# --harness is inferred from the command name
when omitted (claude, codex, omp)`. No other README change.

## 7. Out of scope

Wrapper or alias detection, a configuration file mapping names to harnesses, printing the
chosen harness, any change to the daemon or the wire protocol, any other `--harness` default,
`docs/architecture.md`, and the unrelated flake in `tests/attach_cli.rs`
(`status_line_toggles_and_shows_the_session_address`, about 2 failures in 12 runs on a clean
tree, from spec 2i).

## 8. Acceptance

Run all three, no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

`cargo test` includes `tests/harness_infer.rs` and tests 8 and 9. If `attach_cli`'s
`status_line_toggles_and_shows_the_session_address` fails, rerun it alone up to three times
and report it as the known flake; any other failure is a real one. These need the new code,
so they run after implementation, not at lock time.

`git diff --stat` shows only the files in section 0; `git diff --check` is clean.
