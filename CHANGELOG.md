# Changelog

All notable user-visible changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). There are no releases yet;
the full history is in `git log`.

## [Unreleased]

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
