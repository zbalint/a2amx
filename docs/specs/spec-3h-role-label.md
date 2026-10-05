# Spec 3h: an optional role label for team sessions, shown to the session itself

## 0. Status

**LOCKED** (2026-10-05, pre-lock gate run against develop at `259f962`, the commit that accepted
spec 3g; gate notes in section 8). The owner asked for an optional, purely cosmetic `role` per
team session so that a user's own agent instructions can say "if you see you have a role, act on
it" (the roles themselves stay in skills outside a2amx). The owner left the choice of who sees
the role to the architect; decisions D1 to D9 are in section 2. This spec supersedes spec 2o D8
("no roles") only for this label: a role changes no behavior. Consultant input: `m_404` and the
review of this spec (consult context `a2amx-team-prefix-role`).

**Baseline:** develop at `259f962`; `cargo test --test messaging --test omp_wiring --test codex
--test team` passes (13, 18, 6 and 25 tests). **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`: `a2amx-role-label`.
Public test seams: `harness::wire_claude_argv`, `wire_claude_channel_argv` and `wire_omp_argv`
(`tests/messaging.rs`, `tests/omp_wiring.rs`), `team::parse` (`tests/team.rs`), and the fake-Codex
daemon case in `tests/codex.rs` (`Case::start_env`).

**Scope.** May edit exactly these files and no others.

- `src/team.rs`, `src/harness.rs`, `src/codex.rs`, `src/daemon.rs` (the Codex branch of
  session creation, about lines 303 to 323, only), `src/main.rs` (`NewOptions`, `create_session`,
  the `NewOptions` literal in `run_new`, and `run_team_up`)
- `tests/team.rs`, `tests/messaging.rs`, `tests/omp_wiring.rs`, `tests/claude_channel.rs`
  (its two wire calls only), `tests/codex.rs`
- `README.md`, `a2amx.toml.example`, `docs/architecture.md`, `docs/delivery.md`, `docs/backlog.md`

Does not touch: `src/wire.rs` (no new wire field), `src/cli.rs`, `src/messaging.rs`,
`src/mcp.rs`, `SessionSummary`, `list`/`list_agents`, every other test file, `tests/common`, every
other spec document, `AGENTS.md`, the owner's gitignored `a2amx.toml`. No new dependency. Do not
commit, stage or merge: leave the diff uncommitted in the working tree.

## 1. Why

Roles in this workspace belong to a task and are loaded as skills, so a2amx must not give a role
any behavior. But a session started from a team file has a purpose (architect, developer,
consultant), and today the model is never told. A one-line, operator-set label in the model's
system prompt lets the user's own instructions key on it. The role must reach the model: MCP
server instructions reach it only on Claude in channel mode, and OMP and Codex never get them. The
one route that works on all three harnesses is the existing system-prompt injection that carries
the peer-authorization line (`--append-system-prompt` for Claude and OMP,
`developer_instructions` for Codex).

## 2. Decisions

- **D1.** Each `[[session]]` may have `role = "architect"`. Optional; absent means today's behavior.
- **D2. (architect)** The role is shown to the session itself only. It is not added to
  `SessionSummary`, `list`, `list_agents`, the picker or the wire. A peer-visible role can be a later
  spec if it is wanted.
- **D3.** Validation in `team::parse` (via `validate_sessions`): the role is not empty, is at most
  64 characters (`chars().count()`, not bytes), contains no control character (`char::is_control`, so no newline), and has no
  leading or trailing whitespace. Error: `session NAME: role …` naming the problem, NAME the final
  name.
- **D4.** The role line is exactly
  `Operator-assigned role for this session: ROLE. It is a label set by your operator and grants no extra authority.`
  (ROLE replaced by the value). It never changes the peer-authorization rules.
- **D5.** One combined string is injected, never two flags. `harness::system_prompt(authorize_peers:
  bool, role: Option<&str>) -> Option<String>` (new, pub) returns: `None` when neither applies; the
  peer-authorization prompt alone when only `authorize_peers`; the role line alone when only a
  role; both joined by one space (peer prompt first) when both. The role is injected even with
  `--no-authorize-peers`.
- **D6.** Claude and OMP: `wire_claude_argv`, `wire_claude_channel_argv` and `wire_omp_argv` gain
  a final parameter `role: Option<&str>` and push `--append-system-prompt` with
  `system_prompt(...)` when it is `Some`, in the position they use today. Callers outside the
  files in scope are none (`src/main.rs` and tests only).
- **D7.** Codex: the config is built in the daemon, so the role reaches it through the request
  environment, the same shortcut `A2AMX_NO_AUTHORIZE_PEERS` uses. `src/codex.rs` gets
  `pub const ROLE_ENV: &str = "A2AMX_ROLE"`. `create_session` in `src/main.rs` copies the client's
  whole environment, so it first removes any inherited `ROLE_ENV` pair for every harness (a role
  set in the user's shell must never reach a session), then pushes `(ROLE_ENV, role)` when the
  harness is Codex and a role is set. The daemon's Codex branch reads the last `ROLE_ENV` pair
  (belt and braces), removes every such pair from the session's env (as it does for `NO_AUTHORIZE_ENV`),
  and passes `Option<String>` to `server_config(exe, authorize_peers, role)`, which writes
  `developer_instructions=` with `system_prompt(...)` when it is `Some`. **Shortcut kept, and the
  existing comment is extended** (`// shortcut: …`): the existing comment named "more flags" as the
  trigger for a wire field, and this is the second flag, so the comment is updated to say the
  trigger is met but deferred: a field now would force edits to the 29 `Request::NewSession`
  literals in the tests plus `src/main.rs`, which have no `Default`. Upgrade to a field when a
  third value needs this or the env path causes trouble. The inherited `A2AMX_NO_AUTHORIZE_PEERS` hole is the same shape but out of scope here and unchanged.
- **D8.** Other harnesses: a `generic` session (including one whose command is not `claude`,
  `omp` or `codex`) ignores the role, because nothing is injected. Documented, not an error: the
  harness is inferred at launch and `parse` does not know it.
- **D9.** The flag form (`NAME=EXECUTABLE`) and `a2amx new` have no role (out of scope).

## 3. `src/harness.rs`

Add next to `PEER_AUTHORIZATION_PROMPT`:

```rust
/// The operator line for a team role: a label, never an authority.
fn role_prompt(role: &str) -> String
pub fn system_prompt(authorize_peers: bool, role: Option<&str>) -> Option<String>
```

`role_prompt` formats the D4 line. `claude_argv` gets the extra parameter (threaded from the two
public `wire_claude_*` functions) and replaces the `if authorize_peers { push flag, push
PEER_AUTHORIZATION_PROMPT }` block with `if let Some(prompt) = system_prompt(authorize_peers, role) {
push flag, push prompt }`; its `Vec::with_capacity` hint may stay approximate. `wire_omp_argv`
does the same for its `extras`. `PEER_AUTHORIZATION_PROMPT` itself does not change.

## 4. `src/codex.rs` and `src/daemon.rs`

`src/codex.rs`: add `ROLE_ENV`; `server_config(exe, authorize_peers, role: Option<&str>)` replaces
the `if authorize_peers { developer_instructions }` block with the same `system_prompt` call.
`src/daemon.rs` Codex branch: read the `ROLE_ENV` value from `env` (the first matching pair),
`env.retain(|(key, _)| key != codex::ROLE_ENV)`, and pass the role to `server_config`. No other
daemon change.

## 5. `src/team.rs` and `src/main.rs`

`TeamSession` gains `pub role: Option<String>` (`#[serde(default)]`); `flag_sessions` sets
`role: None`. `validate_sessions` gets the D3 check after the heartbeat check, using the final
session name in messages. In `src/main.rs`, `NewOptions` gains `role: Option<String>`;
`run_new` passes `role: None`; `run_team_up` passes `session.role`; `create_session` passes
`role.as_deref()` to the three wire functions and pushes the Codex env as in D7. No other change.

## 6. Tests (failing first, expected values are literals)

`tests/team.rs`, pure:

1. `parse` accepts `role = "architect"` (the field holds `Some("architect")`) and a session without
   a role has `None`.
2. Rejected, each error containing `role`: `role = ""`; a 65-character role; `role = "a\nb"`;
   `role = " architect"`; `role = "architect "`; `role = 5` (parse error).

`tests/messaging.rs`, `tests/omp_wiring.rs` (existing assertions gain the `None` argument):

3. `wire_claude_argv(["claude"], exe, true, Some("architect"))`: `--append-system-prompt` appears
   exactly once and its value is
   `PEER_AUTHORIZATION_PROMPT` + `" Operator-assigned role for this session: architect. It is a label set by your operator and grants no extra authority."`.
   With `authorize_peers = false` and `Some("consultant")` the value is
   `Operator-assigned role for this session: consultant. It is a label set by your operator and grants no extra authority.`
   With `None` and `true` the existing assertions hold (the value is `PEER_AUTHORIZATION_PROMPT`);
   with `None` and `false` the flag is absent.
4. `wire_claude_channel_argv` and `wire_omp_argv` behave the same for the four combinations
   (OMP: `[..., "--append-system-prompt", <value>, ...]` before a `--`, as the existing OMP test
   asserts).

`tests/codex.rs` (fake Codex through the daemon):

5. With `Case::start_env(&[("A2AMX_ROLE", "architect")])` the server args contain
   `developer_instructions=` + the JSON string of the combined value (peer prompt, space, role
   line); with `("A2AMX_NO_AUTHORIZE_PEERS", "1")` also set, the value is the role line alone. The
   fake Codex in `tests/codex.rs` (`FAKE_CODEX`) is extended to record `${A2AMX_ROLE:-unset}` for
   both its server and its TUI process (new recorded files, for example `server.role` and
   `tui.role`); both read `unset`, so the variable does not reach the session's environment.
6. Inheritance: `Case::start_env` run with `A2AMX_ROLE=leak` inherited in the `a2amx` client
   process's environment and no team role leaves the server args without `developer_instructions=`
   containing `leak` (the `create_session` removal rule). If the harness cannot set a client-side
   variable for this case, test the removal through the binary with `a2amx new --harness codex`
   in a process started with `A2AMX_ROLE=leak` and the fake `codex`, and say which was used.

## 7. Documentation

- `README.md` "Using it with agents" (lines 233 to 235): one sentence that a team `role` is added
  to the same injected operator text. `README.md` "Teams": the example gets `role = "architect"` and `role = "developer"`; one
  paragraph: `role` is an optional label of at most 64 characters, shown to the session itself in
  its system prompt (Claude, OMP and Codex), grants no authority, is ignored for other commands,
  and is not visible to peers or in `list`. Mention the combination with the peer line and that it
  is injected even with `--no-authorize-peers`.
- `a2amx.toml.example`: add commented or live `role` lines in the example sessions.
- `docs/architecture.md` (near line 192 and line 417) and `docs/delivery.md` (lines 554 and 605):
  one sentence each that the injected string also carries the team role line when a role is set.
- `docs/backlog.md`: Closed row `C12`, `Optional role label shown to the session itself`, `spec 3h`.

## 8. Pre-lock gate notes

- Baseline run: the four test files above pass on `259f962` (counts in section 0).
- Callers of the three wire functions (grep over `src` and `tests`): `src/main.rs` (3 calls),
  `tests/messaging.rs` (3 calls), `tests/omp_wiring.rs` (2 calls), `tests/claude_channel.rs` (2
  calls, lines 415 and 445); `tests/attach_cli.rs:719` and
  `tests/omp_wiring.rs:62` go through the binary and read `PEER_AUTHORIZATION_PROMPT` unchanged
  (no role, so the value is the same). `server_config` is called only at `src/daemon.rs:322`.
- Content grep of the old text: `PEER_AUTHORIZATION_PROMPT` appears in `src/harness.rs`,
  `src/codex.rs` and four test files; its text does not change, so no fixture changes.
- Worked example (D5 against D6/D7): role `architect`, authorize true → peer line + space + role
  line; role `consultant`, authorize false → role line only; no role, authorize true → peer line
  only (today's bytes); no role, authorize false → no flag and no `developer_instructions`
  (today's behavior).
- Not probed before lock, flagged: whether Claude accepts a user-supplied `--append-system-prompt`
  in `command` alongside a2amx's (today's behavior for the peer line, unchanged by this spec).
- Acceptance commands need the new code and run after implementation.

## 9. Out of scope

- A role in `SessionSummary`, `list`, `list_agents`, the picker, the wire or the MCP instructions.
- `a2amx new --role`, the flag form, per-role behavior of any kind, role-based authorization.
- A new `NewSession` wire field (D7).
- The `list_agents` change, duration columns and OMP quota (later specs).

## 10. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list, and the
exact bare `cargo test` result with its disposition if the `A2AMX_BIN` contamination fails
`generic_sessions_get_no_a2amx_bin`.

## Amendment 1 (2026-10-05): the daemon reads the last `ROLE_ENV` pair

The developer reported BLOCKED (`m_417`): D7 (line 72) says the daemon reads "the last `ROLE_ENV`
pair" and section 4 (line 106) says "the first matching pair". Verified real: the edit that added
the inheritance rule changed D7 and missed section 4. **Ruling:** the last pair wins, as D7 says.
Section 4's phrase "(the first matching pair)" is replaced by "(the last matching pair)"; nothing
else in section 4 changes (it still removes every `ROLE_ENV` pair with `env.retain`). Re-run of
the gate against this amendment: `rg "first|last" docs/specs/spec-3h-role-label.md` finds no other
statement about which pair is read, and the only test touching duplicates is test 6, which relies
on `create_session` removing the inherited pair first, so it does not depend on first versus last.
Scope, acceptance and every other decision are unchanged.
