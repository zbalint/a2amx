# Spec 3dy: fix the documentation findings of the 0.3.0 audit

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop` (it carries three unpushed docs commits of the architect, 72861e4, 262264d, 912b208; they stay). Context: `a2amx-audit-0-3-0-2026-11-11`. Governing contract: this spec plus the two audit reports it is built from (tester m_6695 with its first-hand follow-up m_6697, consultant m_6693) and the owner's order "fix all" (2026-11-11). Owner decisions taken for the three open questions of the audit: the docs fix goes in one docs-only spec on `develop` and ships with the next release (the `v0.3.0` tag stays); the `delivery.md` restore sentence is corrected to the PTY channel and the code gap becomes a backlog row (no code change here); `SECURITY.md` gets the gateway boundary. No grilling record beyond the audit and these owner words.

Scope (files that may be edited, docs only): `README.md`, `SECURITY.md`, `CONTRIBUTING.md`, `docs/roadmap.md`, `docs/how-this-is-built.md`, `docs/install.md`, `docs/releasing.md`, `docs/using-with-agents.md`, `docs/backlog.md`, `docs/architecture.md`, `docs/teams.md`, `docs/confinement.md`, `docs/cli.md`, `docs/delivery.md`, `docs/remote-messaging.md`.

Does not touch: any file under `src/`, `tests/`, `scripts/`, `.github/`; `CHANGELOG.md` (docs only, no user-visible change); `docs/getting-started.md` (its sample headers still match the code; the audit calls the version line optional); the dated snapshots in `docs/how-this-is-built.md` lines 59-67 and 146-152; `docs/permissions.md` (its "draft, not locked" status is a design decision for the owner, not a fix); `docs/validation-plan.md`; `docs/specs/` other than this spec's own record; the 0.3.0 tag, release and release notes.

Public test seams: none. The judges are the acceptance commands in section 5, the per-sentence truth table the developer returns, the tester's per-sentence truth pass, and the architect's final review.

## 1. Why

The 0.3.0 audit (tester: reference pages against code; consultant: landing and process pages against workflows, scripts and history; architect: release artifacts) found the release artifacts sound and the docs drifting in three ways. The roadmap and the backlog still describe 0.3.0 as planned and remote messaging as an open idea, because the release commit edited only the changelog and `Cargo.toml`. A few process and reference claims are false or incomplete (every commit is the owner's identity; plain `a2amx list` is visibility filtered; the session token powers; the `no_default_hides` effect on the desktop bus). Several 0.3.0 facts are missing from the landing and security pages (README status line, SECURITY scope).

Lessons carried from earlier docs specs, stated here because the developer cannot read memory: every behaviour sentence is read from code by whoever writes it, never copied from an earlier spec or page; absolute words (always, never, only, requires) are the most common false claims, so each one needs its code line; a tester can misread, so each finding below was re-derived against code by the architect or the tester before it went into this spec, and the developer re-derives it again.

Rejected: changing code to fix the delivery restore gap (a separate spec, owner not asked for it); rewriting the dated snapshot pages; editing the CHANGELOG.

## 2. Mechanical changes

Each item names the page and line anchors read on 2026-11-11 at HEAD 912b208, the finding, the true statement and where the code shows it. The developer verifies every behaviour sentence it writes against the code line, edits the smallest text that makes the page true, keeps the page's voice, and marks any claim it cannot verify as unwritten in its report. Line anchors move as earlier edits land; search by the quoted text.

2.1 `docs/roadmap.md` (consultant 1-4).
- "Built today" (lines 9-29) gains remote messaging: an agent can message the paired human and the human can answer through a Discord gateway (opt-in per agent, confined sessions only, presence lease, background gateway with `gateway up|stop|status`), owner presence, viewer mode, and the remote cell of the attach status line. Link to `remote-messaging.md`. The 0.2.0 and 0.3.0 milestone sections (lines 39-45 and 47-68; line 70 is the 0.4.0 heading) leave the "planned" list and become a short "Released" list: one paragraph each, past tense, no "pending"; the 0.2.0 sentence "The flakes themselves are not fixed yet" stays (F88 is open). The intro at line 33 ("in the order they are planned") may be adjusted to fit.
- The sentence "the live test with a real Discord server is pending" is replaced by what `docs/remote-messaging.md` lines 3 says: tested live by the maintainers, with the three items not yet tested live.
- "starts when the operator brings a team up" (lines 59-61) becomes: `team up` prints a single-use code and the gateway hands it back to start the lease (code: `docs/remote-messaging.md` lease section; CHANGELOG [0.3.0] "Lease").
- Owner presence and viewer mode (F83) appear in the Released text of 0.3.0.
- "written as an architecture decision before 0.3.0 starts" (lines 113-116) is replaced by what happened: `docs/permissions.md` holds the first part of the design (draft, not locked) and 0.3.0 was built on its decisions D1 to D10.
- Engineering track: F16 (weekly Semgrep scan, spec 3ct) is described as done, as backlog F16 and F17 already say (`.github/workflows/semgrep.yml` has `schedule` and `workflow_dispatch`); F17 is already marked done and stays.

2.2 `docs/how-this-is-built.md`, `CONTRIBUTING.md`, `README.md` process claims (consultant 5, 10).
- how-this-is-built.md lines 30-33 "Every commit's author is the owner's git identity" and lines 91-93 "the project has no pull-request flow": the true statement is that agents commit through the owner's git identity, with two exceptions: one Scorecard-workflow commit under another author name (commit be78634), and Dependabot, whose 10 update commits reach `develop` through pull requests the architect merges (`git log --format=%an | sort | uniq -c` gives 566 `zbalint`, 10 `dependabot[bot]`, 1 other; do not write "merge commit"; AGENTS.md "Dependency updates"). Say that the pull-request flow exists only for Dependabot updates.
- CONTRIBUTING.md lines 12-13 "no pull request process of its own yet": add that Dependabot update pull requests are the exception and are merged by the maintainers.
- README.md line 94 "All code, tests and documentation are written by AI agents": add the Dependabot exception in the same parenthesis as the other two.
- README.md line 94 and how-this-is-built.md lines 97-99 "CI fails when the SonarQube quality gate fails": scope it as the CI already does (`ci.yml` SonarQube Scan `if:`, `sonar-project.properties` `sonar.qualitygate.wait=true`): the scan runs only on `master`, `develop` and pull requests from the same repository, and the gate fails the run only there.

2.3 `docs/install.md` (consultant 6, 12).
- Line 65 "`CARGO_PKG_VERSION` (currently `0.1.2`)": drop "currently" and the number (`Cargo.toml` line 3 is 0.3.0 and moves each release).
- Lines 14-27: the page says to download both Sigstore bundles but verifies only the archive's. Add the verification of `SHA256SUMS` with its own bundle (`cosign verify-blob --bundle SHA256SUMS.sigstore.json` with the same identity and issuer flags and `SHA256SUMS`), after confirming in `.github/workflows/release.yml` that the workflow signs both files (lines 110-111 name both bundles).

2.4 `docs/releasing.md` (consultant 11): lines 12-13 "wait for CI and the SonarQube quality gate to pass for the release commit", line 8 "Approve moving `master`" and line 25 "require the owner's approval" are conventions that `release.yml` does not check (it checks the tag format, the `Cargo.toml` version, `master` ancestry and a non-empty changelog section: `.github/workflows/release.yml` lines 21-36). Say so in one sentence, in the wording `how-this-is-built.md` already uses at lines 99-100 ("CI runs after the push, so it reports a problem; it does not stop a push"; quote that phrase, not line numbers).

2.5 `README.md` (consultant 8): the status line (line 15) names remote messaging through Discord (Claude Code and OMP, confined sessions; Codex cannot, `docs/remote-messaging.md` lines 35-36) as working, and viewer mode and owner presence get one clause each or one link-bearing clause together; the capability bullet at line 65 stays true and gains "Claude Code and OMP" only if the code and `remote-messaging.md` support it.

2.6 `SECURITY.md` (consultant 9): the "What is in scope" list (lines 16-27) gains the gateway boundary: the Discord bot token and its file, the pairing and the author check on Discord messages, lease codes, the `<state>/link/` directory exposed to the gateway sandbox, and the gateway sandbox itself, each as one bullet or one grouped bullet, pointing to `docs/permissions.md` (attack paths and decisions D1 to D10) and `docs/remote-messaging.md`. Every statement is checked against those two pages and the code lines they cite.

2.7 `docs/backlog.md` (consultant 7; owner "it is a known bug"):
- F81 moves from "Ideas and possible features" to "Closed", closed by specs 3de to 3dw and release 0.3.0, as F83 already is; follow the Closed table's column layout.
- F88 (timing flakes in `tests/attach_cli.rs`) gains `messages_cli_prints_submitted_and_undeliverable_rows`: one CI failure on 6240772 on the header assertion (tests/attach_cli.rs:1470), passed on one rerun and on 40 solo local runs, cause unknown.
- A new Issue row `I25`: on a channel-enabled Claude session the corrupted-submission draft restore never runs and the human_draft hold is never raised (restore only runs from `PtyChannel::prepare`, `src/delivery.rs:131-133`; `NativeChannel::prepare` is empty and `hold_reason` returns `None`, `src/delivery.rs:189,200-202`; the corrupted block still stores the draft, `src/daemon.rs:2057-2061`). Whether the human can recover that text is unverified; the row does not claim it. Reported by the audit tester, 2026-11-11. Status `reported`.

2.8 `docs/architecture.md` (tester 2, 3, 6, 7, 13, 14, 18):
- Lines 419-421 session-token powers: add `report_quota` and `bridge_attach` (`src/daemon.rs:2954-2961`).
- Lines 199-201: the operator peer-authorization line is also injected for OMP (`--append-system-prompt`, `src/harness.rs:605-609`) and Codex (`developer_instructions`, `src/codex.rs:330-332`).
- Lines 140-143: the `list_agents` field list adds the per-entry `confine` summary (`AgentSummary` in `src/wire.rs`) and `viewer` (`src/wire.rs:549-550`, `src/mcp.rs:473`), and a top-level `owner` beside `agents`, present only for a remote-enabled caller (`src/wire.rs:359-361`, `src/mcp.rs:234-237,473`).
- Lines 449-450: the send-path error codes add `owner_present` and `not_permitted` (`src/messaging.rs:283-284`).
- Lines 628-633: the dependency list names the gateway stack pinned in `Cargo.toml` (`ureq`, `tokio-tungstenite`, `futures-util`, `rustls`, `webpki-roots`); read the exact names and versions from `Cargo.toml`.
- Line 73 "Session runtime owns delivery execution" is in "Proposed topology" (the planned multi-host design), not the implemented section. Leave the row and add a parenthetical: today the delivery loop is `src/delivery.rs` (see the implemented messaging core section).
- The Limits bullet and the reset description gain the human rate limits (20 in, 30 out per minute, 8000 bytes: `src/messaging.rs:163-164`) and the reset-step bounds (at most 8 steps, 200 bytes each, `/`-prefixed, no control characters: `src/messaging.rs:381-397`).

2.9 `docs/teams.md` (tester 1, 12, 17):
- Lines 59-60: "omitted from `list_agents`/`list`" becomes `list_agents` and `a2amx list --as`; bare `a2amx list` shows every session (`src/daemon.rs:3010-3058`; the client filters only with `--team` or `--as`, `src/main.rs:2085-2109`).
- Lines 207-208: the daemon samples every running session's screen once a second (`sample_activity`, `ACTIVITY_POLL`, `src/daemon.rs:45, 813, 3499`) and latches the first ready screen (`src/session.rs:570-572`); write: a ready screen that lasts less than about a second can be missed.
- Lines 194-195 "ten to twenty seconds": delete that estimate; lines 193-194 already state the constants (1.5 seconds settle, 30 seconds timeout, `src/session.rs:30,33`).

2.10 `docs/confinement.md` and `docs/cli.md` (tester 4, 11, 15):
- confinement.md line 11 and line 295 and cli.md's sentence about `--confine-default-hides=false`: with `default_hides=false` the home credential hides and the desktop bus are dropped; the state and runtime directories, container sockets without a grant (`hide_containers`, `src/confine.rs:1664-1676`), `/run/WSL` and `/mnt/wslg` stay hidden (`src/confine.rs:1288-1300, 1538-1543`). Name `/run/dbus` literally in both pages. Take any safety statement about the bus from what `docs/confinement.md` already says about it; add none that the code or that page does not support.
- confinement.md line 534 "docker ps fails with EACCES on the socket": a hidden socket path is replaced by an empty read-only file (`MountKind::Empty`, `src/confine.rs:1934-1937, 2077-2084`), so the errno is not EACCES; reword without an errno unless a recorded run in the repo shows it.
- cli.md viewer section: add the yellow `VIEW` marker on the rule above the status line and the overlay text (`src/viewer.rs:88-91,144`).

2.11 `docs/delivery.md` (tester 5, 8): lines 163-164 and 409: the corrupted-submission restore as a paste without Enter once the session is idle, with a `human_draft` hold, is the PTY channel's behaviour (`src/delivery.rs:130-132`); on a channel-enabled Claude session (native channel) it does not run and no hold shows; say that, point to backlog `I25`, and drop or scope the "no human text is lost" sentence at about line 166 to the PTY channel. Lines 334-335: exit-event delivery only reaches watchers that can see the peer (`messaging::visible`, `src/daemon.rs:940-943`).

2.12 `docs/remote-messaging.md` (tester 9, 10, 16): line 336 Files table: `read_secure` (`src/relay_files.rs:228-256`) reads `discord.token`, `config.toml` and `pairing.json` only if the file is a regular file, not a symlink, owned by the user, with no group or other bits, at most 64 KiB; apply that to all three Files-table rows or one note under the table, not "mode 0600" on one row; line 314 troubleshooting: a missing `/etc/resolv.conf`, or one that is not a symlink, starts the gateway without a resolver bind and no error; a symlink whose target cannot be resolved or stat-ed, or whose target is not a regular file, fails with `cannot resolve /etc/resolv.conf; the gateway needs DNS`, as does any other error stat-ing the path itself; a symlink target under `/etc` or `/usr` needs no extra bind (`resolve_resolv_target`, `src/gateway_launch.rs:303-322`). The code never checks read permission; write nothing about readability. line 129: a declined pairing in the default `up --pair` flow prints both "pairing not confirmed" and "pairing did not complete; the gateway was not started" (`src/relay.rs:375-376`, `src/gateway_launch.rs:179-181`). Line 273 ("not yet tested" about Discord's websocket redirect) stays.

2.13 `docs/using-with-agents.md` (consultant 14): add a short paragraph naming `human:owner` as the address for messaging the paired human, the `owner_present` refusal while the owner is at the computer, the top-level `owner` field of `list_agents` (remote-enabled callers only), and a link to `remote-messaging.md`; every sentence read from `src/messaging.rs`, `src/daemon.rs` or `docs/remote-messaging.md`.

## 3. Out of scope

Everything in section 0's "does not touch" list, plus: changing the delivery code, adding the VIEW marker to the changelog, deciding `docs/permissions.md`'s status, refreshing `docs/getting-started.md` samples, a new docs page, a roadmap restructure beyond 2.1, and fixing any finding the developer cannot verify (it is reported, not written).

## 4. Trust-boundary note

Docs only; no code, no input path. The one text that states a security boundary is SECURITY.md (2.6): a wrong sentence there misleads researchers, so each statement carries its code line in the developer's truth table and is re-checked by the tester.

## 5. Acceptance

Baseline at draft time on the unchanged tree (every `rg` below excludes `docs/specs`, where older specs and this one quote the stale strings: `--glob '!docs/specs/**'`); `git status --porcelain` shows only this spec as untracked; `python3 -I` link and anchor check (below) reports 0 broken; `rg -n "live test with a real Discord server is pending" docs --glob '!docs/specs/**'` has 1 match (docs/roadmap.md); `rg -n "currently .0\.1\.2" docs/install.md` has 1; `rg -n "Every commit's author" docs --glob '!docs/specs/**'` has 1.

1. Scope: `git status --porcelain` lists only files from section 0's scope list (this spec file is committed before the ASSIGN, so it does not show; no new files expected), and `git diff --name-only` agrees.
2. Links and anchors: the architect's checker over `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `CHANGELOG.md` and `docs/*.md` finds 0 broken relative links and 0 broken `#fragment`s (the developer runs a `python3 -I -` snippet that resolves each `](path#frag)` against files and GitHub-style heading slugs; the tester and architect run their own).
3. Stale text gone: each of these returns no match: `rg -n "live test with a real Discord server is pending" docs --glob '!docs/specs/**'`, `rg -n "currently .0\.1\.2" docs/install.md`, `rg -n "Every commit's author is the owner" docs --glob '!docs/specs/**'`, `rg -n "the project has no pull-request" docs --glob '!docs/specs/**'`.
4. New text present, per file (a multi-file `rg` passes when one file matches, so use the loop; it prints nothing when all pass): `for f in docs/how-this-is-built.md CONTRIBUTING.md README.md; do rg -qi dependabot "$f" || echo "MISSING $f"; done`; `rg -q report_quota docs/architecture.md || echo MISSING`; `for f in docs/architecture.md docs/using-with-agents.md; do rg -q owner_present "$f" || echo "MISSING $f"; done`; `for f in docs/backlog.md docs/delivery.md; do rg -q I25 "$f" || echo "MISSING $f"; done`; `rg -qi gateway SECURITY.md || echo MISSING`; `for f in docs/confinement.md docs/cli.md; do rg -q 'run/dbus' "$f" || echo "MISSING $f"; done`; F81 position: `awk '/^## Closed/{c=1} c && /^\| F81 /' docs/backlog.md` prints one row and `awk '/^## Ideas/{i=1} /^## Closed/{i=0} i && /^\| F81 /' docs/backlog.md` prints none (F81 appears in other rows; a plain `rg F81` proves nothing).
5. Per-sentence truth table in the developer's report: every behaviour sentence added or changed, with `sentence -> file:line (code or workflow) -> true`. Sentences it could not verify are listed as not written.
6. The tester's pass (assigned after this) re-verifies the table sentence by sentence and runs the link check; the architect reads the real diff, removed lines included.
7. `cargo` is not required for a docs change; do not run it. No `CHANGELOG.md` line is added.
8. Flaky tests: none involved.

## Amendment 1 (developer BLOCKED m_6707, 2026-11-11)

2.12 resolv.conf wording replaced with the checks `resolve_resolv_target` actually makes (symlink resolution/stat failure or non-regular symlink target fail; a non-symlink or missing path passes; no readability check). The developer's finding was verified against `src/gateway_launch.rs:303-322`.

## Review record

Models: architect Claude Sonnet 5.5 (claude-sonnet-5-5); consultant on the claude harness (model not recorded); developer on OMP, working slot openai-codex/gpt-5.6-luna (advisor slot not exposed); tester on OMP, opencode-go/glm-5.3-flash. BLOCKED/amend cycles: 1.

| Where found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | teams.md readiness claim false: the daemon samples every running session | Spec 2.9 rewritten from `sample_activity` |
| Draft review | consultant | `default_hides=false` claim wrong: container sockets stay hidden, anchors off | Spec 2.10 rewritten |
| Draft review | consultant | Acceptance greps passed wrongly (multi-file `rg`; F81 already matches elsewhere) | Per-file loop and positional awk checks |
| Draft review | consultant | Wrong target (architecture.md:73 is Proposed topology), list_agents `viewer` and top-level `owner`, roadmap flakes sentence, read_secure covers three files, unproven "no text is lost", wording items (13 problems in all) | Folded into the spec before lock |
| Implementation | developer (BLOCKED m_6707) | Spec 2.12 resolv.conf claim false: `resolve_resolv_target` has no readability check; non-symlink paths pass | Amendment 1 (9185d4a); the draft and the consultant review both missed it |
| Architect review | architect | Roadmap "Built today" not updated (spec 2.1) | Rework R1 |
| Architect review | architect | Developer wrote that `/run/dbus` stays hidden with `default_hides=false` (false, contradicts the spec); developer truth table said true | Rework R2 |
| Architect review | architect | True content dropped without a destination (viewer note, candidate-dependency sentence, 0600-writes sentence, roadmap 0.2.0 IDs) | Rework R3 |
| Architect review | architect | Unrequested teams.md sentence, hardcoded Ctrl-B in the VIEW overlay text, releasing.md step merged | Rework R4 to R6 |
| Tester pass (addendum, after the push) | tester | Roadmap 0.2.0 sentence misstated I14 (fixed by spec 3cx before the release); SECURITY.md "How the project is built" lacked the Dependabot exception; decline output printed in the default flow only | Follow-up commit: sentence corrected, exception added, "In the default flow" added. The developer's I14 grouping and the spec's not-fixed-yet instruction both missed that I14 was closed |
