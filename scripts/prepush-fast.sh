#!/usr/bin/env bash
# Fast pre-push checks: secrets, dependency policy, workflow audit, repository Semgrep rules and an
# advisory CodeScene report.
# Run before scripts/prepush-sonar.sh. CI runs this same script in its hygiene job, so a local
# pass means the same checks pass there. Needs gitleaks, cargo-deny, actionlint, zizmor, semgrep,
# cs (CodeScene) and jq on PATH; a missing tool is an error, not a skip. The CodeScene step at the
# end only reports: it never fails the script.
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in gitleaks cargo-deny actionlint zizmor semgrep cs jq; do
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

# CodeScene code health, advisory: the owner decided it must never block a push or a CI run, since
# the existing large files would fail every change. It lists the findings the pushed commits
# introduced or made worse. CODESCENE_BASE overrides the base (CI sets it); locally the base is
# the upstream branch, so the range is the commits about to be pushed.
base="${CODESCENE_BASE:-$(git rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || echo origin/develop)}"
status=0
# The timeout keeps a stalled licence or network call from cancelling the CI job as failed.
report="$(CS_DISABLE_VERSION_CHECK=1 timeout 300 cs delta "$base" HEAD --output-format json)" || status=$?
if [[ "$status" -eq 0 ]] && findings="$(jq -r '
  .[] | .name as $file | .findings[]
  | select(.["change-type"] == "introduced" or .["change-type"] == "degraded")
  | "\($file): \(.["change-type"]) \(.category): \(.["change-details"][0].description // "")"' <<<"$report")"; then
  summary="CodeScene (advisory) since ${base}: ${findings:-no introduced or degraded findings}"
else
  summary="CodeScene skipped: cs delta against ${base} or reading its output failed (status ${status}); this step is advisory"
fi
printf '%s\n' "$summary"
if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
  printf '%s\n' "$summary" >> "$GITHUB_STEP_SUMMARY"
fi
