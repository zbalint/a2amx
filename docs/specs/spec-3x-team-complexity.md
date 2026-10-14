# Spec 3x: split `check_global_allowed` and `parse_with` below the Sonar cognitive-complexity limit

## 0. Status

**LOCKED** (2026-10-14; pre-lock gate run against develop at `ce1dc83`; consultant review `m_1617`, findings 1-6 adopted). Architect-directed, backlog I4 part 1, context
`a2amx-sonar-s3776-2026-10-14`. Implementation goes to `a2amx-developer` only.

**Baseline:** develop at `ce1dc83` (CI files only differ from `62e033b`; `src/` and `tests/` are identical); `cargo test --test team` passes 52/52.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `team::check_global_allowed`, `team::parse`, `team::parse_with` (already exercised by
`tests/team.rs`). The new helpers are private and are not tested directly.

**Scope.** May edit exactly these files and no others.

- `src/team.rs` (only `check_global_allowed` at line 213 and `parse_with` at line 384, plus new private helpers
  placed directly after each)
- `tests/team.rs` (only to add the characterization tests of section 4)

Does not touch: every other source and test file, `docs/`, `README.md`, `sonar-project.properties`. No new
dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

SonarQube Cloud rule `rust:S3776` flags `check_global_allowed` (complexity 40) and `parse_with` (42) against a limit
of 30 (backlog I4). Both are long straight-line decision lists, so the fix is to name their parts, not to change
what they decide.

## 2. Rule: behavior is unchanged

Same inputs give the same `Ok`/`Err`, the same error text byte for byte, and the same check order (first matching
reason wins in `check_global_allowed`; first failing check wins in `parse_with`). No public signature changes. No
new `pub` item. Do not change any list of paths, names, or drive letters, only where it lives.

## 3. Changes in `src/team.rs`

### 3.1 `check_global_allowed`

Keep the function, its signature and the final `bail!` as they are. Note for the developer: `components` and `mnt`
(case-insensitive "mnt") are computed once today and shared by the system and container parts; each helper may
recompute them with the same test. The drive-letter test must stay byte-exact: `to_string_lossy().len() == 1` and
`as_bytes()[0].is_ascii_alphabetic()`; the `components.len() >= 3` guard before it is stated only in the system
part today and must stay there. Replace the body that computes `reason` with a
chain of private helpers, each returning `Option<&'static str>` and called in the current order:

1. `home_reason(cwd, user_home)`: the current first `if` (root, home, hidden directory under home, user-data
   folder). The filesystem-root check stays first.
2. `system_reason(cwd, state_home)`: the `system` prefix list, the WSL `/mnt/wsl` and `/mnt/wslg` case, and the
   Windows-drive system folders (`Windows`, `Program Files`, ...), all returning "the daemon state directory or a
   system directory".
3. `container_reason(cwd)`: the exact top-level container list, "another user's home directory", the bare `/mnt/<drive>`
   case, and "a Windows home directory".

The three-line `reason = a.or_else(b).or_else(c)` form is the intent; the existing `is_direct_child` inner function
moves into `container_reason` or becomes a private free function, the developer's choice. A small private helper
for the repeated "`components[2]` is a single ASCII drive letter under `/mnt`" test is allowed and expected,
since it appears three times today.

### 3.2 `parse_with`

Keep the function and its signature. Order of checks today, which must not change: (a) `toml::from_str`, (b) the
`team` plus `default_team` conflict, (c) the `prefix` rejection, (d) merge `team.or_else(default_team)`, (e) `private
requires team`, (f) `allow requires team`, (g) the name, `private`, `allow` checks, (h) the session tables, (i) the
final scope/`watch`/`control_from` loop, (j) `validate_sessions`.

Extract, keeping (a) to (d) inline in `parse_with`, in today's order:

1. `build_scope(team: Option<String>, private: Option<bool>, allow: Option<Vec<String>>)` returning
   `anyhow::Result<Option<TeamScope>>`, covering (e) to (g). `team` is the value after the merge in (d). The
   function derives `allow_given = allow.is_some()` itself and then `allow.unwrap_or_default()`. It has no
   `default_team` or `prefix` parameter.
2. `session_from_table(index: usize, table: toml::value::Table)` returning `anyhow::Result<TeamSession>`: the body
   of the current `.map` closure (role-as-name, the `name or role is required` error, `try_into` with the `session
   N` context). The call site keeps `collect::<anyhow::Result<_>>()`, so the first failing session still wins.

(h) and (i) stay in `parse_with`.

## 4. Safety net

Today the guard test asserts only "filesystem root", "home directory" and "hidden directory"; every other reason is
checked with `is_err()` alone, and `parse_rejects_invalid_team_keys_and_old_prefix` uses substrings (`allow` matches
four different messages). So before the first edit the developer adds characterization tests to `tests/team.rs`,
run on the unchanged code and passing there:

- One test per `bail!` text in `parse_with`/`build_scope` and per reason string in `check_global_allowed`,
  asserting the full message or a substring unique to it (not `contains("allow")`).
- Precedence pairs, because "first match wins" has no test behind it today: `team` set with `default_team` and
  `prefix` set (expect "must not set team"); `prefix` set with `private` and no team (expect "prefix was renamed");
  `private = false` with `allow` on a team (expect "allow needs a private team"); cwd `/home/u/.a2amx` with home
  `/home/u` (expect "hidden directory", not "state directory"); cwd `/home` with home `/home/u` (expect "your home
  directory", not "top-level container"). Developer verifies each expected value against the unchanged code and
  states the observed message in the report.

Expected values are literals taken from the unchanged code's output, never computed.

## 5. Out of scope

Renames, changing messages, `resolve_agent_allow`, `resolve_references`, `validate_sessions`, `plan`, any other
Sonar finding (I4 `main.rs`, I5), and the `fn` placement of any other item in the file.

## 6. Acceptance

Post-implementation, in the main checkout:

```sh
cargo test --test team
env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
git status --short   # only src/team.rs and, if added, tests/team.rs
```

- `cargo test --test team` passes at least 52 tests plus the added characterization tests.
- Final proof of the complexity fix is the Sonar rescan after the owner pushes; no local command measures S3776.
  The local `clippy::cognitive_complexity` lint uses a different metric and does not flag these two functions, so it
  is not an acceptance check.
- Known flaky: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (PTY timing). On a
  failure rerun that test once; if it then passes, record it as a pre-existing flake. Any other failure is new
  unless it reproduces on a clean `ce1dc83`.
