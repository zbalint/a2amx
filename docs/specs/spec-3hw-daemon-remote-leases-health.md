# Spec 3hw: function-level code health of the lease code book in `src/daemon/remote.rs`

## 0. Status

LOCKED. Context `a2amx-3hw-daemon-remote-leases-health`. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`. Behaviour-preserving refactor, test-first where a behaviour is unpinned. Public seams: the control protocol over a real daemon (`Request::IssueLeaseCode`, `Request::RedeemLeaseCode` as the gateway, the remote status views).

Scope (edit): `src/daemon/remote.rs` (`Leases::issue`, `Leases::start`, `Leases::redeem`, the new functions, and their two callers `Runtime::issue_lease_code` at `:171` and `Runtime::redeem_lease_code` at `:184-203`), tests only for Step A gaps appended to existing files named there, never a new file. Does not touch: `Leases::state`, any other function, `src/messaging.rs`, `docs/` other than this spec, `CHANGELOG.md`.

No behaviour change: same replies, same texts, same counts, the same lock scope (the book's mutex is a leaf and is held only inside `Leases` methods), and the same constant-time comparison over every pending code.

## 1. Why

`cs check` reports on `remote.rs`: `:35` `Leases::redeem` "Complex Method (cc = 11)", "Bumpy Road Ahead (bumps = 3)", "Complex Conditional" at `:42` (the `if let .. && now < expires && constant_time_eq(..)`), and `:25` `Leases::issue` and `:76` `Leases::start` "Excess Number of Function Arguments (Arguments = 5)" (Amendment 1 added `start`; the first draft overlooked it). The redeem path is a trust boundary (a pairing code, a wrong-attempt lockout, constant-time comparison), so the split must keep its properties exactly.

Rules that bind (earlier refactors): a helper that exists only to clear a finding is rejected; each new function names one domain job; no out-parameter accumulator; at most four parameters per new function counting `self`; split along contiguous runs; no suppression.

## 2. Step A: pins (tests, green on the current code)

The developer lists, per behaviour, the existing pin and runs it. Candidates (all three exist; verified by the consultant's pre-lock read): `tests/human_inbound/inbound_admission.rs::lease_codes_expire_are_single_use_and_wrong_attempts_reset_on_issue` (expired code refused, single use), `::five_wrong_codes_drop_pending_codes_until_issue_resets_the_counter` (lockout at five, reset on issue), `::issue_lease_code_requires_a_running_remote_member_of_the_team`.

1. A wrong, expired or used code replies `Error "invalid or expired code"`.
2. A matching code consumes the pending entry and returns its team; the lease starts for that team only.
3. Five wrong attempts lock redemption (a correct code is refused) and drop every pending code; a later `issue` resets the counter.
4. The lockout also drops the pending codes of OTHER teams: a code issued for team A, five wrong attempts, then an `issue` for team B (which resets the counter), then redeeming A's code fails. Candidate gap: confirm by grep and add a pin if absent.
5. With two teams holding different pending codes, redeeming team B's code starts B's lease and leaves A's code pending. Candidate gap.

## 3. Step B: target shape

### `src/daemon/remote.rs`

- `Leases::start(&self, team: &str, members: Vec<SessionId>, expires: Instant)` (Amendment 1): the caller computes `now + Duration::from_secs(self.limits.lease_secs)`; `start` stores `expires` as given. Four parameters counting `self`.
- `Leases::issue(&self, team: String, code: String, expires: Instant)`: the caller computes `Instant::now() + ttl` (today `issue` adds `now + ttl` itself), four parameters; the counter reset and the replacement stay.
- `impl PendingLeaseCode { fn accepts(&self, code: &str, now: Instant) -> bool }`: `now < self.expires && constant_time_eq(code, &self.code)`. One condition, one domain job: this entry is redeemable with this code.
- `impl LeaseState { fn take_matching(&mut self, code: &str, now: Instant) -> Option<String> }`: iterate EVERY team (no early exit: the comparison time must not reveal which team or position matched), remember the first match, take its pending entry and return the team. Today's loop and `get_or_insert_with` stay as they are.
- `impl LeaseState { fn record_wrong_code(&mut self, now: Instant) }`: drop expired pending entries, bump `wrong_codes` (saturating), and at `MAX_WRONG_CODES` drop every pending entry.
- `Leases::redeem(&self, code: &str, now: Instant) -> Option<String>` keeps the lock, the early return at the lockout, `take_matching`, then `record_wrong_code` and `None`.

If a finding remains, the developer quotes the cs text, reports per-function cc and stops. Hand counts: `accepts` 1, `take_matching` about 4, `record_wrong_code` about 5, `redeem` about 3.

## 4. Out of scope

`state`, the lease views (`:200` onward), the wire types, the gateway.

## 5. Trust boundary

Attack paths: (a) guessing a pairing code (constant-time comparison, five-attempt lockout, single use, expiry) and (b) timing of the match position. Disposition: constrained, unchanged. The refactor keeps `constant_time_eq` for every pending code on every redeem (no `find`, no early `break`), the lockout, the counter reset on issue and the drop of all pending codes at the lockout. Abuse-case pins: items 1, 3, 4 above (items 1 and 3 exist; item 4 added if absent). The timing property has no deterministic test: the architect and the consultant check by reading that `take_matching` has no early exit.

## 6. Acceptance

Baseline: whole-tree cs count 839 (after 3hv, cb40314).

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cs check src/daemon/remote.rs
git status --short
```

- All pass; `cs check` lists no function finding on `remote.rs` (a file-level finding may remain); whole-tree count below the baseline; new tests add no cs warning.
- `git status --short` lists only the files of section 0. No suppression.
- Mutation check by the tester only (scratch `target/scratch/3hw/`, own `CARGO_TARGET_DIR`, deleted before its REPORT): (a) `accepts` ignores expiry; (b) `accepts` always true; (c) the matched entry is not taken (reusable code); (d) the lockout check at the top of `redeem` dropped; (e) the counter not incremented; (f) the lockout does not drop the pending codes; (g) the counter not reset by `issue`; (h) `issue` keeps an old pending entry; (i) `take_matching` returns the last match instead of the first (expected non-kill unless two teams share a code); (j) `take_matching` stops at the first match (a timing change; expected non-kill, reviewed by reading). The tester names the killing test per mutant.

## 7. Claims

| claim | file:line or test | verified how |
| --- | --- | --- |
| cs findings on remote.rs are `:25` (5 args), `:35` (cc 11, bumpy 3, complex conditional `:42`), `:76` (5 args) | `cs check src/daemon/remote.rs` | run by the developer on 453c0f4 (BLOCKED report) |
| `issue` and `start` each have exactly one caller outside the file | `remote.rs:171`, `:196` | grep `leases\.(start|issue)` across src and tests: no other hit |
| existing pins for expiry, single use, lockout and reset | `tests/human_inbound/inbound_admission.rs:230`, `:273` | read |
| no test issues codes for two teams, so items 4 and 5 of section 2 are new pins | `tests/human_inbound/inbound_admission.rs` | read: every `issue_code` call uses one team per test |
| codes are random 12 hex characters, so a last-match mutant is equivalent | `remote.rs` `random_hex::<6>` | read |
| `take_matching` "first match" depends on HashMap iteration order (arbitrary) | `remote.rs` `LeaseState.teams` | read |

## Amendment 1

The developer's BLOCKED report (m_9867) found that section 1 omitted the `:76` `Leases::start` finding (5 arguments counting `self`) while sections 0 and 4 put `start` out of scope, so the section 6 acceptance could not pass. Resolution: `start` joins the scope and takes `expires: Instant` like `issue`; its caller `redeem_lease_code` computes `now + term`. No behaviour change: the stored expiry value is identical. Sections 0, 1, 3 and 4 above carry the amended text.

## Review record

Models: architect Claude Sonnet 5.5; consultant Claude (consultant session, model not exposed to the architect); developer Gemini 4 Hydrogen (Medium) on antigravity (no advisor); tester opencode-go/deepseek-v4.1-flash on OMP (advisor model not exposed).

BLOCKED/amend cycles: 1 (BLOCKED on the missing `Leases::start` finding, resolved by Amendment 1, 4dffee1).

| Found at | By | Problem | Resolution |
| --- | --- | --- | --- |
| draft review | consultant | candidate pins 4 and 5 (other teams' codes at the lockout; a match takes only its own team) had no existing test; "first match" is HashMap iteration order | pins added in Step A; mutant table adjusted |
| draft review | consultant | the last-match mutant is equivalent in practice (random 12-hex codes) | listed as an expected non-kill |
| implementation | developer (BLOCKED m_9867) | section 1 omitted the `:76` `Leases::start` five-argument finding while sections 0 and 4 put `start` out of scope, so section 6 could not pass | Amendment 1: `start` joins the scope and takes `expires`, like `issue` |
| implementation | developer | section 2 and section 7 still carried drafting placeholders after locking | claims table written and the placeholders removed in Amendment 1 |
| architect review | consultant | the spec wording "every pending code" over-claims: HEAD and the diff compare every unexpired pending code | recorded here; later specs quote the actual predicate |
| architect review | consultant | the developer merged the expiry cleanup and the lockout drop of `record_wrong_code` into one pass, unlike the three steps of section 3 | accepted: the end state is identical and the drop never reads the counter; later specs state end states, not step order |
| tester pass | tester (deepseek-v4.1-flash) | mutants (d) and (n) are one deletion (the early lockout return in `redeem`); equivalent because the lockout also drops every pending code and `issue` resets the counter | confirmed equivalent, no pin |
| tester pass | tester (deepseek-v4.1-flash) | mutant (m) (expired entries never dropped) is observationally equivalent | accepted; only retention differs |
| tester pass | tester (deepseek-v4.1-flash) | mutants (i) first vs last match and (j) early exit survive, as expected | (i) equivalent in practice; (j) is a timing property checked by reading `take_matching` (no early exit), done by the architect and the consultant |

Mutation result: 10 killed, 3 equivalent, 2 expected survivors.
