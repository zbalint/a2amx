# Spec 2x: `list_agents` returns each agent's harness and cwd

## 0. Status

**LOCKED** (2026-10-04, pre-lock gate run against develop at `7b62858` and re-read against `0572edc`; gate notes in section 7).
Decisions D1 to D4 come from the owner's idea of 2026-10-04 (backlog item F10: "list_agents should
return also the harness type for each entry and the cwd of the agents"); field names and absence
rules are the architect's, settled on the owner's instruction to decide while away, and marked
**(architect)**.

**Baseline:** develop at `0572edc`, the commit that implements spec 2w. **Location and branch:**
main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-list-agents-fields`. Public test seams: the wire `Request::ListAgents` against a real
daemon (`tests/broker.rs`, `tests/quota.rs`) and the MCP tool through `a2amx mcp`
(`tests/mcp.rs`, `tests/bridge.rs`).

**Scope.** May edit exactly these files and no others.

- `src/wire.rs` (`AgentSummary` gains two fields)
- `src/daemon.rs` (the `Request::ListAgents` arm fills them)
- tests: `tests/wire.rs`, `tests/mcp.rs`, `tests/bridge.rs`, `tests/broker.rs`, `tests/quota.rs`,
  and `tests/common/mod.rs` if it builds an `AgentSummary` (every place that builds or compares an `AgentSummary` or the `list_agents` JSON)
- `README.md`, `docs/architecture.md` (line 130 area, the `list_agents()` return shape),
  `docs/backlog.md`

Does not touch: `src/mcp.rs` (the tool forwards `agents` as serialized), `src/main.rs`,
`src/session.rs`, `src/harness.rs`, `src/cli.rs`, `scripts/`, `extension/`. No new dependency, no
new `Request`. Do not commit, stage or merge: leave the diff uncommitted for review.

## 1. Why

An agent that reads `list_agents` sees an address, a state and attachment, and a quota. It cannot
tell which harness a peer runs (claude, omp, codex or generic), which matters for how that peer
behaves on reset, exit and delivery, nor where the peer works. `a2amx list` already shows both
for a human; the daemon already holds them (`SessionSummary`). The agent interface should carry
the same facts.

## 2. Decisions

- **D1. (architect)** `AgentSummary` gains `harness: Harness` and `cwd: Option<String>`. Names
  match `SessionSummary` (`src/wire.rs`).
- **D2. (architect)** `harness` is always present in the JSON, including the value `generic`
  (no `skip_serializing_if`): the harness is always known, and an agent must be able to tell
  `generic` from a daemon that predates the field. Values are the lowercase snake_case strings
  of the `Harness` enum: `claude`, `omp`, `codex`, `generic`.
- **D3. (architect)** `cwd` is the session's spawn directory (`Session::cwd`, as `list` reports
  it), `#[serde(default, skip_serializing_if = "Option::is_none")]`: absent when the session was
  started without one, never `null`. It is not the process's current directory; the field's doc
  comment says so.
- **D4.** Deserialization of an old payload without `harness` yields `Harness::default()`
  (`generic`) through `#[serde(default)]` on the field, so a newer client reads an older daemon.
  `list_agents` input, the other tools and every other response are unchanged.

## 3. `src/wire.rs`, `src/daemon.rs`

`AgentSummary` (line 319 at the baseline): add `#[serde(default)] pub harness: Harness` and the
`cwd` field of D3 with its doc comment. `daemon.rs` `Request::ListAgents` arm (the `AgentSummary`
literal): `harness: session.harness()` and `cwd: session.cwd().map(|path|
path.to_string_lossy().into_owned())`, the same expressions the `Request::List` arm uses, copied
not re-derived.

## 4. Tests (failing first)

1. `tests/wire.rs`: the `AgentSummary` literal gains the fields; the expected JSON gains
   `"harness":"generic"` (and `"cwd"` only in a second case where it is set); a payload without
   `harness` deserializes to `generic`.
2. `tests/mcp.rs` (line 288 area) and `tests/bridge.rs` (line 603 area): the expected
   `{"agents":[...]}` literals gain `harness` (and `cwd` where the test's sessions have one).
   The developer reads the real output first and checks each value against how the test starts the
   session; expected values stay literals, never recomputed.
3. `tests/broker.rs` and `tests/quota.rs`: every place that builds an `AgentSummary` or compares
   one gets the fields; add one assertion that a session started with `--harness omp` reports
   `harness == Harness::Omp` and a session started with a working directory reports it.

## 5. Docs

`README.md` (the `list_agents` bullet names the new fields), `docs/architecture.md` (the return
shape of `list_agents()`), `docs/backlog.md` (delete row F10).

## 6. Out of scope

Scoping `list_agents` (F3), a live current directory, argv, hold or pending counts in
`list_agents`, new tools, and any change to `a2amx list`.

## 7. Acceptance and gate notes

```sh
env -u NO_COLOR -u A2AMX_BIN cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
git diff --check
```

All four pass. Gate notes at lock time: `AgentSummary` is built in exactly one place in `src`
(`daemon.rs`, the `ListAgents` arm) and named in `tests/wire.rs` and `tests/broker.rs`;
`tests/mcp.rs:288` and `tests/bridge.rs:603` assert the exact JSON, so they are in scope;
`Harness` already derives `Serialize` and `Deserialize` with `rename_all = "snake_case"` and has
a `Default`; `Session::cwd` exists (`src/session.rs`). `git status` shows only files from
section 0.
