# Spec 3en: the `agy` harness profile, its injected settings, and quota from its status line

## 0. Status

**LOCKED.** Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task
`context_id`: `agy-harness`. Spec 1 of 4 for Google's Antigravity CLI (`agy`): this spec adds the
profile and the quota report; spec 2 (plugin, MCP, confinement grant), spec 3 (delivery and
readiness) and spec 4 (docs, team file, `new` surface) follow.

**Scope.** May edit exactly these files and no others.

- `src/harness.rs`, `src/quota.rs`, `src/statusline.rs`, `src/cli.rs`, `src/main.rs`
- `src/daemon.rs`, `src/session.rs`, `src/confine.rs` (match arms only, section 8)
- `tests/harness_infer.rs`, `tests/quota.rs`, `tests/statusline.rs`, `tests/agy_wiring.rs` (new),
  `tests/daemon_cli.rs` (one test, Amendment 1)

Does not touch: `docs/`, `README.md`, `CHANGELOG.md`, `AGENTS.md` (the architect updates them,
including the Modules table), `Cargo.toml`, `Cargo.lock` (no new dependency), `src/delivery.rs`,
`src/emulator.rs`, `src/wire.rs`, `src/mcp.rs`, `src/team.rs`, every other test file, every other
spec. Do not commit, stage or merge: leave the diff uncommitted in the working tree.

**Public test seams.** `Harness::infer`, `quota::from_agy_status` (pure), `harness::wire_agy_argv`
(pure over its arguments), the `a2amx statusline --harness agy` command as a real process,
`Request::ReportQuota` and `Request::List` through `Client` against a real daemon.

## 1. Why

The owner wants `agy` to be a first-class a2amx harness next to Claude Code, OMP and Codex. A research
pass and live probes (2026-11-14, `agy` 1.5.9) established what the harness offers; this spec uses only the
parts that are verified and cheap:

- `agy --settings '<json>'` is "this session only, never saved". It accepts the keys `statusLine`,
  `queuedMessages` and `trustedWorkspaces` (and `permissions`, `toolPermission`, used by spec 2) and rejects
  unknown keys (`Error: --settings: unknown setting "..."`). It rejects `mcpServers` and `hooks`.
- A `statusLine` command receives a JSON document on stdin on every render tick, with `model.id`,
  `conversation_id`, `agent_state` and a `quota` object keyed by bucket id (`3p-5h`, `3p-weekly`,
  `gemini-5h`, `gemini-weekly`, `gemini-argon-wtu-weekly`), each bucket
  `{remaining_fraction (0..1), reset_time (ISO-8601 Z), reset_in_seconds}`. This is the exact analogue of
  Claude Code's status-line payload (spec 3dc), so the per-session quota report is the existing
  `a2amx statusline` machinery with a second parser.
- The first run in a folder shows a trust dialog; `trustedWorkspaces` in `--settings` containing the
  session's directory suppresses it without writing anything. a2amx never answers the dialog itself.
- While a turn runs, text typed into the composer is queued and submitted after the turn
  (`queuedMessages` default `queue`); `send-immediately` interrupts the turn. a2amx pins `queue`.

Quota has three groups on an account (Gemini models, Claude and GPT models, "Gemini 4 Argon"), each with a
five-hour and a weekly bucket, except Argon with a weekly one only. The session reports the group of its
active model, as the owner decided.

Owner decisions this spec carries (grilling record, `context_id` `agy-harness`): the profile and
executable name is `agy` (no `antigravity` alias); quota and state come from the `statusLine` reporter and
`agy -p "/usage"` is not polled; `queuedMessages` is pinned to `queue`; a2amx never touches the user's
global `~/.gemini` config; tests use payload literals taken from the probes (no real `agy` login), and a
real-`agy` check is an owner step.

Rejected alternatives: polling `agy -p "/usage" --output-format json` (it works, but is account-wide and
needs a separate polling task, while the status line is live, per session and names the model);
`--continue` for anything (it resumed an unrelated old conversation); a screen scrape of the footer (the
status-line command is exact).

## 2. Decisions

- **D1.** `Harness::Agy` serialises as `"agy"`, is the value of `--harness agy`, and `Harness::infer`
  returns it when the command's executable file name is exactly `agy`. Its `default_deliver` is
  `Deliver::Hold` until spec 3 (typed delivery is not built yet); `--deliver auto` stays the explicit
  opt-in, as for `generic`.
- **D2.** `harness::wire_agy_argv(argv, exe, cwd, gemini_dir) -> anyhow::Result<Vec<String>>` returns the
  argument vector with one `--settings <json>` inserted by `insert_extras`, built as in section 3.
- **D3.** `quota::from_agy_status(input: &Value, now_unix: u64) -> Option<ClaudeRateLimits>` (pure) reads the
  status-line JSON as in section 4. The existing types `ClaudeRateLimits` and `ClaudeWindow` are reused
  unchanged; only their doc comments change (a rename is a separate follow-up, section 10).
- **D4.** `a2amx statusline` gains `--harness <claude|agy>` (default `claude`); with `agy` it parses with
  `from_agy_status`. The report path, the daemon request (`Request::ReportQuota`), the token check and the
  fail-open behaviour are unchanged.
- **D5.** The daemon stores the report for `Harness::Agy` sessions exactly as for `Harness::Claude`, and
  `list_agents` shows it as the session's `quota`. For `agy` there is no screen read (`quota::read` returns
  `None`).

## 3. `src/harness.rs`

1. Add `Agy` to `enum Harness` (after `Codex`), `as_str` -> `"agy"`, `infer` -> `Some("agy") => Self::Agy`,
   `default_deliver` -> `Deliver::Hold` for `Agy` (the arm that holds `Generic`).
2. `harness.rs:340` (`escape_for_delivery`, the match that returns `text.to_owned()`) and `harness.rs:281`
   (`ready`, whose `Generic | Omp | Codex` arm is `screen.modes.bracketed_paste && !scrolled`): add
   `Harness::Agy` to both arms. (`ready` is exhaustive and decides `starting()` and delivery readiness;
   spec 3 revisits it with real `agy` signals.)
3. The Claude status-line helpers stay Claude-only and unchanged in behaviour, except for one extraction:
   the shell chain string that `build_status_line` builds inline (the `i=$(cat; echo x); ...` text)
   becomes `fn chained_command(executable: &str, report_flags: &str, user_command: &str) -> String`, called by
   `build_status_line` with `report_flags = "statusline --report"`. Its output for Claude must stay
   byte-identical. The existing tests do not pin the chain string (they only check `contains` and behaviour),
   so section 9.5 adds one literal expected-string test, written and run against the unchanged code first.
4. New `fn resolve_agy_status_line(user_settings: Option<&serde_json::Value>, gemini_dir: &Path) ->
   Option<serde_json::Value>`: the user's `statusLine` from the user `--settings` value, else from
   `<gemini_dir>/antigravity-cli/settings.json` through the existing `read_status_line_file`; never from the
   workspace (`// shortcut: project-level agy settings are not read; add them if agy's status-line
   precedence needs them`). New `fn build_agy_status_line(resolved: &Option<serde_json::Value>, exe: &Path)
   -> serde_json::Value`:
   - `None` -> `{"command": "<quoted exe> statusline --harness agy", "enabled": true}`.
   - a user object with a string `command` that has no NUL byte -> `{"command": <chained_command(exe,
     "statusline --harness agy --report", command)>, "enabled": true}` plus the user's `padding` and
     `stack_with_default` when present. When the user's own `enabled` is `false`, the user's line stays
     invisible and a2amx reports anyway: the result is the silent report-only form `{"command": "<quoted exe>
     statusline --harness agy --report", "enabled": true}` (`--report` prints nothing), so quota still
     reaches the daemon. No `type` key is emitted or required
     (agy's status line object has none) and `refreshInterval` is not copied (it is Claude's key).
   - anything else (not an object, no string `command`, a NUL) -> returned unchanged, so no quota is
     reported for that user (as Claude's helper does).
5. New `pub fn wire_agy_argv(argv: Vec<String>, exe: &Path, cwd: &Path, gemini_dir: &Path) ->
   anyhow::Result<Vec<String>>`. It reuses `take_user_settings` (which removes the user `--settings` from
   `argv`), builds the settings object and inserts one `--settings <json>` with `insert_extras`. The object
   is the user's object (or `{}`) with exactly these keys written or replaced and all other user keys kept:
   - `"statusLine"`: `build_agy_status_line(&resolve_agy_status_line(..), exe)`.
   - `"queuedMessages"`: `"queue"` (a user value is overridden; owner decision Q3).
   - `"trustedWorkspaces"`: the user's array (or `[]`) with the string `cwd` appended once, only when it is
     not already in it. A user `trustedWorkspaces` that is not an array is an error
     (`anyhow!("--settings trustedWorkspaces must be an array")`).
   It does not call `merge_settings` (that function indexes Claude's `hooks.UserPromptSubmit`). A user
   `--settings` that is not a JSON object is the same error `take_user_settings` already gives. It writes
   nothing under `gemini_dir`, and `gemini_dir` is `$HOME/.gemini` (an empty path when `HOME` is unset).
   `resolve_agy_status_line` reads the global file only when `!gemini_dir.as_os_str().is_empty()` (as
   `resolve_status_line` guards `config_dir`, `src/harness.rs:437`): `Path::new("").join(..)` is a relative
   path that would read the process's current directory.
6. `claude_argv`'s behaviour does not change.

## 4. `src/quota.rs`

`pub fn from_agy_status(input: &Value, now_unix: u64) -> Option<ClaudeRateLimits>`:

1. `quota = input.quota` must be a JSON object, else `None`. Each key is a bucket id; a key ending in
   `-5h` is a five-hour bucket of the group named by the prefix, a key ending in `-weekly` a weekly bucket
   (`3p-5h` -> group `3p`, window five-hour; `gemini-argon-wtu-weekly` -> group `gemini-argon-wtu`,
   weekly). Other keys are ignored.
2. A bucket is valid when `remaining_fraction` is a finite number in `0.0..=1.0` and `reset_in_seconds`
   satisfies `as_u64()` (so `12.0` and negative values are invalid) with `1 <= value <=
   CLAUDE_WINDOW_MAX_SECS`; an invalid bucket counts as missing. Its window is `left =
   (remaining_fraction * 100.0 - 1e-9).ceil()` clamped to `0..=100`: the epsilon cancels float noise
   (`0.07 * 100.0` is `7.000000000000001`, which a bare `ceil` would turn into 8) and the `ceil` keeps a
   remainder from showing as 0 (`QuotaInfo::exhausted()` turns 0 into "quota exhausted"). The formula alone
   is the contract (Amendment 2): there is no separate threshold branch, so a fraction of `1e-11` or less
   gives 0 and anything above it gives at least 1.
   `resets_at = now_unix + reset_in_seconds`. Worked values: `1.0` -> 100, `0.763` -> 77, `0.999` -> 100,
   `0.07` -> 7, `0.001` -> 1, `1e-10` -> 1, `1e-12` -> 0, `0.0` -> 0.
3. Group of the session: from `input.model.id`, the first matching rule wins, in this order: starts with
   `claude` or `gpt` -> `3p`; contains `argon` -> `gemini-argon-wtu`; starts with `gemini` -> `gemini`;
   anything else, or a missing `model.id` -> no group. With a group, the five-hour and weekly windows come
   from that group's buckets (a missing bucket is `None`). With no group, each window is the one with the
   smallest `left` over all groups that have it, and its `resets_at` is the reset of that bucket (the
   earlier reset when two buckets tie): the most pessimistic rule spec 3j uses for OMP. Example, unknown id
   `foo` with `3p-5h` 40%, `gemini-5h` 60% and all weekly buckets 90%: five-hour 40% (the `3p` reset),
   weekly 90% (the earlier of the three resets).
4. `None` when neither window is available.
5. Doc comments of `ClaudeWindow` and `ClaudeRateLimits` say they also carry `agy` windows.

## 5. `src/statusline.rs`, `src/cli.rs`, `src/main.rs`

1. `cli.rs`: `Statusline` gains `#[arg(long, value_enum, default_value = "claude")] harness:
   StatuslineHarness`, with a new two-variant clap enum `StatuslineHarness { Claude, Agy }` in `cli.rs`
   (only these two harnesses have a status-line payload; `Harness` is not reused, so `generic` cannot be
   given). The help text of `Statusline` names both harnesses.
2. `main.rs:175`: pass `*harness` to `statusline::run(report, harness)`; `main.rs:324` (the or-pattern that
   names `Command::Statusline { .. }`) stays valid.
3. `statusline.rs`: `run(report, harness)` picks `quota::from_claude_rate_limits` or
   `quota::from_agy_status` by `harness`; `format_line`, `report_quota`, the size bound
   (`MAX_STDIN_BYTES`), the 500 ms timeout and every fail-open path stay as they are. The visible line for
   `agy` is the same short line (`5h N% left 7d N% left`).

## 6. `src/daemon.rs` and `src/session.rs`

1. `daemon.rs` `report_quota` (the line `if session.harness() == Harness::Claude {` before
   `session.set_quota_report`, near 2012): becomes `matches!(session.harness(), Harness::Claude |
   Harness::Agy)`.
2. `daemon.rs` `session_quota` (starts at 3430): the early return `!matches!(harness, Harness::Claude |
   Harness::Codex | Harness::Omp)` (3436) adds `Harness::Agy`; the Claude branch `if harness ==
   Harness::Claude` (3440) becomes `matches!(harness, Harness::Claude | Harness::Agy)`.
   `quota::read(.., Harness::Agy)` returns `None`, so only the report fills the windows.
3. `daemon.rs` activity signal match (the `Harness::Omp | Harness::Generic => None` arm, 1014): add
   `Harness::Agy` (`None`; spec 3 gives `agy` a real signal).
4. `session.rs:1001` (`draft_is_stale`): add `Harness::Agy` to the `Generic | Codex` arm (`false`).
5. `session.rs:589` (`starting()`): `!matches!(self.harness, Harness::Generic)` becomes
   `!matches!(self.harness, Harness::Generic | Harness::Agy)`: an `agy` session is not shown as "starting"
   until spec 3 knows its ready signal (it is not known that `agy` sets bracketed-paste mode).
6. (Amendment 1) `session.rs:1252` (`kill`) is unchanged: `agy` gets the graceful exit like Claude, OMP
   and Codex. In `graceful_exit` (`session.rs:1304`) the second Ctrl-D is sent for `agy` too: `if
   self.harness == Harness::Claude` (1330) becomes `if matches!(self.harness, Harness::Claude |
   Harness::Agy)`, and the constant `CLAUDE_EXIT_KEY_GAP` (`session.rs:35`, used at 1333) is renamed
   `EXIT_KEY_GAP` with its `// shortcut:` comment reworded to "fixed 300 ms against the pending-exit window
   of Claude and agy; use a screen-based wait if a release shortens that window". The owner tested
   2026-11-14 that `agy` exits on Ctrl-D sent twice, as Claude does; the 300 ms gap for `agy` is unprobed.
   The existing gate (`reset_gate_decision`: a recorded human draft, a screen that is not ready by
   `harness::ready`, a native hold) still skips the graceful exit, and a timeout still falls back to
   `HUP`.
7. `quota.rs:107` (`read`): add `Harness::Agy` to the `Generic` arm (`None`).

## 7. `src/main.rs`: launch wiring

`main.rs` `wired_command` (starts at 1020; the Claude arm is first, the `Harness::Omp if !install_omp` arm is
at 1068) gets a `Harness::Agy` arm: spawn_blocking `harness::wire_agy_argv(command,
&exe, cwd, &gemini_dir)` with `exe = std::env::current_exe()?` and `gemini_dir = $HOME/.gemini`
(empty when `HOME` is unset, which `resolve_agy_status_line` treats as "no file"), as the Claude arm does for
`CLAUDE_CONFIG_DIR`. The peer-authorization prompt and role line are NOT injected for `agy` in this spec
(spec 2 decides how; `agy` has no `--append-system-prompt`). `main.rs:1081` (the `Generic | Codex`
passthrough) is unchanged and `Agy` is not added to it. `main.rs:1263`/`1266` (Codex-only checks) are
unchanged. `cwd` is passed as `wired_command` receives it (`cwd.to_owned()`); the spec does not
canonicalize it (see the claims table: whether `agy` compares the literal string is unprobed).

## 8. `src/confine.rs`: match arms only

Every exhaustive `match` on `Harness` in `confine.rs` gets `Agy` treated **as `Generic` for data and
roots**: `488`, `606`, `661`, `1042` (the `unreachable!()` arm; the guard above it at `1036` adds `||
harness == Harness::Agy`), `1127` (`Harness::Generic | Harness::Agy => unreachable!()`, which holds because
the `matches!` guard at `1121` excludes `Agy`) and `1325` (`Harness::Generic | Harness::Agy => None`). The
launch match in `launch_argv` (2545-2558) is the exception: `Agy` joins the `Harness::Codex => plan.wrap(argv)`
arm (2557), a plain wrap with no `env --default-signal=INT` and no Ctrl-C wrapper and no notice, because
`agy` is a TUI and not a shell, and because `daemon.rs:421` runs `confine::preflight_env` only for
`Harness::Generic`, so the Generic arm would use the env wrapper without that preflight. Known limitation, to be fixed by spec 2: a
confined `agy` session starts but its TUI fails (it cannot write `~/.gemini`); this spec does not hide
that, and does not add a refusal.

## 9. Tests

1. `tests/harness_infer.rs`: `agy`, `/usr/local/bin/agy` -> `Agy`; `agyx`, `my-agy` -> `Generic`;
   `Harness::Agy.as_str()` is `"agy"`; `Deliver` default is `Hold`.
2. `tests/quota.rs`: `from_agy_status` over literal payloads taken from the probe: all five buckets present
   with `model.id` `gemini-3.8-flash-low` (group `gemini`); a payload with model `claude-sonnet-x` (group
   `3p`); model absent and an unknown model id `foo` (smallest over groups, with the `resets_at` of the
   bucket that gave the minimum, and the earlier reset on a tie: the section 4 example); the Argon group
   (weekly only); `remaining_fraction` `0.763` -> 77, `0.07` -> 7, `0.001` -> 1, `1e-10` -> 1, `1e-12` -> 0, `0.0` -> 0; `reset_in_seconds` `0` and `700000` rejected; `remaining_fraction` `1.2`
   and `"x"` rejected; unknown bucket keys ignored; no `quota` key -> `None`.
3. `tests/agy_wiring.rs` (new): `wire_agy_argv` with no user settings produces exactly one `--settings` with
   the exact JSON of section 3 (literal expected string); a user `--settings` keeps its other keys,
   gets `queuedMessages` overridden to `"queue"`, and `cwd` appended once to its `trustedWorkspaces`;
   a user `statusLine` object with a string `command` is chained (the output contains both commands; the
   `--harness agy --report` call comes first; no `type` and no `refreshInterval` key; the user's `enabled`,
   `padding` and `stack_with_default` are kept); a user `statusLine` without a `command` is returned
   unchanged; a `statusLine` in a temp `gemini_dir/antigravity-cli/settings.json` is chained when the user
   passes none; a user `statusLine` with `"enabled": false` becomes the silent report-only form; an empty
   `gemini_dir` with a decoy `antigravity-cli/settings.json` in the process's current directory reads
   nothing (the test sets the current directory with a guard that restores it, or runs the wiring in a
   child process); a non-array `trustedWorkspaces` is an error; nothing is created under `gemini_dir`
   (directory listing before and after).
4. `tests/statusline.rs`: add an `agy_case` beside `claude_case` (`tests/statusline.rs:132`) that creates the
   session with `common::new_agent(.., Harness::Agy, ..)` (no fake executable: the daemon does not wire
   argv, `main.rs` does). With the existing real-process helpers, `a2amx statusline --harness agy` fed an
   agy payload prints `5h N% left 7d N% left` and, with `--report`, prints nothing and reports to the real
   daemon; `Request::List` then shows the quota. The size bound and the fail-open behaviour are covered by
   the existing Claude tests and are not repeated.

6. (Amendment 1) `tests/daemon_cli.rs`: `kill_sends_agy_two_key_exit_sequence`, a copy of
   `kill_sends_claude_two_key_exit_sequence` (`tests/daemon_cli.rs:1437`) with
   `ExitFixture::start("two", "agy")`: it asserts the recorded bytes are exactly `\x04\x04` and the log shows
   `outcome="exited"`. The fixture's screen sets bracketed-paste mode, which is what `harness::ready` checks
   for `agy`, so the gate passes. It fails before the change (one Ctrl-D, then the 10 s timeout).
5. `tests/statusline.rs` (Claude, a contract test that passes on the old code): one test calls
   `harness::wire_claude_argv` with a user `--settings` that has an inline status line, a fixed `exe`
   (`/x/a2amx`), a fixed `cwd` and an empty config directory, and asserts the whole `statusLine.command`
   string against a literal copied from the current output (written first, run against the unchanged code,
   then it must still pass after `chained_command` is extracted).

## 10. Out of scope

Everything in specs 2 to 4: the plugin and its auto-install, MCP registration, the peer-authorization
prompt, permission rules, the `~/.gemini` confinement grant, typed delivery, composer and idle
detection, OSC 7501, `agent_state` reporting, `/clear` reset, `team` file and `a2amx new` surface, docs.
Also out: renaming `ClaudeRateLimits`/`ClaudeWindow` (follow-up), reading project-level `agy` settings,
`agy -p "/usage"`, any alias for the profile name, a `--confine` refusal for `agy`.

## 11. Acceptance

Run after the implementation, in this order, with the ambient environment (the suite passes inside an
a2amx-hosted session, `AGENTS.md`):

1. `cargo test` passes, including the new tests above (the section 9.1 to 9.4 tests fail before the change
   because `Harness::Agy` and the functions do not exist: they are compile-red, not logic-red; the 9.5
   Claude test is a contract test that passes on the old code and must keep passing).
2. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass with no warnings.
3. `git status --short` shows only files named in section 0 (the architect commits this spec before
   assigning it, so the spec file is not listed); `git diff` has no change under `docs/`.
4. `rg -l "Agy" src` lists exactly `src/cli.rs`, `src/confine.rs`, `src/daemon.rs`, `src/harness.rs`,
   `src/main.rs`, `src/quota.rs`, `src/session.rs` and `src/statusline.rs`; every `match` site named in
   sections 3, 6, 7 and 8 has its arm (the compiler enforces the exhaustive ones; the architect reads the
   diff for the rest).
5. The architect reads the diff, including removed lines, and checks that no Claude behaviour changed
   (the Claude tests in `tests/statusline.rs`, `tests/claude_channel.rs`, `tests/messaging.rs` pass
   unchanged, and `claude_argv`'s exact JSON is byte-identical).
6. Owner post-implementation check, after the architect commits: `a2amx new --harness agy -- agy` in a
   trusted folder shows the quota in `a2amx list` after the first turn; the trust dialog does not
   appear; the user's own status line, if any, still shows.

Trust boundaries: the status-line payload comes from the harness process, which the operator already
trusts as the session's child; the reporter's size bound, timeout and fail-open behaviour are unchanged.
The injected settings contain no secret; the session token stays in the environment as for Claude. No new
path is created, so no abuse case is added; the reuse of the token check is covered by the existing
`statusline_report_is_silent_and_works_with_only_session_environment` test.

## 12. Claims about existing code (pre-lock table)

| Claim | Where | Verified how |
| --- | --- | --- |
| `Harness` has four variants, `infer` matches executable names | `src/harness.rs:22-68` | read |
| `escape_for_delivery` match returns `text.to_owned()` for `Generic/Omp/Codex` | `src/harness.rs:340` | read |
| `build_status_line`/`resolve_status_line` are Claude-only helpers with a hardcoded `statusline --report` | `src/harness.rs:421-486` | read |
| `merge_settings` handles Claude's hook array and assumes `statusLine` | `src/harness.rs:553-590` | read |
| `report_quota` stores a report only for Claude | `src/daemon.rs:2010-2013` | read |
| `session_quota` has a Claude-only branch and rejects other harnesses | `src/daemon.rs:3430-3452` | read |
| `Statusline { report }` in `cli.rs:191` and `main.rs:175` | as stated | read |
| `quota::read` match has no `Agy` arm yet | `src/quota.rs:105-110` | read |
| `AnyChannel::for_session` already falls through to the PTY channel for unknown harnesses, so `delivery.rs` needs no arm | `src/delivery.rs:98-105` | read |
| `confine.rs` arms at 488, 606, 661, 1042, 1127, 1325, 2557 are exhaustive matches that name `Generic` (1127 and 1325 included: `Generic => unreachable!()` and `Generic => None`) | `src/confine.rs` | read at 1118-1130 and 1318-1328; the others by grep on `Harness::Codex` (2545-2558 read: `Agy` joins the `Harness::Codex` arm, a plain `plan.wrap`, not the Generic env-wrapper arm) |
| no test or source mentions `Agy` or `from_agy` yet | `rg -n "Agy\|from_agy" tests src` | probed (no output) |
| `statusLine` payload keys and quota bucket shape | tester report `m_7374` section D | probed live by the tester, not re-run by the architect |
| `--settings` accepts `statusLine`, `queuedMessages`, `trustedWorkspaces` and rejects unknown keys | tester reports `m_7374`, `m_7382` | probed live by the tester |
| model ids other than `gemini-3.8-flash-low` map to the groups as in section 4 | section 4 | **recall**: only that one id was observed; other prefixes are a guess and the "no group" rule is the fallback |
| `agy` replaces (does not merge) a user `statusLine` when `--settings` supplies one | section 3 | **recall**: assumed from Claude's behaviour, not probed |
| `agy` runs the `statusLine` command through a shell (the chain is `sh` syntax) | section 3 | **recall**, not probed; the default command is a single quoted executable and works either way, only the chain needs a shell |
| the global settings path is `~/.gemini/antigravity-cli/settings.json` with a `statusLine` key | section 3 | the path was seen by the tester (stat only); the key name is **recall** from the docs |
| `agy` compares `trustedWorkspaces` against the literal cwd string, not a canonical path | section 3 | **recall**, not probed; the owner check (acceptance 6) covers a symlinked cwd |
| `--settings` takes inline JSON or a file path; the last `--settings` wins when repeated | section 3 | file-path form read in `agy --help` ("inline or as a file path"); last-wins is **recall** (Claude semantics) |
| `harness::ready` has the arms `Generic | Omp | Codex` and `Claude`; `starting()` excludes only `Generic`; `kill` skips graceful exit only for `Generic` (it stays so: Amendment 1); `graceful_exit` sends the second Ctrl-D only for `Claude` (`session.rs:1330`); `append_system_prompt` returns early only for `Generic` | `src/harness.rs:281`, `src/session.rs:589`, `src/session.rs:1252`, `src/harness.rs:678` | read (consultant finding, re-read by the architect) |
| `daemon.rs:421` runs `confine::preflight_env` only for `Harness::Generic` | `src/daemon.rs:421` | read |
| no test pins the `Statusline` help text | `tests/` | consultant grep, not re-run |
| exact `claude_argv` JSON is pinned by `tests/messaging.rs` and the status-line chain by `tests/statusline.rs` | as stated | consultant read, not re-run |

## Amendment 1 (2026-11-14, architect, before the developer's first production edit)

The owner tested that `agy` exits on Ctrl-D sent twice, the same as Claude Code. Section 6 item 6
therefore no longer excludes `agy` from the graceful exit: `kill` is unchanged, `graceful_exit` sends the
second Ctrl-D for `agy`, the gap constant is renamed `EXIT_KEY_GAP`, and one test is added in
`tests/daemon_cli.rs` (scope list in section 0 and the claims table updated; section 9 item 6). The
decision to keep `agy` out of `starting()` (item 5) stands. Rejected: leaving `HUP` as the stop for `agy`
(it works but skips the clean exit that preserves the resume line and closes the conversation); typing
`/exit` (needs the idle detection of spec 3). Reconciled against the rest of the spec: no other section
mentions Ctrl-D for `agy`; acceptance step 4 (`rg -l "Agy" src`) is unchanged because `session.rs` and
`daemon_cli.rs` already appear (`daemon_cli.rs` is a test file and not in that `src` list).

## Amendment 2 (2026-11-14, architect, after the tester's BLOCKED m_7415)

The tester found that section 4 item 2 stated the rounding twice with different results. The formula
`(f * 100.0 - 1e-9).ceil()` gives 1 for `f = 1e-10`; the prose I added after the consultant's nit
("a fraction below 1e-9 counts as 0") gives 0 (the epsilon applies to `f * 100`, so the threshold the
formula implies is `f <= 1e-11`, not `1e-9`). The implementation followed the prose with an extra
`if fraction < 1e-9 { 0 }` branch. Decision: the formula is the contract and the prose was wrong. The
extra branch is removed (`ceil` of a negative tiny value is `-0.0`, which `clamp(0.0, 100.0)` turns into
0, so `0.0` still gives 0); the worked values gain `1e-10` -> 1 and `1e-12` -> 0 (section 4 item 2 and
section 9.2). Rejected: keeping the branch and rewriting the formula (it adds a special case that protects
nothing the formula does not already do).

The tester also reported three test gaps. They are implementation gaps and not contradictions: section
9.2 requires positive cases for a `claude-` model (group `3p`), for an Argon model (weekly only, five-hour
`None`) and for a missing `model.id` (smallest over groups), and section 9.3 requires the empty
`gemini_dir` test with a decoy `antigravity-cli/settings.json` in the process's current directory.
Confirmed against `tests/quota.rs:1289-1384` and `tests/agy_wiring.rs`: the claude case is only the
negative "known group missing" one, no Argon-model or absent-model case exists, and no decoy test exists.
The developer adds them. Reconciled: no other section states the rounding threshold.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (spec review, two rounds); developer OMP
openai-codex/gpt-5.6-luna; tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 1 BLOCKED
(tester m_7415, Amendment 2) plus Amendment 1 from owner information.

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review, round 1 | consultant | No `harness::ready` arm for `Agy`; the chained status-line design did not fit agy's shape (no `type` key) | Separate agy helpers sharing `chained_command`; `ready` arm added |
| Draft review, round 1 | consultant | `confine.rs` anchor contradicted the preflight rule (`preflight_env` only for Generic) | `Agy` joins the Codex plain-wrap arm in `launch_argv` |
| Draft review, round 1 | consultant | Bare `ceil(f*100)` gives 8 for 0.07 | Epsilon formula `(f*100 - 1e-9).ceil()` |
| Draft review, round 2 | consultant | Empty `gemini_dir` reads a relative path; "byte-identical proven by tests" claim was false; `enabled:false` silently disabled quota | Guard and decoy test; literal contract test; report-only form |
| Owner review | owner | Spec said `agy` gets no Ctrl-D; it exits on a double Ctrl-D like Claude | Amendment 1; `EXIT_KEY_GAP`; kill test added (not run red, disclosed) |
| Implementation | developer | First report missed the quota cases, decoy test, `--report` on a fresh session, chain-order assertions and the shortcut comment | Added in the next report |
| Implementation | developer | `starting()` latch conjunct dropped; found by the full suite | Restored |
| Tester pass | tester | Spec prose ("below 1e-9 counts as 0") contradicted the formula; an extra branch was built from it (architect-introduced) | Amendment 2; branch removed |
| Tester pass | tester | Two surviving mutants: `starting()` Agy exclusion and the confined Agy launch arm have no test | Accepted; covered by specs 2 and 3 tests |
| Architect review | architect | Blank line deleted in `quota.rs` | Restored |
