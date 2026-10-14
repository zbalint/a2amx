#!/usr/bin/env bash
# Local Sonar gate before a push: same coverage steps as .github/workflows/ci.yml, then a scan
# of the working tree as the scratch branch develop-prepush, measured against develop.
# Needs SONAR_TOKEN, sonar-scanner and cargo-llvm-cov on PATH. Slow: it runs the whole test suite.
set -euo pipefail
cd "$(dirname "$0")/.."

# --no-fail-fast and a failing test run still produce a report, as in CI; the exit code is kept
# and returned last so a red suite is never hidden behind a green gate.
tests=0
cargo llvm-cov --locked --no-fail-fast --no-report || tests=$?

# A test that kills a child mid-write leaves a truncated .profraw that crashes llvm-profdata.
profdata="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"
for f in target/llvm-cov-target/*.profraw; do
  "$profdata" merge -sparse "$f" -o /dev/null >/dev/null 2>&1 || { echo "dropping unreadable $f"; rm -f "$f"; }
done
cargo llvm-cov report --lcov --output-path lcov.info

sonar-scanner \
  -Dsonar.branch.name=develop-prepush \
  -Dsonar.branch.target=develop \
  -Dsonar.qualitygate.wait=true

exit "$tests"
