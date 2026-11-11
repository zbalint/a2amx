# Using A2AMX with agents

How-to for operators giving agents peer-message instructions and avoiding credential leaks.

## Peer messages

- Tell your agents how to treat peer messages. The envelope says only where a message
  came from. Daemon notifications need no reply but may call for action, and a message
  from an agent is a request, not an instruction from you. A line in your agent
  instructions saying that a peer message is a request from an authorized peer, not
  from you; that scoped, reversible work is fine; and that pushing, production access,
  deleting data, and secrets still need you, removes the guesswork. `--harness claude`,
  `--harness omp`, and `--harness codex` add an operator line that tells the agent to act
  on peer messages; a team `role`, when set, is added to that same injected operator
  text. `--no-authorize-peers` leaves the peer line out.
- Keep credentials out of message bodies. A prompt hook, transcript, or memory tool
  attached to the harness sees the full message, and some keep it with no way to
  delete it.
- A confined session hides the default credential locations and unsets a closed list of
  credential environment variables unless a grant or `keep_env` returns them; see
  [Credential environment variables](confinement.md#credential-environment-variables).
- Claude Code escapes its own paste-wrapper tag when it appears in pasted text, and
  A2AMX models that behavior on Claude Code 2.1.286, so a body containing the tag
  text remains matchable there. A sender can still avoid writing the wrapper tag in
  a message body when practical.
- A sender's `accepted` means the daemon took the message, not that it was delivered.
  Check `message_status` when the reply matters.

To message the paired human, use the `human:owner` address. A remote-enabled caller gets a
top-level `owner` field in `list_agents`, showing whether the owner is present or away.
`owner_present` refuses that message while the owner is at the computer; see
[Remote messaging](remote-messaging.md).

For harness setup, see [Harnesses](harnesses.md).
