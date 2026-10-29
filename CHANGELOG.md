# Changelog

All notable user-visible changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The full history is in `git log`.

## [Unreleased]

### Added

- Team line: a session in a team shows a row at the top of the attach screen, in the status bar's
  style, listing the team's members with an activity dot (grey for exited members). A click on a
  member, or the prefix then `1`-`9`, `n` or `p`, switches to it; a click on the team label opens the
  picker; the prefix then `s` hides it together with the status bar. It needs a terminal of at least
  five rows. Additive protocol fields: the attach request's `top_rows`, the `AttachedTop` reply and
  the status frame's `members`; an older daemon simply shows no line (`docs/cli.md`).
- A dim rule under the team line, like the one above the status bar, on terminals of at least six rows
  (at five rows the line shows without it). A click on the rule does nothing.

### Changed

- **Breaking:** a2amx and its daemon refuse to run when the effective uid is 0 (as root, under `sudo`, or as a
  root user in a container). The command prints `a2amx: refusing to run as root (effective uid 0): run a2amx
  as an ordinary user` and exits 1. There is no override. `--help` and `--version` still work. Start it as an
  ordinary user, and use a non-root user in containers (`docs/install.md`, `SECURITY.md`).
- The activity dot follows one colour table, in the status bar and on the team line: idle green, working
  blue (it was yellow), starting yellow (a Claude, Codex or OMP session that has not been ready yet),
  resetting orange, held red (a team member waiting on a delivery hold or for input; the attached
  session's own hold still shows as the red `HELD` alert instead of a dot), and exited grey. The status bar's
  words are idle, working, starting and resetting. Additive protocol fields: `held` and `starting` on team
  members and `starting` on the status; an older daemon sends neither, so a newer client draws members by
  the old rules, and an older client ignores them (`docs/architecture.md`).

### Fixed

- A mouse drag that starts in the session and ends on the team line is forwarded to the session at the
  grid's first row instead of a row lower.

## [0.1.2] - 2026-10-26

### Added

- Activity notices: a session started with `activity` (team file `activity = { working_after =
  "2m", idle_after = "30s" }`, or `a2amx new --activity-working-after 2m --activity-idle-after
  30s`) is told when a watched peer starts or stops working. Each direction is debounced
  separately, the first settled state is silent, and the notice is one ordinary daemon message
  per watcher at a time. `team up` rejects teams whose watchers would notify each other in a
  loop. New optional field on the `new` request and the team file; older files and clients keep
  working (`docs/teams.md`, `docs/cli.md`).

### Fixed

- The quota shown for an OMP session now follows the provider of the model it is using, instead
  of the smallest value across all providers on the account. The OMP extension reports the
  current provider on each state frame (additive field `provider`), so a switch between
  providers changes the quota shown on the next list. A session whose provider is unknown shows
  `-` instead of another provider's quota (`docs/cli.md`).
- Updated `tokio` to 1.53.3 for RUSTSEC-2026-0386 (the safe `AsyncFd` constructors). A2AMX does
  not call `AsyncFd` directly.

## [0.1.1] - 2026-10-16

### Added

- Signed releases: pushing a version tag on `master` builds a Linux x86_64 archive in CI, signs it
  and its checksums with Sigstore, attests its build provenance and publishes a GitHub release
  with notes from this changelog (`docs/releasing.md`, `docs/install.md`).

### Changed

- The README is now a short landing page with an example and a quick start; the command,
  team-file, harness and installation reference moved to `docs/` (`cli.md`, `teams.md`,
  `harnesses.md`, `install.md`, `using-with-agents.md`, `roadmap.md`).

## [0.1.0] - 2026-10-15

### Added

- A local daemon that owns PTY sessions and keeps their screen state while detached, with
  attach, detach, switch, and scroll from a client that redraws from the terminal model.
- Durable SQLite messages, per-session tokens, and a stdio MCP server with `list_agents`,
  `send_message`, `message_status`, and `reset_session`.
- Message delivery into a session's composer that protects an unfinished human draft, with
  submission receipts.
- Native delivery channels for Claude Code (a channel), OMP (an extension), and Codex (a
  private app-server).
- Session names, a status line with quota, a session picker with team sections, and
  `a2amx team` to start and stop a team of sessions.

### Fixed

- An idle OMP session that holds queued next-turn messages of its own (for example after a failed `todo` call) no longer blocks message delivery or `reset_session`.
