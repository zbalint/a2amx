#!/usr/bin/env bash
# Fast pre-push checks, seconds not minutes: secrets, dependency policy, workflow lint and audit.
# Run before scripts/prepush-sonar.sh. CI runs this same script in its hygiene job, so a local
# pass means the same checks pass there. Needs gitleaks, cargo-deny, actionlint and zizmor on PATH;
# a missing tool is an error, not a skip.
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in gitleaks cargo-deny actionlint zizmor; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done

# Only the commits about to be pushed; the whole history when there is no upstream (CI checkout).
if upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null)"; then
  gitleaks git --redact --no-banner --log-opts="${upstream}..HEAD" .
else
  gitleaks git --redact --no-banner .
fi
cargo deny --locked check --hide-inclusion-graph
actionlint
zizmor --no-progress .github/workflows
