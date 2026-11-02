#!/usr/bin/env bash
# Local Sonar gate before a push: same coverage steps as .github/workflows/ci.yml, then a scan
# of the committed HEAD as the scratch branch develop-prepush, measured against origin/develop.
# Needs SONAR_TOKEN, sonar-scanner and cargo-llvm-cov on PATH. Slow: it runs the whole test suite.
set -euo pipefail
cd "$(dirname "$0")/.."

# The scanner finds the new code from the merge base of HEAD and the local ref `develop`. The
# architect commits on local develop, so there that base is HEAD and the gate would pass over zero
# new lines. Scan a scratch clone of HEAD whose `develop` is origin/develop instead.
head_sha="$(git rev-parse HEAD)"
base_sha="$(git rev-parse --verify origin/develop)"
scratch="$(mktemp -d)"
trap 'rm -rf "$scratch"' EXIT
git clone -q . "$scratch"
git -C "$scratch" update-ref refs/heads/develop "$base_sha"
git -C "$scratch" checkout -q --detach "$head_sha"
merge_base="$(git -C "$scratch" merge-base HEAD develop)"
if [[ "$merge_base" = "$head_sha" ]]; then
  echo "HEAD is already in origin/develop: no new code to scan, the gate would be vacuous" >&2
  exit 2
fi

# --no-fail-fast and a failing test run still produce a report, as in CI; the exit code is kept
# and returned last so a red suite is never hidden behind a green gate.
tests=0
# A missing Node must fail the push gate, not skip the OMP extension tests.
A2AMX_REQUIRE_NODE=1 A2AMX_JS_LCOV=js.lcov cargo llvm-cov --locked --no-fail-fast --no-report || tests=$?

# A test that kills a child mid-write leaves a truncated .profraw that crashes llvm-profdata.
profdata="$(rustc --print sysroot)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"
for f in target/llvm-cov-target/*.profraw; do
  "$profdata" merge -sparse "$f" -o /dev/null >/dev/null 2>&1 || { echo "dropping unreadable $f"; rm -f "$f"; }
done
cargo llvm-cov report --lcov --output-path lcov.info

# Reports hold absolute paths; point them at the clone.
sed "s#$PWD/#$scratch/#" lcov.info > "$scratch/lcov.info"
if [[ -f js.lcov ]]; then sed "s#$PWD/#$scratch/#" js.lcov > "$scratch/js.lcov"; fi

cd "$scratch"
sonar-scanner \
  -Dsonar.branch.name=develop-prepush \
  -Dsonar.branch.target=develop \
  -Dsonar.qualitygate.wait=true

exit "$tests"
