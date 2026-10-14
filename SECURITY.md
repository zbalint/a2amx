# Security policy

## Supported versions

A2AMX has no releases yet (version `0.0.0`). Only the latest commit on the default
branch is supported.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability. Use GitHub's private
vulnerability reporting: open the repository's **Security** tab and choose **Report a
vulnerability**.

Include what you found, how to reproduce it, and the commit you tested. Reports are read
on a best-effort basis; there is no response-time guarantee.

## What is in scope

A2AMX runs a daemon that owns terminal sessions and delivers messages into them, so these
areas matter most:

- authentication on the daemon's TCP connections (session tokens and the admin token);
- one session acting as another, or sending as a sender it does not hold the token for;
- message content that forges or breaks out of the message envelope that a recipient sees;
- delivery that destroys a human's unfinished composer draft;
- state files (tokens, address files, database) readable by other users, or tokens
  appearing in argv, logs, envelopes, or terminal output.

The design and its stated limits are in [docs/architecture.md](docs/architecture.md).
In particular, a process that holds a session token can act as that session; the
project does not claim to defend against that.

## How the project is built

Most of the code is written by AI coding agents working from reviewed specifications,
with separate developer, tester, and architect roles and a human owner. Its checks
(`cargo test`, `cargo clippy`, `cargo fmt`, SonarQube) are described in
[AGENTS.md](AGENTS.md) and run in CI.
