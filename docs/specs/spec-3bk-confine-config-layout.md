# Spec 3bk: the `confine` table, named grants, and one settings value through the launch paths (F46)

## 0. Status

**LOCKED** (2026-10-31; pre-lock gate run against develop at `b2f04ab`; consultant review `m_4703`, one pass, findings
applied). Architect-directed, context `a2amx-confine-grants-2026-10-31`.
Implementation goes to `a2amx-developer` only; the tester pass goes to `a2amx-tester`.

**Baseline:** develop at `b2f04ab`; full suite 700 passed, 0 failed, 3 ignored, with
`env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR cargo test --offline` (the Cargo and Rust home variables and a
writable `HOME` and `CARGO_TARGET_DIR` as in the project's confined-session setup).
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** `team::parse` / `team::parse_with` (the team-file text), `confine::plan` (the request), the wire
round trip of `ConfineRequest`, and `a2amx team up --dry-run` through `tests/team.rs`.

**Scope.** May edit exactly these files and no others: `src/grant.rs` (new), `src/lib.rs` (one `pub mod grant;` line),
`src/confine.rs`, `src/team.rs`, `src/cli.rs`, `src/main.rs`, `tests/team.rs`, `tests/confine.rs`, `tests/wire.rs`,
`tests/docs_examples.rs`, a new `tests/grant.rs`, `a2amx.toml.example`, `docs/confinement.md`, `docs/teams.md`,
`docs/cli.md`, `docs/harnesses.md`, `AGENTS.md` (one row in the Modules table). Does not touch: `a2amx.toml` (the owner's
file, protected inside the session), `CHANGELOG.md` (the architect edits it), `src/daemon.rs`, `src/wire.rs`, every other
file. No new dependency. Leave the diff uncommitted.

## 1. Why

Confinement is unreleased; its options are five flat keys per session (`confine_write`, `confine_hide`,
`confine_unhide`, `confine_no_default_hides`, `confine_allow_containers`) plus `confine = true`, and the owner's team file
repeats the same `confine_write` list on four sessions. Every integration the owner wants next (a `gh` token, ssh keys,
container runtimes one by one, another agent's harness) would add another flat key. The owner's goal is a configuration
that is easy to read and edit: **strict deny-by-default, and the user opens only what they name**. This spec changes the
shape of the configuration and nothing else; it keeps every behaviour of the plan. Later specs add the grants
(F48, F51 to F54), the default hardening (F49) and the loud-error fixes (F47).

Decisions already taken with the owner (2026-10-31):

- One `[confine]` table at team level and one per session; a session's lists add to the team's, scalars override.
- A **grant** is a name for something a user opens on purpose (a credential, a socket, a harness). Grants live under the
  key `grants`. The key is not called `allow`, because `allow` already means peer authorization in a team file, and a
  misplaced line would parse silently as a peer list (`src/team.rs:46`, `:71`).
- Credential grants are set per session only: a team-level `grants` that names one is an error naming the fix.
- The old flat keys and `confine = true` are replaced outright. Nobody uses them and the confinement feature is not in
  any release (`src/confine.rs` is absent at `v0.1.2`; the `CHANGELOG.md` entry is under Unreleased), so this breaks no
  released interface. There is no alias and no migration shim, only a clear error for each old key.
- Safety net, not a sandbox: no policy inside a grant, and `write = ["/"]` stays refused (use `confine = false`).

## 2. Target shape

### 2.1 The file layout

```toml
team = "myproject"

[confine]                  # optional team defaults; applies to every session
enabled = true             # default false
write = ["~/.ctrscm"]      # lists add to every session's lists
hide = []
unhide = []
default_hides = true       # false turns the optional default hides off

[[session]]
name = "architect"
command = ["claude"]
[session.confine]          # optional; an inline table is equally valid
grants = ["containers"]    # per session only in this spec (section 2.3)
write = ["~/.cache"]       # added to the team's write list

[[session]]
name = "scratch"
command = ["omp"]
[session.confine]
enabled = false            # opts out of confinement; this session takes no confine option of its own
```

### 2.2 Merge rules (exactly)

For one session, with `T` the team table and `S` the session table (either may be absent):

1. `enabled = S.enabled`, else `T.enabled`, else `false`.
2. `write`, `hide`, `unhide`: the team entries then the session entries, in file order, with an exact-string duplicate
   dropped (first one kept); a repeated entry inside one list is dropped the same way, and the same rule applies to
   `grants` (the session's own list). The existing limits (at most `MAX_LIST_ENTRIES`, 64, entries, each at most 4096
   bytes; use the constant at `src/confine.rs:19`, do not repeat the literal) apply to the merged list, per list, with
   the existing wording (`session X: write has at most 64 entries`, with the new key name).
3. `default_hides = S.default_hides`, else `T.default_hides`, else `true`.
4. `grants`: only the session table may set it (section 2.3).
5. If the effective `enabled` is `false`: the session is not confined, and any key the **session's own** table sets
   other than `enabled` (a non-empty `write`, `hide`, `unhide` or `grants`, or a `default_hides` of either value) is an
   error: `session X: [confine] options need enabled = true; add enabled = true to the session's confine table or to
   the team's`. An empty list (`grants = []`) is not an option and is accepted. Team lists with `enabled` false are fine
   and ignored. Note the consequence of rule 1 and 2 together: with the team table `enabled = false` and a session
   table `enabled = true`, the session is confined and the team's lists still apply to it. A team table that has
   options but no `enabled` anywhere confines nothing and raises no error; the docs say so.
6. If the effective `enabled` is `true`: the session carries `Some(ConfineRequest)` (section 2.4) with
   `no_default_hides = !default_hides`; otherwise `None`.

### 2.3 Grants (the new module `src/grant.rs`, pure)

```rust
/// One named thing a user opens on purpose.
pub struct Grant {
    pub name: &'static str,
    /// One line for the docs and for the later `confine list`: what the grant exposes.
    pub exposes: &'static str,
    /// May a team-level table name it? false for anything that exposes a credential or privileged access.
    pub team_default_ok: bool,
}

pub fn all() -> &'static [Grant];
/// Look a name up. The error names the valid names and, when exactly one valid name is within edit distance 2,
/// a closest match.
pub fn lookup(name: &str) -> anyhow::Result<&'static Grant>;
/// Validate a list: every name known, none repeated.
pub fn validate(names: &[String]) -> anyhow::Result<()>;
```

The table has exactly one row in this spec: `containers`, `team_default_ok: false`, `exposes`: "every container and
virtualization socket on the host (docker, podman, containerd, lxd, libvirt, snapd and others): effectively root on the
host". It reproduces today's `--confine-allow-containers` behaviour exactly. Spec 3bm (F48) splits it into separate
runtime grants; do not anticipate that here. The closest-match code is a small private function in `grant.rs` (edit
distance, no new dependency).

A team-level table naming any grant whose `team_default_ok` is false fails the parse:
`[confine] grants: "containers" exposes <exposes>; grant it in the session that needs it, not team-wide`. In this spec
that makes every team-level `grants` entry an error.

### 2.4 One settings value

`ConfineRequest` (`src/confine.rs:99`, also the wire type) is already the bag the plan reads and the daemon receives.
It becomes the one value carried everywhere; five loose fields and the boolean disappear from the launch plumbing.

```rust
pub struct ConfineRequest {
    pub write: Vec<String>,
    pub hide: Vec<String>,
    pub unhide: Vec<String>,
    pub no_default_hides: bool,   // stays negative on the wire: an absent field must mean "defaults on"
    pub grants: Vec<String>,      // replaces allow_containers
    pub team_file: Option<String>,
}
```

Every field keeps `#[serde(default)]`. `allow_containers` is removed from `ConfineRequest`. `Plan` (`src/confine.rs:122`)
loses its `pub allow_containers: bool` field (no test reads it; `rg '\.allow_containers' tests` finds nothing) and
gains `pub grants: Vec<String>` (sorted); `notice` and `env_unsets` ask a private method
`fn allows_containers(&self) -> bool` ("`grants` contains `containers`"), so `grants` is the only source of truth.

`TeamSession` (`src/team.rs:26`) loses `confine: bool` and the five `confine_*` fields and gains
`pub confine: Option<ConfineRequest>` (`#[serde(skip)]`, filled by `parse_with` after the merge; `None` means not
confined; `team_file` stays `None` there and is filled at launch). `NewOptions` (`src/main.rs:507`) loses its six
confinement fields **and** `team_file` (seven fields; `team_file` only fills `ConfineRequest.team_file`, at
`src/main.rs:521` and `:713`) and gains `confine: Option<ConfineRequest>` with the paths already resolved and the team
file already attached. `resolved_confine_request` is the only place that sets `ConfineRequest.team_file`.

One helper in `src/main.rs` replaces the four copies of the resolve-and-build block:

```rust
/// Resolve a request's relative paths against the session cwd and attach the team file.
fn resolved_confine_request(
    request: &ConfineRequest,
    cwd: &Path,
    team_file: Option<&Path>,
) -> ConfineRequest
```

It calls the existing `resolve_confine_paths` (unchanged) for `write`, `hide` and `unhide` and copies the rest.

## 3. `src/confine.rs`

1. `ConfineRequest`: as in 2.4.
2. `plan()` (`src/confine.rs:661`): the grant list comes off the wire, so bound it like the other lists before it is
   looked at. In `check_request_limits` (`src/confine.rs:680-692`, which already holds the Codex refusal and calls
   `check_entry_count` per list, `:186-196`), add `check_entry_count(&request.grants, "grants")`. Then, immediately after
   `check_request_limits`, call `grant::validate(&request.grants)` (an unknown or repeated name is an error). Compute
   one local `allow_containers` from the grant list and use it where `request.allow_containers` is used today
   (`hide_containers`, `release_operator_writes`, `apply_operator_hides`, `finish`; `src/confine.rs:839`, `:886`,
   `:902`, `:948`). The existing error-order tests (`tests/confine.rs:335-441`, `:525-544`) build requests without
   grants and are unaffected. Behaviour is otherwise unchanged.
3. The labels `check_request_limits` puts in its errors (`"confine_write"`, `"confine_hide"`, `"confine_unhide"`,
   `src/confine.rs:681-683`) become `"write"`, `"hide"`, `"unhide"`, matching section 2.2; the assertions that pin the
   old text change with them (`tests/confine.rs:335`, `:359`, `:367`, `:375`, `:437`, `:441`).
4. The refusal message at `src/confine.rs:877` becomes
   `{option} path {} requires the containers grant (--confine-grant containers, or grants = ["containers"])`, and the
   `option` strings passed to `check_container_socket` (`"confine_write"`, `"confine_unhide"`) become `"write"` and
   `"unhide"`.
5. `Plan.grants` as in 2.4. `notice()` (`src/confine.rs:1298`) adds one line `Granted: <names>.` after the "Skipped
   missing paths" line when `grants` is not empty; the existing container sentence stays. The byte cap logic
   (`truncate_before_closing`) is unchanged and the new line is a few dozen bytes.

## 4. `src/team.rs`

1. Remove the six fields from `TeamSession` and add `confine: Option<ConfineRequest>` as in 2.4. Remove `confine` from
   the `Default`-style constructor at `src/team.rs:660-680` (the flag-session constructor sets `confine: None`; sessions
   made from `NAME=EXECUTABLE` flags are never confined, as today).
2. New private deserialization type, with `#[serde(deny_unknown_fields)]` and every field defaulted:

```rust
struct ConfineTable {
    enabled: Option<bool>,
    write: Vec<String>,
    hide: Vec<String>,
    unhide: Vec<String>,
    grants: Vec<String>,
    default_hides: Option<bool>,
}
```

   `TeamFile.confine` becomes `Option<ConfineTable>`. In `parse_with`, remove the injection of a `confine` boolean into
   every session table (`src/team.rs:437-444`). Instead, before `session_from_table`, remove the session table's
   `confine` value, parse it into `Option<ConfineTable>`, and after the session is built set `session.confine` from one
   function `merge_confine(session_name, team: Option<&ConfineTable>, own: Option<&ConfineTable>) ->
   anyhow::Result<Option<ConfineRequest>>` implementing 2.2 and 2.3.
3. Old-format errors, checked before deserialization so serde's "unknown field" text never shows. `parse_with`
   (`src/team.rs:420`) deserializes `TeamFile` straight from the text today. First parse the text once as a plain
   `toml::Table` and look for the old shapes below; if none is found, keep the existing typed
   `toml::from_str::<TeamFile>` unchanged (so the line and column in serde's other error messages are not lost):
   - a top-level `confine` whose value is a boolean, `true` or `false`: `confine is now a table: write [confine] with
     enabled = true; see docs/confinement.md`;
   - a session `confine` whose value is a boolean: `session N: confine is now a table: use [session.confine] with
     enabled = true`;
   - a session key `confine_write`, `confine_hide`, `confine_unhide`, `confine_no_default_hides` or
     `confine_allow_containers`: `session N: confine_write was replaced by write in [session.confine]` (and
     `hide`, `unhide`, `default_hides = false`, `grants = ["containers"]` for the others).
4. `validate_session_confine_options` (`src/team.rs:832`) now reads `session.confine`: the Codex refusal and the
   `--append-system-prompt-file` refusal stay, keyed on `session.confine.is_some()`; the "options require
   `confine = true`" check and the 64/4096 limits move into `merge_confine` (the limits must use the new key names in
   their messages). `validate_session_confine` (`src/team.rs:886`) keys on `session.confine.is_some()`.

## 5. `src/cli.rs` and `src/main.rs`

1. `src/cli.rs:55-65`: keep `--confine`, `--confine-write`, `--confine-hide`, `--confine-unhide` and
   `--confine-no-default-hides`; remove `--confine-allow-containers`; add repeatable `--confine-grant NAME`. The clap
   field names (`confine_write`, `confine_hide`, `confine_unhide`, `confine_no_default_hides`) stay as they are, so the
   flag spellings are unchanged; they and their destructuring in `new` (`src/main.rs:400-403`) are the legitimate
   remaining uses of those identifiers (section 11 item 3). A repeated `--confine-grant NAME` is dropped to one.
2. `new` (`src/main.rs:395-480`): build `Option<ConfineRequest>` from the flags (`None` unless `--confine`); the existing
   error `--confine-* flags require --confine` also covers `--confine-grant`. Call `grant::validate` on the grant names
   before connecting to the daemon, so a typo fails at once with the valid names listed. Pass the resolved request in
   `NewOptions.confine`.
3. `create_session` (`src/main.rs:610-731`): `Request::Confine { confine: ..., request }` is built from
   `NewOptions.confine` (`Some` means wrap); `NewOptions.team_file` is gone (section 2.4). The error mapping for an old daemon (`message.contains("invalid control
   request")`) keys on `confine.is_some()`.
4. The three team paths (`src/main.rs:1042-1067` dry run, `:1128-1160` validation, `:1220-1250` start) call
   `resolved_confine_request` once each instead of building a request by hand, and read `session.confine` instead of
   the boolean.
5. The dry run adds one line `  grants: <names>` after `  confinement: confined` when the plan has grants (names joined
   by `, `); a session with no grants prints no `grants:` line.

## 6. Documentation and example file

Rewrite every example and sentence that uses the old keys: `docs/confinement.md` (21 matching lines; every TOML fence
must use the new layout and still parse under `tests/docs_examples.rs`; this is the largest single item, give it its own
step in your plan), `docs/teams.md` (the confinement paragraph near line 112),
`docs/cli.md` (lines 66 to 75: `--confine-grant`), `docs/harnesses.md` (lines 66 and 123), and the commented example
block in `a2amx.toml.example` (between the `# --- confinement example ---` markers). `docs/confinement.md` gains a short
"How the options combine" paragraph stating the merge rules in one place (lists add, scalars override, a credential
grant is per session only, `enabled = false` needs no other key, a team table with options but no `enabled` anywhere
confines nothing, and team `enabled = false` with a session `enabled = true` confines that session with the team's
lists) and a "Grants" table with the one `containers` row and
its `exposes` text. The statement "Hide and write are strict by default; you open what you name" stays the framing.
`AGENTS.md` Modules table gets one row: `grant | Named confinement grants: the table of what each one exposes, name
validation and the closest-match error (pure)`.

The owner's own `a2amx.toml` uses the old keys and is outside the scope of this spec. The architect hands the migrated
file to the owner at acceptance; after this change an old `a2amx.toml` fails to parse with the section 4.3 errors until
migrated.

## 7. Tests

Existing tests are updated to the new layout; their behavioural assertions do not change. Tests that pin the changed
surface (listed by the architect's grep of `tests/`; the developer greps again before editing):

- `tests/team.rs`: `dry_run_expands_home_relative_confine_paths`, `dry_run_rejects_existing_relative_omp_prompt_file`,
  `confined_claude_prompt_file_after_separator_is_ignored`, `confinement_team_defaults_and_validation_are_explicit`,
  `flag_sessions_support_bare_executables_and_first_equals_only`, `parse_preserves_commands_cwd_order_and_attach_selection`,
  and the fixtures with `confine = true` at lines 1175, 1204, 1214, 1580, 1642.
- `tests/confine.rs`: `a_list_of_exactly_64_entries_is_accepted`, `confine_write_error_precedes_bad_unhide_error`,
  `container_socket_hides_are_bound_and_can_be_disabled`, `hide_limit_error_precedes_unhide_limit_error`,
  `mandatory_hides_remain_when_default_hides_are_disabled`, `missing_container_socket_paths_are_reported_as_skipped`,
  `notice_describes_restrictions_and_keeps_closing_sentence`, `omp_exposes_only_state_omp_on_top_of_mandatory_state_hide`,
  `run_dbus_default_hide_is_not_skipped_when_missing`, `unhide_cannot_lift_container_socket_restriction` (asserts the old
  message at `tests/confine.rs:745`), `wrap_unsets_base_and_container_environment_in_stable_order`,
  `writable_paths_override_exact_hides_but_not_container_sockets` (asserts the old message at `:903`),
  `write_limit_error_precedes_hide_limit_error`.
- `tests/wire.rs`: `confine_request_round_trips_and_defaults_missing_fields` (`allow_containers` becomes `grants`;
  the missing-field defaults still hold).
- `tests/docs_examples.rs`: `toml_examples_in_the_confinement_guide_parse`. Its special case
  `confine_allow_containers =` (line 67) never matches today (the example line is double-commented) and is removed.
- `tests/confine.rs` also: the assertions on the error labels at `:335`, `:359`, `:367`, `:375`, `:437`, `:441` change
  from `confine_write`, `confine_hide`, `confine_unhide` to `write`, `hide`, `unhide` (section 3.3).

New tests (write each first and watch it fail):

1. `tests/team.rs`, through `team::parse`: (a) the section 2.1 example parses and each session's effective
   `ConfineRequest` equals the expected literal (`architect`: write `["~/.ctrscm", "~/.cache"]`, grants
   `["containers"]`, defaults on; the second session has `None`); (b) lists add and a repeated entry is kept once
   (also a repeated grant inside one session's list); team `enabled = false` with a session `enabled = true` confines
   that session and the team's lists apply to it; a team table with `write` and no `enabled` anywhere confines nothing
   and is accepted; a top-level `confine = false` is the old-format error, like `true`;
   (c) a session table with `write` and no `enabled` anywhere is the `need enabled = true` error; (d) `enabled = false`
   in the session with team lists parses, and with its own `write` is the error; (e) a team-level `grants =
   ["containers"]` fails with the section 2.3 message; (f) each old-format input of section 4.3 fails with the stated
   replacement text; (g) an unknown key inside `[confine]` and an unknown grant name fail, the latter listing
   `containers` and, for `container`, naming it as the closest match; (h) the 64-entry and 4096-byte limits apply to the
   merged list: a team list of 40 plus a session list of 30 is refused.
2. `tests/grant.rs`, through `grant::lookup` and `grant::validate`: the `containers` row exists and is not
   team-default-ok; a repeated name is refused; `lookup("containr")` names `containers`; `lookup("zzz")` lists the valid
   names and offers no closest match.
3. `tests/confine.rs`: `plan` with `grants: ["containers"]` equals today's `allow_containers: true` result (container
   sockets unhidden, container env kept, `Plan.grants` set); an unknown grant in a request is an error; the notice
   contains `Granted: containers.`.
4. `tests/wire.rs`: an old-style request JSON that still carries `allow_containers` deserialises with `grants` empty
   (the field is ignored, so an old client gets the stricter default; document this in the changelog).
5. `tests/team.rs`, `a2amx team up --dry-run`: a session with `grants = ["containers"]` prints `  grants: containers`; a
   session without grants prints no `grants:` line.

## 8. Out of scope

- The runtime grants, `a2amx confine list`, credential-env unsetting, `write = ["~"]`, the `~/.a2amx` protection, the
  loud-error fixes, WSL hides, and every other grant: specs 3bl to 3bs (F47 to F54).
- Any change to what the plan mounts, hides or unsets. This spec is behaviour-preserving apart from the new messages.
- `src/daemon.rs` and `src/wire.rs`: the daemon reads `ConfineRequest` through `confine::plan` and needs no change;
  `wire.rs` only re-exports the type.
- `a2amx.toml` (the owner's), `CHANGELOG.md`, the version, and the `confine = false` handling of `NAME=EXECUTABLE` flag
  sessions.
- Restructuring `src/main.rs` beyond collapsing the four request-building blocks into `resolved_confine_request`
  (F45 covers placement).

## 9. Trust boundaries

The wire request is client-controlled but the client is the session owner's own process; the daemon already validates
every path. New surface: a `grants` list from the wire. The daemon validates it in `plan()` (section 3.2): an unknown
name is refused, a repeated name is refused. The old `allow_containers` field is ignored, which only makes an old client's
session stricter. Attack paths: none opens a new reach; a grant name cannot carry a path or a command. Abuse-case test:
section 7, new test 3 (unknown grant on the wire is refused).

## 10. Placement check (spec-writing step 14)

`grant.rs` is a new small pure module: one design decision (what a grant is and what it exposes) behind three
functions. `confine.rs` (1390 lines) keeps planning; this spec removes a field and adds a list, nothing more. `team.rs`
(977 lines) owns the team-file shape, so the table and the merge belong there. `main.rs` (3389 lines) shrinks: four
copies of the request-building block become one helper. Splitting `main.rs` further is F45.

## 11. Acceptance

Run from the main checkout. Environment: `env -u A2AMX_BIN -u A2AMX_ADDR -u A2AMX_TOKEN -u NO_COLOR`, with the build
variables the architect's baseline used (section 12).

1. `cargo test` passes with no failure; the count is the baseline's plus the new tests; no test is deleted, and every test
   named in section 7 still exists and asserts the same behaviour under the new layout. Judge: the architect's rerun.
2. `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` pass. Judge: the architect's rerun.
3. `rg -n 'confine_(write|hide|unhide|no_default_hides|allow_containers)|allow_containers|confine-allow-containers' src tests a2amx.toml.example docs/confinement.md docs/teams.md docs/cli.md docs/harnesses.md`
   (the live files only; `docs/specs/` and `docs/backlog.md` quote the old names on purpose and are not searched)
   matches only: the old-key error texts in `src/team.rs` and the tests of those errors in `tests/team.rs`; the clap
   field names and their destructuring in `src/cli.rs` and `src/main.rs` (section 5.1); local variables and
   parameters named `allow_containers` inside `src/confine.rs` (no `allow_containers` struct field remains, in
   `ConfineRequest` or `Plan`); the old-field test in
   `tests/wire.rs` (test 4 of section 7); and docs sentences that say the old keys were replaced. Judge: the architect
   reads every match.
4. The new tests of section 7 fail on the old code (they do not compile or assert the new behaviour) and pass now. Tests
   that pass on the old code and stay green are the updated existing ones (contract tests).
5. `a2amx.toml.example` and `docs/confinement.md` TOML fences parse (`cargo test --test docs_examples`).
6. Mutation check by the developer on a scratch copy, then restored: (a) make `merge_confine` replace instead of add team
   lists; (b) skip the credential-grant check at team level; (c) make an absent `default_hides` mean off; (d) drop the
   unknown-grant check in `plan()`; (e) let `enabled = false` accept its own lists. Name the failing test for each.
7. Tester pass (`a2amx-tester`): reads the diff against this spec, runs the commands above, checks the mutants, and
   dry-runs a hand-written team file with the section 2.1 layout and one with the old layout (the second must fail with
   the replacement text).
8. Known flaky tests: none new; a failure of a test this spec does not touch is rerun once, then recorded as
   pre-existing.

## 12. Claims

| Claim | Where | Verified how |
| --- | --- | --- |
| `ConfineRequest` has `write`, `hide`, `unhide`, `no_default_hides`, `allow_containers`, `team_file`, each `#[serde(default)]`, and is the wire type | `src/confine.rs:97-112`, `src/wire.rs:14,149` | read |
| `TeamSession` has `confine` and five `confine_*` fields, `deny_unknown_fields`; `TeamFile.confine` is `Option<bool>`; `parse_with` injects a `confine` boolean into every session table | `src/team.rs:24-61`, `:63-78`, `:437-444` | read |
| `allow` already means peer authorization at team and session level | `src/team.rs:46`, `:71` | read |
| The options-need-`confine = true` check and the 64/4096 limits live in `validate_session_confine_options` | `src/team.rs:832-883` | read |
| `NewOptions` carries `confine` and the five fields plus `team_file`; `create_session` builds `Request::Confine` from them; `team_file` is used only for `ConfineRequest.team_file` | `src/main.rs:507-531`, `:610-731`, `:521`, `:713` | read (consultant `m_4703` finding 2) |
| `check_request_limits` holds the Codex refusal and labels its errors `confine_write`, `confine_hide`, `confine_unhide`; `MAX_LIST_ENTRIES` is a public constant of 64 | `src/confine.rs:19`, `:680-692`, `:186-196` | read (consultant `m_4703`) |
| `TeamSession` derives `Debug, Clone, PartialEq, Eq, Deserialize`; `ConfineRequest` derives `Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize`; an inline `confine = {..}` and a `[session.confine]` header give the same `toml::Value::Table` | `src/team.rs:24-25`, `src/confine.rs:98` | read (consultant `m_4703` finding 10) |
| Neither `ConfineRequest` nor `Request` has `deny_unknown_fields`, so an old `allow_containers` field is ignored | `src/confine.rs:98`, `src/wire.rs:140-141` | read (consultant `m_4703` finding 11) |
| The notice's byte cap reserves room for the closing sentence; the notice tests assert `contains` and `ends_with` | `src/confine.rs:1283-1295`, `tests/confine.rs:1695-1709` | read (consultant `m_4703` finding 12) |
| `tests/daemon.rs` uses `ConfineRequest::default()` and compiles unchanged | `tests/daemon.rs:110,141,143,229,239` | read (consultant `m_4703` finding 13) |
| The resolve-and-build block exists four times: `new`, the dry run, the validation, the start | `src/main.rs:461-463`, `:1049-1056`, `:1136-1144`, `:1228-1247` | read, `rg resolve_confine_paths` |
| `resolve_confine_paths` leaves `~` and `~/...` alone and joins other relative paths to the base | `src/main.rs:592-604` | read |
| `allow_containers` is read in `hide_containers`, `check_container_socket`, `release_operator_writes`, `apply_operator_hides`/`unhide`, `finish`, `env_unsets`, `notice` | `src/confine.rs:839,870,886,902,907,948,1032,1038,1318` | read |
| The refusal text names `--confine-allow-containers`; two tests assert it | `src/confine.rs:877`, `tests/confine.rs:745,903` | read, grep |
| The dry run prints `confinement: confined` then the path lines | `src/main.rs:1085-1112` | read |
| Flag-built sessions are never confined | `src/team.rs:669` | read |
| No existing closest-match helper | `rg -i 'did you mean\|levenshtein\|edit_distance' src` found nothing | grep |
| `tests/docs_examples.rs` parses every non-`# fragment` TOML fence of the guide and the example block of `a2amx.toml.example`, and special-cases `confine_allow_containers =` | `tests/docs_examples.rs:21-76` | read |
| Confinement is absent at `v0.1.2` and listed under Unreleased | `git cat-file -e v0.1.2:src/confine.rs` fails; `CHANGELOG.md:18-28` | probed |
| Baseline: develop at `b2f04ab`, full suite 700 passed, 0 failed, 3 ignored | architect run, 2026-10-31 | probed |

## Review record

Models: architect Claude Sonnet 5.5; consultant on the Claude harness, model `claude-opus-5-5`; developer and tester on the
OMP harness, model `openai-codex/gpt-5.6-luna` for planning and working (the advisor model was not exposed to either).
BLOCKED/amend cycles: 1 BLOCKED, 0 amendments (the architect answered it without changing the spec).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant (`claude-opus-5-5`) | Several mechanical gaps: an acceptance grep that matched docs, specs and the backlog; `NewOptions.team_file` left in place; the old-format scan placed after the typed parse; `Plan.allow_containers` public; a missing use of `MAX_LIST_ENTRIES`; wrong anchors | Spec fixed before the lock |
| Implementation | Developer (`gpt-5.6-luna`) | BLOCKED: the confinement notice truncates the writable list ("and 8 more") and the checkout looked read-only | Architect: the working directory is writable by design, the notice is misleading. Backlog item filed for the notice (always name the working directory) |
| Tester pass | Tester (`gpt-5.6-luna`) | The closest-match wording was not asserted (threshold mutant survived) | Developer added the assertions; mutant killed |
| Tester pass | Tester | Removing the grants count bound survived | Developer added `grant_count_limit_precedes_grant_lookup`; mutant killed |
| Tester pass | Tester | Section 9 names a wire-level unknown-grant test, but section 7 test 3 calls `plan()` directly | Accepted: `plan()` is the single check every entry point passes; spec wording imprecise, text left as locked |
| Architect review | Architect | New `#[allow(clippy::large_enum_variant)]` on `Command` without a reason (clippy needs it once `grants` grew `New`) | Architect added a one-line reason; precedent in `src/wire.rs` |
