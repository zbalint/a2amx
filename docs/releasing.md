# Releasing

Releases are built and published by GitHub Actions from an annotated version tag. The workflow
builds only a Linux x86_64 binary archive; it does not publish crates.io packages or other
platforms.

## Cut a release
1. Approve moving `master` for the release.
2. Update `Cargo.toml` and add the matching changelog section, then commit those release
   changes. A prerelease tag uses the same version and changelog section as the final release,
   with only the `-rc.N` suffix on the tag.
3. Push `master`, then wait for CI and the SonarQube quality gate to pass for the release
   commit.
4. Create and push an annotated tag:

   ```sh
   git tag -a vX.Y.Z -m "a2amx vX.Y.Z"
   git push origin vX.Y.Z
   ```

   Push the tag only after its commit is on `master`: the workflow rejects tags that are not
   ancestors of `origin/master`. A failed run cannot be repaired by overwriting the tag.

A trial tag such as `v0.1.1-rc.1` publishes a prerelease and is the recommended first test of
this workflow. Creating a tag and moving `master` require the owner's approval.

## What the workflow does

For a `vX.Y.Z` or `vX.Y.Z-suffix` tag, the `release` job checks out the complete history with
credentials disabled, validates the tag version, confirms that the tagged commit is on `master`,
and requires a non-empty matching changelog section. It installs the repository toolchain,
builds with `cargo build --release --locked`, and checks that the binary reports the tag.

It packages `a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` with `a2amx`, `LICENSE`, and `README.md`,
then writes `SHA256SUMS`. Cosign keyless-signs both the archive and checksum file with Sigstore
bundles. GitHub's build-provenance action attests the archive. Finally, `gh release create`
publishes the archive, checksums, both `.sigstore.json` bundles, and changelog notes; tags with a
hyphen are marked prereleases.

The job uses `ubuntu-22.04` so the binary links against glibc 2.35 and runs on more systems. If
GitHub retires that runner image, update the workflow and this note together.

## Verify a release

Download the archive, `SHA256SUMS`, and both Sigstore bundles from the GitHub release. Verify and
install as described in [Install a release](install.md#install-a-release).
