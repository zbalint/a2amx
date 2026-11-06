# Spec 3cv: Confinement docs around the default set, the recoverability rule, the enable switches and the limits

## 0. Status

LOCKED.

Scope (files that may be edited): `docs/confinement.md` only. Does not touch: every other file under `docs/`
(the architect updates `docs/backlog.md`, `docs/roadmap.md` and `CHANGELOG.md`), `docs/philosophy.md`, `src/`,
`tests/`, `README.md`, any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-confinement-docs-2026-11-06`. No code seam; the docs are checked by `tests/docs_examples.rs` and
`tests/team.rs`, and by hand against the code and a real session.

## 1. Why

Backlog F64 step 5, plus the docs parts of F73 and F71 that spec 3cr left open. The page grew spec by spec.
It lacks four things a new reader needs: (1) one statement of the default set that matches the code after
specs 3cd, 3cr and 3cu; (2) the rule behind which tool gets a plain grant, which a grant with the risk
spelled out, which is session-only and which stays closed; (3) how the three ways to turn confinement on
relate (owner decision 2026-11-06: keep `--confine`, `[confine] enabled` and global `auto`, and document the
precedence); (4) one place that says what confinement does not do. The rules this page serves are in
`docs/philosophy.md`; read it first, in particular rules 1, 3, 6, 7, 9 and 10.

Facts from the code (read 2026-11-06), which the text must follow:
- The three ways do not layer. `--confine` exists only on `a2amx new` (`src/cli.rs:62`); `team up` has no
  confine flag. `new` reads no team file. A team run reads one file, the local `a2amx.toml` if present, else
  the global `team.toml`, never both (`src/main.rs:973-1013`).
- Inside a team file, a session scalar beats the team scalar (`merge_confine`, `src/team.rs:708-711`).
  `auto = true` is valid only in the global file's `[confine]` table and needs `enabled = true`
  (`src/team.rs:672-680`); it applies only to sessions whose confinement ends up enabled
  (`src/team.rs:57-58`, `521-524`).
- `a2amx confine suggest` prints a grants line; it does not show the effective plan. `a2amx team up
  --dry-run` and, after start, `a2amx confine show` do.
- `docker` is a team-allowed grant (grants table, "Yes; team or session"), so the backlog's "container
  grants are session-only" does not hold for it. The session-only grants are exactly the rows whose last
  column says "No".

Rejected: restating the grants table (it is correct and tested by `tests/docs_examples.rs`
`grant_table_matches_the_code`; the class list goes outside the `### Grants` section, in bullets, never in
rows that start with "| `"); new behavior (this is docs only; a statement the code does not support is
reported, not changed); rewriting the recipes by ecosystem (not part of F64 step 5).

## 2. `docs/confinement.md`

Four changes. Every factual sentence is checked against the code or a probe by the developer, who lists in
the plan a table `claim | file:line or probe | result`; the tester re-checks a sample by hand. A claim that
cannot be checked is left out or marked in the text as untested (philosophy rule 10). Keep these headings
(other pages link to them): "Defaults at a glance", "Auto mode in one file", "The method: open what an
honest mistake can recover from", and the first line of the page's global team file TOML example
(`# global team file`, read by `tests/docs_examples.rs`).

2.1 Defaults at a glance (lines 5-14). Make the table complete and true after specs 3cd, 3cr and 3cu:
writable (working directory and its git directory, a private `/tmp`, the harness's own config root for
Claude and OMP); read-only (the rest, plus the protected launch inputs the row lists; `~/.cache` and the tool
directories under home such as `~/.cargo` are read-only, not opened and not hidden, and the `cache` grant
opens `~/.cache`; the working directory is writable, `target/` and `node_modules/` included, except the protected
launch inputs the row lists (this session's Git `hooks` and `config`, the team file); Amendments 3 and 4); hidden (credential
locations until a grant unhides them). Add one line on `--confine-default-hides=false`: it shows the
optional home hides and keeps the mandatory ones (verify with `src/cli.rs:70` help, the page's line 37 and
`MANDATORY_ROOTS` in `src/confine.rs`), linking to `#one-command`. Do not repeat the 3cr refusals here; link
to section 2.4 for them.

2.2 The recoverability rule. Replace the first paragraph of "The method" (line 269) with the rule from
backlog F64, worded as a rule about recovering from accidents, not about execution later (several grant rows
say that a build executes the config later; those sentences stay). Add the class list as bullets, outside
`### Grants` and with no row starting with "| `". The classes, defined by what the grant's row already says:
- plain grant: the row names no credential, token or account action (toolchain and cache directories);
- grant with the risk spelled out: the row says credentials, a token or an account action come with it;
  team or session as the table says (`docker` belongs here, with a note that it is allowed in a team file).
  A row's note that your own builds run installed programs later is a disclosure, not a class: it stays in
  the row and does not move a toolchain out of the plain class (Amendment 2, architect decision on the
  developer's question m_5715: `cargo`, `go`, `python`, `node`, `c-cpp`, `cache` are plain; `java`,
  `semgrep`, `sonar` are risk-spelled-out; `libvirt`, `snap`, `harness-config` are session-only);
- session-only: exactly the rows whose last column says "No" (for example `podman`, `containerd`, `crio`,
  `lxd`; read the table, do not copy this list);
- closed: no grant opens the system or the whole home; `/` is refused (philosophy rule 9); the whole home
  opens only with an explicit `write = ["~"]`, which is the operator's choice (line 271).
The developer places each grant by these definitions and lists in the plan every grant whose row makes the
placement a judgement call (the consultant expects `cargo`, `go`, `python`, `java`, `cache`, `semgrep`,
`sonar`, `node`, `c-cpp`, `libvirt`, `snap`, `harness-config`); the architect decides each of those, so the
developer reports the list before writing the class bullets and does not invent risk text.

2.3 Turning confinement on. In "How the options combine" (lines 136-149) add a subsection "Three ways to turn
it on", in this order, saying plainly that they are three scopes, not layers:
- `--confine` (with its `--confine-*` options) belongs to `a2amx new` for one session and reads no team
  file; `--confine-*` without `--confine` is an error;
- `[confine] enabled = true` belongs to a team file; a team run uses one file, the local `a2amx.toml` if it
  exists, else the global `team.toml`; they are never merged; a session's `enabled` beats the team's;
- global `auto = true` belongs to the global file's `[confine]` table, needs `enabled = true` there, and adds
  the detected grants plus `cache` only for sessions that end up confined.
Then the two cases that can disagree: team `enabled = true` with a session `enabled = false` (that session
is unconfined, and gets no auto grants; the developer proves the last part with `team up --dry-run`), and
a local file that exists, which hides the global file entirely. Say that `a2amx team up --dry-run` shows the
plan before anything starts and `a2amx confine show` shows a running session; do not say that `confine
suggest` shows the plan. Error cases (`auto` in a local file, `auto` with `enabled = false`) go in prose or
in a fenced block marked `text`, not in a `toml` block, because `tests/docs_examples.rs` parses every `toml`
block whose first line is not `# fragment`.

2.4 Limits. Add a section "What confinement does not do", before "What stays open" (line 275), as a short
list in the disclosure style of the page (say what the agent can do, not a worst case):
- it is a safety net, not a security sandbox, container or VM, and does not stop a rogue agent or a crafted
  program (link to `philosophy.md`); the page's other uses of "sandbox" name the bubblewrap boundary;
  real isolation is the user's job;
- the network is not restricted;
- beyond the listed hides (including the container sockets, the runtime directory and the desktop bus),
  whatever you can reach as the same user the agent can reach;
- `sudo` and root: `a2amx` refuses to run as uid 0 (philosophy rule 9). What `sudo` does inside a
  confined session is stated only if the developer's probe shows it, in the form "the maintainers ran X on
  DATE and saw Y" (line 574 style), never "sudo cannot work"; the probe never types a password and an
  agent is not involved; otherwise the line says it is untested;
- hooks, MCP server entries and settings under the harness config root, and build files in the project
  (`Makefile`, package scripts, `.envrc`, hooks of other repositories), run with your rights outside the
  sandbox, immediately for a running sibling or on your next run, so review them like any code an agent
  wrote (the docs part of F73; no behavior change);
- what the plan does with paths it picks (spec 3cr; Amendment 1): it refuses a working directory or git
  directory inside a default hide and a harness data link that leaves its harness folder, each with the
  `write` line that opens it; it ignores a `.git` pointer that does not lead to a git directory (no `HEAD`
  file or no `objects` directory): the session starts and the pointer is not followed. Listed as tested
  limits; move the sentence at line 273 here and keep its scope, naming the
  tests in `tests/confine.rs` that pin them in the developer's plan, not in the page; link here from 2.1 and
  from the troubleshooting row instead of repeating it.
"What stays open" keeps its numbered list; remove only a sentence the new section now holds.

## 3. Out of scope

Code, tests and the grants table text; `docs/philosophy.md`; the recipes by ecosystem; the troubleshooting
table beyond one link; `docs/cli.md`, `docs/teams.md` and `docs/getting-started.md` (they may link to the new
subsection later); step 6 (the tester's beginner walk-through, a separate assignment after this spec); the
escape-text wiring in `run_team_up` (F73 code part).

## 4. Acceptance

1. `cargo test --test docs_examples` and `cargo test --test team`, then `cargo test`, `cargo clippy
   --all-targets -- -D warnings`, `cargo fmt --check`; `TMPDIR` unset. Only `docs_examples` can fail from
   this edit; it passes on the old page (contract), so its job is to stay green.
2. `git status --short` lists only `docs/confinement.md`.
3. The developer's claims table is in its report: every sentence added or changed in sections 2.1 to 2.4 has a
   row with its source or probe; section 2.3 has explicit probe rows (isolated temp home, `target/debug`
   binary, `team up --dry-run`, scratch under `target/scratch`, deleted afterwards). The tester re-checks at
   least eight rows by hand, at least two per subsection, and reports each as confirmed or wrong.
4. Overselling check by the architect, on the added lines only: `git diff -U0 HEAD -- docs/confinement.md |
   rg -i '^\+.*(secure|safe|guarantee|isolat|sandbox|prevent|protect|cannot|never)'`, each hit judged against
   philosophy rules 1, 3 and 10.
5. The anchors other pages use still resolve: `rg -n "confinement.md#" docs README.md` lists only headings
   that exist in the edited page.
