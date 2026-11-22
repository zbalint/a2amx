use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use super::views::session_activity;
use super::{Runtime, lookup, random_hex};
use crate::codex;
use crate::confine::{self, Host, Plan};
use crate::delivery::{AnyChannel, RecheckFn};
use crate::emulator::Size;
use crate::harness::{self, Harness};
use crate::messaging;
use crate::session::{Session, SessionId, SessionSpec};
use crate::wire::{Request, Response};

const ACTIVITY_POLL: Duration = Duration::from_secs(1);

impl Runtime {
    fn split_confine(
        request: Request,
    ) -> Result<(Option<confine::ConfineRequest>, Request), String> {
        match request {
            Request::Confine { confine, request } => {
                if matches!(request.as_ref(), Request::Confine { .. }) {
                    return Err("nested confinement requests are not allowed".into());
                }
                Ok((Some(confine), *request))
            }
            request => Ok((None, request)),
        }
    }

    /// Resolve bubblewrap and build the mount plan off the async runtime.  `Err` carries the
    /// message for the client; `Ok(None)` means the session is not confined.
    async fn plan_confinement(
        &self,
        confine_request: Option<confine::ConfineRequest>,
        harness: Harness,
        cwd: Option<&str>,
    ) -> anyhow::Result<Result<Option<Plan>, String>> {
        let Some(confine_request) = confine_request else {
            return Ok(Ok(None));
        };
        let Some(cwd) = cwd else {
            return Ok(Err("confinement requires an explicit cwd".into()));
        };
        let cwd = PathBuf::from(cwd);
        let layout = self.layout.clone();
        let bwrap_path = self.bwrap_path.clone();
        let env_path = self.env_path.clone();
        let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Plan> {
            let bwrap = confine::resolve_bwrap(Some(&bwrap_path))?;
            if matches!(harness, Harness::Generic) {
                confine::preflight_env(&env_path)?;
            }
            confine::preflight(&bwrap)?;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
            let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
                .map(PathBuf::from)
                .ok_or_else(|| anyhow::anyhow!("XDG_RUNTIME_DIR is not set"))?;
            confine::ensure_empty_file(&layout)?;
            let host = Host {
                layout: layout.clone(),
                home: home.clone(),
                runtime_dir,
            };
            confine::ensure_data_dirs(harness, &home)?;
            confine::ensure_data_files(harness, &home)?;
            let preflight = confine::ensure_grant_dirs(&confine_request, harness, &host, true);
            let mut plan = confine::plan_with_preflight(
                &confine_request,
                harness,
                &cwd,
                &host,
                std::env::var_os("PATH"),
                preflight,
            )?;
            if !plan.overlay_parents().is_empty()
                && let Err(reason) = confine::supports_tmp_overlay(&bwrap, plan.overlay_parents())
            {
                plan = plan.without_overlays(&reason);
            }
            Ok(plan.with_bwrap(bwrap))
        })
        .await?;
        Ok(result.map(Some).map_err(|error| error.to_string()))
    }

    pub(super) async fn create_session(
        self: &Arc<Self>,
        request: Request,
    ) -> anyhow::Result<Response> {
        let (confine_request, request) = match Self::split_confine(request) {
            Ok(split) => split,
            Err(message) => return Ok(Response::Error { message }),
        };
        let Request::NewSession {
            argv,
            cols,
            rows,
            cwd,
            mut env,
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            name,
            harness,
            deliver,
            team,
            role,
            remote,
        } = request
        else {
            return Ok(Response::Error {
                message: "expected new_session".into(),
            });
        };
        if argv.is_empty() || cols == 0 || rows == 0 {
            return Ok(Response::Error {
                message: "command and nonzero dimensions are required".into(),
            });
        }
        if let Err(error) = messaging::validate_reset_steps(&reset) {
            return Ok(Response::Error {
                message: error.to_string(),
            });
        }
        if let Some(message) = Self::validate_watch_lists(&control_from, &watch, name.as_deref()) {
            return Ok(Response::Error { message });
        }
        let heartbeat = match heartbeat
            .map(|seconds| messaging::interval_from_seconds(seconds, "heartbeat"))
            .transpose()
        {
            Ok(heartbeat) => heartbeat,
            Err(error) => {
                return Ok(Response::Error {
                    message: error.to_string(),
                });
            }
        };
        if heartbeat.is_some() && watch.is_empty() {
            return Ok(Response::Error {
                message: format!(
                    "session {}: heartbeat needs a non-empty watch",
                    name.as_deref().unwrap_or("unnamed")
                ),
            });
        }
        if let Some(activity) = &activity
            && let Err(error) = activity.validate(!watch.is_empty())
        {
            return Ok(Response::Error {
                message: format!("session {}: {error}", name.as_deref().unwrap_or("unnamed")),
            });
        }

        if let Some(scope) = &team
            && let Some(message) = Self::validate_team_scope(scope, name.as_deref())
        {
            return Ok(Response::Error { message });
        }
        if let Some(role) = &role
            && let Err(error) = messaging::validate_role(role)
        {
            return Ok(Response::Error {
                message: error.to_string(),
            });
        }

        if remote.is_some() && confine_request.is_none() {
            return Ok(Response::Error {
                message: "remote messaging needs a confined session".into(),
            });
        }
        if confine_request.is_some() && cwd.is_none() {
            return Ok(Response::Error {
                message: "confinement requires an explicit cwd".into(),
            });
        }
        let _gate = self.sessions.begin_creation().await;
        if let Some(message) = self.check_team_and_name(team.as_ref(), name.as_deref()) {
            return Ok(Response::Error { message });
        }
        let number = self
            .sessions
            .next_number()
            .ok_or_else(|| anyhow::anyhow!("session id space exhausted"))?;
        let id = SessionId(format!("s{number}"));
        let address = self.address.clone();
        let token = tokio::task::spawn_blocking(random_hex::<32>).await??;
        env.push(("A2AMX_TOKEN".into(), token.clone()));
        env.push(("A2AMX_ADDR".into(), address));
        let mut argv = argv;
        let mut codex = None;
        let channel = harness == Harness::Claude
            && env
                .iter()
                .any(|(key, value)| key == harness::CHANNEL_ENV && value == "1");
        env.retain(|(key, _)| key != harness::CHANNEL_ENV);
        let mut confine_plan = match self
            .plan_confinement(confine_request, harness, cwd.as_deref())
            .await?
        {
            Ok(plan) => plan,
            Err(message) => return Ok(Response::Error { message }),
        };
        if harness == Harness::Codex
            && let Some(plan) = confine_plan.take()
        {
            confine_plan = Some(plan.with_codex_dir(codex::session_dir(&self.layout, &id.0)));
        }
        if harness == Harness::Codex {
            (argv, codex) = match self
                .prepare_codex(&id, argv, cwd.as_deref(), &mut env, confine_plan.as_ref())
                .await?
            {
                Ok(prepared) => (prepared.0, Some(prepared.1)),
                Err(message) => return Ok(Response::Error { message }),
            };
        }
        let session_spec = SessionSpec {
            argv,
            cwd: cwd.map(PathBuf::from),
            size: Size { cols, rows },
            env,
            name,
            harness,
            channel,
            deliver: deliver.unwrap_or_else(|| harness.default_deliver()),
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            team,
            role,
            remote,
            token,
            codex,
            confine: confine_plan,
        };
        let session = if session_spec.confine.is_some() {
            Session::spawn_confined(id, session_spec).await
        } else {
            tokio::task::spawn_blocking(move || Session::spawn(id, session_spec)).await?
        };
        match session {
            Ok(session) => {
                let session = Arc::new(session);
                let id = session.id().0.clone();
                self.sessions.insert(number, session.clone());
                self.start_session_tasks(session.clone(), channel);
                Ok(Response::Created { session: id })
            }
            Err(error) => Ok(Response::Error {
                message: error.to_string(),
            }),
        }
    }

    fn validate_watch_lists(
        control_from: &[String],
        watch: &[String],
        name: Option<&str>,
    ) -> Option<String> {
        for controller in control_from {
            if let Err(error) = messaging::validate_name(controller) {
                return Some(format!("control_from entry {controller:?}: {error}"));
            }
        }
        for watcher in watch {
            if let Err(error) = messaging::validate_name(watcher) {
                return Some(format!("watch entry {watcher:?}: {error}"));
            }
            if name == Some(watcher.as_str()) {
                return Some(format!("session {watcher} watches itself"));
            }
        }
        None
    }

    fn validate_team_scope(scope: &messaging::TeamScope, name: Option<&str>) -> Option<String> {
        if let Err(error) = messaging::validate_name(&scope.name) {
            return Some(format!("team: {error}"));
        }
        if scope.name.ends_with('-') {
            return Some("team must not end in '-'".into());
        }
        if !scope.private && !scope.allow.is_empty() {
            return Some("allow needs a private team".into());
        }
        let mut seen = HashSet::with_capacity(scope.allow.len());
        for entry in &scope.allow {
            if let Err(error) = messaging::validate_name(entry) {
                return Some(format!("team allow entry {entry:?}: {error}"));
            }
            if entry == &scope.name {
                return Some("team allow entry must not be the team itself".into());
            }
            if !seen.insert(entry) {
                return Some(format!("team allow entry {entry:?} is repeated"));
            }
        }
        if !scope.private && !scope.agents.is_empty() {
            return Some("allow needs a private team".into());
        }
        if scope.agents.len() > 16 {
            return Some("session allow has at most 16 entries".into());
        }
        let mut seen_agents = HashSet::with_capacity(scope.agents.len());
        for entry in &scope.agents {
            if let Err(error) = messaging::validate_name(entry) {
                return Some(format!("session allow entry {entry:?}: {error}"));
            }
            if name == Some(entry.as_str()) {
                return Some("session allow entry must not name itself".into());
            }
            if !seen_agents.insert(entry) {
                return Some(format!("session allow entry {entry:?} is repeated"));
            }
        }
        None
    }

    fn check_team_and_name(
        &self,
        team: Option<&messaging::TeamScope>,
        name: Option<&str>,
    ) -> Option<String> {
        if let Some(scope) = team {
            let differs = self.sessions.snapshot().iter().any(|session| {
                session.exit_code().is_none()
                    && session.team().is_some_and(|existing| {
                        existing.name == scope.name
                            && (existing.private != scope.private || existing.allow != scope.allow)
                    })
            });
            if differs {
                return Some(format!(
                    "team {} is already running with different settings",
                    scope.name
                ));
            }
        }
        if let Some(name) = name {
            if let Err(error) = messaging::validate_name(name) {
                return Some(error.to_string());
            }
            if self
                .sessions
                .snapshot()
                .iter()
                .any(|session| session.name() == Some(name))
            {
                return Some(format!("session name {name} is already in use"));
            }
        }
        None
    }

    async fn prepare_codex(
        &self,
        id: &SessionId,
        mut argv: Vec<String>,
        cwd: Option<&str>,
        env: &mut Vec<(String, String)>,
        plan: Option<&confine::Plan>,
    ) -> anyhow::Result<Result<(Vec<String>, Arc<codex::Link>), String>> {
        // shortcut: the client signals --no-authorize-peers and the role through
        // the request environment; a wire field is deferred until a third
        // value needs this or the env path causes trouble. A field now would
        // force edits to the 29 Request::NewSession literals in tests plus
        // src/main.rs, which have no Default.
        // The role also travels in the additive wire field for display only.
        let authorize = !env
            .iter()
            .any(|(key, value)| key == codex::NO_AUTHORIZE_ENV && value == "1");
        env.retain(|(key, _)| key != codex::NO_AUTHORIZE_ENV);
        let exe = self.exe.clone();
        let codex_role = env
            .iter()
            .rev()
            .find_map(|(key, value)| (key == codex::ROLE_ENV).then(|| value.clone()));
        env.retain(|(key, _)| key != codex::ROLE_ENV);
        if let Some(message) =
            tokio::task::spawn_blocking(move || codex::executable_error(&exe)).await?
        {
            return Ok(Err(message));
        }
        let notice = plan.map(confine::notice);
        let started = codex::start(
            &argv[0],
            &self.layout,
            &id.0,
            cwd.map(Path::new),
            env,
            codex::server_config(
                &self.exe,
                authorize,
                codex_role.as_deref(),
                notice.as_deref(),
            ),
            plan,
        )
        .await;
        let link = match started {
            Ok(link) => link,
            Err(error) => return Ok(Err(format!("{error:#}"))),
        };
        argv = codex::wire_argv(argv, link.endpoint());
        Ok(Ok((argv, link)))
    }

    fn start_session_tasks(self: &Arc<Self>, session: Arc<Session>, channel: bool) {
        let store = self.store.clone();
        let boot = self.boot.clone();
        let runtime = Arc::downgrade(self);
        let recheck: RecheckFn = Arc::new(move |message| {
            if message.sender_session != messaging::HUMAN_SESSION {
                return None;
            }
            let Some(runtime) = runtime.upgrade() else {
                return Some("recipient is no longer eligible for remote messaging");
            };
            let Some((_, recipient)) = lookup(&runtime, &message.recipient_session) else {
                return Some("recipient is no longer eligible for remote messaging");
            };
            runtime
                .remote_ineligible(&recipient, Instant::now(), messaging::Direction::ToAgent)
                .map(|(_, detail)| detail)
        });
        let mut tasks = vec![tokio::spawn(crate::delivery::run(
            AnyChannel::for_session(session.clone()),
            store.clone(),
            boot.clone(),
            recheck,
        ))];
        tasks.push(tokio::spawn(self.clone().observe_exit(session.clone())));
        tasks.push(tokio::spawn(self.clone().sample_activity(session.clone())));
        if let Some(interval) = session.heartbeat() {
            tasks.push(tokio::spawn(
                self.clone().heartbeat(session.clone(), interval),
            ));
        }
        if session.activity().is_some() {
            tasks.push(tokio::spawn(self.clone().activity(session.clone())));
        }
        if channel {
            tasks.push(tokio::spawn(accept_channel_dialog(session.clone())));
        }
        if let Some(link) = session.codex().cloned() {
            tasks.push(tokio::spawn(codex::run(session, link, store, boot)));
        }
        self.deliveries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(tasks);
    }

    async fn sample_activity(self: Arc<Self>, session: Arc<Session>) {
        // shortcut: sample once per second and read each session screen; use an output-path
        // transition hook if the cost matters.
        let mut ticker = tokio::time::interval(ACTIVITY_POLL);
        loop {
            ticker.tick().await;
            let now = Instant::now();
            let signal = match session.harness() {
                Harness::Codex => session
                    .codex()
                    .and_then(|link| harness::label_for_codex_reason(link.reason())),
                Harness::Claude => {
                    // shortcut: the Claude signal is a screen heuristic shared with
                    // `claude_ready`; replace it with a protocol signal if Claude Code ever
                    // exposes one.
                    let state = session.lock();
                    harness::needs_input(Harness::Claude, &state.emulator.screen())
                        .then_some("input?")
                }
                Harness::Omp | Harness::Generic | Harness::Agy => None,
            };
            session.note_needs_input(signal, now);
            if session.exit_code().is_some() {
                return;
            }
            let Some(activity) = session_activity(&session) else {
                return;
            };
            session.note_activity(activity, now);
        }
    }
}
async fn accept_channel_dialog(session: Arc<Session>) {
    // shortcut: fixed 60 s window and one string match; re-probe the dialog text
    // when Claude Code changes it.
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if session.exit_code().is_some() {
            return;
        }
        let visible = harness::channel_dialog_visible(&session.lock().emulator.screen());
        if visible {
            match tokio::task::spawn_blocking(move || session.write_input(b"\r")).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => tracing::debug!(%error, "channel dialog Enter failed"),
                Err(error) => tracing::debug!(%error, "channel dialog input task failed"),
            }
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
pub(super) async fn kill_session(runtime: &Runtime, id: &str, now: bool) -> Response {
    match lookup(runtime, id) {
        Some((number, session)) => match session.kill(!now).await {
            Ok(()) => {
                runtime.sessions.remove(number);
                Response::Ok
            }
            Err(error) => Response::Error {
                message: error.to_string(),
            },
        },
        None => Response::Error {
            message: format!("unknown session {id}"),
        },
    }
}
