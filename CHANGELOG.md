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
