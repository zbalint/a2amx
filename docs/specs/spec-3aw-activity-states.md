# Spec 3aw: Activity dot states: starting, held, resetting

## 0. Status

**LOCKED** (2026-10-28; gate run against `develop` at `cb15138`; consultant review `m_4213`, findings X1-X3 and
items 1-6 adopted). Architect-directed, owner request, context `a2amx-activity-states-2026-10-28`. Implementation
goes to `a2amx-developer` only.

**Baseline:** `env -u A2AMX_BIN -u NO_COLOR cargo test` (629 passed, 0 failed, 3 ignored on `a0c5034`; the developer
records its own count before the first edit), `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`. Known
flaky test: `tests/attach_cli.rs::mouse_wheel_scrolls_three_lines_and_q_returns_to_live` (backlog I14); a failure is
rerun once and recorded as pre-existing if it passes.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `status::render` and `status::render_team_line` (`tests/status.rs`); `wire` serde round trips
(`tests/wire.rs`); the daemon's attach status and `list` (`tests/daemon.rs`, `tests/attach_cli.rs`).

**Scope.** May edit exactly these paths and no others.

- `src/wire.rs`, `src/daemon.rs`, `src/status.rs`, `src/session.rs`, `src/delivery.rs`
- `tests/status.rs`, `tests/wire.rs`, `tests/daemon.rs`, `tests/attach_cli.rs`
- `docs/architecture.md` (the team-line dot paragraph and the activity paragraph at line 436 only)

Does not touch: `CHANGELOG.md` (architect), `Cargo.toml`, `Cargo.lock`, `docs/cli.md`, `docs/backlog.md`,
`docs/specs/`, every other file. No new dependency. Leave the diff uncommitted.

## 1. Why

Owner request, 2026-10-28. The activity dot (status bar and team line) follows this table:

| State | Color | 256-color | Meaning |
| --- | --- | --- | --- |
| Idle | green | 78 | ready and available |
| Working | blue | 75 | actively processing (done in `cb15138`) |
| Starting | yellow | 221 | initializing, not yet ready |
| Resetting | orange | 209 | reset in progress |
| Held / Blocked | red | 203 | needs manual intervention |
| Exited | grey | 244 | session terminated |

Today (`src/status.rs` `activity_color` at 479-484, `src/daemon.rs` `session_activity` at 2397-2409): `Activity` has
Idle, Working and Busy; Busy means a delivery hold or a reset and is drawn orange with the word "resetting"; a session
that is not yet ready counts as Working; a member that needs input shows working or idle. The table needs two states
the dot does not have (starting, held) and splits Busy.

## 2. Design (owner decisions in bold)

Do **not** add variants to `wire::Activity`: it has no `#[serde(other)]`, so an older client would fail on an unknown
variant, and the activity notices compare `!= Idle` (`src/daemon.rs:117,125`). Add additive display fields and derive
the dot state in one pure function.

- `wire::TeamMember` gains `held: bool` and `starting: bool`; `wire::StatusInfo` gains `starting: bool`. All three use
  `#[serde(default, skip_serializing_if = "is_default")]` (`is_default` is at `src/wire.rs:396`). Neither struct
  derives `Default` today; adding fields breaks every struct literal, so list and update them all (section 4).
- **Held** for a team member is the same value `StatusInfo.hold` already carries for the attached session: the
  delivery channel's `hold_reason()` or `session.needs_input()` is `Some` (`attachment_status`, `src/daemon.rs:2476-2479`).
  Compute it once per member in `attachment_status`. A quota `limit_reached` is **not** held (the quota text beside the
  dot already says it).
- **The attached session's own dot is unchanged when held.** `status::render` drops the activity dot when `info.hold`
  is `Some` (`src/status.rs:232-234`) and bar contract E17 (`tests/status.rs:220-226`) pins it: the red `HELD ...` alert
  already says it. Red 203 applies to team-line members only. Do not touch E17.
- **Starting.** One new method on `Session` computes `harness::ready(...)` for the session and latches an
  `AtomicBool` (set-once, monotonic) the first time it is true; `starting` is `running && !latched`. Both
  `session_working` (`src/daemon.rs:2378-2395`) and `PtyChannel::unready_reason` (`src/delivery.rs:146-153`), which
  evaluate readiness independently, use that method, so a session that becomes ready and receives a prompt inside the
  1 s `ACTIVITY_POLL` still latches. Latch on raw readiness, not on `working` (the 10 s `ACTIVITY_QUIET` rule is
  separate). A harness dialog later, or a reset, does not clear the latch.
- **Only Claude, Codex and OMP can be Starting.** `harness::ready` for Generic is just `bracketed_paste && !scrolled`
  (`src/harness.rs:205-210`), so a generic program that never enables bracketed paste would be yellow for life; a
  Generic session never reports `starting`. (The latch is still kept for it; the field is forced false.)
- One pure function in `src/status.rs`, `dot_state(activity: Option<Activity>, held: bool, starting: bool, exited:
  bool) -> DotState`. Precedence: exited (or `activity` `None`), then held, then starting, then resetting
  (`Activity::Busy` and not held), then working, then idle. `activity_color` and `activity_word` become `DotState`
  lookups. Words for the status bar: idle, working, starting, resetting; held and exited have no word in the bar (the
  bar's alert and the grey dot cover them). The attached session's bar passes `held = false` (E17 above) so its word
  set is idle, working, starting, resetting.
- Compatibility: an older daemon sends no new fields, so a newer client draws members by the old rules (a member that
  only needs input shows working or idle; a real hold shows orange; starting shows blue). An older client ignores the
  new fields. State both in the architecture paragraph.
- Colour note for the owner (no action): the low-quota text is already yellow 221 and the limit-reached text red 203
  beside the dot; the word beside the dot disambiguates.

## 3. Out of scope

`wire::Activity`, the `list` activity column (words idle, working, busy), the picker, activity notices, quota colours
and text, the sandbox indicator, `needs_input` timing (`NEEDS_INPUT_AFTER`), `CHANGELOG.md`.

## 4. Changes

### 4.1 `src/wire.rs`
The three fields of section 2.

### 4.2 `src/session.rs`
The latch and the one readiness method of section 2 (name: `Session::observe_ready`; returns the bool it computed).

### 4.3 `src/delivery.rs`, `src/daemon.rs`
Use `observe_ready` in `PtyChannel::unready_reason` and `session_working`. `attachment_status` fills `held` and
`starting` for each member and `starting` for the attached session (Generic forced false). Update the struct literals at
`src/daemon.rs:2464` and `:2474`.

### 4.4 `src/status.rs`
`DotState`, `dot_state`, the colour and word lookups, and the team-line and bar call sites (`src/status.rs:93-97`,
`:232-234`, `:471-484`). Keep the quota colours (`:436-440`).

### 4.5 Tests and docs
Struct literals at `tests/status.rs:10` and `:378`, `tests/attach_cli.rs:109,115,121`, `tests/wire.rs:768,841,851,857`.
`docs/architecture.md`: the dot colours and states, and the compatibility sentence.

## 5. Acceptance

Judge: the commands; the tester or the architect reads the diff.

- `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `tests/status.rs`: worked examples for each table row, team line and bar: idle 78, working 75, starting 221 (word
  "starting"), resetting 209 (existing E16 keeps passing), held member 203, exited and unknown activity 244; precedence
  (held beats starting, working and resetting; exited beats held). E13-E17 stay as they are, byte for byte.
- `tests/wire.rs`: `TeamMember` and `StatusInfo` round trip with the new fields true; a JSON object without them
  parses to false; serialization omits them when false (contract tests on the old shape stay green).
- `tests/daemon.rs`: a freshly spawned Claude-harness fake session reports `starting` in the attach status, then not
  `starting` after it becomes ready, and stays not `starting` when it later shows a dialog; a Generic session never
  reports `starting`; a member that needs input reports `held`. Name `list_activity_reports_working_then_idle_and_non_ready_working`
  (`tests/daemon.rs:481`) as a contract test: its behaviour must not change.
- `rg -n "observe_ready" src` finds the definition and exactly two call sites (`session_working`,
  `unready_reason`).
- Mutation check (the developer breaks each, names the test that fails, restores): drop the latch (starting returns),
  swap the held and starting precedence, report starting for Generic, drop `held` for members.
- `git status --short` shows only the scope paths.

## 6. Claims (gate step 9)

| Claim | Evidence | Verified how |
| --- | --- | --- |
| `Activity` has no `#[serde(other)]`; no wire reply type uses `deny_unknown_fields` (only the team file does) | `src/wire.rs:316-330`; `src/team.rs:14,23,50` | read (consultant) |
| `is_default` exists | `src/wire.rs:396` | read |
| Held attached session shows no dot, pinned by E17 | `src/status.rs:232-234`, `tests/status.rs:220-226` | read |
| `session_working` is called from `sample_activity` (1 s), `observe_peers`, notices, `list`, `attachment_status` | `src/daemon.rs:39,113,705-735,786,2048,2120,2469,2482` | read (consultant) |
| `PtyChannel::unready_reason` evaluates readiness itself | `src/delivery.rs:146-153` | read (consultant) |
| `harness::ready` for Generic/Omp/Codex is `bracketed_paste && !scrolled`; Claude needs the prompt between rules | `src/harness.rs:181-212` | read |
| `StatusInfo.hold` = `hold_reason()` or `needs_input()` | `src/daemon.rs:2476-2479` | read |
| `hold_reason` is `Some` only for PTY delivery | `src/delivery.rs:142-144,202,274,392-397` | read (consultant) |
| Struct-literal sites of `TeamMember`/`StatusInfo` | `src/daemon.rs:2464,2474`; `tests/status.rs:10,378`; `tests/attach_cli.rs:109,115,121`; `tests/wire.rs:768,841,851,857` | grepped (consultant) |
| Codex/OMP enable bracketed paste early so they rarely show starting | inferred; not run | tests use a fake harness; do not assert real-harness timing |
| A Claude screen without dialog markers can stay Starting | inferred | accepted limitation; documented |

## 7. Trust boundary

No new input from a less trusted source; the fields are daemon-computed display data on an authenticated connection. No
attack path.
