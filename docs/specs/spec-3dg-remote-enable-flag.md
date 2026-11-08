# Spec 3dg: The operator enables remote messaging for one agent, as send-only or both ways, and only a confined session can have it

## 0. Status

LOCKED (architect, 2026-11-08), after the consultant's pre-lock review (`m_6017`, one round, all findings applied). 3dg and 3dh must not ship in a release separately: with 3dg alone a team file or any admin-token caller makes a session inbound-eligible with no lease.

Scope (files that may be edited): `src/messaging.rs` (the mode type), `src/wire.rs` (`Request::NewSession` field), `src/session.rs` (`SessionSpec` and `Session` field and accessor), `src/daemon.rs` (store the mode at creation, refuse it without confinement, extend `remote_ineligible`), `src/team.rs` (`TeamSession.remote`, validation), `src/cli.rs` and `src/main.rs` (`a2amx new --remote`, the team path to `NewSession`), `tests/human_inbound.rs`, `tests/team.rs`, `tests/teams.rs`, and mechanically (`remote: None` added to struct literals) every other test file that builds `Request::NewSession`, `SessionSpec` or `TeamSession` literals: `tests/{attach_cli,broker,claude_channel,codex,common/mod,confine,daemon,daemon_cli,delivery,explain,hook,quota,wire}.rs` (`tests/status.rs` builds a `StatusInfo`, not a `NewSession`, and needs no edit). Does not touch: `src/store.rs`, `src/client.rs`, `src/mcp.rs`, the docs, `CHANGELOG.md`, `Cargo.toml`.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: a real daemon (`tests/common`), the CLI (`a2amx new --remote`), team file parsing (`tests/team.rs`).

Third slice of remote human messaging (F81, `docs/permissions.md` D3 items 1 and 4). Builds on 3df. After it a session carries an explicit remote setting and the inbound request of 3df honors it. The lease is spec 3dh.

## 1. Why

D3: remote messaging is off by default, enabled per agent in one of two modes (`send`: the agent may message the human and nothing from the human reaches it; `both`), and only for confined sessions. 3df's `remote_ineligible` checks only that the recipient runs and is confined; this spec adds the per-agent setting and the direction rule so a session the operator did not enable cannot receive a human message.

Alternatives rejected:

- Deciding eligibility in the gateway: the daemon is the only place that knows a session's configuration, and the gateway is the less trusted side.
- A mode on the team rather than on each agent: the owner wants to choose agents one by one (D3 item 1).
- Enabling by default for confined sessions: "off by default" (roadmap 0.3.0).

## 2. Changes

Claims (read at `d361693`; the developer re-checks each anchor):

| claim | where | verified |
|---|---|---|
| `NewSession` carries `role: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`; an added optional field with the same attributes keeps the wire form of every existing request | `src/wire.rs:182-184` | read |
| `SessionSpec` and `Session` carry `role`; `Session::role()` returns `Option<&str>` | `src/session.rs:83`, `:104`, `:484`, `:614` | read |
| `TeamSession` has `#[serde(deny_unknown_fields)]`, so a new key needs a field; `role: Option<String>` is its model | `src/team.rs:25-52` | read |
| `a2amx new` options are clap fields in `src/cli.rs` (`heartbeat: Option<String>` at `:95`) and reach `NewOptions` in `src/main.rs`; the team path builds `NewOptions` at `src/main.rs:1455-1470` | read |
| creating a Codex session with confinement is refused, so a confined session is never Codex | `src/daemon.rs:499-503` | read |
| about 60 struct literals set `role: None` in tests (`rg -c "role: None" tests src`) and break at compile time when a field is added | probed |
| the 3df function `remote_ineligible(&Session)` is the single place that decides eligibility | spec 3df | `src/daemon.rs`, `remote_ineligible` near `lookup` | read |

### 2.1 The mode

`src/messaging.rs`: `pub enum RemoteMode { Send, Both }` with serde names `"send"` and `"both"` and a parse function for the CLI and team file (clap `ValueEnum` is not used in `messaging.rs`; the CLI maps the string). Unknown strings are an error that names both values.

### 2.2 Carrying it to the session

- `Request::NewSession { .. , remote: Option<RemoteMode> }` with `#[serde(default, skip_serializing_if = "Option::is_none")]`.
- `SessionSpec.remote`, `Session.remote`, `Session::remote(&self) -> Option<RemoteMode>`.
- `Runtime::create_session`: if `remote.is_some()` and the request carries no confinement, answer `Response::Error { message: "remote messaging needs a confined session" }` before the confinement gate and any spawn, next to the Codex check at `src/daemon.rs:499-503`. A Codex request with `remote` and confinement is refused by that Codex check; one without confinement by the new check. The daemon is the authority; the team file check below only fails early.
- `a2amx new --remote <send|both>`; `a2amx team` files accept `remote = "send"` or `remote = "both"` on a session. The team path hands the value to `NewSession` (`src/main.rs:1462` builds `NewOptions` with `role: session.role`; add `remote` beside it). `RemoteMode` derives `Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize` with `#[serde(rename_all = "lowercase")]`, so the team key needs no custom parser and `remote = "maybe"` already fails naming `send` and `both`; the parse function serves only the CLI. `team::validate_sessions` (`src/team.rs:1085-1096`, beside `validate_session_confine_options`) also rejects a session with `remote` and no `confine`, so `team up` fails before it starts any session (no partial start). A team file does not grant anything by itself once 3dh lands; until then it does, which is why 3dg and 3dh do not ship separately.
- `a2amx new --remote` without `--confine` is refused by the daemon message above; the CLI shows that message. (`a2amx new` has no auto confine; only team files do.)

### 2.3 The direction rule

`remote_ineligible` (3df) gains, after the confinement check, a check of `session.remote()`: `Some(RemoteMode::Both)` passes; `Some(RemoteMode::Send)` and `None` are refused with `not_permitted` and the text `remote messages to this session are not enabled`. The match has no wildcard.

Existing accept tests turn red by this rule and are changed on purpose, not by adding `remote: None`: in `tests/human_inbound.rs` the helper `create_confined` (`:56`) gains a mode parameter, and `gateway_inbound_is_stored_as_a_human_message_for_a_confined_session` (`:108`) and `gateway_can_only_send_human_inbound_and_keeps_connection_usable` (`:318-345`, expects `Accepted`) create their session with `Some(RemoteMode::Both)`. The forged-envelope test is unaffected (content is validated first) and so is the exited-recipient test (exited is checked first). The outbound direction (3di) allows `Send` and `Both`.

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `HumanInbound` to a session without `remote` | reaches an agent the operator never enabled | constrain: `remote_ineligible` | refused, nothing stored |
| `HumanInbound` to a `send` session | a forged message reaches an agent that was only meant to notify | remove: the mode forbids the direction | refused, nothing stored |
| `NewSession` with `remote` and no confinement | an unconfined session receives remote messages | constrain: the daemon refuses at creation | `Response::Error`, no session created |
| team file from a repository | enables remote messaging for an agent | accepted until 3dh (no lease yet, so the file grants inbound eligibility); constrained by 3dh's lease; the two ship together | in 3dh |
| admin token | a same-user unconfined process creates a `both` session | accepted: out of scope (philosophy); only the admin role may send `NewSession` (`request_permitted`) | a session or gateway token sending `NewSession` is refused (existing broker test and the 3df gateway loop) |
| `team up` | a session with `remote` and no confinement starts the earlier sessions first | constrain: `validate_sessions` rejects before any start | team test |
| `remote` value | an unknown or empty mode | constrain: parse rejects it with both names | CLI and team parse tests |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests red first.
2. Tests in `tests/human_inbound.rs`: `HumanInbound` to a `both` confined session is accepted; to a `send` confined session and to one with no setting, refused with the text above, nothing stored (`ListMessages`); `NewSession` with `remote` and no confinement is refused and no session is listed.
3. `tests/team.rs`: `remote = "both"` and `"send"` parse; `remote = "maybe"` fails naming both values; a session with `remote` and no `confine` is rejected by validation; the key is accepted in the same place as `role`.
4. CLI tests in `tests/human_inbound.rs`, modelled on `tests/confine_show.rs:106-160` (`link_real_bwrap` plus a binary run of `new --detach --confine`; the test returns early without bwrap): `a2amx new --remote both` without `--confine` prints the daemon's message and creates nothing; with `--confine` and `--remote send`, then `both`, the session exists and a `HumanInbound` through the gateway is refused for `send` and accepted for `both`. That last pair is what kills a mode dropped between `a2amx new` and `NewSession`.
4a. `tests/teams.rs`: a team file with `remote = "both"` and no confinement fails `team up` before any session starts; with confinement (real bwrap, same pattern) the started session accepts `HumanInbound`. That kills a mode dropped in the team path.
5. Wire pin, following `tests/wire.rs:1226`: a `NewSession` without `remote` has no `remote` key; with `Some(Both)` it serializes `"remote":"both"` after `"role"`; old JSON without the key deserializes to `None`.
6. Mutation check, each restored by checksum, naming the failing test: `Send` treated as `Both`; `None` treated as `Both`; the no-confinement refusal removed; the mode dropped between the team file and `NewSession`; the mode dropped between `a2amx new` and `NewSession`.
7. `git diff --stat` shows only the files in section 0, and in the listed test files only mechanical `remote: None` lines.
8. `scripts/prepush-fast.sh` (architect). Sonar after the push.

## 5. Out of scope

Lease and `team up` output (3dh), outbound direction (3di), rate limits (3dj), showing the mode in `a2amx list` or `ls --details`, docs, changelog (3di).
