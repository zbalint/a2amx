# Spec 3ca: the `cache` and `huggingface-token` grants

## 0. Status

**LOCKED** (2026-11-02; one consultant pass applied). Context `a2amx-confine-redesign-2026-11-02`. Location: main checkout, branch `develop`.
Public test seams: `grant::all`, `grant::lookup`, `grant::render_list`, `confine::plan` with a `ConfineRequest` built
directly (`Plan::writable`, `Plan::hidden_paths`, `Plan::with_bwrap(..).wrap(..)`), the `a2amx confine list` binary
test in `tests/grant.rs`.

Scope (may edit): `src/grant.rs`, `src/confine.rs` (only the test list in
`credential_grant_unhides_are_default_hide_sources`), `tests/grant.rs`, `tests/confine.rs`, `docs/confinement.md`.
Does not touch: `src/cli.rs`, `src/daemon.rs`, `src/main.rs`, `src/team.rs`, `docs/backlog.md`, `CHANGELOG.md`,
`a2amx.toml*`, other specs. The grant table keeps its shape (no new fields).

## 1. Why

Owner decision (2026-11-02): named grants wherever possible; a raw `write` entry is for the occasional odd path. The
whole `~/.cache` is a common case (shared target directories, scratch homes, tool caches), and no grant covers it: `go`,
`python` and `c-cpp` open only their own subfolders. Today `~/.cache` is opened by the automatic build paths
(`build_paths`, on by default, `docs/confinement.md:36`), which redesign step 3 removes, or by a raw
`write = ["~/.cache"]`. The grant is the named, build-independent way; it changes nothing for a session that
already has build paths on. This spec adds a `cache` grant for it, and a `huggingface-token` credential grant, because
`~/.cache/huggingface/token` is a default hide that stays hidden under a writable `~/.cache`.

`cache` is all-or-nothing like every other grant: it opens the whole directory and has no options. The default hides
under it still apply (that is what a raw `write` of `~/.cache` does today); a hidden path opens only through its own
credential grant. The narrower grants (`go`, `python`, `c-cpp`) keep opening only their own subfolders with or
without `cache`; given together they overlap harmlessly.

## 2. Behaviour

### 2.1 The new rows

Both rows are appended after `netrc`, in this order, with `team_default_ok: true`, `sockets: NO_PATHS`,
`own_config: false`, `stand_ins: NO_PATHS`. `GRANTS` becomes `[Grant; 30]`.

| name | unhides | env | exposes |
| --- | --- | --- | --- |
| `cache` | `~/.cache` | none | your whole ~/.cache, read-write: every tool's cache and scratch data, including tool environments and hooks whose programs your own shell and builds outside the session run; the default hides under it (for example the Hugging Face token) stay hidden only while they exist when the session starts, and only until something moves them; a credential file created or renamed later is open to the agent, and a state directory under ~/.cache makes the grant fail |
| `huggingface-token` | `~/.cache/huggingface/token` | `HF_TOKEN`, `HUGGING_FACE_HUB_TOKEN` | your Hugging Face login (~/.cache/huggingface/token, read-write in place) and the HF_TOKEN and HUGGING_FACE_HUB_TOKEN environment variables: the agent can act on your Hugging Face account and change the file |

Constants: `CACHE_UNHIDES: &[&str] = &["~/.cache"]`, `HUGGINGFACE_TOKEN_UNHIDES: &[&str] =
&["~/.cache/huggingface/token"]`. No change to `apply_grant_unhides`: `cache` goes through the same path as any
directory grant (symlink refusal, `check_grant_path_writable`), `huggingface-token` through the same path as
`cargo-token`.

### 2.2 Tests

In `src/confine.rs`, add `"huggingface-token"` to the `credential_grants` list of
`credential_grant_unhides_are_default_hide_sources` (the path is in `DEFAULT_HIDES_EXTRA`, so it passes). The
uniqueness test and the credential-environment test already cover the new env names.

In `tests/grant.rs`, extend the three tables that list every grant (unhides/flags at ~l.100-270, `exposes` text at
~l.300-370, the name list at ~l.465-480) with the two rows, using the exact text above.

In `tests/confine.rs`:

1. Add both to the grant matrix `cases` (`[(&str, &[&str]); 17]` becomes 19); the file check in the loop gains
   `"~/.cache/huggingface/token"`.
All new plan tests set `build_paths: false` explicitly (the default is `false` for a directly built `ConfineRequest`,
but the matrix tests state it and these must too; with build paths on, `~/.cache` is writable regardless).

2. New test, `cache_grant_keeps_the_default_hides`: fixture with `~/.cache` and `~/.cache/huggingface/token`;
   plan with `grants: ["cache"]`. Expect `~/.cache` in `Plan::writable` and the token in `Plan::hidden_paths`.
3. New test, `huggingface_token_grant_opens_only_the_token`: same fixture, `grants: ["cache", "huggingface-token"]`.
   Expect the token not in `hidden_paths`, `~/.cache` writable, and `HF_TOKEN` and `HUGGING_FACE_HUB_TOKEN`
   released in the wrapped environment (follow the pattern of the existing team `cargo-token` wrapper test, which looks for `--unsetenv` windows, at
   l.1406-1435; the Hugging Face names must be absent from the unset arguments). Also with `grants: ["huggingface-token"]` alone: the token is not hidden and `~/.cache` is not
   writable.
4. New test, `cache_grant_with_narrower_grants`: `grants: ["cache", "go", "python"]` with their directories present
   plans without error and `~/.cache` stays writable.
5. New test, `cache_grant_refuses_a_state_directory_under_it`: a fixture whose state directory is under `~/.cache`;
   `grants: ["cache"]` fails with the text "overlaps the state directory". (A raw `write` of the same path plans; the
   grant is stricter, and the docs say so.)
6. New real-bubblewrap probe, `real_bubblewrap_cache_grant_hides_the_token_and_writes_the_cache`, in the pattern of
   the probe at `tests/confine.rs:4655`: fixture `~/.cache/huggingface/token` with content, `grants: ["cache"]`,
   `build_paths: false`; inside the wrapped command `test ! -s` on the token succeeds and a new file under
   `~/.cache` is written. Skip when bubblewrap is unavailable, as the existing probe does.

### 2.3 Docs

`docs/confinement.md`: two rows in the grant table after `netrc` (same text, "Yes; team or session."), and one
passage at l.180 saying: the `cache` grant is the named way to open `~/.cache`, independent of build paths; the
default hides under it protect only paths that exist when the session starts, so a token created or renamed
afterwards is not hidden (`mv` of `~/.cache/huggingface` acts on the host); a state directory under `~/.cache`
fails the grant, where a raw `write` plans; and a `~/.cache` that is a symlink is refused (name the real path with
a `write` entry).

## 3. Claims about existing code

| claim | where | verified how |
| --- | --- | --- |
| `~/.cache/huggingface/token` is a default hide | `src/confine.rs:97-102` (`DEFAULT_HIDES_EXTRA`) | read |
| a credential grant's unhide paths must be default-hide sources | `src/confine.rs:2391-2420` | read |
| credential env names need a grant with `unhides` | `src/confine.rs:2435` | read |
| `HF_TOKEN`, `HUGGING_FACE_HUB_TOKEN` are in `CREDENTIAL_ENV` | `src/confine.rs:56-57` | read |
| a directory grant over a parent keeps nested default hides | `docs/confinement.md:180`, spec 3bx | read; **to be confirmed by test 2** |
| the narrower grants overlapping a wider one plan without error | `apply_grant_unhides` (no overlap check between grants) | read; **to be confirmed by test 4** |
| a grant path that overlaps the state directory is an error, a raw `write` is not (`explicit`) | `check_write_allowed`, `src/confine.rs:265-275`, `:1446-1447` | read (consultant too); test 5 |
| `~/.cache` is writable by default through build paths | `docs/confinement.md:36`, `src/main.rs:506` | read |
| deeper hide wins over a shallower writable bind | `mounts()`, `src/confine.rs:1770-1817` | read (consultant); test 6 |
| no other test counts the grant rows | `rg -n 'Grant; 28\|28 grant'` | **to run before lock** |

## 4. Out of scope

Overriding a default hide from a grant that does not name it; hiding `~/.cache/huggingface/stored_tokens`
(newer `huggingface_hub` may keep tokens there; unverified, so it is a follow-up for the architect's backlog, not a
change to the default hides here); options on `cache`; honouring `XDG_CACHE_HOME`; a
`pre-commit` or other per-tool grant; the team config change (`a2amx.toml`, after this lands); backlog and changelog
(architect).

## 5. Acceptance

- `env -u A2AMX_BIN -u NO_COLOR cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check` pass.
- `cargo run -q -- confine list` prints 30 rows, the last two being `cache` and `huggingface-token` with the exact text
  above (judge: the `tests/grant.rs` exact-text test).
- Tests 2, 3 and 4 above fail on the old code (unknown grant) and pass on the new. Test 2 is the abuse-case test: it
  shows `cache` does not open the token.
- Tester pass, mutation check in a scratch copy (its tree must match its own diff afterwards): (a) in
  `apply_grant_unhides` change `self.hidden.retain(|existing| existing != &path)` to also drop hides nested under the
  granted path; tests 2 and 6 must fail. (b) Change the `cache` unhide string; the exact-table test in `tests/grant.rs`
  and the matrix fail first, which is expected. Name the failing test for each mutant.
- Tests 2, 3, 4, 5 and 6 are new behavior; none is a contract test that already passes on the old code.
