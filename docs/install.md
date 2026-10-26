# Installation and live builds

How-to for operators installing a stable executable separately from development builds.

## Install

From a clone, with Rust 1.88 or newer:

```sh
scripts/install.sh
~/.a2amx/bin/a2amx --version
```

## Install a release

Download the archive, `SHA256SUMS`, and both Sigstore bundles from the GitHub release. Releases
support Linux x86_64 with glibc 2.35 or newer:

```sh
sha256sum -c SHA256SUMS --ignore-missing
cosign verify-blob --bundle a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz.sigstore.json \
  --certificate-identity "https://github.com/zbalint/a2amx/.github/workflows/release.yml@refs/tags/vX.Y.Z" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
gh attestation verify a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz --repo zbalint/a2amx
tar -xzf a2amx-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
install -Dm755 a2amx-vX.Y.Z-x86_64-unknown-linux-gnu/a2amx ~/.a2amx/bin/a2amx
```

`gh attestation verify` needs a recent GitHub CLI (the `attestation` subcommand is missing from
older packaged versions, for example 2.46). Without it, the `cosign` check above still verifies the
signature and the workflow identity. The build-from-source route above is unchanged. Start the installed binary from
`~/.a2amx/bin/a2amx`, not from `target/`.

## Dev build versus live install

Build and test from the repository with `cargo build` and `cargo test`. Install a release
copy with `scripts/install.sh`, then run the daemon from `~/.a2amx/bin/a2amx` and point
your `amx` alias or launcher at that installed binary. `A2AMX_INSTALL_DIR` overrides the
install directory; the state directory remains separate.

Never run the daemon from `target/`: development builds replace that executable and can
break the MCP command of a later Codex session. Install freely; the atomic install leaves
a running daemon using its old image. Restart deliberately to pick up the new version,
because restarting the daemon ends all sessions. The script leaves PATH, aliases, services,
and daemon start or stop to the operator.

Each build carries its git description (`a2amx --version`), and the running daemon reports
the same value in `a2amx daemon status`. A `-dirty` suffix marks a build from a modified
source tree. Status writes a warning to stderr when the daemon was built from a different
version and recommends a restart; restarting ends all sessions. When a source tarball has
no git metadata, `build.rs` uses `CARGO_PKG_VERSION` (currently `0.1.2`); only when that
value is absent or empty does it fall back to `0.0.0`. A package-version fallback cannot be
compared with a git-described daemon version.

For development checks, see [Contributing](../CONTRIBUTING.md#building-and-testing).
