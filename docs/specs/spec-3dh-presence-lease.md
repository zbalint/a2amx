# Spec 3dh: A presence lease, started at the computer, must be valid before a remote message is exchanged

## 0. Status

LOCKED (consultant review applied; base 21c6152).

Scope (files that may be edited): `src/messaging.rs` (lease constants), `src/wire.rs` (two requests, one response), `src/daemon.rs` (lease state, the two requests, the eligibility check), `src/main.rs` (`team up` prints the enabled agents and the code), `src/client.rs` only if a typed helper beside the existing ones is needed (the developer says so before editing), `tests/human_inbound.rs`, `tests/teams.rs`, `tests/wire.rs` (pins for the new variants), and `tests/broker.rs` mechanically (the three `Limits { .. }` literals at `:669`, `:979`, `:1041` get the two new fields at their defaults). Does not touch: `src/store.rs`, `src/session.rs`, `src/team.rs`, `src/cli.rs`, `src/mcp.rs`, the docs, `CHANGELOG.md`, `Cargo.toml`.

Location: main checkout, branch `develop`. Context id `a2amx-remote-human-messaging-2026-11-08`. Public seams: a real daemon with admin, gateway and session connections (`tests/common`, `start_confined_daemon`); `a2amx team up` for the printed output.

Fourth slice of remote human messaging (F81, `docs/permissions.md` D3 item 2, D4). Builds on 3df and 3dg. After it an enabled, confined session can receive a human message only while its team holds a valid lease. 3dg and 3dh ship together.

## 1. Why

D4: a team file can come from a repository, so the file alone must grant nothing. The operator, at the computer, starts a lease; the paired phone confirms it by returning a one-time code; the lease lives at most 24 hours, ends with the team, and a message with no valid lease is rejected.

Alternatives rejected:

- Starting the lease from the terminal alone: any process with the admin token could then open the phone channel without the human's phone; the code makes the paired Discord account part of starting it (D4 "who can start one").
- Persisting the lease: a daemon restart must need a new lease (D4).
- Hooking session exit to end the lease: `observe_exit` returns early for sessions the daemon ended (`src/daemon.rs:779`), so an exit hook would miss a `team down`. The lease records the ids of the sessions it was started for and is valid only for those ids.
- Putting the pairing record into the daemon: the daemon cannot verify a Discord identity, so a stored id would catch only gateway bugs (`docs/permissions.md`, attack-path table). The gateway keeps who is paired. This changes D4 and the pairing row of the attack table; the docs are updated in spec 3di.

## 2. Changes

Claims (to verify at lock; the consultant re-checks):

| claim | where | verified |
|---|---|---|
| a session's team is `Session::team() -> Option<&TeamScope>` with `name: String`; team names are not unique across files (`src/team.rs:501-544` takes the name from the file) | `src/session.rs:607` | read |
| `remote_ineligible(&Session)` is the single eligibility function (3df, 3dg); its order is exited, confined, mode | `src/daemon.rs:2878` | read |
| the daemon keeps sessions in `Runtime.sessions` and `lookup` finds one by id | `src/daemon.rs` | read |
| random values come from `random_hex::<N>()` | `src/daemon.rs:2077` | read |
| `DaemonConfig` is built in many places, so the new times go into `Limits`, whose only full literals are three in `tests/broker.rs` (`:669`, `:979`, `:1041`; two others use `..Limits::default()`) | `src/daemon.rs:53-65`, `src/messaging.rs:133-152`, `rg "Limits \{" src tests` | probed |
| `run_team_up` starts the sessions of a team file and prints a summary | `src/main.rs:1527` | read |
| `request_permitted` is exhaustive per role (3de, 3df) | `src/daemon.rs` | read |

### 2.1 Lease state

`Runtime` gains one in-memory structure behind a mutex: per team name, an optional active lease `{ expires: Instant, members: Vec<SessionId> }` and an optional pending code `{ code, expires: Instant }`. Nothing is written to disk.

- Constants in `messaging.rs`: `LEASE_MAX: Duration = 24 h`, `CODE_TTL: Duration = 10 min`, `MAX_WRONG_CODES: u32 = 5`. Tests need shorter times: `Limits` gains `lease_secs: u64` and `code_secs: u64` (defaults the constants); tests pass them through the `Limits` they already give the daemon.
- A lease is **valid** for a session when its team has a lease, `Instant::now() < expires`, and the session's own id is in `members`. `members` is the set of ids of the running sessions of that team that have a remote setting, taken when the code is redeemed. The exited check of `remote_ineligible` already covers a session that stopped. Because the set is fixed at redeem, a team that goes down and comes back (new ids), an agent restarted under a live lease, and a later session that joins the same team name (from another team file) are all refused until a new code is issued and redeemed. Stopping one agent does not affect the others. Whether `Reset` keeps a session's id decides if a reset agent keeps its lease; the developer states what they found, either result is accepted.
- The check `lease_valid(team, id, now)` is one function used by eligibility only.
- Lock order: the daemon-wide lease mutex is taken after, never before, the `sessions` mutex; code that needs both takes a snapshot of the sessions first, releases it, then locks the lease state. No code nests them the other way.

### 2.2 Requests

`wire.rs`: `Request::IssueLeaseCode { team: String }` (admin only) and `Request::RedeemLeaseCode { code: String }` (gateway only); `Response::LeaseCode { code: String, agents: Vec<String>, valid_secs: u64 }`. Redeem answers `Response::Ok` or `Response::Error`. `request_permitted`: Admin true for Issue, false for Redeem; Gateway the reverse; Session false for both; no wildcards.

- **Issue.** The team must have at least one running session with a remote setting; otherwise `Response::Error` ("no agent in this team has remote messaging enabled"). It generates a code (`random_hex::<6>`, 12 hex characters), replaces any pending code for that team, resets the daemon-wide wrong-code counter, and answers the code, the enabled agent names (with their mode), and `CODE_TTL`.
- **Redeem.** Compare with `constant_time_eq` against every pending, unexpired code. A match removes the code (single use), and starts the lease: `expires = now + LEASE_MAX`, `members` as above; it replaces an earlier lease of that team. A wrong or expired code counts: the wrong-code counter is one daemon-wide number, because a redeem names no team; after `MAX_WRONG_CODES` wrong attempts since the last issue (for any team), all pending codes are dropped and every redeem answers `Response::Error` until a new code is issued. Error texts are fixed and reveal neither which team nor whether a code existed (`invalid or expired code`).
- The redeem work is split in two small functions: one finds the pending code that matches (returns its team), one starts the lease. Each stays well under the cognitive-complexity limit.
- Rate limits for the gateway as a whole are spec 3dj; this spec has the attempt cap only.

### 2.3 Eligibility

`remote_ineligible` is a free function over `&Session`; the lease check needs the daemon, so it becomes `Runtime::remote_ineligible(&self, session: &Session, now: Instant)` (one caller, `human_inbound`, which holds no lock at the call) and gains, after the setting check, the lease check: a session with no team, or whose team has no lease that lists this session's id as valid, is refused with `not_permitted` and the fixed text `remote messaging is not active for this team; start it at the computer`. This is the fixed error line the gateway shows the human after expiry. The order is: exited, confined, mode (3dg), lease. A session with a remote setting but no team name can never be reached; that is accepted, and `team up` warns about it (2.4).

### 2.4 `team up`

`run_team_up`, after its sessions have started and before it attaches (a terminal attach would hide later lines), not on `--dry-run`, asks the admin for a code for the file's team name if any session of the file has a remote setting, and prints to stdout (agent names and modes come from `Response::LeaseCode.agents`, so already-running agents are covered; the minutes come from `valid_secs` with `messaging::display_duration`, not a literal):

```
remote messaging for: architect (both), developer (send)
post this code in the control channel within <minutes>: <code>
```

Nothing is printed for a team file with no remote setting. If a session has a remote setting but the file has no team name, it prints one warning line (`remote messaging is set but the file has no team name; no lease can start`) and asks for no code. When every session was already running, the command still asks for a code; the new code replaces any pending one and redeeming it replaces the lease, which is the renewal path. A failure to obtain the code is printed as a warning and does not stop the team (the sessions are already running, and the operator can run `team up` again for the code; the developer shows what the command does when the team is already up, the existing behavior).

## 3. Attack paths

| entry point | abuse | disposition | abuse-case test |
|---|---|---|---|
| `HumanInbound` with no lease | a team file from a repository enables a session; a message arrives | constrain: lease check in `remote_ineligible` | refused with the fixed text, nothing stored |
| `HumanInbound` after expiry | late message | constrain: `now < expires` | with `lease_secs` set to 1, refused after it passes |
| `HumanInbound` after the team went down | stale lease | constrain: the session id must be in `members` | kill all members, refused; new members of the same name, still refused |
| a second team file with the leased team name | joins a live lease without a code | constrain: id membership | the joiner is refused while the original member is accepted |
| an agent restarted under a live lease | new id inherits | constrain: id membership | restarted agent refused until a new code is redeemed |
| partial `team down` | one agent stopped | no gap: the other members keep their ids | one member killed, others still accepted |
| `IssueLeaseCode` | a session or gateway token requests a code | constrain: admin only | refused for both |
| `RedeemLeaseCode` | admin or session redeems | constrain: gateway only (the admin token cannot start a lease without the phone side) | refused for both |
| code guessing | brute force through the gateway | constrain: 48-bit random, single use, expires, attempt cap | five wrong attempts, then the right code fails until reissue |
| code reuse and expiry | second redeem; redeem after `code_secs` | constrain: removed on use; TTL | each fails |
| fixed error text | tells an attacker which team or agent exists | constrain: same text for every failure | text equality across failures |
| compromised gateway | redeems a code it saw pass | not closed: the gateway sees the code (D4); bounded by 10 minutes, single use and the 24 h lease | none; documented |

## 4. Acceptance

1. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass. New tests red first.
2. `tests/human_inbound.rs`: sessions in the 3df/3dg accept tests (`create_confined` at about `:56`, the CLI `new --remote both` test at about `:316-370` (`a2amx new` has no team flag, so it cannot get a team scope: that test now expects the lease text for `cli-both` and the mode text (`remote messages to this session are not enabled`) for `cli-send`, which still proves the mode reached each session because the mode check runs before the lease check), the final Accepted at about `:536-564`) get a team scope (`--team` / `TeamScope`), then issue and redeem a code first (a helper); `gateway_other_requests` (about `:478`) gains `IssueLeaseCode` and the admin/session refusal test (about `:570`) gains `RedeemLeaseCode`; new tests per attack-path row above, one daemon per scenario (the wrong-code counter is daemon-wide), each with small `lease_secs`/`code_secs` passed through `Limits` (`start_confined_daemon` and `start_daemon_in` already take `Limits`; `tests/common` needs no change).
3. `tests/teams.rs`: the existing remote team file at about `:84-124` gets a `team =` line (otherwise its sessions have team None and cannot be accepted); `a2amx team up` for a file with a `remote` session and a team name prints the two lines (agent names with modes, a 12-hex code); a file without a remote setting prints neither; a file with a remote setting and no team name prints the one warning; `--dry-run` prints neither. The printed code redeemed through the gateway token starts the lease (test uses real bwrap with the existing return-early pattern).
4. `tests/wire.rs`: pins for `IssueLeaseCode`, `RedeemLeaseCode` and `LeaseCode` spelled as the neighbouring variants.
5. Mutation check, each restored by checksum, naming the failing test: lease check removed from `remote_ineligible`; expiry not compared; membership not checked (valid while the team name has a lease); wrong-code cap removed; code not removed on use; admin allowed `RedeemLeaseCode`; gateway allowed `IssueLeaseCode`; code compared with a non-constant-time equality is not testable, so the developer names it as unkilled by design.
6. `git diff --stat` shows only the files in section 0; `scripts/prepush-fast.sh` (architect); Sonar after the push (the lease functions each stay below the cognitive-complexity limit; the developer states the hand count).

## 5. Out of scope

The Discord side of pairing and who is paired (the gateway keeps it), rate limits and the burst allowance (3dj), the human-bound direction (3di), docs and changelog (3di), renewing a lease without `team up`, showing the lease in `a2amx list`.

## Amendments

1. (after BLOCKED m_6040) `a2amx new` has no team flag; the CLI acceptance test in `tests/human_inbound.rs` expects the lease denial for `both` and the mode denial for `send` instead of a team scope. The consultant's review wrongly named `a2amx new --team`; the three mentions are corrected.
