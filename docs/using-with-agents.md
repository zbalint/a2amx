# Using A2AMX with agents

How-to for operators giving agents peer-message instructions and avoiding credential leaks.

## Peer messages

- Tell your agents how to treat peer messages. The envelope says only where a message
  came from. A line in your agent instructions saying that a peer message is a request
  from an authorized peer, not from you; that scoped, reversible work is fine; and that
  pushing, production access, deleting data, and secrets still need you, removes the
  guesswork. `--harness claude` and `--harness omp` add an operator line that tells
  the agent to act on peer messages; a team `role`, when set, is added to that same
  injected operator text. `--no-authorize-peers` leaves the peer line out.
- Keep credentials out of message bodies. A prompt hook, transcript, or memory tool
  attached to the harness sees the full message, and some keep it with no way to
  delete it.
- Claude Code escapes its own paste-wrapper tag when it appears in pasted text, and
  A2AMX models that behavior on Claude Code 2.1.286, so a body containing the tag
  text remains matchable there. A sender can still avoid writing the wrapper tag in
  a message body when practical.
- A sender's `accepted` means the daemon took the message, not that it was delivered.
  Check `message_status` when the reply matters.

For harness setup, see [Harnesses](harnesses.md).
