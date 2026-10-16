# Spec 3am: signed releases built by CI from a tag

## 0. Status

**LOCKED** (2026-10-16). Architect-directed, context `a2amx-signed-releases-2026-10-16`. Implementation goes to `a2amx-developer` only.

**Owner decisions (2026-10-16):** a release contains a Linux x86_64 binary tarball plus GitHub's source archive (no crates.io; `Cargo.toml` has `publish = false`); signing is Sigstore keyless (cosign bundle plus a GitHub build attestation, no key to hold); a release is a tag `vX.Y.Z` on a commit of `master`, built and published by a tag-triggered workflow. Creating the tag and moving `master` need the owner's explicit approval each time and are not part of this spec.

**Baseline:** develop at `8237e05` (the locking commit follows it and changes only this file). At that commit `env -u A2AMX_BIN scripts/prepush-fast.sh` exits 0: no leaks, `advisories ok, bans ok, licenses ok, sources ok`, and zizmor `No findings to report` (10 suppressed; it runs offline without a GitHub token). No Rust source changes, so `cargo test` is not part of this spec's acceptance; the developer runs `scripts/prepush-fast.sh` before the first edit and records the result.
**Location and branch:** main checkout `/home/zbalint/workspace/a2amx`, branch `develop`.
**Public test seams:** none in Rust. The workflow is checked statically (section 6) and proven by the first real release (section 7, architect-owned).

**Scope.** May edit exactly these files and no others.

- `.github/workflows/release.yml` (new)
- `docs/releasing.md` (new)
- `docs/install.md`: add a section for installing and verifying a release (section 4)
- `README.md`: one bullet in the Documentation list for `docs/releasing.md`

Does not touch: every other file, including `CHANGELOG.md`, `Cargo.toml`, `AGENTS.md`, `SECURITY.md`, `docs/backlog.md` and `.github/workflows/ci.yml` and `scorecard.yml` (the architect edits the changelog and backlog). No new dependency. Leave the diff uncommitted.

## 1. Why

The repository has a `v0.1.0` tag but no GitHub release, so a user has nothing to download and nothing to verify, Scorecard's Signed-Releases check scores -1, and the OpenSSF Best Practices criteria `release_notes` and `version_unique` have no release to point at. This workflow cannot release the existing `v0.1.0` tag: a tag-triggered run uses the workflow file as it exists at the tagged commit, and `v0.1.0` predates it. The first release it can make is the next version. (Scorecard recognises `.sigstore.json` assets; build attestations live in GitHub's attestation API, not as release assets, so expect Signed-Releases below the maximum. No Packaging improvement is promised.) This spec adds a workflow that builds the release binary from the tagged commit in CI, signs it without a long-lived key, and publishes a GitHub release with notes taken from `CHANGELOG.md`.

## 2. `.github/workflows/release.yml`

Match the conventions of `ci.yml`: actions pinned by commit SHA with the version in a trailing comment, `permissions: {}` at the top and per-job permissions, `persist-credentials: false`, no secret except `GITHUB_TOKEN`. The file must pass `actionlint` and `zizmor` (the CI hygiene job runs both).

Trigger and job:

- `on: push: tags: ["v*"]`. A tag is the only trigger. No `workflow_dispatch` (a dispatch needs the file on the default branch and cannot be tried before `master` has it).
- One job `release`, `runs-on: ubuntu-22.04` with a `# shortcut:` comment: pinned to the oldest supported runner so the binary links against an older glibc (2.35) and runs on more systems; upgrade when GitHub retires the image and say so in `docs/releasing.md`.
- Job `permissions`: `contents: write` (create the release), `id-token: write` (keyless signing), `attestations: write` (build attestation). Nothing else.
- Pass the tag to every shell step as `env: TAG: ${{ github.ref_name }}` and use `"$TAG"`, never `${{ }}` inside a `run:` script (template injection, zizmor).

Steps, in order, each a separate named step:

1. `actions/checkout` with `fetch-depth: 0` and `persist-credentials: false`. `build.rs` stamps the version from `git describe`, which needs the tags and history.
2. **Validate the tag.** Let `BASE` be `$TAG` without the leading `v` and without any `-suffix`. Fail unless all hold: `$TAG` matches `^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$`; `BASE` equals `version` in `Cargo.toml` (so `v0.1.1-rc.1` and `v0.1.1` share the version `0.1.1` and one changelog section); `git merge-base --is-ancestor HEAD origin/master` succeeds (the tagged commit is on `master`); the `CHANGELOG.md` section for `BASE` exists and has at least one non-blank line. A tag containing a hyphen is a prerelease.
3. Toolchain as in `ci.yml`: `rustup toolchain install` (reads `rust-toolchain.toml`). Use no cache action (no `Swatinem/rust-cache`): a release build must not restore a cache that other runs can write, and zizmor flags that.
4. `cargo build --release --locked`.
5. **Package.** Directory `a2amx-$TAG-x86_64-unknown-linux-gnu/` holding `a2amx`, `LICENSE` and `README.md`; archive `a2amx-$TAG-x86_64-unknown-linux-gnu.tar.gz`; `SHA256SUMS` with the archive's hash made by `sha256sum`. Fail unless `test "$(./a2amx --version)" = "a2amx $TAG"` where `./a2amx` is the built binary (`build.rs` stamps `git describe --tags --always --dirty`, so a clean tagged checkout prints `a2amx vX.Y.Z`, with the prefix and the `v`).
6. **Notes.** Extract the body of the section for `BASE` from `CHANGELOG.md` into `notes.md`. The section starts at the line beginning with the literal text `## [BASE]` (dots and brackets are not regex; the line continues with a space and a date, or ends), and ends before the next line that begins with `## ` (exactly two hashes and a space; `###` lines belong to the section) or at the end of the file, because the last section runs to the end. A prerelease gets the same text. Use `awk` or `sed` with a fixed-string match, not a regular expression built from the version.
7. **Sign.** Install cosign with `sigstore/cosign-installer` (pinned SHA, pinned cosign version; record the version in the report). `cosign sign-blob --yes --bundle <file>.sigstore.json <file>` for the archive and for `SHA256SUMS`. If the pinned cosign is a 2.x release, also pass `--new-bundle-format` so the `.sigstore.json` file is the Sigstore bundle format that `cosign verify-blob --bundle` and Scorecard expect; cosign 3.x writes it by default. Then `actions/attest-build-provenance` (pinned SHA) with the archive as subject.
8. **Publish.** `gh release create "$TAG" --verify-tag --title "$TAG" --notes-file notes.md` plus `--prerelease` when the tag has a hyphen, uploading the archive, `SHA256SUMS` and both `.sigstore.json` bundles. `GH_TOKEN: ${{ github.token }}` in `env`.

The developer picks the current pinned SHAs and versions of the actions and looks each up on its release page, recording them in the report; an unpinned `uses:` fails zizmor.

## 3. `docs/releasing.md` (new)

A how-to for the architect and owner, in the style of the other `docs/` pages, true today:

- **Checklist for cutting a release:** the owner approves moving `master`; push `master` first and wait for CI and the Sonar gate to be green for that commit; `Cargo.toml` and the `CHANGELOG.md` section carry the new version (a prerelease tag uses the same version and section as the final tag, with a `-rc.N` suffix on the tag only); then `git tag -a vX.Y.Z` and push the tag. The tag must be pushed after `master` has the commit, because the workflow checks that, and a failed run cannot be retried by overwriting the tag. A trial tag such as `v0.1.1-rc.1` publishes a prerelease first and is the way to test the workflow.
- **What the workflow does**, as the eight steps above in prose, and that it refuses a tag that is not on `master`.
- **Verify a release** (commands in section 4).
- The glibc note and that the runner image will need upgrading when GitHub retires it.

Do not describe behavior the workflow does not have. Do not invent a version, hash or URL; use `vX.Y.Z` placeholders.

## 4. `docs/install.md` and `README.md`

`docs/install.md` gains a section "Install a release" with: download the archive, `SHA256SUMS` and the bundles from the release; verify and install:

```sh
sha256sum -c SHA256SUMS --ignore-missing
cosign verify-blob --bundle a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sigstore.json \
  --certificate-identity "https://github.com/zbalint/a2amx/.github/workflows/release.yml@refs/tags/vX.Y.Z" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
gh attestation verify a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo zbalint/a2amx
```

and states the requirement: Linux x86_64, glibc 2.35 or newer. It says the existing build-from-source route is unchanged. `README.md` gets one Documentation bullet: `[Releasing](docs/releasing.md) — how releases are cut, signed and verified.`

## 5. Out of scope

- Creating any tag, release or `master` commit; the first release is the owner's call (section 7).
- Other platforms or architectures, a musl build, crates.io, a container image, SLSA generic provenance, GPG.
- Changing `ci.yml`, version numbers, the changelog, `AGENTS.md`, `SECURITY.md` or any Rust source.
- Updating the Scorecard workflow or Dependabot config.

## 6. Acceptance (static; the workflow cannot run before a tag exists)

Run from `/home/zbalint/workspace/a2amx` after the edits:

```sh
scripts/prepush-fast.sh                 # gitleaks, cargo-deny, actionlint, zizmor on the new workflow
actionlint .github/workflows/release.yml
zizmor --no-progress .github/workflows/release.yml     # offline: it skips the online audits
rg -n 'uses: ' .github/workflows/release.yml        # every line has a 40-hex SHA and a version comment
rg -n '\$\{\{' .github/workflows/release.yml        # only in env:, with: and if:, never inside run:
git status --short                      # only the four files of section 0 (and nothing else)
git diff --stat
```

`scripts/prepush-fast.sh` passes with no warning (its gitleaks step scans committed history, not the uncommitted file; actionlint and zizmor scan the file); `zizmor` reports no finding at any severity or each remaining one is listed in the report with a reason. Because zizmor runs offline without a token, a mistyped or fork-only SHA would pass every static check, so also confirm each pinned action SHA exists in its repository: for each `uses:` line run `gh api repos/OWNER/REPO/commits/SHA --jq .sha` and check it prints the SHA, or run zizmor once with `GH_TOKEN` set for its online audits (never print the token). A local dry run of steps 4 and 5 by hand (`cargo build --release --locked`, `tar`, `sha256sum` in a scratch directory) shows the archive layout. Its `a2amx --version` will print `v0.1.0-NN-g<sha>-dirty` because the checkout has no new tag and modified files; the developer reports that and does not create a local tag. The developer does not install cosign or push a tag.

**Post-implementation, architect-owned:** the first real run is a prerelease tag (`v0.1.1-rc.1`) on `master` after the owner approves; the architect then runs the verification commands of section 4 against the published assets and records the result in the review record.

## 7. Notes for the developer

- `build.rs` runs `git describe`; a shallow checkout would stamp the wrong version, hence `fetch-depth: 0`.
- A mistake here cannot be fixed by a retry that overwrites the tag; the workflow is deliberately strict in step 2. If a step fails after the release exists, report it instead of re-running.

## Review record

**Models.** Architect: Claude Sonnet 5.5. Consultant: Claude Opus 5.5. Developer: reported planner and worker `gpt-5.6-luna`; its advisor model is not exposed to the session. Tester: reported planner and worker `gpt-6.1-sol` (the tester's config assigns a GLM model to its working slot, so this report and the config disagree; unresolved). **BLOCKED/amend cycles:** 0.

| Found in | Found by | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | Consultant | A prerelease tag (`-rc.1`) could not pass the version and changelog checks, though the trial plan used one | Compare the tag's base version; one changelog section for rc and final |
| Draft review | Consultant | `--version` check needed the exact string (`a2amx vX.Y.Z`) and the local dry run cannot match it | Exact test added; local dry run documented as dirty |
| Draft review | Consultant | "Toolchain as in ci.yml" invited copying the cache action, which zizmor flags in a publishing workflow | "No cache action" added |
| Draft review | Consultant | Changelog extraction could stop at `### Added` and produce empty notes | Start and end of the section defined; fixed-string match |
| Draft review | Consultant | Tagging before moving `master` fails the run and cannot be retried | Checklist order: push `master`, wait for CI, then tag |
| Draft review | Consultant | Offline zizmor does not catch a mistyped action SHA | SHA existence check added to acceptance |
| Draft review | Consultant | Claims about Scorecard Packaging and the `v0.1.0` release were wrong or misleading | Removed; `v0.1.0` limit stated |
| Implementation | Developer (advisor) | Checklist order in `docs/releasing.md` put the tag before the version commit; artifact names used a real version | Order fixed; placeholders used |
| Tester pass | Tester | `docs/how-this-is-built.md` still said the CI Sonar result does not fail the run, stale since commit `8237e05` | Corrected by the architect |
| Architect review | Architect | The README sentence about the Sonar gate was incomplete after the same change | Corrected by the architect |
| After the push | Architect | First prerelease `v0.1.1-rc.1` (run 38847380931, 2m19s) published a prerelease with the archive, `SHA256SUMS` and both `.sigstore.json` bundles. Checks: `sha256sum -c` OK; both bundles verify with the documented identity and issuer using sigstore-python (`cosign` itself was not installed); one build attestation exists in the GitHub API; the extracted binary reports `a2amx v0.1.1-rc.1`; the release notes match the changelog section | None needed for the workflow |
| After the push | Architect | `gh attestation verify` could not be run: the packaged `gh` 2.46 has no `attestation` subcommand | Note added to `docs/install.md`; the command stays unproven locally |

