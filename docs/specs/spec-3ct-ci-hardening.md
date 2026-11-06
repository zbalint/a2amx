# Spec 3ct: The release build cannot sign, and a weekly broad Semgrep scan

## 0. Status

LOCKED.

Scope (files that may be edited or created): `.github/workflows/release.yml` (split into two jobs),
`.github/workflows/semgrep.yml` (new). Does not touch: `.github/workflows/ci.yml`,
`.github/workflows/scorecard.yml`, `.github/dependabot.yml`, `scripts/`, `.semgrep/`, `src/`,
`tests/`, `docs/` (the architect updates `docs/backlog.md`, `docs/roadmap.md` and `CHANGELOG.md`),
any other spec.

Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context id:
`a2amx-ci-hardening-2026-11-06`. Public test seams: none in Rust; the checks are the linters the
repository already runs (`actionlint`, `zizmor`, through `scripts/prepush-fast.sh`).

## 1. Why

Backlog F17: `cargo build --release --locked` runs every dependency's `build.rs` and proc-macros in
the one `release` job, and that job holds `id-token: write`, `attestations: write` and
`contents: write`. Code that runs during the build can request a GitHub OIDC token and obtain a
Sigstore certificate for the identity `release.yml@refs/tags/vX.Y.Z`, the identity `docs/install.md`
tells users to trust, or can publish a release. Nothing in the build step needs those permissions.
The `check` job in `ci.yml` already keeps `SONAR_TOKEN` out of the compile step; this closes the
release side.

Split the work in two jobs. `build` compiles with `contents: read` only and uploads the archive. `publish`
holds the three write permissions, never compiles and never runs repository code, and signs,
attests and publishes what `build` uploaded. The signing identity stays `release.yml@refs/tags/<tag>`
because both jobs live in the same workflow file, so `docs/install.md` and the cosign and
`gh attestation verify` commands do not change. A compromised build can still produce a bad
binary; what it can no longer do is mint a signature or publish by itself. The binary is built
from the tagged commit as before, and a tagged commit is reviewed source (rule 8 of the
philosophy: the user's own trust in a dependency is theirs). The `SHA256SUMS` file is checked
again in `publish` so a corrupted transfer is caught before signing.

Backlog F16 (owner decision 2026-11-05: include, cheap): a weekly and manual workflow that runs
Semgrep's community rules over the repository and uploads SARIF to code scanning, like Scorecard.
It is non-blocking; someone reads it before each release.

Rejected: building in a container with no token (the runner token is the exposure, not the
network); a third job for notes (the notes come from `CHANGELOG.md` in the checkout, so they are
extracted in `build` and uploaded with the archive, and `publish` needs no checkout); pinning
`semgrep` transitive dependencies (as `ci.yml` and `zizmor`, `pipx` pins only the top package;
accepted); `--config auto` (it refuses `--metrics=off`; the explicit `p/default` pack runs with metrics off and
no login); making the scan fail the run (blocking is the repository-rules gate in
`prepush-fast.sh`, F15).

Trust boundary. The artifact crosses from `build` (runs untrusted dependency code) to `publish`
(holds signing authority). Paths and disposition: (1) the build job requests an id-token: removed
(permissions `contents: read` only); (2) the build job pushes to the repository or releases:
removed (read only, and `persist-credentials: false` already); (3) the build job overwrites
`SHA256SUMS` so it matches a malicious archive: not removable, because the build is what makes the
archive; accepted (the binary comes from the tagged source and dependency set the owner chose, and
users verify the identity, not the content); (4) a file in the artifact named like a path
(`../x`) escapes the download directory: constrained by `download-artifact`'s own extraction
checks and by the upload step listing exactly three paths; `publish` then reads and uploads only
the three fixed names (`test -f` checks presence, it is not the guard); (4a) the boundary relies on
`publish` running on a fresh GitHub-hosted VM, so a process left by a build script dies with the
build VM (a self-hosted runner would break this; none is used); (4b) `download-artifact` by
`name` without a run id reads only this run's artifacts, so no other run can plant one; (5)
`publish` runs a script from the artifact: removed (it only runs `sha256sum`, `cosign`, `gh`).

## 2. `.github/workflows/release.yml`

Keep `name`, the `on` trigger and top-level `permissions: {}`. Replace the single `release` job
with two jobs.

Job `build` (`runs-on: ubuntu-22.04`, with the existing `# shortcut:` comment kept above it;
`permissions: { contents: read }`): the existing steps **Check out tagged source**, **Validate the
tag**, **Install Rust toolchain**, **Build release binary**, **Package release binary** and
**Extract release notes**, unchanged (the `Validate the tag` step needs `fetch-depth: 0` and
`origin/master`, both already in place). Append one step:

```yaml
      - name: Upload release files
        uses: actions/upload-artifact@cf430e030ddbb5b0abf93d22962f4752f3646cd9 # v7.0.2
        with:
          name: release-files
          path: |
            a2amx-${{ github.ref_name }}-x86_64-unknown-linux-gnu.tar.gz
            SHA256SUMS
            notes.md
          if-no-files-found: error
          retention-days: 7
```

Job `publish` (`runs-on: ubuntu-22.04`, `needs: build`, permissions `contents: write`, `id-token:
write`, `attestations: write`; no checkout): steps in this order.

1. **Download release files**: `actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2`
   with `name: release-files`, `path: release`.
2. **Check the files** (`working-directory: release`, `env: TAG: ${{ github.ref_name }}`):
   `test -f "a2amx-$TAG-x86_64-unknown-linux-gnu.tar.gz"`, `test -f SHA256SUMS`, `test -f notes.md`,
   then `test "$(wc -l < SHA256SUMS)" = 1`, `grep -qE "^[0-9a-f]{64}  a2amx-$TAG-x86_64-unknown-linux-gnu\.tar\.gz$" SHA256SUMS`
(it names the archive) and `sha256sum -c SHA256SUMS`.
3. **Install cosign**: the existing step, unchanged.
4. **Sign release assets**: the existing step with `working-directory: release`.
5. **Attest build provenance**: the existing step with `subject-path:
   release/a2amx-${{ github.ref_name }}-x86_64-unknown-linux-gnu.tar.gz`.
6. **Publish release**: the existing step with `working-directory: release` and one added env
   `GH_REPO: ${{ github.repository }}` (there is no checkout, so `gh` cannot read a git remote).

Every other line of those steps stays as it is, including `--verify-tag`.

## 3. `.github/workflows/semgrep.yml` (new)

Model it on `scorecard.yml`: top-level `permissions: {}`, `name: Semgrep broad scan`, triggers
`schedule: - cron: '17 3 * * 1'` (Monday 03:17 UTC) and `workflow_dispatch:`. One job `scan`
(`runs-on: ubuntu-latest`, `timeout-minutes: 20`, permissions `contents: read` and
`security-events: write`). Steps: checkout (`actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`,
`persist-credentials: false`); **Install semgrep** (`env: SEMGREP_VERSION: 1.181.0`, run
`pipx install "semgrep==${SEMGREP_VERSION}"`, same version as `ci.yml`); **Scan**
(`env: SEMGREP_ENABLE_VERSION_CHECK: "0"`, `SEMGREP_SEND_METRICS: "off"`; run
`semgrep scan --config p/default --oss-only --metrics=off --disable-version-check --sarif --output semgrep.sarif`,
no `--error`, so findings do not fail the run); **Upload SARIF** with
`github/codeql-action/upload-sarif@9b3254085494b55a0f7c4c2b3d2d52cc712ce370 # v4.39.1` and
`sarif_file: semgrep.sarif`, `category: semgrep-broad`.

Scheduled workflows run only on the default branch; until `master` has this file the schedule does
not fire, and `workflow_dispatch` is the manual path (the workflow must exist on the default branch before it can be dispatched at all). That is
expected and not a defect.

## 4. Out of scope

Changing the signing identity, `docs/install.md`, the cosign or `gh` verification commands, the
runner image, the toolchain pin, `ci.yml`, Scorecard, Dependabot, caching the cargo build (a cache
shared with `publish` would reintroduce a path from the build into the signer), a Semgrep
Supply Chain scan, fuzzing (F10, moved past 0.2.0), the first release run (the owner's tag).

## 5. Acceptance

All run by the developer, and again by the tester and the architect, from the checkout:

1. `scripts/prepush-fast.sh` exits 0 (runs `actionlint`, `zizmor`, `gitleaks`, `cargo-deny`, Semgrep
   rules and their tests). Judge: the exit code and its output.
2. `actionlint` (no arguments, as the script runs it) exits 0.
3. `zizmor --no-progress .github` (as the script runs it) exits 0 with no finding.
4. `rg -n 'id-token|attestations: write|contents: write' .github/workflows/release.yml` matches only
   lines inside the `publish` job (the tester reads the file and confirms; the `build` job has
   `contents: read` and nothing else).
5. `rg -n 'cargo ' .github/workflows/release.yml` matches only lines inside the `build` job.
6. `rg -n 'download-artifact|upload-artifact' .github/workflows` shows both with the full SHAs of
   section 2 and a version comment (the `download-artifact` SHA was confirmed as the `v8.0.2` tag commit through the
   GitHub API by the architect before lock).
7. `rg -n 'needs: build' .github/workflows/release.yml` and `rg -n -- '--verify-tag'
   .github/workflows/release.yml` and `rg -n 'GH_REPO' .github/workflows/release.yml` each match
   once; `rg -n 'security-events: write' .github/workflows/semgrep.yml` matches once.
8. `git status --short` lists exactly `.github/workflows/release.yml` and
   `.github/workflows/semgrep.yml`.
9. Post-push (the architect, not the developer): the workflow cannot run before a tag. The
   `v0.2.0-rc.1` run is the first proof; the architect checks that run, that `publish` shows
   `Verified`/signed bundles and an attestation, and the documented `cosign verify-blob` identity
   still passes (owner approval for the tag is separate).

Unprobed before lock: the `p/default` scan command could not be run in the architect's confined
session (the registry fetch did not finish in 120 s, no SARIF written). Its flags are the ones
`ci.yml` and `prepush-fast.sh` already use plus `--sarif --output`, and `--config p/default` is the
documented public pack; the first `workflow_dispatch` after the file reaches `master` is its proof,
and a failure there is a follow-up fix, not a blocker for the release.

Mutation check (tester): on the checkout, restore each by checksum. (a) Add `id-token: write` to
`build`: check 4 must find it outside `publish`. (b) Move `Build release binary` into `publish`:
check 5 must fail. (c) Remove `needs: build` from `publish`: check 7 must fail. (d) Drop `--verify-tag`: check 7 must
fail. (e) Remove `security-events: write` from `semgrep.yml`: check 7 must fail. (f) Make
`SHA256SUMS` list only `notes.md` in a scratch copy of the step's shell lines: the new checks in
step 2 of `publish` must exit non-zero. Checks 4 and 5 are judgments of the reader (the tester
reads the file), not exit codes.

Known flaky tests: none involved; no Rust test runs for this spec.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude Opus 5.5; developer OMP
`openai-codex/gpt-5.6-luna` (slot names not exposed). No tester pass: the diff is two workflow files,
the developer ran mutants (a) to (f) and the architect re-ran the checks (the tester's weekly quota was
low and the mutation targets are greps and linters). BLOCKED/amend cycles: 0.

| Where found | By (role, model) | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant, Opus 5.5 | Trust path 4 credited `test -f` as the guard against a path-escaping artifact member; the real guard is the download action's extraction | Reworded, paths 4a and 4b added |
| Draft review | consultant, Opus 5.5 | `sha256sum -c` passes when `SHA256SUMS` names a different file than the archive | One-line and archive-name checks added to `publish` |
| Draft review | consultant, Opus 5.5 | Mutations (c) to (e) had no mechanical check; reasons for rejecting `--config auto` and for `workflow_dispatch` were worded wrongly | `rg` checks added, wording fixed |
| Architect review | architect, Sonnet 5.5 | `docs/releasing.md` still described a single `release` job and the roadmap still listed fuzzing and the flaky-test fix before 0.2.0 | Architect updated both docs and the roadmap in the commit |

Unproven until the first run: the `p/default` scan (needs the file on `master` for a manual dispatch) and
the `publish` job (needs the `v0.2.0-rc.1` run). A failure in either is a follow-up fix.
