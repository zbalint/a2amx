# Spec 3eq: confinement of `agy`, its credential hides and an empty status-line command (spec 2b of 4)

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`, HEAD `392f063` or later (spec 3ep is
committed. Shared task `context_id`: `agy-harness`. Spec 2b of the `agy` work. Spec 3 (typed delivery) and
spec 4 (docs, team file) follow.

**Scope.** May edit exactly these files and no others.
- `src/confine.rs` (match arms, one data arm, one hide rule; sections 2-4), `src/harness.rs`
  (`build_agy_status_line` only, section 5)
- `tests/confine.rs`, `tests/agy_wiring.rs`, `tests/daemon.rs` (one test case, section 6)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md`, `Cargo.toml`, `Cargo.lock`, `src/daemon.rs`,
`src/session.rs`, `src/grant.rs`, `src/main.rs`, every other file and spec. Do not commit, stage or merge.

**Public test seams.** `confine::plan`, `confine::launch_argv`, `confine::ensure_data_dirs`,
`confine::default_hide_names` (the existing `Fixture` in `tests/confine.rs`), `harness::wire_agy_argv`, a real
daemon through `Client` (`tests/daemon.rs`).

## 1. Why

Probes (tester, 2026-11-15, `agy` 1.5.9, a confined session under bubblewrap with the real `~/.gemini`):

- With no extra write access `agy` signs in and runs, but every write under `~/.gemini` fails (EROFS on
  `conversations`, `brain`, `annotations`, the summaries database; no CLI log file is created). With
  `~/.gemini/antigravity-cli` writable and everything else read-only the session is fully usable: logs,
  conversations, plugin data, and a plugin MCP tool call all work. `~/.gemini/config` (the installed plugins)
  needs no write at run time. `~/.local/bin` read-only only disables self-update (`agy` logs that it skips the
  update and carries on). `~/.config/antigravity` can stay read-only (sign-in and plugin MCP work); a write only
  removes a credential-scope warning.
- A confined session of any **other** harness can read the `agy` credentials: the OAuth token
  `~/.gemini/antigravity-cli/antigravity-oauth-token` and `~/.config/antigravity/` (`credential-user-scope`).
  `DEFAULT_HIDES` names neither. The existing rule hides `~/.codex/auth.json` from every harness but Codex;
  this spec adds the same for `agy`.
- The tester also saw `sh: Syntax error: ")" unexpected` in a2amx sessions when the user's global `agy`
  `statusLine` has an empty `command` (the owner's real configuration: `enabled: true`, empty command). The
  chained command from spec 3en wraps the user command as `(<command>\n)`, which is `(\n)` for an empty command,
  a syntax error in `dash`.

Side effects, intended: `ensure_data_dirs` runs for every confined launch and sets an existing
`~/.gemini/antigravity-cli` to mode 0700 on the host (as for Claude's directories); a confined `agy` launch fails when
`antigravity-cli` is a symlink leaving `~/.gemini` (the existing data-path validation).

Decisions: `agy` follows the Claude pattern (named writable data directory, rest of the harness root
read-only), not the Codex pattern (whole root writable). The grant `harness-config` stays available explicitly.
Rejected: making all of `~/.gemini` writable by default (installed plugins and hooks would be writable by a
confined agent); a `~/.config/antigravity` write bind (not needed).

## 2. `src/confine.rs`: data directory and root

1. `harness_data`: an `Harness::Agy` arm returns `dirs = [home/.gemini/antigravity-cli]`, no files. Remove
   `Agy` from the empty `Generic | Codex` arm.
2. `validate_harness_data_paths`: `expected_sources` for `Agy` is `vec![home.join(".gemini")]`.
3. `ensure_data_dirs`: root for `Agy` is `home.join(".gemini")` (the existing "root must exist, then create the
   data dirs with mode 0700" logic applies unchanged).
4. `add_harness_data_mounts`: root for `Agy` is `home.join(".gemini")`; remove `Agy` from the early-return
   condition and from the `unreachable!` arm.
5. `session_harness_root`: `Agy` returns `Some(home.join(".gemini"))` so an explicit `harness-config` grant works;
   `bind_harness_config`'s error text becomes `... needs a Claude, OMP, Codex, or Agy session` (the Generic test that
   asserts the old text exactly, `tests/confine.rs` ~6703-6706, is updated to the new literal). `harness-config`
   stays **not** implicit for `Agy` (the `matches!(harness, Claude | Omp | Codex)` condition in
   `plan_with_preflight` and its inner match do not change).

## 3. `src/confine.rs`: default hides

`default_hide_names`: after the `~/.codex/auth.json` rule and before `/run/dbus`, add: when `harness !=
Harness::Agy`, push `~/.gemini/antigravity-cli/antigravity-oauth-token` and `~/.config/antigravity` in that
order. Every harness except `agy` then hides both; `agy` hides neither (its token is inside its own writable
data directory, and `~/.config/antigravity` is read-only visible).

## 4. `src/confine.rs`: `launch_argv`

No change. Spec 3en put `Agy` in the plain-wrap arm with Codex; section 6 adds the test that pins it.

## 5. `src/harness.rs`: `build_agy_status_line`

When the resolved user status line's `command` is empty or only whitespace (`trim().is_empty()`), treat it like
`enabled: false`: return the report-only form `{"command": "<exe> statusline --harness agy --report",
"enabled": true}` (prints nothing, still reports quota, leaves the harness's default status line). A `command`
containing a NUL byte and a missing or non-string `command` are unchanged. The check comes after the NUL check and
before the `enabled: false` check; like the `enabled: false` form it drops `padding` and `stack_with_default`. The
Claude helper `build_status_line` has the same defect for an empty command; out of scope (section 7).

## 6. Tests (write first)

`tests/confine.rs`:
1. Update the literal sets in `default_hide_names_and_sources_match_the_literal_sets`: the generic, claude and
   codex lists gain the two paths (after the codex entry, before `/run/dbus`; counts +2); add an `agy` list: the
   generic list's entries exactly as before (no new paths; it includes the Claude and Codex credential hides and
   `/run/dbus`).
2. A plan test for `Harness::Agy` with a `~/.gemini/antigravity-cli` directory and a `~/.gemini/config` directory in the
   fixture home: `antigravity-cli` is in the plan's writable set and no writable path is `~/.gemini` or an ancestor of
   `~/.gemini/config`; with the explicit grant `harness-config` the root `~/.gemini` is writable (and `antigravity-cli` is
   then not listed separately, because `bind_harness_config` removes it: do not assert both). The fixture must create both
   credential sources (the token file `~/.gemini/antigravity-cli/antigravity-oauth-token` and the directory
   `~/.config/antigravity`), because a missing source is reported as skipped, not hidden: for `Harness::Generic` both are in
   `hidden_paths`; for `Agy` neither is in `hidden_paths` nor in `skipped_paths`.
3. `ensure_data_dirs(Harness::Agy, home)` creates `~/.gemini/antigravity-cli` (mode 0700) when `~/.gemini` exists
   and does nothing when it does not.
4. `launch_argv` with `Harness::Agy` equals `plan.wrap(argv)` (compare against a plan built for the same
   argv): it adds no `--default-signal=INT` and no notice (model on the existing Codex test `codex_launch_argv_matches_plan_wrap`, `tests/confine.rs` ~3408).
   Covers the surviving mutant of spec 3en.

`tests/daemon.rs`: a new sibling test (the existing one at line ~796 sleeps about 14.5 s: do not extend it, and do not
parameterise `create_claude_team`, which has Claude hard-coded at ~716): an `agy` session created with the existing
`common::new_team_agent(..., Harness::Agy, ...)` helper and the command `sh -c 'sleep 30'` (wiring is client-side, so the
daemon spawns it as is) is never reported as starting, with the same assertions as the generic case. Covers the other
surviving mutant of spec 3en. Readiness for `Agy` is `bracketed_paste && !scrolled`, so the sleeping command never latches.

`tests/agy_wiring.rs`: a user `statusLine` with `{"command": "", "enabled": true}` and one with
`{"command": "   "}` both produce exactly `{"command": "'/x/a2amx' statusline --harness agy --report",
"enabled": true}`; and the chained command for a non-empty user command is unchanged (existing tests).
The first two cases fail on the current code.

Mutation check (the developer, in the checkout, `sha256sum` restore): (a) drop the new `Agy` data arm,
(b) drop the two default-hide pushes, (c) make `Agy` implicit `harness-config`, (d) change the `launch_argv`
arm to the Generic wrapper, (e) in `Session::starting()` (`src/session.rs:589`, outside the edit scope: a temporary mutation
restored by `sha256sum`) remove `Agy` from the `!matches!(self.harness, Harness::Generic | Harness::Agy)` exclusion, so
`starting()` is true for `Agy` until ready latches, (f) remove the empty-command guard; name the failing test for each.

## 7. Out of scope

The same empty-command defect in Claude's `build_status_line` (a backlog item for the architect); `harness-config` implicit behaviour; a `~/.config/antigravity` write bind; plugin install inside the sandbox
(the launcher installs outside it, spec 3ep); settings of `agy` that the confined agent can rewrite
(`~/.gemini/antigravity-cli/settings.json` is inside the writable data directory, like Claude's own settings:
documented by spec 4); self-update; docs.

## 8. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
2. `git status --short` lists exactly the scoped files.
3. Red on the old code: the updated default-hide literals, the plan test, the `ensure_data_dirs` test, the updated
   error-text literal and the two empty-command wiring cases. Contract tests for already-correct code (they fail only
   under mutants (d) and (e)): the `launch_argv` test and the `starting()` test.
4. The mutation check names a failing test per mutant (the tester reruns).
5. Trust boundary: a generic confined session must not read the two credential paths (test 6.2). An `agy`
   session sees its own token only inside the writable data directory.
6. Live check (architect, after implementation): a confined `agy` session started by `a2amx new --harness agy
   --confine` signs in, runs one prompt, writes under `antigravity-cli`, and cannot write `~/.gemini/config`.

## 9. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| Token and credential-scope paths; both readable from a generic confined shell today | tester P11-7 | tester measurement |
| `antigravity-cli` alone is enough for a working confined session; `config` needs no write | tester P11-5 | tester measurement |
| `harness_data`, `validate_harness_data_paths`, `ensure_data_dirs`, `add_harness_data_mounts`, `session_harness_root` have `Generic \| Codex \| Agy` arms from spec 3en | `src/confine.rs:417-492, 594-610, 656-664, 1028-1045, 1320-1330` | read |
| Literal hide-set tests are in `tests/confine.rs` only | `rg "codex/auth.json" tests src docs` | grep |
| `launch_argv` Agy arm is `plan.wrap(argv)` | `src/confine.rs:2557` | read |
| `starting()` excludes `Agy`; the generic never-starting test | `src/session.rs:586-590`, `tests/daemon.rs:796` | read |
| Empty user command yields `(\n)` | `src/harness.rs` `chained_command` | read; dash error reproduced by the tester (m_7492 point 8) |

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (pre-lock review); developer OMP
openai-codex/gpt-5.6-luna with advisor (identity not exposed); tester OMP opencode-go/deepseek-v4.1-flash.
BLOCKED/amend cycles: 0.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | Review findings on the draft (fixture must create hide sources, truncate-vs-error wording, daemon test placement) | Applied before locking |
| Tester pass | tester | Empty-command test input covered only `""` and spaces and had no `padding`/`stack_with_default`, so a narrowed trim and a passthrough mutant could survive | Test inputs extended by the architect (test-only); trim mutant now killed |
| Tester pass | tester | Live probe infrastructure: a fixture HOME under /tmp is writable inside the confinement (tmpfs on /tmp) | Probe-only; not a product defect |

Open follow-ups (not defects of this diff): `docs/confinement.md` harness-config default list omits Agy (spec 4); no repo test for the Agy data-path symlink escape or 0700 tightening of an existing data dir. Final gates (architect, fresh): cargo fmt --check and clippy -D warnings clean; cargo test 0 failed.
