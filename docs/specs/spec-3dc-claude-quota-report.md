# Spec 3dc: A Claude session reports its quota through an injected status-line command (fresh installs show `-`)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/wire.rs` (one new `Request` variant and one small type), `src/quota.rs`
(one pure conversion function and its result type), `src/session.rs` (one field, its setter and getter),
`src/daemon.rs` (token permission, the handler, `session_quota`), `src/statusline.rs` (new module: the
`statusline` subcommand), `src/lib.rs` (module line), `src/cli.rs` and `src/main.rs` (the `Statusline` command,
its dispatch, and `wired_command` passing the config directory), `src/harness.rs` (`claude_argv`,
`wire_claude_argv`, `wire_claude_channel_argv`: inject and chain the status line), `tests/wire.rs`,
`tests/quota.rs`, `tests/statusline.rs` (new), `tests/messaging.rs` and `tests/claude_channel.rs` (the existing
callers of the two wire functions: a signature change, plus the exact-JSON expectations named in section 4),
and one daemon test in `tests/broker.rs` or `tests/explain.rs` (the developer names the file). Does not touch:
`docs/`, `CHANGELOG.md`, `AGENTS.md` (the architect updates them, including the module table), the OMP quota
path, `read_windows` and the screen scrape (kept as the fallback, unchanged), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seams: `a2amx::quota::from_claude_rate_limits` (pure), `Request::ReportQuota`
through a real daemon, the `a2amx statusline` command (a real process), `harness::wire_claude_argv`.
This spec changes the public interface compatibly: a new request variant, a new subcommand, and a new
`statusLine` entry in the `--settings` JSON that a2amx passes to Claude Code.

## 1. Why

A Claude session's QUOTA column and `list_agents` quota come from `quota::read_windows`, which scrapes
`5h N% left` and `weekly|7d N% left` tokens from the last three screen rows. Only a user-made status line
prints those tokens. A fresh Claude Code install (v2.1.319, probed 2026-11-07) has none, so QUOTA is `-`. The
owner decided this is a bug that is fixed in a release candidate before 0.2.0.

Source (consultant advice, checked against the Claude Code status-line documentation; the exact field names
were verified there, not in this repository): Claude Code runs a configured `statusLine` command and writes a
JSON document to its stdin that carries `rate_limits.five_hour.used_percentage`,
`rate_limits.seven_day.used_percentage` (numbers 0 to 100, possibly fractional, the share USED) and
`rate_limits.five_hour.resets_at`, `rate_limits.seven_day.resets_at` (Unix seconds). The field exists only for
claude.ai Pro, Max and Ultra plans, only after the first API response of the session, and each window can be
missing alone. No `claude` subcommand prints usage (`claude --help`, v2.1.319), hook input carries no numbers,
and the usage API needs the user's OAuth token.

Decision: inject a `statusLine` command into the `--settings` JSON a2amx already builds in `claude_argv`; the
command is `a2amx statusline`. It reads the JSON, reports the windows to the daemon with the session's own
token (as `a2amx hook` does), then prints a status line. `statusLine` is one object, and `--settings` replaces
the same key from the user, project and local files, so a user's own status line would vanish unless it is
chained: the injected command is a shell snippet that reads stdin once, hands a copy to `a2amx statusline --report` and a copy to the user's own command, and passes the user's stdout through (no `sh` in Rust: the `no-shell-command` semgrep rule forbids it, and Claude Code already runs the string through a shell). The daemon prefers a fresh report over the screen scrape. The owner chose the visible text for a user
who has no status line of their own: a short line `5h 77% left 7d 59% left`.

Visible change, stated for the owner: every a2amx Claude session now has a custom status line. Claude Code adds
a footer row for it and drops most of its default footer hints (docs, recalled, not re-fetched). A session with
no quota yet (an API-key user, or before the first API response) shows an empty row.

Rounding: `left = clamp(ceil(100 - used), 0, 100)`. `ceil` so that a remaining fraction is never shown as 0:
`QuotaInfo::exhausted()` turns 0 into "quota exhausted" in the `send_message` recipient hint
(`src/daemon.rs:1200`), which must not happen while 0.1% is left. Worked values: used 23.7 gives 77 (`round` would give 76), 23 gives 77, 41 gives 59, 99.9 gives 1 (`round` would give 0), 100 gives 0, 0 gives 100.

Rejected, with reasons: typing `/usage` into the composer (it can destroy a draft, AGENTS.md); calling the
usage API (needs credentials); polling a `claude` usage command (none exists); reading the status line from
the screen only (the bug); injecting without chaining (hides the owner's own status line); writing the
quota to a file instead of the daemon (a second state store, and no token check); `round` instead of `ceil` (99.9 used would read as exhausted, `round` gives 0); a Rust `sh -c` chain behind a semgrep exemption (the owner preferred no rule change).

## 2. Changes

1. `src/quota.rs`: add `pub struct ClaudeWindow { pub left: u8, pub resets_at: u64 }` and `pub struct
   ClaudeRateLimits { pub five_hour: Option<ClaudeWindow>, pub weekly: Option<ClaudeWindow> }` and `pub fn
   from_claude_rate_limits(input: &serde_json::Value, now_unix: u64) -> Option<ClaudeRateLimits>`. It reads
   only `rate_limits.five_hour` and `rate_limits.seven_day`. A window counts when `used_percentage` is a finite
   number in `0.0..=100.0` and `resets_at` is an integer greater than `now_unix` and at most `now_unix + 8 days`
   (a missing, past, far-future or malformed `resets_at` drops the window). `left` follows the rounding rule of
   section 1. Returns `None` when neither window counts. `QuotaInfo` in `src/wire.rs` does not change.
2. `src/wire.rs`: add `pub struct QuotaWindow { pub left: u8, pub resets_at: u64 }` (serde) and `Request::ReportQuota {
   five_hour: Option<QuotaWindow>, weekly: Option<QuotaWindow> }`. The success answer is `Response::Ok` (`src/wire.rs:295`,
   used by cancel). The daemon rejects, with a `Response::Error`, a `left` above 100 and a `resets_at` more than 8
   days after the daemon's clock; an already expired window is accepted and dropped by the expiry rule.
3. `src/daemon.rs`: `session_token_permitted` allows `ReportQuota`; the handler, like `ReportPrompt`
   (line 2097), answers an error for `Role::Admin` ("report_quota needs a session token") and stores the
   report on the session for `Role::Session(id)`: the sender is the token's session, never a field. A report
   for a non-Claude session is accepted and ignored (return `Response::Ok`, store nothing), so a stray call
   cannot mark an OMP session. `session_quota` (line 2570) returns, for `Harness::Claude`, the stored windows
   that are not expired (each window is expired when its own `resets_at` is not after the daemon's clock, so
   an expired 5h window does not hide a valid weekly one), else the screen scrape for the missing windows; the
   `limit_reached` flag still comes from the screen scrape, combined as the OMP branch does at
   `src/daemon.rs:2583-2598` (a report never clears a screen-seen usage limit). A report may be replaced by a
   later one; a later report with only one window keeps the stored other window until it expires.
4. `src/session.rs`: one field on the session state for the report (the two `Option<QuotaWindow>`), a setter,
   and a getter that takes the clock and returns the unexpired part.
5. `src/statusline.rs`, `src/lib.rs`, `src/cli.rs`, `src/main.rs`: a `Statusline` command `a2amx statusline [--report]`,
   dispatched without a daemon home like `Hook` (`src/main.rs:122`, and the exclusion at line 213). It never runs
   a shell and never starts a child. Behavior: (a) read stdin, at most 256 KiB plus one byte (`Read::take`); more
   than 256 KiB is "too large": report nothing; (b) parse the JSON and call `from_claude_rate_limits` with the
   current time; (c) when it returns `Some`, send `ReportQuota` with `mcp::configuration()` and a 500
   millisecond timeout (the user's status line must not wait long on a hung daemon), ignoring every error (a note
   on stderr like the hook, never the token or stdin); (d) with `--report`, print nothing and exit 0; (e) without
   `--report`, print one line: the windows that are known as `5h N% left` and `7d N% left` joined by a space
   (`5h 77% left 7d 59% left`), or an empty line when neither is known, and exit 0. It never prints stdin.
6. `src/harness.rs`: `claude_argv` adds a `statusLine` object to the settings JSON:
   `{"type": "command", "command": <string>}`. With no chained user command the string is `<shell_quote(exe)> statusline`. With one it is
   `i=$(cat); printf '%s' "$i" | <shell_quote(exe)> statusline --report; printf '%s' "$i" | (<user command>\n)`: the report
   runs first, then the user's command receives the same bytes (the closing parenthesis follows a newline so a trailing
   comment in the user's command cannot swallow it), its stdout and exit status are the status line's; the user's
   command text is placed in the string as the user wrote it, exactly as Claude Code would run it on its own,
   and is never parsed or split by a2amx (a user command with an unterminated quote or construct fails the way
   it would fail without a2amx)
   plus the user's `padding` and `refreshInterval` when the resolved user object has them. The user's status
   line is resolved in Claude Code's precedence order, first hit wins: the `statusLine` of the `--settings`
   value the user passed (already parsed by `take_user_settings`), then `<cwd>/.claude/settings.local.json`, then
   `<cwd>/.claude/settings.json`, then `<config dir>/settings.json`. Only `statusLine.type == "command"` with a
   string `command` is chained, and a command containing a NUL byte or an unreadable value is not (it is kept as the user wrote it); a user `statusLine` of any other shape is kept as the user wrote it and a2amx
   injects nothing (the screen scrape stays the source). Unreadable or invalid files are skipped (they are the
   user's config, not credentials; never read `~/.claude.json`). The command is captured at launch, so a later
   edit of the user's settings takes effect after a restart. `wire_claude_argv` and `wire_claude_channel_argv`
   gain a `config_dir: &Path` parameter (`claude_argv` too); `wired_command` in `src/main.rs` computes it as
   `CLAUDE_CONFIG_DIR` when set, else `$HOME/.claude`, and when `HOME` is also unset the config-dir scope is
   skipped. `merge_settings` stays for hooks; the status line is merged by one new branch, not a second merge
   path. A `// shortcut:` comment notes that only `<cwd>/.claude` is read, not a project root or a git root.

## 3. Out of scope

OMP and Codex quota, `read_windows` and its fixtures, the managed-settings file (`allowManagedHooksOnly` or a
managed `statusLine` makes Claude Code ignore ours, and the scrape fallback then runs), a user whose
`disableAllHooks` is true (the status line is inert, the scrape runs), plugin-supplied status lines, the
`spend_limit` object, documenting the new line (the architect), an `a2amx` option to disable the injection (a
follow-up if the owner wants it), and sub-project root detection for the `.claude` directory (cwd only). Claude Code cancels an in-flight status-line run when a new one starts; a2amx starts no child, so nothing of a2amx's can be orphaned, and the user's own command behaves as it would without a2amx.

## 4. Tests

Literal values, not recomputed. Every new test and every updated exact test passes a temporary directory as `cwd`
and another as `config_dir`, never `Path::new(".")`: the checkout holds a globally ignored
`.claude/settings.local.json` that would make the expected output depend on the machine.
1. `from_claude_rate_limits` with `now` = 1000 and `resets_at` 2000 unless stated: `used` 23.7 gives `left` 77; 23 gives 77; 41
   gives 59; 99.9 gives 1; 100 gives 0; 0 gives 100; only the 5h window present: `weekly` is `None`; `used` 101,
   -1 and a string drop the window; `resets_at` 1000 and 999 drop it; `resets_at` 1000 + 691200 (8 days) is kept and 1000 + 691200 + 1 drops it;
   no `rate_limits`: `None`; both windows dropped: `None`.
2. `tests/wire.rs`: `ReportQuota` round trip with both windows and with one; the exact-shapes test at
   `tests/wire.rs:139` gains `ReportQuota`.
3. Daemon, through a real daemon and a Claude stub session's token (as `tests/hook.rs:230` does): a report
   with both windows makes the List quota `five_hour_left` and `weekly_left` equal the reported `left`; the same
   request with the admin token is an error; a `left` of 101 and a `resets_at` over 8 days ahead are errors; a
   report whose 5h window is already expired and whose weekly window is valid shows only the weekly value (past
   `resets_at` values, no sleeping); a report from an OMP stub session changes nothing; with a stored report and
   a screen showing `5h 11% left`, the report wins for the windows it holds.
4. `tests/statusline.rs`, running `a2amx statusline` as a process, and the composed command string as a shell
   would run it. Mechanisms: a real daemon with a Claude stub session token for the end-to-end cases (as
   `tests/hook.rs:230`), and a raw TCP listener that completes the handshake, decodes the request frame and records
   its arrival `Instant` for the ordering case (the failure paths imitate `tests/hook.rs:303` and `:360`). The stdin
   is valid JSON with `rate_limits` used 23 and 41 (so 77 and 59), and a `resets_at` far enough ahead. Cases: no
   `--report`: stdout is exactly `5h 77% left 7d 59% left` plus a newline and the daemon got one `ReportQuota`;
   `--report`: stdout is empty and the daemon got the report; a stdin of exactly 256 KiB of valid JSON (padding
   inside a long string field, `rate_limits` before it) reports; a stdin of 256 KiB + 1 bytes of valid JSON (the
   same shape, `rate_limits` first) reports nothing, observed by the real daemon's List quota staying absent (the stdin
   writer tolerates a broken pipe because the command stops reading); invalid JSON, and no daemon reachable: stdout and
   exit as rule (e) says, within 2 seconds; neither stdout nor stderr contains the session token in any case.
   The composed string (the `command` value that `wire_claude_argv` produced, run with `sh -c` in the test and the
   same stdin): user command `cat`: stdout equals the stdin bytes exactly; ordering with the user command `sleep 1;
   echo done`: the listener's arrival time is earlier than the instant the first stdout byte reaches the test;
   user command `exit 3`: exit status 3; user command `echo ok # trailing comment`: stdout `ok`, exit 0; and with a
   daemon that never answers, the user's output still appears within 2 seconds.
5. `harness` tests through `wire_claude_argv`: no user status line anywhere: `statusLine.command` is
   `'<exe>' statusline`; a user `statusLine` command in each of the four scopes alone, and the precedence when two
   scopes hold one, yields the composed string above with exactly that command text inside the parentheses, including one that
   contains a single quote and one that ends in a comment; `padding` and `refreshInterval` are carried; a `statusLine` object of another shape, and a command containing a NUL byte, is
   left untouched and not chained; `hooks` still hold the `UserPromptSubmit` entry; `HOME` and `config_dir`
   handling is covered by the caller computing it (test `wired_command`'s helper if it has a seam, else name the
   gap in the report). The existing exact-JSON tests are edited on purpose to include the new `statusLine` entry:
   `harness_defaults_and_claude_argv_wiring_are_exact` (`tests/messaging.rs:244`), the user-model merge test
   (`tests/messaging.rs:403`) and the exact `json!` in `tests/claude_channel.rs:527-530`;
   `tests/claude_channel.rs:536` (`plain.len() == 7`) is unaffected because the settings stay one argument.

## 5. Acceptance

1. `cargo test` focused: the new tests pass; the red phase before the change is that tests 1 to 5 do not
   compile or fail; the developer shows it. Contract tests (they pass on the old code): the Claude scrape cases in
   `tests/quota.rs` (`quota_of`, line 273) and the OMP and Codex quota cases.
2. Mutation check by restore-by-checksum edits in the checkout (the tree must equal the developer's own diff
   afterward): (a) `ceil` replaced by `round`, test 1 (23.7 and 99.9) fails; (b) drop the `resets_at > now` test,
   test 1 fails; (c) put the `--report` step after the user's command in the composed string, test 4's ordering
   case fails; (d) let the Admin role store a report, test 3 fails; (e) drop the 256 KiB bound, test 4's
   256 KiB + 1 case fails; (f) change the composed string so the user command is not inside a subshell with a
   newline before `)`, test 4's trailing-comment case fails; (g) make `session_quota` read the stored report for
   every harness, test 3 fails (it asserts the OMP stub's List quota stays at its no-report value); (h) expire the
   whole report on the earliest `resets_at`, test 3's one-expired-window case fails; (i1) drop the 8-day bound in
   `quota.rs`, test 1 fails; (i2) drop the 8-day bound in the daemon, test 3 fails; (j) change `<=` to `<` at
   the 8-day boundary, test 1's kept case fails. The developer names the failing test for each.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient
   environment of a hosted session. Known flakes I7, I18: measure a failing test by rerunning it alone several
   times and state how often it failed.
4. Release gate, a tester pass on the next rc build with a fresh `CLAUDE_CONFIG_DIR`, read-only probe, in both a
   subscription session and a session with no quota (an API-key account, or before the first turn): `a2amx list`
   shows a QUOTA value after the first turn of a subscription session, the short line is visible in the
   footer, a user with a custom status line sees it unchanged, and the render probe captured on s5 for spec 3db
   (empty composer with the suggestion, a typed draft, a leading-space draft) is repeated WITH the injected status
   line and still classifies as in 3db: `claude_ready` true, the composer rule as expected. The consultant noted
   that every a2amx Claude session now has a custom status line, so 3db's evidence from a default install must
   be re-checked under it.
5. Scratch files stay under `target/scratch/3dc/` and are removed before the report.
