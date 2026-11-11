# Security policy

## Supported versions

Only the latest release and the latest commit on the default branch are supported.

## Reporting a vulnerability

Please do not open a public issue for a vulnerability. Use GitHub's private
vulnerability reporting: open the repository's **Security** tab and choose **Report a
vulnerability**.

Include what you found, how to reproduce it, and the commit you tested. Reports are read
on a best-effort basis; there is no response-time guarantee.

## What is in scope

A2AMX runs a daemon that owns terminal sessions, delivers messages into them, and can relay
messages through a confined Discord gateway, so these areas matter most:

- authentication on the daemon's TCP connections (session tokens, the admin token, and the
  gateway token);
- one session acting as another, or sending as a sender it does not hold the token for;
- message content that forges or breaks out of the message envelope that a recipient sees;
- delivery that destroys a human's unfinished composer draft;
- state files (tokens, address files, database) readable by other users, or tokens appearing in
  argv, logs, envelopes, or terminal output;
- the gateway's Discord bot token and file, pairing and author checks on Discord messages, lease
  codes, the `<state>/link/` directory exposed to the gateway sandbox, and the gateway sandbox
  boundary ([permissions](docs/permissions.md), [remote messaging](docs/remote-messaging.md));
- a2amx and its daemon refusing to run as root, under `sudo`, or as a root user in a container;
  there is no override because state-file permissions and agent commands assume an ordinary user.

The design and its stated limits are in [docs/architecture.md](docs/architecture.md).
In particular, a process that holds a session token can act as that session; the
project does not claim to defend against that.

## Not handled as a vulnerability

Confinement is a safety net against honest mistakes, not a sandbox or a security boundary
([project philosophy](docs/philosophy.md)). An escape from it is welcome as a normal bug report
(a public issue is fine) and is fixed on a best-effort basis. An escape that also reaches an item
above—tokens, another user's state files, or acting as another session—is still reported privately.

## How the project is built

All code, tests and documentation are written by AI coding agents (apart from a
GitHub-generated initial commit and the OpenSSF Scorecard workflow template) working from reviewed
specifications, with separate architect, developer, tester and consultant roles and a human
owner who sets direction and approves releases. There has been no independent security audit.
How a change is specified, checked and gated is in
[docs/how-this-is-built.md](docs/how-this-is-built.md); the checks themselves (`cargo test`,
`cargo clippy`, `cargo fmt`, SonarQube) run in CI.
