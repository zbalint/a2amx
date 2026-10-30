# Spec 3bb: confinement docs state the threat model, the residuals and the background-process behavior

## 0. Status

**LOCKED** (2026-10-30; pre-lock gate run against develop at `9780cbb`; consultant review `m_4546`, findings 1-9 adopted, 3ax section 1's stale 'process list' line left as a historical record). Architect-directed, backlog F38, context
`a2amx-confine-docs-2026-10-30`. Implementation goes to `a2amx-developer` only. No tester pass: the diff is prose, and
the architect checks every sentence against the claims table (section 6).

**Baseline:** develop at `9780cbb`; docs only, so no suite change is expected.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seam:** none; documentation. Acceptance is the `rg` and `git diff` checks of section 5.

**Scope.** May edit exactly two files: `docs/harnesses.md` (section "Confinement and harness boundaries", lines 50-70)
and `docs/cli.md` (section "Confinement", lines 49-74). Does not touch: every other file, including `docs/teams.md`,
`docs/architecture.md`, `docs/specs/*`, `CHANGELOG.md` (the architect adds the entry), `docs/backlog.md` and `src/`.
Leave the diff uncommitted.

## 1. Why

The confinement docs shipped with spec 3ax say what the flags do but not what the feature is for or where it stops.
The owner decided on 2026-10-30 that `--confine` is a safety net for a trusted but fallible agent that lowers the blast
radius of accidents (a stray `rm -rf`, `chmod`, `chown`), not isolation from a malicious agent or dependency; anyone
who needs that uses a container or a VM. The docs must not suggest otherwise. The tester's read-only audit and the
architect's escape probes of 2026-10-30 found no boundary crossed, and found these documentation gaps:

1. `docs/harnesses.md:56` says host and container environment variables are unset unless `--confine-allow-containers`;
   six variables are always unset and only the container ones depend on the switch.
2. Spec 3ax section 2.7 requires the residuals in `docs/harnesses.md` and `docs/cli.md`; neither has them.
3. `docs/harnesses.md` does not say that for OMP the `<state>/omp` directory stays visible read-only although the rest
   of the state directory is hidden.
4. `docs/cli.md:64-65` names the mandatory hides as "state, runtime, `/proc`, and descriptor restrictions" and omits
   `/run/WSL` and `/mnt/wslg`.
5. A descriptor the process holds before a nested `bwrap` stays writable after its path is masked; and
   `XDG_RUNTIME_DIR` stays set while the directory it names is hidden.
6. Backlog F38: a helper process that an MCP server or hook starts inside one session lives in that session's PID and
   mount namespaces. The owner saw a shared local daemon started this way: its own PID was small (such as 120) inside
   the sandbox and different on the host, so a pid recorded by it means nothing to the other sessions.

## 2. `docs/harnesses.md`

Edit the section "Confinement and harness boundaries". Keep the existing headings and the Ctrl-C paragraph unchanged.
Prefer short paragraphs and lists in the file's present style.

1. **Threat model first.** The section's first paragraph (before the "`--confine` is an opt-in bubblewrap boundary"
   sentence, which is reworded so it no longer says "boundary") states: confinement is a safety net for a trusted but
   fallible agent; it lowers the damage an honest mistake can do (such as `rm -rf`, `chmod`, `chown`); it is not
   isolation from a malicious agent or dependency (the session shares the kernel, the network and the operator's user
   account with the host: files are read and written as you; it does not share the process list, which is private);
   anyone who needs that uses a container or a VM. Do not write that the session shares the process list.
2. **Environment sentence.** Replace "unsets host/container environment variables unless `--confine-allow-containers`
   is selected" by a statement that matches `src/confine.rs`: `DBUS_SESSION_BUS_ADDRESS`, `SSH_AUTH_SOCK`,
   `GPG_AGENT_INFO`, `WSL_INTEROP`, `DISPLAY` and `WAYLAND_DISPLAY` are always unset; the container variables
   (`DOCKER_HOST` and the others the code lists) and `LIBVIRT_DEFAULT_URI` stay set only with
   `--confine-allow-containers`. Name the variables; do not write "and others" for the always-unset six.
3. **OMP exception.** One sentence: for an OMP session the `omp` directory inside the state directory stays visible,
   read-only; the rest of the state directory is hidden.
4. **A new subsection "What confinement does not cover"** (`###` under the existing `##`), placed after the Ctrl-C paragraph, as a list. Each item is
   one or two sentences. Required items (wording free, meaning fixed):
   - Network access, including services listening on localhost (an editor's MCP server, Jupyter, dev servers,
     databases, and on WSL2 the Windows host's forwarded ports): a confined process can ask such a service to act for
     it. Only a network namespace would stop that, and it would also stop the agent's own API access.
   - Abstract Unix sockets.
   - Messages to peer sessions through `send_message`.
   - Kernel keyrings.
   - CPU, memory outside `/tmp` and disk exhaustion: there are no cgroup limits.
   - The Windows drives under `/mnt` on WSL stay readable.
   - A sandbox started inside the session (a harness's own, for example) does not revoke a descriptor opened before it:
     masking a path leaves an already-open descriptor usable. a2amx's own child holds no descriptor above 2 (spec 3ax
     section 2.2), so this is about descriptors the agent or its harness opens itself.
   - `XDG_RUNTIME_DIR` stays set although the directory is hidden.
   - There is no syscall filter.
   - Everything the agent may write is writable, including files that run later outside the sandbox: `package.json`
     scripts, `Makefile`, `.pre-commit-config.yaml`, `lefthook.yml`, `.husky/`, `.vscode/tasks.json`, `.envrc`, and the
     repository's `.claude/settings*.json` and `.mcp.json`. Instructions planted in data the next unconfined run reads
     (a harness's per-project memory, todos and plans, `CLAUDE.md`, `AGENTS.md`, OMP's agent databases) are likewise out of
     reach. Confinement protects files and the listed code-carrying paths; it does not make the writable set
     trustworthy.
   - Advice, in bold or as its own sentence: **review the workspace diff before building, testing or committing it
     outside a sandbox.**
5. **A new subsection "Background processes"** (`###`), after the list. It states these facts, and no others about lifetime:
   - Everything a confined session starts (file tools, hooks, MCP servers, shell commands) runs in the session's own
     PID and mount namespaces. Inside, process ids are small numbers; the host shows other ids, so a process id
     recorded by a helper started in one session is wrong in every other session and in the host. A liveness check
     that tests a recorded process id from another session tests the wrong process; check a port or a socket instead.
   - Sessions still share the host network and any directory that is writable in each of them (for example one named
     with `--confine-write`), so a helper started in one session can be found by the others through a loopback port or
     a file in such a directory. (Observed by the owner: four sessions found one daemon this way.) Do not call this
     "the state directory": in these docs that is a2amx's own, which is hidden. `/tmp` is private to each session and
     `$XDG_RUNTIME_DIR` is hidden, so a helper whose socket, lock or pid file lives in either cannot be found by the
     other sessions.
   - When the session's command exits, or the sandbox is killed, every process in it ends, including a helper started
     by an MCP server or hook; other sessions that use that helper lose it. (Probed twice with bubblewrap 0.11.1;
     claims table.)
   - Advice: start a helper that several sessions share by hand, outside any sandbox, before the team, and let the
     sessions find it by port. Do not name any specific product; write "a shared helper daemon".

## 3. `docs/cli.md`

Edit the section "Confinement" only.

1. After the first paragraph add one paragraph: confinement is a safety net for a trusted but fallible agent, not a
   security boundary; it does not isolate a malicious agent or dependency (use a container or a VM); the residuals are in
   [Confinement and harness boundaries](harnesses.md#confinement-and-harness-boundaries) (check the anchor against the
   heading; a broken anchor is a defect).
2. In the `--confine-no-default-hides` sentence (`docs/cli.md:64-65`), replace "mandatory state, runtime, `/proc`, and
   descriptor restrictions" by: it never removes the mandatory hides, which are the state directory, the runtime
   directory, `/run/WSL` and `/mnt/wslg` (`hide_mandatory`, `src/confine.rs:800-813`, `MANDATORY_ROOTS` line 95). The
   private `/proc` and PID namespace are base options and the descriptor rule is a daemon property; if the sentence
   mentions them it says so separately and does not call them hides.
3. One sentence pointing to the "Background processes" subsection of `docs/harnesses.md` for helpers that an MCP server
   or hook starts.

Do not copy the residuals list into `docs/cli.md`; link to it.

## 4. Out of scope

`src/`, tests, `docs/teams.md` and `docs/architecture.md` (they say nothing wrong; `docs/teams.md` already links the
reader to the CLI doc), a changelog entry, the unexplained read-write mount entry for a docker path seen in the audit
(unreproduced and unreachable), whether a persistent helper daemon skips an idle shutdown (unverified, and about another
product), renames, rewording of text this spec does not name, and any statement about a product other than a2amx.

## 5. Acceptance

```sh
git diff --stat                     # only docs/harnesses.md and docs/cli.md
git diff --check
rg -n 'a2amx new --confine' docs/cli.md                       # still present
rg -n -i 'ctrscm|saltmdb' docs/harnesses.md docs/cli.md       # must print nothing (public repo rule)
rg -n 'host/container' docs/                                  # must print nothing (the old env sentence is gone)
for v in DBUS_SESSION_BUS_ADDRESS SSH_AUTH_SOCK GPG_AGENT_INFO WSL_INTEROP DISPLAY WAYLAND_DISPLAY; do rg -q -w "$v" docs/harnesses.md || echo "missing $v"; done   # must print nothing
for v in /run/WSL /mnt/wslg; do rg -q -F "$v" docs/cli.md || echo "missing $v"; done                                                                      # must print nothing
for v in 'What confinement does not cover' 'Background processes'; do rg -q -F "$v" docs/harnesses.md || echo "missing $v"; done                          # must print nothing
rg -c 'security boundary' docs/harnesses.md docs/cli.md       # at least 1 in each file
rg -n -i 'boundar' docs/harnesses.md docs/cli.md              # allowed hits only: the heading, "not a security boundary" sentences, link text
rg -n '^## Confinement and harness boundaries' docs/harnesses.md   # the cli.md link anchor still exists
rg -n '^### (What confinement does not cover|Background processes)' docs/harnesses.md   # both at heading level 3
```

- The architect reads the final prose against the claims table and the code, sentence by sentence, before commit.
  Judge: the architect at final review. A sentence the table does not support is removed.
- Every link added to `docs/cli.md` resolves: the anchors `#confinement-and-harness-boundaries` and, if item 3 of
  section 3 is a link, `#background-processes`.
- `env -u A2AMX_BIN -u NO_COLOR cargo test` still passes (a docs diff cannot break it; the run guards against a stray
  edit), plus `cargo fmt --check`. Known flaky tests (backlog I9 and F37): rerun once, record as pre-existing.

## 6. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| Six variables are always unset | `ALWAYS_UNSET`, `src/confine.rs:29-36` | read |
| Container variables and `LIBVIRT_DEFAULT_URI` are unset unless `allow_containers` | `CONTAINER_UNSET`, `CONTAINER_UNSET_EXTRA`, `env_unsets` (`src/confine.rs:37-45`, `:1028-1048`) | read |
| For OMP, `<state>/omp` is mounted read-only when it exists | `src/confine.rs:1003-1007` | read; the tester saw it in `mountinfo` |
| Mandatory hides include `/run/WSL` and `/mnt/wslg` | `MANDATORY_ROOTS`, `src/confine.rs:95`; guard at `:215` | read |
| The bwrap launch uses `--die-with-parent`, `--unshare-ipc`, `--unshare-pid` | `src/confine.rs:1059-1062`, `:1217-1220` | read |
| A descriptor opened before a nested `bwrap` stays writable after its path is masked | spec 3ax claim C25; tester run 2026-10-30 (fd 9 write rc 0) | probed twice |
| Residual list content (network, abstract sockets, peers, keyrings, cgroups, `/mnt`, writable set) | spec 3ax section 2.7 | read |
| The confined child holds no descriptor above 2 | spec 3ax section 2.2 and its real-bwrap test | read |
| `XDG_RUNTIME_DIR` stays set while hidden; no syscall filter (`Seccomp: 0`) | architect probe and tester run 2026-10-30 | probed |
| A process started inside a session has a small pid inside and another on the host | owner's `ps` and `NSpid` output, 2026-10-30 | owner's output, not re-run |
| Four sessions found one shared daemon through loopback and a writable state directory | owner's daemon log and `ps`, 2026-10-30 | owner's output, not re-run |
| Killing `bwrap` with `--die-with-parent --unshare-pid` ends background `sleep` processes started inside | architect probe 2026-10-30, bubblewrap 0.11.1: six matching processes before `kill -9` of the `bwrap` pid, one (the probe's own command line) after | probed |
| A background process also ends when the sandbox's command exits normally | consultant probe and architect rerun 2026-10-30, bubblewrap 0.11.1: `bwrap --die-with-parent --unshare-user --unshare-ipc --unshare-pid --ro-bind / / --dev /dev --proc /proc --tmpfs /tmp -- sh -c 'sleep 4243 & exit 0'` returned 0 and no `sleep 4243` remained | probed twice |
| The session's PID list is private (`--unshare-pid`, fresh `--proc /proc`); it is not shared with the host | `src/confine.rs:1062`, `:1068-1069`; spec 3ax section 2.2 | read |
| The mandatory hides are exactly the runtime and state directories plus `MANDATORY_ROOTS` | `hide_mandatory`, `src/confine.rs:800-813` | read |
| `/tmp` is private per session; `$XDG_RUNTIME_DIR` is hidden | spec 3ax section 2.3 item 1; `src/confine.rs:800-802` | read |
| Not claimed: that a sandbox is killed when the session's `a2amx` process is killed | not reproduced | the docs say "killing the sandbox" only |

## 7. Pre-lock record

Gate steps 1-9 run (claims table above); step 7 re-run after the consultant's edits (the process-list wording appears
once in section 2 item 1 and once in the claims table, both saying it is private). Steps 10-11 do not apply; step 12:
the acceptance commands are `rg`, `git` and the repo's own baseline commands, and the `rg` loops were run on the
current tree for the "must print nothing" cases; step 13: no trust boundary is crossed (prose only).
