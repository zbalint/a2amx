# Spec 3r: `a2amx new` without `--name` generates a name

## 0. Status

**LOCKED** (2026-10-09, pre-lock gate run against develop at `fdceee3`; gate notes in section 8;
consultant advice `m_859`, context `a2amx-generated-names`, applied with the deviations in section 2).
Owner request in this session: make the name optional on `new` by generating a silly name when the
user passes none. Owner decisions: a generated name is shown and treated exactly like an explicit
one; there is no unnamed state for sessions started with `a2amx new`; team files are unchanged;
the attach screen prints nothing extra. Architect decisions (silly word list, client-side
generation) are in section 2.

**Baseline:** develop at `fdceee3`. **Location and branch:** main checkout
`/home/zbalint/workspace/a2amx`, branch `develop`. Shared task `context_id`:
`a2amx-generated-names`. Public test seams: `a2amx::names::pick` (pure, `tests/names.rs`) and the
`a2amx` binary through `new --detach` and `list` (`tests/refs_and_list.rs`).

**Scope.** May edit exactly these files and no others.

- `src/names.rs` (new), `src/lib.rs`, `src/daemon.rs` (visibility of one function only), `src/main.rs`
- `tests/names.rs` (new), `tests/attach_cli.rs`, `tests/refs_and_list.rs`
- `AGENTS.md` (one row in the module table), `README.md`, `docs/architecture.md`, `docs/backlog.md`

Does not touch: `src/wire.rs` (no new field), the daemon request and registry logic in
`src/daemon.rs`, `src/team.rs`, `src/messaging.rs`, `src/cli.rs`, every other test file, every spec
document. No new dependency. Do not commit, stage or merge: leave the diff uncommitted.

## 1. Why

`--name` is optional today, and an unnamed session is only reachable by its id (`s2@host-a`), shows
`-` in `list`, and cannot be named in `--watch` or `--control-from`. Starting a quick session by hand
should still give it a usable name.

## 2. Decisions

- **D1. Client-side.** Generation happens in `run_new` (`src/main.rs`) before the request is built.
  The daemon, the wire and `Request::NewSession` do not change, so the 41 `NewSession` literals in
  tests and raw wire clients that send `name: None` keep working, and message addressing by id for
  sessions without a name does not change. This deviates from the consultant's daemon-side advice:
  the consultant's concern was a race, but `create_session` holds `new_session_gate` from
  `src/daemon.rs:397` through the insert at line 525, so the daemon's own duplicate check is already
  atomic; a client-picked name that loses a race with a concurrent `new` simply gets the daemon's
  existing `session name N is already in use` error (D4). Daemon-side generation would force a
  `Created` wire change and rewrite many tests for a benefit this spec does not need.
- **D2. Word list and format.** The name is `ADJECTIVE-NOUN`, drawn from the two 64-word lists in
  section 3. All 4096 combinations are valid under `validate_name` and at most 19 bytes.
- **D3. Deterministic picker.** `names::pick(taken, seed)` is pure. The index is `seed % 4096`, with
  adjective `index / 64` and noun `index % 64`. If that name is in `taken`, probe `index + 1`,
  wrapping from 4095 to 0, until a free name is found; if all 4096 are taken it returns `None`.
  `taken` is every session name the daemon lists, exited sessions included (they stay registered
  until `a2amx kill`).
- **D4. Race and reuse.** Between `list` and `new` another client can claim the picked name; the
  command then fails with the daemon's error and the user reruns it. A name freed by `kill` can be
  generated again; with 4096 combinations and a random start this is rare, and it only matters for a
  message still queued for the old session. Both are accepted; leave a `// shortcut:` comment naming
  both (ceiling: concurrent `new` on one daemon and a very churned registry; upgrade trigger: daemon-side
  generation if either is reported).
- **D5. Randomness.** The seed is `u64::from_str_radix(&random_hex::<8>()?, 16)` using the existing
  `random_hex` in `src/daemon.rs`, whose visibility becomes `pub` (nothing else about it changes).
  No `rand` dependency.
- **D6. Scope of use.** Only `run_new`, only when `--name` is absent. An explicit `--name` behaves
  exactly as today. `team up`, the flag form of `team` and `create_session`'s other callers are
  unchanged (team entries keep requiring a name or a role).
- **D7. Output.** The detach path still prints the session id alone on stdout (existing tests and
  scripts rely on `s1\n`). When the name was generated, `run_new` also writes `name: ADJ-NOUN\n` to
  **stderr**, before attaching or returning. The attach screen takes over the terminal, so the status
  bar (which shows the session's name) is the attach-time display; do not print on stdout.
- **D8. No free name.** If `pick` returns `None`, fail with `no free generated session name; pass
  --name` before any request is sent.

## 3. `src/names.rs` (new)

Public module `names` (add `pub mod names;` to `src/lib.rs` in alphabetical position, after
`messaging`). Contents: two `const` arrays of `&str` and one public function.

```
const ADJECTIVES: [&str; 64] = [
    "soggy", "grumpy", "wobbly", "sneaky", "confused", "fluffy", "jolly", "sleepy",
    "bouncy", "cranky", "dizzy", "giddy", "goofy", "grouchy", "hasty", "itchy",
    "jittery", "lumpy", "mellow", "nervous", "noisy", "peppy", "plucky", "quirky",
    "rowdy", "scruffy", "shaky", "silly", "snappy", "sparkly", "squeaky", "stubborn",
    "sulky", "sunny", "tangy", "twitchy", "wacky", "whimsical", "zany", "bashful",
    "breezy", "chatty", "clumsy", "cuddly", "dapper", "feisty", "fidgety", "frisky",
    "gloomy", "hungry", "jumpy", "kooky", "lazy", "merry", "nifty", "perky",
    "pudgy", "rumpled", "spunky", "wiggly", "zesty", "brisk", "crispy", "dusty",
];
const NOUNS: [&str; 64] = [
    "walrus", "waffle", "pickle", "noodle", "toaster", "otter", "heron", "badger",
    "biscuit", "bagel", "muffin", "pretzel", "turnip", "pancake", "kazoo", "banjo",
    "llama", "penguin", "platypus", "narwhal", "hedgehog", "gecko", "moose", "ferret",
    "marmot", "pelican", "puffin", "quokka", "wombat", "tapir", "lemur", "newt",
    "goose", "cactus", "pumpkin", "potato", "radish", "cabbage", "sprout", "crouton",
    "dumpling", "nugget", "pudding", "spatula", "teapot", "kettle", "mitten", "sock",
    "slipper", "button", "pebble", "trombone", "ukulele", "bubble", "balloon", "doughnut",
    "gadget", "sandwich", "croissant", "tortoise", "beetle", "meatball", "acorn", "raccoon",
];

/// First free `ADJECTIVE-NOUN` at or after `seed % 4096`, wrapping; `None` when all are taken.
pub fn pick(taken: &[String], seed: u64) -> Option<String>
```

The doc comment on `pick` says why it takes a seed (tests need literals). The module is pure: no
I/O, no `validate_name` call (the test in section 4 covers validity).

## 4. `src/main.rs`, `src/daemon.rs`

- `src/daemon.rs`: change `fn random_hex` (line 1616) to `pub fn random_hex`. Nothing else.
- `src/main.rs`, `run_new` (line 345): after the `heartbeat` check and `Client::connect`, and before
  `create_session`, when `name` is `None`: call `request_sessions(&mut client)` (line 2497), collect
  the `name`s into a `Vec<String>`, compute the seed (D5), call `names::pick` (D8 on `None`), and use
  the result as the session name in `NewOptions`; remember that it was generated. After
  `create_session` returns, if generated, write `name: …` to stderr (D7) with `eprintln!`-equivalent
  existing style (use `std::io::Write` on `stderr` and return its error with `?`; do not use
  `unwrap`). Add the `// shortcut:` comment from D4 at the pick site. `name` becomes `let mut name`
  or is shadowed; do not change the `Command::New` destructure.

## 5. Tests (failing first, expected values are literals)

`tests/names.rs`, pure `names::pick`:

1. `pick(&[], 0) == Some("soggy-walrus")`; seed `1` gives `soggy-waffle`; seed `64` gives
   `grumpy-walrus`; seed `4095` gives `dusty-raccoon`.
2. Seed `4096` gives `soggy-walrus` (modulo).
3. `taken = ["soggy-walrus"]`, seed `0` gives `soggy-waffle`.
4. `taken = ["dusty-raccoon"]`, seed `4095` gives `soggy-walrus` (wraps).
5. With all 4096 combinations taken, any seed gives `None`. Build `taken` by calling `pick` 4096
   times, adding each result (a loop in the test is fine; the expected value is the literal `None`).
6. Every one of the 4096 names returned that way passes `a2amx::messaging::validate_name`.

`tests/refs_and_list.rs`, through the binary (`run_binary`, daemon from `common::start_daemon`):

7. `new --detach -- sh` without `--name`: exit success, stdout exactly `s1\n`, stderr starts with
   `name: ` and the remainder (trimmed) passes `validate_name`; `list` shows that name in the NAME
   cell of the `s1` row (`cells[1]`), not `-`.
8. A second `new --detach -- sh` without `--name`: its stderr name differs from the first's.
9. `new --detach --name plain -- sh`: stderr is empty and the NAME cell is `plain`.

Existing tests: update only assertions that expected `-` in the NAME cell of a session started by
`new` without `--name`: `tests/attach_cli.rs` (the picker rows near lines 351 and 395 assert
`"-"` as cell 1; assert instead that cell 1 is not `-` and passes `validate_name`) and
`tests/refs_and_list.rs` (`list_reports_explicit_omp_and_unnamed_generic_harnesses`, lines 358–359,
same change; rename it to drop "unnamed"). The `cells[4]`/`cells[6]` assertions at lines 101–102 are
other columns and stay. If any other existing test fails because of this change, stop and report
`BLOCKED — SPEC ADJUDICATION REQUIRED` with the test name; do not edit it.

## 6. Documentation

- `AGENTS.md`: module table row `names | Generated session names (pure, ADJECTIVE-NOUN with a seeded picker)`,
  placed after `messaging`.
- `README.md`: where `--name` is described, one sentence: without `--name`, `new` generates a name
  such as `soggy-walrus`, prints it on stderr, and treats it like an explicit one.
- `docs/architecture.md`: one sentence in the section that describes session names (the `a2amx kill
  <id|name>` paragraph near line 305): generated client-side, by the `names` module, from the daemon's
  current session list.
- `docs/backlog.md`: Closed row `C22`, `a2amx new without --name generates a name`, `spec 3r`.

## 7. Out of scope

Daemon-side generation, a `Created` wire change, marking generated names in `list`, team-file name
generation, printing anything on the attach screen, an opt-out flag, changing recipient lookup by id,
retrying on a name clash, and renaming a session after start.

## 8. Acceptance

After implementation, from the main checkout, on the final tracked snapshot, with the contaminated
variables unset as in the earlier specs:

```sh
env -u A2AMX_BIN -u NO_COLOR cargo test
env -u A2AMX_BIN -u NO_COLOR cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

All pass with no warnings. Known-flaky PTY tests (see spec 2n) are rerun once and reported, not
hidden. Also report `git status --short` showing only the files in section 0's scope list. These
commands need the new code and run after implementation.

## 9. Pre-lock gate notes

- Facts checked against the code at `fdceee3`: `name` is `Option<String>` in `Command::New`
  (`src/cli.rs:43`); `create_session` holds `new_session_gate` from line 397 until after the insert,
  and rejects a taken name with `session name N is already in use` (lines 421–436 area);
  `request_sessions` (`src/main.rs:2497`) returns `SessionSummary` with `name: Option<String>`;
  the detach path prints the id alone (`src/main.rs:392`) and `tests/refs_and_list.rs:153` asserts
  `s1\n`; `random_hex` is private in `src/daemon.rs:1616`; `[detached from …]` messages print the
  reference the user typed (`state.session`), so tests attaching by `s1` keep passing.
- Word lists verified with a throwaway script: 64 unique adjectives, 64 unique nouns, all `[a-z]`,
  longest combined name 19 bytes, so every combination passes `validate_name` (also asserted by test 6).
- Not probed (needs the new code): that no other existing test depends on an unnamed CLI session;
  section 5 makes that a stop-and-report condition. The attach-time status bar showing the name was
  taken from the consultant's reading of `status.rs:55`, not run.
- Rules stated twice, to diff after the last edit: the index formula and wrap rule (D3, section 3 doc,
  tests 1–4); the stdout/stderr output rule (D7, tests 7–9).
