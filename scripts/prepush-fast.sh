#!/usr/bin/env bash
# Fast pre-push checks: secrets, dependency policy, workflow audit and repository Semgrep rules.
# Run before scripts/prepush-sonar.sh. CI runs this same script in its hygiene job, so a local
# pass means the same checks pass there. Needs gitleaks, cargo-deny, actionlint, zizmor and
# semgrep on PATH; a missing tool is an error, not a skip.
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in gitleaks cargo-deny actionlint zizmor semgrep; do
  command -v "$tool" >/dev/null || { echo "missing tool: $tool" >&2; exit 2; }
done

# Locally only the commits about to be pushed. In CI the whole history: actions/checkout sets the
# branch to track origin/<branch>, so upstream..HEAD would be empty on a push and scan nothing.
if [[ -z "${GITHUB_ACTIONS:-}" ]] && upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null)"; then
  gitleaks git --redact --no-banner --log-opts="${upstream}..HEAD" .
else
  gitleaks git --redact --no-banner .
fi
cargo deny --locked check --hide-inclusion-graph
actionlint
zizmor --no-progress .github
semgrep scan --config .semgrep/rules --metrics=off --disable-version-check --oss-only --error --strict --quiet \
  --include '*.rs' src
test_output="$(semgrep scan --test --config .semgrep/rules .semgrep/tests \
  --metrics=off --disable-version-check --oss-only --include '*.rs' 2>&1)" || {
  printf '%s\n' "$test_output" >&2
  exit 1
}
printf '%s\n' "$test_output"
test_count=0
if [[ "$test_output" =~ ([0-9]+)/[0-9]+: ]]; then
  test_count="${BASH_REMATCH[1]}"
fi
rule_files=(.semgrep/rules/*.yml)
if [[ "$test_count" -eq 0 || "$test_count" -ne "${#rule_files[@]}" ]]; then
  echo "Semgrep fixture count does not match rule count" >&2
  exit 1
fi
