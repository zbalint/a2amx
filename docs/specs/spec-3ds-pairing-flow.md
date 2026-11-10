# Spec 3ds: pairing warns before it replaces a pairing, and says when it is done

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-live-test-fixes-2026-11-10`.

Scope (files that may be edited): `src/relay.rs` (the `pair` function only), `src/relay_files.rs` (only the visibility of `valid_id`, line 213: `fn` becomes `pub(crate) fn`), `tests/relay.rs`, `docs/remote-messaging.md`.

Does not touch: the rest of `src/relay_files.rs`, `src/main.rs`, `src/gateway_launch.rs`, the daemon, the pairing file format, running the gateway in the background (spec 3dt), `CHANGELOG.md`.

Public test seams: `a2amx::relay::run` with a `PairIo` (as `tests/relay.rs:283` does) against `FakeDiscord`, and the `gateway run --pair` binary (`tests/relay.rs:1278`).

## 1. Why

Two gaps from the first live Discord test (2026-11-10):

1. `gateway up --pair` on a setup that is already paired replaces `<state>/gateway/pairing.json` after one `y`, with no word that an existing pairing exists and is about to change. The pairing decides which Discord account may message the agents, so a silent replacement is a destructive step. `relay::pair` (`src/relay.rs:311-344`) never looks at `loaded.pairing`, and `relay_files::write_pairing` (`src/relay_files.rs:168`) overwrites the file.
2. After the owner confirms, the command prints nothing and keeps relaying (the relay loop starts right after `pair` returns, `src/relay.rs:51-69`), so the owner cannot tell that pairing worked.

Alternatives rejected: refusing to re-pair without `--force` (a second gate on the same decision; the warning plus the existing `y` prompt is enough, owner chose "at least warn"); deleting `pairing.json` as the way to re-pair (a manual step nobody would find).

## 2. `src/relay.rs`, function `pair`

Anchors at commit `dbc2233`. `pair` receives `loaded: &Loaded`, which carries `loaded.pairing: Option<Pairing>` (`src/relay_files.rs:22-29`).

2.1. When `loaded.pairing` is `Some(old)`, call `(io.announce)` once, before the line that announces the code, with exactly:
`warning: this gateway is already paired with Discord account {old.user_id} in server {old.guild_id}; confirming a new pairing replaces it, and the old account can no longer message your agents`

2.2. The confirmation prompt stays `bind Discord account {author}? [y/N]` when there is no existing pairing (an existing test asserts that exact text, `tests/relay.rs:333`). When there is an existing pairing it is `replace Discord account {old.user_id} with {author}? [y/N]`.

2.3. After `relay_files::write_pairing` succeeds (line 342), call `(io.announce)` once with exactly `paired with Discord account {author}; starting the gateway (Ctrl-C stops it)`. It says only what is true at that point: `run` still has to build the gateway state after `pair` returns. This line is printed only on success: a declined or timed-out pairing returns an error before it, as today.

2.4. Before the prompt is built, check that `event.author` is a valid Discord ID with `relay_files::valid_id` (digits only, 1 to 20 characters; make it `pub(crate)`. `src/discord.rs:459` keeps its own equivalent `valid_id`; merging the two copies is a backlog item, not part of this spec). If it is not, `bail!("pairing refused: the message author is not a Discord user ID")` and write nothing. Reason: the author string comes from the network and is printed to the owner's terminal in the prompt; `write_pairing` already rejects a non-ID later (`src/relay_files.rs:169`) but only after the prompt has printed it.

2.5. Nothing else in `pair` changes: the code, its ten-minute timeout, the author match, the file write and its mode.

## 3. Documentation

`docs/remote-messaging.md`, section "Pair your Discord account": say that pairing is a one-time setup step; that `--pair` on an existing pairing prints a warning naming the current account and server and asks `replace ... with ...?`; that after a confirmed pairing the command prints `paired with Discord account ...; starting the gateway` and keeps running as the gateway (stop it with Ctrl-C). Also say that moving to a different server needs `<state>/gateway/pairing.json` deleted first, because the gateway refuses a pairing whose server differs from `config.toml`.

## 4. Out of scope

Moving the pairing to a different Discord server: `relay_files::load` fails when `pairing.json` records a different `guild_id` than `config.toml` (`src/relay_files.rs:207-208`), before `pair` runs, so the warning's server ID always equals the configured one and re-pairing across servers still needs `pairing.json` deleted by hand. The docs say so (section 3); making `--pair` load past a mismatch is a separate spec.

Running the gateway in the background and any change to what `gateway up` does after pairing (3dt); a `--force` flag; reading or showing the old account's name (only its ID is known); the gateway log; the other 2026-11-10 items.

## 5. Acceptance

1. Tests in `tests/relay.rs` (red first; each fails on the current code). The announce capture in these tests records every line (the test at line 283 stores only the last one and takes the code with `rsplit_once(':')`, which would take the tail of the warning); the code line is found by its prefix `post this code in the control channel: `, as the binary test does at line 1335.
   a. With a valid `pairing.json` present (old account `111`, server `123`), `run` with a `PairIo` announces the section 2.1 warning before the code line; the `confirm` prompt is `replace Discord account 111 with 789? [y/N]`; after confirmation `announce` receives the section 2.3 line. When the announce closure receives the 2.3 line it also records whether `gateway/pairing.json` already holds user_id `789`; the test asserts that it did. After confirmation `pairing.json` holds guild `123`, user `789`, mode 0600.
   b. With no existing pairing, the prompt is `bind Discord account 789? [y/N]`, no warning line is announced, the 2.3 line is announced after the file holds the new pairing (same recorded check).
   c. A declined confirmation (`confirm` returns false) on an existing pairing leaves `pairing.json` byte-identical and announces no 2.3 line (the warning was announced).
   d. Abuse case (section 6 below): author `9\u{1b}[31mX` with the right code: `run` returns an error whose text is `pairing refused: the message author is not a Discord user ID`, the code line was announced, `confirm` was never called, no `pairing.json` is written.
   Command: `cargo test --test relay`.
2. Binary test (red first), extending `gateway_run_pair_binary_prompts_and_writes_record`: after the first confirmation, poll stderr for the section 2.3 line, then kill the child. Start a second child with `gateway run --pair` on the same home; push the printed code again from the fake server (it accepts successive connections; the developer verifies this in `tests/common/fake_discord.rs` before relying on it); stderr shows the section 2.1 warning and the prompt `replace Discord account 999 with 999? [y/N]`; answer `n`; the process exits non-zero with `pairing not confirmed` and `pairing.json` still holds user `999`.
3. `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`.
4. `git diff` touches only the files in section 0.
5. Mutation check (developer, assigned checkout, restore by checksum, `timeout 120` each): drop the warning (test a fails); keep the `bind` prompt text when a pairing exists (test a fails); announce the section 2.3 line before the `if !accepted` check (test c fails); announce it between that check and `write_pairing` (test a or b fails through the recorded file check); print the warning when no pairing exists (test b fails); remove the section 2.4 author check (test d fails). Name the failing test for each. Name the failing test for each.
6. Trust boundary. Attack path: the author ID of the Discord event that carries the pairing code is printed to the owner's terminal in the confirmation prompt without validation (terminal escape sequences in the prompt). Disposition: constrained by section 2.4 (a non-ID author is refused before anything is printed). Abuse test (red on the current code): `FakeDiscord` pushes the matching code with author `9\u{1b}[31mX`; `run` returns an error whose text is `pairing refused: the message author is not a Discord user ID`, `confirm` is never called, and no `pairing.json` is written. The old and new IDs in the section 2.1 and 2.2 lines come from `pairing.json`, which `relay_files::load` validates with the same function.
