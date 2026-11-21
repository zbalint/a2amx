# Spec 3fy: typed control requests on `Client`

## 0. Status

LOCKED. Location: main checkout `/home/zbalint/workspace/a2amx`, branch `develop`; the governing version is the
commit that adds this file. Context `a2amx-main-daemon-redesign-2026-11-20`. Public test seam: the new `Client`
methods through a scripted fake control daemon in `tests/client_requests.rs`; the existing binary suites judge
the command call sites unchanged.

Scope (edit):
- `src/client.rs` (two lines: `mod requests;` and `pub use requests::IssuedLeaseCode;`, nothing else), new `src/client/requests.rs`;
- `src/lookup.rs` (remove `request_sessions`, two call sites), `src/attach/state.rs` (two call sites),
  `src/commands/sessions.rs`, `src/commands/team.rs`, `src/commands/launch.rs`, `src/commands/daemon.rs` (call
  sites of section 3.3);
- new test `tests/client_requests.rs`.

Does not touch: every other file under `src/` (`src/mcp.rs`, `src/statusline.rs`, `src/hook.rs`, `src/daemon.rs`, `src/wire.rs` included), every other
test, `Cargo.toml`, `Cargo.lock`, `AGENTS.md`, `CHANGELOG.md`, `docs/` other than this spec. The architect updates the `client` and `lookup` rows of `AGENTS.md` when committing.

## 1. Why

The command handlers repeat one pattern 15 times: send a `Request`, `match` the `Response`, turn `Error`
into an `anyhow` error and anything else into the text `unexpected daemon response: {other:?}`. The copies
differ in ways that matter and that nothing documents: `kill` and `shutdown` accept `Ok`; `reset` also maps
`Failed{code,message}` to `code: message`; `messages` and `cancel` map `Error` and `Failed` alike; `version`
turns `Error` into "no version" (an old daemon); `create_session` rewrites the error `invalid control request`
into a restart hint when the request was confined (a version-skew mapping written as a string match inside the
handler); three copies say `unexpected explain|screen|lease response`. A reviewer or an agent cannot tell
which difference is intended. The owner wants the code readable, maintainable and testable. Protocol knowledge
(which reply answers which request, and what each failure means) belongs in `client`, next to the
request/response types, as one typed method per control request with its own semantics, testable against a
scripted daemon without a real one. The handlers then only decide what to print.

Rejected, with the reason: a generic `expect_ok`/`expect_variant` helper (most copies have their own `Failed`
and `Error` semantics, so a generic helper would flatten differences or take a closure per case: shallow);
new typed `Response` structs in `wire` (the wire format does not change); also converting `src/mcp.rs`,
`src/statusline.rs`, `src/hook.rs` and `src/gateway.rs` (they use `Client::request` with their own reply handling; a later spec if worth it);
moving `kill` or `team down` output into the methods (they print, the methods return).

Prior lessons, stated here because specs cannot cite memory: in a typed-reply refactor, version-skew error
mappings that live as string matches inside handlers are dropped unnoticed unless a test pins them (the
`invalid control request` mapping); a mutant "flip this `if`" is not killable by a new test when a callee has a
second guard with the same text (read the callee chain before listing a mutant).

## 2. Target shape

`src/client/requests.rs`, one `impl Client` block, module doc `//! Typed control requests: one method per
request, each with the reply semantics its callers need.` Every method returns `anyhow::Result`, the transport
error of `Client::request` propagates unchanged with `?`, and the method signatures are exactly:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuedLeaseCode { pub code: String, pub agents: Vec<String>, pub valid_secs: u64 }

impl Client {
    pub async fn sessions(&mut self) -> anyhow::Result<Vec<SessionSummary>>;
    pub async fn kill(&mut self, session: String, now: bool) -> anyhow::Result<()>;
    pub async fn reset(&mut self, session: &str) -> anyhow::Result<u32>;
    pub async fn shutdown(&mut self, now: bool) -> anyhow::Result<()>;
    pub async fn version(&mut self) -> anyhow::Result<Option<String>>;
    pub async fn explain(&mut self, session: String) -> anyhow::Result<ExplainInfo>;
    pub async fn screen(&mut self, session: String) -> anyhow::Result<Vec<String>>;
    pub async fn messages(&mut self, session: Option<String>, state: Option<String>)
        -> anyhow::Result<Vec<MessageInfo>>;
    pub async fn cancel_message(&mut self, id: String) -> anyhow::Result<()>;
    pub async fn issue_lease_code(&mut self, team: String) -> anyhow::Result<IssuedLeaseCode>;
    pub async fn create_session(&mut self, request: Request) -> anyhow::Result<String>;
}
fn unexpected(response: Response) -> anyhow::Error; // private: "unexpected daemon response: {response:?}"
```

Semantics table (the whole contract; `E(m)` is `Response::Error { message: m }`, `F(c,m)` is `Response::Failed
{ code: c, message: m }`, "other" is any other variant):

| Method | Request sent | Reply and result |
|---|---|---|
| `sessions` | `List` | `Sessions{sessions}` Ok(sessions); `E(m)` Err(m); other Err(unexpected) |
| `kill` | `Kill{session, now}` | `Ok` Ok(()); `E(m)` Err(m); other Err(unexpected) |
| `reset` | `Reset{session}` | `Reset{steps}` Ok(steps); `F(c,m)` Err("c: m"); `E(m)` Err(m); other Err(unexpected) |
| `shutdown` | `Shutdown{now}` | `Ok` Ok(()); `E(m)` Err(m); other Err(unexpected) |
| `version` | `Version` | `Version{version}` Ok(Some(version)); `E(_)` Ok(None) (a daemon that predates version reporting); other Err(unexpected) |
| `explain` | `Explain{session}` | `Explain{info}` Ok(info); `E(m)` Err(m); other Err(unexpected) |
| `screen` | `Screen{session}` | `Screen{lines}` Ok(lines); `E(m)` Err(m); other Err(unexpected) |
| `messages` | `ListMessages{session, state}` | `Messages{messages}` Ok; `E(m)` or `F(_,m)` Err(m); other Err(unexpected) |
| `cancel_message` | `CancelMessage{id}` | `Ok` Ok(()); `E(m)` or `F(_,m)` Err(m); other Err(unexpected) |
| `issue_lease_code` | `IssueLeaseCode{team}` | `LeaseCode{code, agents, valid_secs}` Ok(IssuedLeaseCode); `E(m)` Err(m); other Err(unexpected) |
| `create_session` | `request` as given; `confined` is `matches!(request, Request::Confine { .. })` | `Created{session}` Ok(session); `E(m)` with `confined` and `m` containing `invalid control request` Err("the running daemon does not support --confine; restart it"); `E(m)` Err(m); other Err(unexpected) |

## 3. The change

### 3.1 New code

`src/client/requests.rs` as in section 2. `src/client.rs` gets `mod requests;` and `pub use requests::IssuedLeaseCode;`
(the only edit to the file; add the `mod` after the existing `use` block and the re-export next to it). All
methods are `pub`; `unexpected` is private.

### 3.2 Behaviour changes (stderr text of rare paths; everything else is identical)

1. The three texts `unexpected explain response: ...`, `unexpected screen response: ...`, `unexpected lease response: ...`
   become `unexpected daemon response: ...`. The daemon this repository builds never sends those variants.
2. `team up` prints a lease problem as `warning: could not obtain lease code: {error}` for every error. Before, that
   prefix appeared only for a transport error; a daemon `Error` printed `warning: {message}` and an unexpected variant
   `warning: unexpected lease response: ...`. This IS reachable: the daemon answers `Error { message: "no agent in this
   team has remote messaging enabled" }` when the team has no remote member (`src/daemon.rs:1498-1503`), so that
   operator-visible warning gains the prefix. No test pins it (section 7). The architect adds a CHANGELOG line under
   Unreleased when committing. Success output (`remote messaging for: ...`, `post this code ...`) is unchanged.
No other output, exit code or ordering changes.

### 3.3 Call sites (each replaces a hand-written request and `match`; surrounding logic is unchanged)

- `src/lookup.rs`: delete `request_sessions` (and its imports of `Request`/`Response`/`anyhow` if unused);
  `resolve_reference` is unchanged; `resolve_session` (line 28) and `session_in_team` (line 35) call `client.sessions()`.
- `src/attach/state.rs:74,84`: `request_sessions(&mut control)` becomes `control.sessions()`.
- `src/commands/sessions.rs`: lines 21, 73, 132, 276 use `client.sessions()` (line 21 `run_list` has the same
  reply handling); `kill_session` is deleted and its two callers (142, 175) call `client.kill(id, now)`;
  `reset_one` is deleted and its callers (205 here, `team.rs` 736) call `client.reset(&id)`; `run_explain`,
  `run_screen`, `run_messages`, `run_cancel` call `explain`, `screen`, `messages`, `cancel_message`.
- `src/commands/team.rs`: lines 215, 504, 633, 698 use `sessions()`; `note_lost_messages` keeps its shape
  (a failed connect or request returns silently) with `client.messages(None, Some("undeliverable".to_owned()))`;
  `issue_lease_code` (the function) calls `client.issue_lease_code(team)` and prints the `Ok` arms as before and
  the `Err(error)` arm as `warning: could not obtain lease code: {error}`; the team-down kill (648) calls
  `client.kill(session.id.clone(), now)` and formats the error as `failed {name}: {error}` (an `Err` carries the same
  text as before: the daemon message, `unexpected daemon response: ...`, or the transport error); the
  `reset_one` import is removed.
- `src/commands/launch.rs`: 112 uses `sessions()`; `create_session` (the function) calls
  `client.create_session(request)` (the `confined` bool is derived inside from the request, so the caller no longer threads it) instead of the request and `match` at 438-451.
- `src/commands/daemon.rs`: 176 and 210 use `sessions()`; 177 uses `client.version()`; 228 uses `client.shutdown(now)`.
- `lookup::request_sessions` has no other caller (`rg` in section 7).
- Handlers keep `Client::connect`; `Client::request` stays `pub` and is used afterwards by `src/mcp.rs`,
  `src/statusline.rs`, `src/hook.rs`, `src/gateway.rs`, `client.rs` itself, the tests and `requests.rs`.

## 4. Tests (`tests/client_requests.rs`, a new target; contract tests, they fail to compile before the methods exist)

A scripted fake control daemon, local to the file: `TcpListener::bind("127.0.0.1:0")`, one accepted connection per
test, the 4-byte big-endian length framing of `tests/attach_cli/main.rs:53-70` (`fake_recv_frame`,
`fake_send_frame`, copied small), `Request::Hello` answered `Response::Ok`, then for each further request: record it
in a `Vec<Request>` behind `Arc<Mutex<_>>` and reply with the next scripted `Response`; after the last reply the
connection closes. The fake records each request before it writes the reply (so receiving the reply proves the request was recorded and no assertion depends on timing). The client is `Client::connect_addr(addr, "token")`. A row that lists several replies is one test per reply (or a fake scripted with N replies on one connection). Each test scripts one reply, calls one method,
and asserts the result (`Ok` value or the exact error text of `err.to_string()`) and, for the first row of each
method, the exact `Request` recorded (including the arguments).

Worked rows (reply gives result; the exact literals):
1. `kill("s1".into(), true)`, reply `Ok` gives `Ok(())`, recorded `Request::Kill{session:"s1", now:true}`.
2. `kill`, reply `Error{message:"no such session"}` gives Err `no such session`.
3. `kill`, reply `Created{session:"x"}` gives Err `unexpected daemon response: Created { session: "x" }`.
4. `reset("s1")`, reply `Reset{steps:2}` gives `Ok(2)`, recorded `Reset{session:"s1"}`.
5. `reset`, reply `Failed{code:"draft_present", message:"draft"}` gives Err `draft_present: draft`; reply `Error{message:"boom"}` gives Err `boom`.
6. `version()`, reply `Version{version:"1.2"}` gives `Ok(Some("1.2"))`; reply `Error{message:"old"}` gives `Ok(None)`; reply `Ok` gives Err `unexpected daemon response: Ok`.
7. `messages(None, Some("undeliverable".into()))`, reply `Failed{code:"c", message:"m"}` gives Err `m`; reply `Error{message:"e"}` gives Err `e`; recorded `ListMessages{session:None, state:Some("undeliverable")}`.
8. `cancel_message("m1".into())`, reply `Ok` gives `Ok(())`; reply `Failed{..message:"gone"}` gives Err `gone`; recorded `CancelMessage{id:"m1"}`.
9. `create_session(confined)` where `confined` is `Request::Confine { confine: ConfineRequest::default(), request: Box::new(Request::List) }` (`ConfineRequest` derives `Default`, `src/confine.rs:89`), reply `Error{message:"invalid control request: unknown variant"}` gives Err `the running daemon does not support --confine; restart it`; the same reply to an unconfined `Request::List` request gives Err `invalid control request: unknown variant`; reply `Created{session:"s9"}` gives `Ok("s9")`; reply `Error{message:"other"}` to the confined request gives Err `other`.
10. `sessions()`, reply `Sessions{sessions: vec![]}` gives `Ok(vec![])`; reply `Error` gives Err message; `shutdown(false)` reply `Ok` gives `Ok(())` and records `Shutdown{now:false}`; `explain`/`screen`/`issue_lease_code` each: the success variant gives the typed value, `Error` gives Err(message), another variant gives Err `unexpected daemon response: ...` (for `Created{session:"x"}`, literal text as in row 3). For `issue_lease_code`, success reply `LeaseCode{code:"AB12", agents:vec!["a"], valid_secs:60}` gives those three fields.
11. `reset` returns the daemon's `u32` step count unchanged (`Response::Reset { steps: u32 }`, `src/wire.rs:339`); both callers only format it and compare it with 1, so the old `usize` conversion and its `reset step count does not fit in usize` error disappear (unreachable on 64-bit targets, never tested).
12. A transport failure: the fake closes after Hello; `kill(...)` gives an `Err` (the text of the transport error is not pinned).

Use `Request`/`Response` constructors from `a2amx::wire`; `SessionSummary`, `ExplainInfo` and `MessageInfo` literals
only where a success row needs them (the developer builds the smallest valid value; if a type has many fields use
one of the existing test literals, e.g. `tests/attach_cli/main.rs:81` `fake_sessions` for `SessionSummary`, `tests/wire/requests.rs:88` for `ExplainInfo`).

## 5. Out of scope

Converting `src/mcp.rs`, `src/statusline.rs`, `src/hook.rs` and `src/gateway.rs`; changing `Client::request`; the typed structs for replies; any change to
what handlers print other than section 3.2; `note_lost_messages`' silent failure (a `shortcut:` comment would be a
behaviour-neutral later edit); the launch type, renderers and the team up split (next specs).

## 6. Acceptance

- `cargo test --test client_requests` passes (the new rows); `cargo test --no-fail-fast`: all pass. Baseline at
  the base commit (architect, 2026-11-21): 65 suites ok, 1396 passed, 0 failed, 3 ignored; after: 66 suites and
  1396 plus the new tests passed (the developer states the numbers).
- `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check`: pass.
- `rg -n 'unexpected (explain|screen|lease|daemon) response' src --glob '!src/hook.rs'` prints only
  `src/client/requests.rs` (one line, in `unexpected`; `src/hook.rs:24` has its own constant and stays); `rg -n 'request_sessions|\.request\(Request::' src/commands src/lookup.rs src/attach` prints
  nothing; `rg -n 'client\.request\(|\.request\(' src/commands` prints nothing.
- `wc -l src/client.rs src/client/requests.rs src/commands/*.rs`: every file under 1,000 lines; `requests.rs` at most 220.
- `git status --short` lists only the files of section 0; `git diff --stat -- Cargo.toml Cargo.lock` is empty.
- The binary suites pass unchanged (they run the call sites through the real daemon): `tests/daemon_cli`, `tests/team`,
  `tests/teams`, `tests/refs_and_list.rs`, `tests/explain.rs`, `tests/auto_confine`, `tests/confine`, `tests/attach_cli`.
  Known flaky tests: `tests/agy_plugin.rs::descendant_pipe_holder_fails_with_pipe_error`,
  `tests/attach_cli/scroll_and_mouse.rs::attach_does_not_capture_session_already_in_drag_mode`,
  `tests/attach_cli/status_and_team_line.rs::status_toggle_reattaches_to_resize_the_session`,
  `tests/presence` `viewers::viewer_attach_keeps_input_read_only_while_sizing_session`,
  `tests/attach_cli/viewer.rs::viewer_team_click_acts_on_release_and_keeps_viewer_mode`,
  `tests/attach_cli/attach_session.rs::client_file_prefix_reaches_attached_session`,
  `tests/discord/gateway_queue.rs::gateway_queue_reads_heartbeats_while_consumer_is_slow`,
  `tests/daemon_cli/reset.rs::team_reset_file_resets_sessions_in_file_order`: rerun a failure once, record as
  pre-existing if it passes.
- Mutation check (developer, on the assigned checkout, one line each in `src/client/requests.rs`, restored by
  sha256 before and after, non-empty edits; each must be killed by a named test of `tests/client_requests.rs`):
  `kill` `Response::Ok` arm to return an error; `reset` `Failed` arm to drop the code prefix; `reset` `Failed` arm to format `{message}: {code}`; `version` `Error` arm to `Err`; `messages` drop the `Failed` arm; `cancel_message`
  drop the `Failed` arm; `create_session` derive `confined` as `false`; `create_session` change the
  restart text; `unexpected` change its prefix; `shutdown` send `now: true` always; `kill` send `now: false` always.
  The developer lists the killing test per mutant; `git diff --stat -- src/client` shows only the developer's own
  files afterwards.
- The tester repeats the mutation pass independently; the architect reviews the diff; a consultant reads the design
  (a refactor).

## 7. Claims (claim | source | verified how)

| Claim | Source | Verified |
|---|---|---|
| Each call site of section 3.3 handles its reply as the table says (arms, texts) | `src/commands/sessions.rs:21,179-186,188-206,215-221,316-321,345-352,360-366`, `team.rs:269-276,427-459,648-661`, `launch.rs:438-451`, `daemon.rs:177-181,210-214,228-232`, `lookup.rs:11-15` | read by the architect this session |
| `request_sessions` callers are exactly `lookup.rs` (2), `attach/state.rs` (2), `commands/sessions.rs` (3: lines 73, 132, 276; line 21 is a hand-written `List`), `commands/team.rs` (4), `commands/launch.rs` (1), `commands/daemon.rs` (1) | `rg -n request_sessions src tests` | run; no test uses it |
| No test pins the three `unexpected ... response` texts, nor the lease warning texts; the success lease lines are pinned | `rg -n 'could not obtain lease\|unexpected (explain\|screen\|lease)' tests` (none); `tests/teams/team_up_remote.rs:163` | run; read |
| The `invalid control request` text is emitted by the daemon for an unparseable request (`src/daemon.rs:2774`) and asserted in `tests/daemon/serve_protocol.rs:197,325`; an old daemon sends it for an unknown variant, which the real daemon cannot reproduce, hence the fake | `rg 'invalid control request' src tests` | run |
| Frame format: 4-byte big-endian length then the JSON payload; `Hello` is answered `Ok`; `Client::connect_addr(addr, token)` exists | `src/client.rs:99-114,320-327`, `tests/attach_cli/main.rs:53-70,232-236` | read |
| `Response::Created{session: String}`, `Messages{messages: Vec<MessageInfo>}`, `Explain{info: ExplainInfo}` | `src/wire.rs:308-330,365-370` | read |
| `src/client.rs` has 336 lines, no `requests` module exists | `wc -l`, `rg` | run |
| `Client::request` stays used by `mcp.rs`, `statusline.rs`, `hook.rs`, `gateway.rs`, `client.rs` and tests | `rg -c '\.request\(' src`; consultant read | run; read (`daemon.rs` has none) |

## Review record

Models: architect and consultant Claude Sonnet 5.5 (fresh consultant sessions for the draft review and the design read); senior developer Antigravity CLI, Gemini 4 Hydrogen (Medium); tester OMP opencode-go/deepseek-v4.1-flash. BLOCKED/amend cycles: 0.

| Found at | By (role, model) | Problem | Resolution |
|---|---|---|---|
| draft review | consultant | the acceptance grep `unexpected ... response` also matches `src/hook.rs:24`, so it could not print only `requests.rs` | grep gets `--glob '!src/hook.rs'` |
| draft review | consultant | claim "the daemon never sends the lease Error" was false (`src/daemon.rs:1498-1503`); the lease warning change is reachable | 3.2 reworded, CHANGELOG line, claims row fixed |
| draft review | consultant | `Response::Reset` steps is `u32`, not `u64`; the usize conversion was an untestable branch | `reset` returns `u32` |
| draft review | consultant | `confined` bool is redundant with the request and could disagree | `create_session(request)` derives it |
| draft review | consultant | scope said one line in `client.rs`, 3.1 said two; `IssuedLeaseCode` needs `Debug`; wrong test file named for `ExplainInfo`; `Client::request` users omitted `gateway.rs`; `request_sessions` count off by one; `resolve_session`/`session_in_team` mislabelled; mutant "swap Failed and Error formats" ill-defined | spec text corrected |
| architect draft | architect | the acceptance grep was not run against out-of-scope matches before locking; the "unreachable" claim was not checked against the daemon handler | consultant caught both; lesson recorded |
| tester pass | tester | the `team up` lease warning prefix and the `team down` `failed {name}: {error}` line are unpinned (mutants survive) | recorded in the backlog (F94) as test hardening, not blocking |
| design read | consultant | `create_session` still takes the raw `Request`; `reset(&str)` vs `String`; `Failed` code kept for `reset` only; test scaffolding repeated about 45 times | later items in the backlog (F94) |
