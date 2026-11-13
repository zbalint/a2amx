# Your first team

The samples below came from `a2amx v0.1.2-133-g91c802a`. Long paths are shortened to `/home/user/demo/proj` and `/home/user/.a2amx`.

## Install and start the daemon

See [Installation](install.md) for the install choices. From the repository:

```sh
scripts/install.sh
export PATH="$HOME/.a2amx/bin:$PATH"
a2amx daemon start
```

The daemon detaches by default, so you can use the terminal again immediately.

## Write the team file

From the project directory, copy the example and replace its contents with a small two-session
team:

```sh
cp a2amx.toml.example a2amx.toml
```

```toml
team = "demo"

[[session]]
name = "architect"
command = ["claude"]
cwd = "."
attach = true

[[session]]
name = "developer"
command = ["omp"]
cwd = "."
```

In this repository, `a2amx.toml` is gitignored. Session names are unique per daemon, so choose
names that do not collide with another running team.

## Let a2amx suggest grants

From the project directory, ask for the grants implied by its markers and Git remote:

```sh
a2amx confine suggest .
```

The output of `a2amx confine suggest .` is:

```text
# Suggested grants:
grants = ["cargo", "cache"]
```

Add confinement and that line to the team file. This is a complete local file; `auto = true`
belongs only in the global file described in [Confinement](confinement.md#auto-mode-in-one-file).

```toml
team = "demo"

[confine]
enabled = true
grants = ["cargo", "cache"]

[[session]]
name = "architect"
command = ["claude"]
cwd = "."
attach = true

[[session]]
name = "developer"
command = ["omp"]
cwd = "."
```

## Preview

Check the effective plan before starting the team:

```sh
a2amx team up --dry-run
```

The preview for this local file is shown below (abridged):

```text
would start demo-architect
  harness: claude
  cwd: /home/user/demo/proj/.
  team: demo, private, allow: -
  attach: true
  confinement: manual
  grants: cache, cargo
  writable: /home/user/demo/proj/.git
  protected: /home/user/demo/proj/.git/config
  hidden: /home/user/.a2amx
  hidden: /mnt/wslg
  ...
```

`confinement:` says whether the session is off, manually configured, or automatic. In auto mode,
a line such as `cargo: Cargo.toml found` gives the detection reason. `grants:` lists the effective
capabilities; `writable:` lists opened paths; `protected:` lists launch inputs kept read-only; and
`hidden:` lists paths overlaid or hidden (the omitted lines are more hidden paths).

## Start and look around

Start the team, inspect it, and read the recorded plan:

```sh
a2amx team up
a2amx team status
a2amx list
a2amx confine show demo-architect
```

For this inspection run, both `command` values were changed to `["sleep", "600"]`, so no
agent started. After detaching the attached session with Ctrl-B d, the captured output was:

For `a2amx list`:

```text
ID  NAME            TEAM            HARNESS  CONFINE  STATE    UPTIME  ACTIVITY  IN-STATE  ATTACHED  PENDING  HELD  QUOTA  SIZE
s1  demo-architect  demo (private)  generic  manual   running  24s     working   24s       no        0        -     -      80x20
s2  demo-developer  demo (private)  generic  manual   running  24s     working   24s       no        0        -     -      80x24
```

For `a2amx team status`:

```text
NAME TEAM STATE CONFINE NOTE
demo-architect demo (private) running manual s1
demo-developer demo (private) running manual s2
```

For `a2amx confine show demo-architect`:

```text
demo-architect s1 generic confine manual
GRANT  SOURCE
cache  set
cargo  set
```

Press Ctrl-B w for the session picker, then choose a session to attach.

## Send a message

An agent calls `send_message(to, subject, message)` through its native A2AMX tools; the recipient
gets an attributed peer message when its harness can receive it. See [Using with agents](using-with-agents.md)
for the peer-message instructions and credential hygiene.

## Stop

When you are finished, stop the sessions from the project directory:

```sh
a2amx team down
```

## If something fails

Run `a2amx team up --dry-run` without a daemon; the error names `a2amx daemon start` as the fix:

```text
a2amx: cannot reach the a2amx daemon: start it with `a2amx daemon start`
```

A local file that tries to enable auto mode is refused; use `a2amx confine suggest .` and paste
its grants line instead:

```text
a2amx: team file /home/user/demo/proj/a2amx.toml: [confine] auto is only read from the global team file; run a2amx confine suggest and paste the grants line into your team file
```
