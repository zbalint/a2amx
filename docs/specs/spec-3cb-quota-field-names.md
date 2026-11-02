# Spec 3cb: quota fields say that they are percent left

## 0. Status

**LOCKED** (2026-11-02; one consultant pass applied). Context `a2amx-confine-redesign-2026-11-02`. Location: main checkout, branch `develop`
(start after spec 3ca is committed; the two share no file).
Public test seams: `QuotaInfo` serialisation (`tests/wire.rs`), `quota::parse_omp_usage` and `quota::read`
(`tests/quota.rs`), `status` formatting (`tests/status.rs`), the `list_agents` tool schema (`tests/mcp.rs`).

Scope (may edit): `src/wire.rs`, `src/quota.rs`, `src/status.rs`, `src/mcp.rs`, `tests/wire.rs`, `tests/quota.rs`,
`tests/status.rs`, `tests/mcp.rs`, `docs/architecture.md`.
Does not touch: `src/daemon.rs` (it only builds `QuotaInfo { limit_reached, ..quota }` and calls `exhausted()`; both
unchanged), `CHANGELOG.md` (architect), `docs/backlog.md`, `a2amx.toml*`, other specs.

**Breaking change** to the public interface (the `quota` object in `list_agents` output and in the control protocol's
session entries). Allowed now: 0.x prerelease, one user.

## 1. Why

`list_agents` returns `"quota": {"five_hour": 85, "weekly": 30}`. The numbers are the percent **left**
(`src/wire.rs:376`, `docs/cli.md:126`), but the names and the tool description do not say so, and an agent
(the architect, twice) read 100 as "used up". `send_message`'s `recipient_quota` has the opposite sense ("quota is used
up"), which makes the mix-up worse. Fix both: name the fields for what they hold, and say it in the tool descriptions.

## 2. Behaviour

### 2.1 Field names

In `QuotaInfo` (`src/wire.rs:377-386`) rename `five_hour` to `five_hour_left` and `weekly` to `weekly_left`. Rust
fields and JSON keys are the same name (no `#[serde(rename)]`). `limit_reached` stays. The doc comment already says
"percent left"; extend it to name the two fields. Rename every use: `src/quota.rs` (lines 97, 98, 108, 117, 144,
145), `src/status.rs` (396, 397), `src/wire.rs:391` (the two field accesses in the `match` condition only; the arm strings
`"5h and weekly"`, `"5h"`, `"weekly"` at :392-394 are `recipient_quota` text and stay), and the tests that name the
fields: `tests/wire.rs:777-778`, `tests/quota.rs` (the field accesses and struct literals; the helpers at l.34-37 and l.177-180 use field-init
shorthand, so rename their parameters or write `weekly_left: weekly`, which a field-name search alone misses),
`tests/status.rs`. These stay: the screen text at `tests/quota.rs:126`, the `exhausted()` expectations at :185 and
:187, and `"5h and weekly quota exhausted"` at :991. Use `ast-grep` or a careful search; do not rename unrelated `weekly` strings (the screen
parser's `("weekly", Window::Weekly)` row at `src/quota.rs:61` and the `Window` enum are about what the harness
prints and stay).

An old client reading a new daemon's entry, or the reverse, sees no quota (the fields are `#[serde(default)]`); there
is no compatibility shim.

### 2.2 Tool descriptions (`src/mcp.rs`)

- `list_agents` (l.467): append: `"quota" gives the percent left in each window ("five_hour_left", "weekly_left"; 100
  means all of it is left, 0 means used up); "limit_reached" means the harness reported a usage limit.` Update the
  copy of the description in `tests/mcp.rs:225` to match, byte for byte.
- `send_message` (l.234): leave "reports that quota is used up"; it already states the sense.

### 2.3 Docs

`docs/architecture.md:140`: write the entry's field list as "optional `quota` (`five_hour_left`, `weekly_left`:
percent left, 100 = all left; `limit_reached`)". No other doc line names the keys (`docs/cli.md:126` is about the
`a2amx list` QUOTA column and is unchanged).

## 3. Claims about existing code

| claim | where | verified how |
| --- | --- | --- |
| only `src/quota.rs`, `src/status.rs`, `src/wire.rs` use the fields in `src/` | `rg -n '\bfive_hour\b\|\bweekly\b' src` | probed |
| `src/daemon.rs` does not name the fields | same grep, no hit | probed |
| `rg -n '\bfive_hour\b|\bweekly\b'` hits in tests: `tests/wire.rs` 2, `tests/quota.rs` 35, `tests/status.rs` 9; not all are renames (section 2.1) | same grep over `tests` | probed; consultant checked |
| the OMP extension and its tests do not read the fields | `rg -l 'five_hour\|weekly' --glob '*.mjs' --glob '*.js'` finds nothing | probed |
| `list_agents` description is pinned byte for byte in `tests/mcp.rs:225` | read | read |

## 4. Out of scope

Renaming `quota` itself, `recipient_quota`, `Window` or the screen-parser strings; changing what the numbers mean; the
status-line text (`5h N%`); any daemon logic.

## 5. Acceptance

- `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `rg -n '\bfive_hour\b|\bweekly\b' src tests docs --glob '!docs/specs/*'` shows only these keepers: `src/quota.rs:61`
  and :75, the strings at `src/wire.rs:392-394`, `tests/quota.rs` :126, :185, :187, :991, and unrelated prose
  (`docs/roadmap.md`, `docs/backlog.md`). Judge: the architect reads the output.
- A serialisation test in `tests/wire.rs` (new, fails on old code) compares `serde_json::to_value` of a `QuotaInfo`
  for two cases: `{five_hour_left: Some(15), weekly_left: None, limit_reached: false}` equals `{"five_hour_left":15}`;
  `{five_hour_left: Some(15), weekly_left: Some(0), limit_reached: true}` equals
  `{"five_hour_left":15,"weekly_left":0,"limit_reached":true}` (`None` and a false `limit_reached` are not
  serialised, `src/wire.rs:378-385`). Equality also proves the old keys are gone.
- The `tests/mcp.rs` schema test equals the new `list_agents` description.
- Tester pass, in a scratch copy: mutate `src/quota.rs:97-98` so `"5h"` fills `weekly_left` (and `"7d"` fills
  `five_hour_left`), `src/status.rs:396` (`<= 15` to `>= 15`) and the `exhausted` match condition; name the failing
  test for each.
