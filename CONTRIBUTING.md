# Contributing

## Issues

Bug reports and questions are welcome as issues. Please include the commit you built,
your platform, and the harness involved (Claude Code, Codex, OMP, or another command).
For vulnerabilities, use the private route in [SECURITY.md](SECURITY.md) instead.

## Pull requests

Pull requests are not accepted at the moment. The project is developed through a
spec-first workflow in which agents implement, test, and review locked specifications,
and an outside change does not fit it yet. This may change; an issue is the place to
propose something.

## Building and testing

You need the Rust toolchain pinned in `rust-toolchain.toml`. Before relying on a change,
run all three; they must pass with no warnings:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The conventions the code follows are in [AGENTS.md](AGENTS.md), and the design is in
[docs/architecture.md](docs/architecture.md).

## License

A2AMX is licensed under AGPL-3.0-or-later (see [LICENSE](LICENSE)).
