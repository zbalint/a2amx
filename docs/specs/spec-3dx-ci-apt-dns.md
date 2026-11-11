# Spec 3dx: CI survives a runner that cannot resolve the Ubuntu mirrors

## 0. Status

LOCKED. Location: the main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Context: `a2amx-ci-apt-dns-2026-11-11`. No grilling record beyond the owner's two phone messages of 2026-11-11 (m_6658 "can we set the dns server in a runner?", m_6660 "lets spec it now"); the design below follows the architect's reply m_6659 (DNS step plus apt retries).

Scope (files that may be edited): `.github/workflows/ci.yml` (the one step "Install bubblewrap and allow unprivileged user namespaces", lines 107-117, and its comment).

Does not touch: any other step or job in `ci.yml`, `release.yml`, `scorecard.yml`, `semgrep.yml`, `scripts/`, `src/`, `tests/`, `CHANGELOG.md` (not user-visible), `docs/` (no page describes the CI step: `rg -n "apt-get" docs` finds nothing; the `bubblewrap` mentions in `docs/remote-messaging.md` are about the runtime requirement of `gateway up`).

Public test seams: none in Rust. The judges are `actionlint` and `zizmor` (the pre-push gate), and the real `check` job on GitHub after the architect pushes.

## 1. Why

From 2026-11-11 13:53 UTC the `check` job fails on `develop` f1d9cd9 (run 43033216623) and on both dependabot PRs (15, 17), including after three reruns. The failing step is "Install bubblewrap and allow unprivileged user namespaces": `apt-get update` prints `Temporary failure resolving 'azure.archive.ubuntu.com'` (also `archive.ubuntu.com` and `security.ubuntu.com`), `apt-get install` then cannot fetch `bubblewrap_0.9.0-1ubuntu0.3_amd64.deb`, and the job ends with exit 100. `hygiene` and `msrv` pass; the same job resolved crates.io minutes earlier; githubstatus.com reported no incident. The job runs on `ubuntu-latest`; the image was `ubuntu-24.04` 20261101.360.1. The last green push was 05:05 UTC. Each failed run also spent about 3 minutes in apt timeouts.

Owner request: set the DNS server in the runner. Decision: the step resolves through the runner's own resolver first and only when that cannot resolve an Ubuntu mirror name does it add public resolvers, then runs apt with retries and short timeouts. Rejected: always setting public DNS (changes the normal path for a problem that is not constant); downloading a `.deb` from a pinned URL (a second source of truth for the package version and its hash); caching the `.deb` in `actions/cache` (a cold cache hits the same outage; a cached binary is trusted without apt's signature check); dropping the sandbox tests in CI (`A2AMX_REQUIRE_BWRAP=1` exists on purpose).

Cause is not proven: whether it is the runner's DNS or a mirror outage is not known from outside, and no probe of a runner is possible before the push. The step therefore prints its resolver state, so the next failure is diagnosable from the log.

## 2. Mechanical changes

2.1. `.github/workflows/ci.yml`, replace the step's `run:` body (current lines 113-117) and extend its comment (lines 107-110) with one sentence naming this spec and the outage. The job runs on `ubuntu-latest` (currently 24.04, ci.yml:87). The body runs under the default `bash -e` shell of Actions, without pipefail, so every diagnostic or best-effort command that may fail carries its own `|| echo "<what failed>"` (or `|| true` for pure output), and only the apt install and the final three lines may end the step. Order:

1. Diagnostics, each guarded: `resolvectl status | head -n 20 || true`, `ip -o route show default || true`, `cat /etc/apt/apt-mirrors.txt || true`.
2. A resolver check: `if ! getent hosts azure.archive.ubuntu.com >/dev/null || ! getent hosts archive.ubuntu.com >/dev/null; then` (`azure.archive.ubuntu.com` is the mirror the runner is configured with, `archive.ubuntu.com` the plain one; either failing triggers the fallback). Inside, in order:
   - print `resolver cannot reach the Ubuntu mirrors; adding public DNS`;
   - `iface=$(ip -o route show default | awk '{for(i=1;i<NF;i++) if($i=="dev"){print $(i+1); exit}}')`, which exits 0 and leaves `iface` empty when there is no match;
   - `if [ -n "$iface" ]; then sudo resolvectl dns "$iface" 1.1.1.1 8.8.8.8 || echo "resolvectl dns failed"; sudo resolvectl flush-caches || echo "resolvectl flush-caches failed"; else echo "no default-route interface"; fi`;
   - the check again, each name guarded: `getent hosts azure.archive.ubuntu.com || echo "still unresolved: azure.archive.ubuntu.com"` and the same for `archive.ubuntu.com`.
   None of these ends the step; the fallback is best effort and apt below decides the outcome.
3. `sudo apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 update -qq`, then the same options on `sudo apt-get install -y --no-install-recommends bubblewrap`. The install fails the step as today when the package cannot be fetched. Expectation, stated so nobody over-reads it: the retries help with a flaky mirror and the DNS fallback with the resolver; the timeouts bound connect and transfer stalls, not name resolution, so neither bounds the total time; the job's `timeout-minutes: 45` stays the outer limit.
4. Unchanged: `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`, `bwrap --version`, `unshare -Ur true`.

`actionlint` runs `shellcheck` over `run:` blocks, so quoting findings fail acceptance check 1; write the script so it passes.

The step name does not change.

## 3. Out of scope

Any change to what the tests require (`A2AMX_REQUIRE_*`), caching the package, a runner image or `runs-on` change, retry of the whole job, a `continue-on-error` on this step, and the dependabot PRs 15 and 17 themselves (they merge after this lands and the check is green). If the public resolvers also fail, the step fails with the printed diagnostics and the architect reports it; no further fallback is added here.

## 4. Trust-boundary note

The step runs on pull requests, including dependabot's. No `${{ }}` expression is interpolated into the script, so no PR-controlled text reaches the shell. Adding public resolvers lets an on-path attacker answer DNS, not install code: apt verifies the package against the Ubuntu archive signing keys, and a forged answer ends in a failed install. Disposition: constrained by apt's signature check. The abuse case is therefore a contract test only in the sense that `zizmor` shows no template injection finding; there is no way to run a poisoned resolver in CI.

## 5. Acceptance

Baseline taken at draft time on the unchanged tree: `actionlint .github/workflows/ci.yml` exits 0; `python3 -c "import yaml;yaml.safe_load(open('.github/workflows/ci.yml'))"` prints no error; `zizmor .github/workflows/ci.yml` exits 0 with "No findings to report (7 suppressed)".

1. `actionlint .github/workflows/ci.yml` exits 0 and `zizmor .github/workflows/ci.yml` exits 0 with no findings and the same 7 suppressed (`scripts/prepush-fast.sh` runs both).
2. The YAML parses (`python3 -c` above).
3. `git diff --stat` shows only `.github/workflows/ci.yml`, and the diff is limited to the one step and its comment.
4. After the architect pushes `develop`: the `check` job's step log shows `resolvectl status` output and, when the runner cannot resolve the mirrors, the line `resolver cannot reach the Ubuntu mirrors; adding public DNS` followed by a `getent hosts` line. This step runs only in CI, so the developer cannot run it; the developer reports that and runs checks 1-3. A green `check` job is the architect's evidence of the fix; if the outage is still present and the public resolvers do not help, the architect reports the failure with the logged diagnostics and does not loop (shared circuit breaker).
5. No required test fails on the old code: this change has no Rust test, so there is no red phase.
6. Flaky: none known for this step.

## Review record

Models: architect Claude Sonnet 5.5 (claude-sonnet-5-5); consultant on the claude harness (model not recorded); developer on OMP, working slot openai-codex/gpt-5.6-luna, advisor slot present (model not asserted); no tester pass (a workflow-only change). BLOCKED/amend cycles: 0.

| Where found | By | Problem | Resolution |
| --- | --- | --- | --- |
| Draft review | consultant | A bare `getent hosts` that fails ends the step under `bash -e`, before apt runs | Every best-effort command carries its own `|| echo` (2.1) |
| Draft review | consultant | `resolvectl dns` or `flush-caches` failing was undecided | Best effort, never ends the step (2.1) |
| Draft review | consultant | The interface lookup could exit non-zero on no match | Pinned `awk` form that always exits 0 (2.1) |
| Draft review | consultant | Retries and timeouts do not bound name resolution or total time | Stated as an expectation in 2.1 item 3 |
| Draft review | consultant | Diagnostics missing (`ip route`, `apt-mirrors.txt`); runner named `ubuntu-24.04` but the job uses `ubuntu-latest`; line anchors off by one | Added, reworded, corrected |
| Architect, before lock | architect | The spec said `rg bubblewrap docs` finds nothing; it finds the runtime-requirement mentions in `docs/remote-messaging.md` | Claim narrowed to `apt-get` |
