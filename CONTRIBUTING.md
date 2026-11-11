# Contributing

## Issues

Bug reports and questions are welcome as issues. Please include the commit you built,
your platform, and the harness involved (Claude Code, Codex, OMP, or another command).
For vulnerabilities, use the private route in [SECURITY.md](SECURITY.md) instead.

## Pull requests

Pull requests from outside contributors, people or agents, are welcome. The project's own
team commits straight to `develop` through a spec-first workflow in which agents implement,
test, and review locked specifications. Dependabot update pull requests are the exception:
maintainers merge them. A pull request may wait for a while before it is reviewed. For anything
larger than a small fix, please open an issue first so the change can be agreed before you write it.

Design feedback and reproducible, sanitized compatibility findings are welcome. Identify
whether a suggestion changes a requirement, a proposal, or an open question.

## Building and testing

You need the Rust toolchain pinned in `rust-toolchain.toml`. Before relying on a change,
run all three; they must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Run `cargo test`, also inside an a2amx-hosted session; tests that
start the `a2amx` binary are refused as root. The OMP extension tests need Node 22 or newer
and skip without it unless `A2AMX_REQUIRE_NODE=1`.

See [AGENTS.md](AGENTS.md) for the rest of the repository commands and conventions;
the design is in [docs/architecture.md](docs/architecture.md).

## Public repository rules

This is a public repository. Use fictional identities and example hosts in docs and
fixtures. Keep credentials, personal infrastructure details, private agent conversations,
and local memory exports out of files, issues, commits, and pull requests. Keep runtime
databases and logs outside the checkout; review any proposed diagnostic attachment before
publishing it. Ignore rules are not a substitute for reviewing staged content.

## License

A2AMX is licensed under AGPL-3.0-or-later (see [LICENSE](LICENSE)).
