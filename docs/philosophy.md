# Philosophy

This page says what A2AMX is for, which rules it follows when a design choice comes up, and what
to expect from it and what not to. Read it before changing a feature that touches safety,
defaults, or what a user is allowed to do. It is the reference that specs, reviews and
`AGENTS.md` point to.

## What A2AMX is

A2AMX helps agents coordinate their work through each harness's native messages,
roles, and control commands such as `/clear`. It also has built-in confinement that can be
enabled to prevent honest agent mistakes like a recursive `rm`, `chmod` or `chown`. Its
confinement is not a sandbox. It is not designed to stop a rogue agent from harming the system
or to contain malicious programs.

## The rules

### 1. A safety net, not a sandbox

Confinement protects a system from accidental damage by a trusted but fallible
agent. It is not a sandbox, a container or a VM, and the docs and CLI text must not claim to be.
Elsewhere in the docs, "sandbox" names the bubblewrap process boundary and makes no security
claim. Real isolation is the user's job (see rule 8).

Where it is cheap and does not break the daily flow, confinement also closes ways a faithful
agent could get out when it cannot reach its goal. This is best effort, not a guarantee, and
not at any cost. A grant the user adds or a confirmation they give is their decision. As with a
real sandbox, the result is as secure as the user configures it: Docker can be set up securely,
or run as root with none of that.

### 2. Safe by default, open by choice

Defaults protect the system from accidents: the project directory is writable, the rest of the
filesystem is read-only, and credential locations are hidden until a grant opens them (see
[Defaults at a glance](confinement.md#defaults-at-a-glance)). A user may open up as much as they
want. The project follows the Linux spirit here: your machine, your choice. If the user wants to
shoot their own foot, that is up to them, and the tool does not scold. For example, the daemon
binds loopback by default, and an operator who chooses another address gets no warning or
refusal.

### 3. Disclose, do not scold

Every grant states plainly what it exposes: what the agent can do, change or delete,
and which credentials come with it. The text should say what the grant adds, not a worst-case
claim that is false for some setups. A user with a hardened Docker daemon, for example, should
not be told that Docker access is root on the host. Some container grant texts are stricter than
this today and will be brought in line.

### 4. Trust what the user chose, confirm what the tool chose

A grant the user names (a flag or a line in the team file) needs no extra prompt:
choosing it after reading its description is the confirmation. That includes a grants line
printed by `confine suggest` that the user copies into a team file, because pasting it is the
user's choice. A grant the tool picks for them, which only global auto mode does, is shown with
its exposure and needs a yes, with No as the default. The yes is remembered by grant name, and
`--yes` accepts for one run without remembering.

### 5. Grants are all or nothing

A grant opens the whole thing it names, read-write, with no per-grant options.
Precision comes from the raw `write`, `unhide` and `hide` lines. If a granted directory
contains a secret, the secret comes with the grant, and it is read-write too.

### 6. Compare risk with an unconfined session

When judging the risk of a grant or a default, compare it with the same agent running
with no confinement, never with a locked-down sandbox. A grant that adds little over the
unconfined baseline is fine to offer. For example, with the `docker` grant a mistyped
`rm -rf ~/` still fails, and destroying host files through Docker takes a deliberate mount.

### 7. A beginner's first run must work

If confinement breaks the daily flow, the user turns it off and ends up with no
protection at all. So detection should cover what a freshly cloned project needs. The goal,
not yet built everywhere, is that a blocked operation names the grant or flag that fixes it. A
suggestion that leaves out the one thing a project is for is worse than none.

### 8. The user's responsibility

Passphrases on keys, running agents in a VM or container when real isolation is
needed, and firewalls are the user's job. Confinement does not replace them and does not
pretend to.

### 9. Where A2AMX says no

The project trusts the operator, with a narrow list of refusals. Each one is a case
where A2AMX cannot do what it promises, or would destroy a person's work:

- It refuses to run with effective uid 0 (root, `sudo`, or root in a container), for every
  command, confined or not. A2AMX cannot work in the designed way as root. Run it as an ordinary
  user; to work as root, run the harness without A2AMX.
- It refuses a writable `/`, which would make confinement identical to none while claiming to
  confine.
- It never sends input that destroys human work, such as Ctrl-C to clear a composer draft.

Everything else the user may do gets disclosure and trust. Confinement also refuses a few
configurations that would silently break its own protection, such as an implicit writable home
directory, a symlinked grant path, or an auto run with no terminal to answer the prompt.

### 10. Claims go no further than what is tested

Docs and CLI text do not oversell. If a behavior is untested, the page says so.

## Messaging

The same stance applies to coordination:

- A sender is derived from its authenticated token, never from a field the caller supplies.
- A peer message is a request from an authorized peer, not from the user. A2AMX states only the
  origin; how an agent treats the request is the operator's setup, which A2AMX cannot decide or
  enforce.
- Delivery evidence is reported as it is. Writing bytes to a PTY does not prove that a harness
  submitted a prompt, and a hook receipt does not prove that a model processed it. Protecting
  unfinished human input is a central constraint.

## What to expect, and what not

Expect accident protection on the defaults, named grants, a consent prompt for
tool-picked grants, and plain disclosure of what each grant exposes.

Do not expect protection from a rogue or malicious agent, isolation from the kernel or from
other programs and sockets you run as the same user, or any guarantee beyond the accident net
once a powerful grant is added.

## Worked examples

Both grants below carry risk. With a careful user, both can be enabled safely.

### The `ssh` grant

The grant gives the agent the contents of `~/.ssh`. A careful user limits what is there:

- If the agents run in a VM, a container or on a dedicated computer, `~/.ssh` can hold only the
  keys the agents need, for example one for git, and ones for systems the user wants the
  agents to reach.
- Only those keys are left without a passphrase. A key protected by a passphrase cannot be used
  by the agent even with the grant present: the session gets no `SSH_AUTH_SOCK`, so no running
  ssh agent is reachable (see [Confinement](confinement.md)). The same limit means a user who
  wants the agent to use a key has to leave that key without a passphrase. The grant is
  read-write, so the agent can still read, copy, overwrite or delete a key it cannot use.
- Keep no more keys than needed. A machine that only pushes to a git host needs one key, and
  that is all `~/.ssh` should hold there.

### The `docker` grant

The grant lets the agent run any docker command as the user. A careful user hardens the daemon
first:

- `userns-remap` in `daemon.json`, so container root maps to an unprivileged user on the host.
- A sandboxed runtime such as `runsc` (gVisor).

With these, the risk of the grant is much smaller than on a default install.

### Other examples

- **The loopback bind.** The daemon listens on `127.0.0.1` by default. An operator who
  configures another address gets no warning and no refusal; the address is their choice.
- **The root refusal.** `a2amx` exits with an error when its effective uid is 0, because it
  cannot work in the designed way as root. A user who wants to work as root runs the harness
  without a2amx.
