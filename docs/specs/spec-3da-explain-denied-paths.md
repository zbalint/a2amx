# Spec 3da: `a2amx explain` says what a confined session cannot touch and how to open it (F79)

## 0. Status

LOCKED.

Scope (files that may be edited): `src/wire.rs` (three fields on `ConfineInfo`), `src/confine.rs` (store the
three request fields on `Plan`, fill them in `confine_info`, one new `pub fn default_hide_names`, and make
`default_hide_sources_for` use it), `src/grant.rs` (one new pure function and its test seam), `src/main.rs`
(`run_explain` only), `tests/wire.rs`, `tests/confine.rs`, `tests/grant.rs`, `tests/explain.rs`. Does not touch: `src/daemon.rs`
(the `Explain` request and `ExplainInfo` stay as they are), `src/team.rs`, `confine show`, the exposure and
escape texts, `docs/` and `CHANGELOG.md` (the architect updates them), any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-rc2-2026-11-06`. Public seams: `a2amx::grant::denial_help` (pure) and the `a2amx explain` command.
This spec changes the public interface compatibly: `ConfineInfo` is part of the control protocol and gains
fields that default when absent (the architect adds the changelog line; a patch bump under 0.x).

## 1. Why

Backlog F79. The beginner walk-through (F64 step 6) showed that after a confined denial the commands
report only an operating-system error: `mkdir ~/.cache/x` says `Read-only file system`, `cat ~/.ssh/x` says
`No such file or directory`, `git config --local` says `Device or resource busy`. Nothing names the grant,
flag or `write` line that opens the path, which breaks `docs/philosophy.md` rule 7. For an exited session
`a2amx explain` prints only `exited (code N)`, so the one place a user looks has nothing to say.

Owner decision, 2026-11-06: `a2amx explain <session>` names the read-only and hidden paths of the session's
plan and the grant, flag or `write` line that opens each, for a running or an exited session.

Design, from the consultant's read of the plan code (confirmed by the architect where noted):
- The client builds the block; the daemon's `Explain` response does not change. The client already receives
  `SessionSummary.confine`, `.harness` and `.cwd` from the `List` response for running and exited sessions, as
  `run_confine_show` (`src/main.rs:1815`) uses them.
- `ConfineInfo` (`src/wire.rs:391`) carries only `auto`, `grants`, `write`, `reasons`. A session started with
  `--confine-hide`, `--confine-unhide` or `default_hides = false` has a different hidden set, so the block would
  be wrong exactly for operators who customised. Add the three request fields to `ConfineInfo`
  (`hide`, `unhide`, `no_default_hides`) with `#[serde(default, skip_serializing_if = "is_default")]` like
  the other additive fields (`src/wire.rs:466` `is_default`). An old daemon sends none of them and the block
  then reads as the default set; both client and daemon are the same release in practice, so this is stated
  in the docs and not detected.
- A grant's `unhides` are made visible and writable (`src/confine.rs:1596-1599`), so "path in an ungranted
  grant's unhides, or its `create` list or its sockets" names the right fix for both a hidden path
  (`~/.ssh`, `~/.config/gh`) and a read-only one (`~/.cargo`, `~/.cache`). A grant's `stand_ins` work the
  other way (the host file shows empty) and are never offered as a fix.
- Many default hides have no grant (`~/.gnupg`, the keyrings, browser profiles, `~/.password-store`,
  `~/.config/op`, `~/.vault-token`, `~/.pgpass`, `~/.config/rclone`, `~/.s3cfg`, `~/.claude/ide`, `/run/dbus`
  and the other harness's credential file). Their fix is `--confine-unhide PATH` (team and session key
  `unhide`), not `write`: a writable path inside a hidden one is refused (spec 3cr). `write` is offered only
  for "any other home path".
- Protected paths (git hooks and config, the `hooksPath` target, tool directories on `PATH` inside writable
  paths) and the mandatory hides (state directory, runtime directory, `/run/WSL`, `/mnt/wslg`) have no
  option that opens them: say so by class, do not list host paths.
- `ConfineInfo.grants` is the effective list and includes `harness-config` (`src/confine.rs:1873`), which a
  Claude or OMP session gets implicitly and may also name explicitly; the wire cannot tell the two apart. Show it
  neutrally as `harness config`, never as a fix.

Rejected: carrying the plan's `hidden`, `readonly` and `protected` lists over the wire (host-sized, about 35
hides plus `PATH` directories, duplicates plan logic, and any later change of their shape would break the
protocol); listing all 30 grants (filtering to grants with a path present on the host still gives up to
  about 28 rows on a developer host, so the block has no line bound; the typical-session test pins one literal); deriving the block in the daemon's
`explain_of` (needs the host's current files and the grant table in the daemon response for no gain);
a per-path query `explain <session> PATH` (useful follow-up, left for the owner to ask for).

## 2. Changes

1. `src/wire.rs`: add `pub hide: Vec<String>`, `pub unhide: Vec<String>`, `pub no_default_hides: bool` to
   `ConfineInfo`, each with `#[serde(default, skip_serializing_if = "is_default")]`. Update the one literal in
   `tests/wire.rs:955`.
2. `src/confine.rs`: `Plan` stores the request's `hide`, `unhide` and `no_default_hides` (it already stores
   `write`); `Plan::confine_info` (line 1870) fills the three fields. Add `pub fn default_hide_names(harness:
   Harness) -> Vec<String>` returning the entries of `DEFAULT_HIDES` and `DEFAULT_HIDES_EXTRA` as written
   (tilde form), then `~/.claude/.credentials.json` when `harness != Harness::Claude`,
   `~/.codex/auth.json` when `harness != Harness::Codex`, then `/run/dbus`. Rewrite
   `default_hide_sources_for` (line 1254) to map `default_hide_names` through `user_path` so the list exists
   once; its output must be unchanged (a test that compares the old and new output on a fixture home is
   required, section 4).
3. `src/grant.rs`: add `pub fn denial_help(info: &ConfineInfo, cwd: Option<&str>, default_hides: &[String],
   rows: &[Grant], exists: &dyn Fn(&str) -> bool) -> String`, pure (no file system, no environment; the
   harness difference is already in `default_hides`, so there is no harness parameter): `exists` answers for
   a tilde-form or absolute path. It returns newline-terminated
   lines in this order, with the literal texts shown (angle brackets are filled in):
   - `confinement: <auto|manual>; grants: <names joined by ", " or none>` followed, when `harness-config`
     is in `info.grants`, by `; harness config`. The names exclude `harness-config`.
   - `writable: working directory <cwd or unknown>, its git directory` plus `, harness config` when
     `harness-config` is in `info.grants`, plus `, <entry>` for each `info.write` entry.
   - `read-only: the rest of the home directory and the system`
   - `protected (no option opens them): git hooks and config, the hooksPath target, tool directories on PATH inside writable paths, the a2amx state and runtime directories, /run/WSL, /mnt/wslg`
   - `hidden (present on this host): <names joined by ", ">` where the names are, in order, the entries of
     `default_hides` for which `exists` is true and which are not in `info.unhide`, then each entry of
     `info.hide`; an `info.unhide` entry matches a default name by the same trimmed string equality (an absolute
     spelling of a home path such as `/home/u/.ssh` is not normalised; accepted, documented limitation); when `info.no_default_hides` is set the default entries are left out, and when the list is
     empty the line is `hidden: nothing from the default set is present`.
   - `to open a denied path:`, then one two-space-indented row per ungranted grant (not in `info.grants`) that
     has at least one existing path among its `unhides` and `sockets` (`create` is always a subset of `unhides`,
     `src/grant.rs:27`, so it adds nothing; print each existing path once, in table order, joined by ", ",
     then two spaces). Container sockets of ungranted runtimes appear only in these rows, not on the hidden line: `<paths>  grant <NAME> (<team or session|session only>)`; rows in table order.
     Then, when at least one present default hide is covered by no grant's `unhides` or `sockets` (covered means exact string equality after trimming a
     trailing `/` from both sides; never a prefix match: `~/.docker` holds the docker grant's sockets but
     no grant unhides it, and the `cargo` grant unhides `~/.cargo` while `~/.cargo/credentials` stays hidden
     until `cargo-token`), one row
     `<those names joined by ", ">  --confine-unhide PATH (no grant opens it)`. Then always
     `any other home path  write = ["PATH"] in [session.confine], or --confine-write PATH`.
   - `a grant goes in [session.confine] grants = ["NAME"], or --confine-grant NAME`
   - when `info.unhide` or `info.hide` is not empty: `this session also unhides: <list>` and/or `this
     session also hides: <list>` (one line each, entries joined by ", ").
4. `src/main.rs` `run_explain` (line 1955): after the existing output, for a session whose `List` summary has
   `confine: Some(info)`, append `denial_help(&info, summary.cwd.as_deref(),
   &confine::default_hide_names(summary.harness), grant::all(), &exists)`, where `exists` expands `~/` against
   the `HOME` environment variable (read in `run_explain`; `auto_grants_for` at `src/main.rs:752` takes the
   home as a parameter) and tests `Path::exists`; with `HOME` unset every tilde path counts as absent. The
   roughly 70 `exists` calls run inside `tokio::task::spawn_blocking` (AGENTS.md: file syscalls that can stall
   do not run on the async runtime), with owned copies of the inputs. The block is appended for an exited session too (today `run_explain` returns right after
   the `exited (code N)` line; restructure so the block still prints) and for a running one, after the `reset:`
   line. An unconfined session, or one the `List` response does not contain, prints exactly what it prints
   today. `request_sessions` is the existing call (`src/main.rs:365`, `:1102`).

## 3. Out of scope

`confine show`, the `Explain` wire request and response, a per-path `explain <session> PATH`, a launch
notice, a shell hook, error messages of the denied commands themselves, translating a denial into the exact
path the user touched, `docs/` (the Troubleshooting preface and `docs/cli.md`), and detecting an old daemon.

## 4. Tests

Expected values are literals written in the test, not recomputed with the code under test.
1. `tests/wire.rs`: a `ConfineInfo` without the three new fields (the JSON of today's four fields)
   deserializes with empty vectors and `false`; a round trip with the fields set is equal; the serialized form
   of a value with the defaults omits them (`skip_serializing_if`).
2. `tests/confine.rs` (the existing confine tests file; add one test): `default_hide_sources_for` is private,
   so test through `confine::plan` on a fixture home: `plan.default_hidden` for `Harness::Claude`,
   `Harness::Codex` and `Harness::Generic` contains the credential files of the other harnesses exactly as
   before (compare with a literal list of the entries present in the fixture) and a `default_hide_names`
   test pins the literal list for `Harness::Generic` (24 + 4 entries, the two harness credentials, `/run/dbus`
   = 31 names) and for `Harness::Claude` (30: no `~/.claude/.credentials.json`). The developer states the
   counts after reading the three arrays.
3. `tests/grant.rs`: `denial_help` on literal inputs with a closure `exists` that returns true for a fixed
   list: (a) a typical session (`grants` = `cache`, `cargo`, `harness-config`; `write` = `~/notes`;
   `exists` true for `~/.ssh`, `~/.gnupg`, `~/.config/gh`, `/run/docker.sock`): the whole output equals a
   literal string; it contains a `grant ssh (team or session)` row, a `grant gh` row, a `grant docker` row
   and `~/.gnupg  --confine-unhide PATH (no grant opens it)`, and it does not contain `harness-config` as a
   fix; (b) `no_default_hides` set: the hidden line is `hidden: nothing from the default set is present`
   and no `--confine-unhide` row; (c) an `unhide` entry removes that path from the hidden line and adds
   the `this session also unhides:` line; (d) a granted `ssh` is not offered as a row; (e) the typical block of (a) has at most 25 lines (assert on that literal; no bound is claimed for a
   developer host); (f) `~/.docker` present with no grant for it gets the `--confine-unhide` row, and a
   `docker` grant that is not granted still gets its own row for `/run/docker.sock` (exact-match rule).
4. `tests/explain.rs`: a confined session that has exited: `a2amx explain <session>` prints the
   `exited (code N)` line first and then a line starting `confinement: manual; grants:`. The file's
   `run_binary` runs with the state directory as its working directory (`tests/explain.rs:14-22`), which a
   confined plan refuses (writable path overlaps the state directory, `src/confine.rs:304`), and it does not
   set `HOME`; the test therefore starts the session and runs `explain` through its own command with a
   fresh working-directory tempdir and `HOME` set to the fixture home (the daemon already gets that `HOME`
   from `spawn_daemon_process`, `tests/common/mod.rs:73`), so `exists` is deterministic. Skip the test when
   `bwrap` is missing with `bwrap_available()` (`tests/confine_show.rs:280`; `link_real_bwrap` belongs to the
   in-process daemon and does not apply here); mutation (e) can only be shown on a host with `bwrap`, and the
   developer says so. An unconfined session's explain output is unchanged (the existing tests keep passing).

## 5. Acceptance

1. `cargo test --test wire --test grant --test confine --test explain` passes; before the change the
   developer shows tests 1 and 3 failing to compile or fail (the new function and fields do not exist), which
   is the red phase; test 2's `default_hide_names` part is red the same way, its before/after comparison part
   is a contract test; test 4 is red on the old code (no block).
2. `rg -n 'default_hide_sources_for' src` shows the function still exists and calls `default_hide_names`;
   `rg -n 'DEFAULT_HIDES' src` shows no second copy of the lists (the `cfg(test)` use at
   `src/confine.rs:2556-2558` chains the arrays and stays).
3. Mutation check by restore-by-checksum edits in the checkout (the tree must equal the developer's own diff
   afterward): (a) drop the `harness-config` exclusion from the `grants:` list, test 3(a) fails; (b) drop the `info.unhide` filter,
   3(c) fails; (c) ignore `no_default_hides`, 3(b) fails; (d) offer a granted grant, 3(d) fails; (e) drop the
   exited-session branch in `run_explain`, test 4 fails; (f) drop `skip_serializing_if` on one field, test 1
   fails. The developer names the failing test for each.
4. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass with the ambient
   environment of a hosted session. Known flakes I7, I14 (fixed by spec 3cx once it lands), I18: rerun once
   and record as pre-existing.
5. Scratch files stay under `target/scratch/3da/` and are removed before the report.
