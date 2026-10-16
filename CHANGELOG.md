# Changelog

All notable user-visible changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). The full history is in `git log`.

## [Unreleased]

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
