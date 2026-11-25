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

struct NewSessionValidation<'a> {
    argv: &'a [String],
    cols: u16,
    rows: u16,
    cwd: Option<&'a str>,
    reset: &'a [String],
    control_from: &'a [String],
    watch: &'a [String],
    activity: Option<&'a crate::wire::ActivityWatch>,
    heartbeat: Option<u64>,
    name: Option<&'a str>,
    team: Option<&'a messaging::TeamScope>,
    role: Option<&'a str>,
    remote: Option<messaging::RemoteMode>,
    confined: bool,
}

struct LaunchPrep<'a> {
    id: &'a SessionId,
    harness: Harness,
    cwd: Option<&'a str>,
    confine_request: Option<confine::ConfineRequest>,
    argv: Vec<String>,
    env: &'a mut Vec<(String, String)>,
    codex_role: Option<&'a str>,
}

struct CodexPrep<'a> {
    id: &'a SessionId,
    cwd: Option<&'a str>,
    plan: Option<&'a confine::Plan>,
    codex_role: Option<&'a str>,
}

struct PreparedSession {
    argv: Vec<String>,
    env: Vec<(String, String)>,
    channel: bool,
    heartbeat: Option<Duration>,
    token: String,
    codex: Option<Arc<codex::Link>>,
    confine: Option<Plan>,
}

fn into_session_spec(request: Request, prepared: PreparedSession) -> anyhow::Result<SessionSpec> {
    let Request::NewSession {
        cols,
        rows,
        cwd,
        reset,
        control_from,
        watch,
        activity,
        name,
        harness,
        deliver,
        team,
        role,
        remote,
        ..
    } = request
    else {
        anyhow::bail!("expected new_session");
    };
    Ok(SessionSpec {
        argv: prepared.argv,
        cwd: cwd.map(PathBuf::from),
        size: Size { cols, rows },
        env: prepared.env,
        name,
        harness,
        channel: prepared.channel,
        deliver: deliver.unwrap_or_else(|| harness.default_deliver()),
        reset,
        control_from,
        watch,
        activity,
        heartbeat: prepared.heartbeat,
        team,
        role,
        remote,
        token: prepared.token,
        codex: prepared.codex,
        confine: prepared.confine,
    })
}

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

    fn has_valid_geometry(params: &NewSessionValidation<'_>) -> bool {
        !params.argv.is_empty() && params.cols > 0 && params.rows > 0
    }

    fn validate_session_inputs(
        params: &NewSessionValidation<'_>,
    ) -> Result<Option<Duration>, String> {
        if !Self::has_valid_geometry(params) {
            return Err("command and nonzero dimensions are required".into());
        }
        messaging::validate_reset_steps(params.reset).map_err(|error| error.to_string())?;
        if let Some(message) =
            Self::validate_watch_lists(params.control_from, params.watch, params.name)
        {
            return Err(message);
        }
        let heartbeat = params
            .heartbeat
            .map(|seconds| messaging::interval_from_seconds(seconds, "heartbeat"))
            .transpose()
            .map_err(|error| error.to_string())?;
        let unnamed = params.name.unwrap_or("unnamed");
        if heartbeat.is_some() && params.watch.is_empty() {
            return Err(format!(
                "session {unnamed}: heartbeat needs a non-empty watch"
            ));
        }
        if let Some(activity) = params.activity
            && let Err(error) = activity.validate(!params.watch.is_empty())
        {
            return Err(format!("session {unnamed}: {error}"));
        }
        Ok(heartbeat)
    }

    fn validate_team_and_role(params: &NewSessionValidation<'_>) -> Result<(), String> {
        if let Some(scope) = params.team
            && let Some(message) = Self::validate_team_scope(scope, params.name)
        {
            return Err(message);
        }
        if let Some(role) = params.role
            && let Err(error) = messaging::validate_role(role)
        {
            return Err(error.to_string());
        }
        Ok(())
    }

    fn validate_confinement_preconditions(params: &NewSessionValidation<'_>) -> Result<(), String> {
        if params.remote.is_some() && !params.confined {
            return Err("remote messaging needs a confined session".into());
        }
        if params.confined && params.cwd.is_none() {
            return Err("confinement requires an explicit cwd".into());
        }
        Ok(())
    }

    fn validate_new_session_params(
        params: &NewSessionValidation<'_>,
    ) -> Result<Option<Duration>, String> {
        let heartbeat = Self::validate_session_inputs(params)?;
        Self::validate_team_and_role(params)?;
        Self::validate_confinement_preconditions(params)?;
        Ok(heartbeat)
    }

    fn validate_new_session_request(
        request: &Request,
        confined: bool,
    ) -> Result<Option<Duration>, String> {
        let Request::NewSession {
            argv,
            cols,
            rows,
            cwd,
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            name,
            team,
            role,
            remote,
            ..
        } = request
        else {
            return Err("expected new_session".into());
        };
        Self::validate_new_session_params(&NewSessionValidation {
            argv,
            cols: *cols,
            rows: *rows,
            cwd: cwd.as_deref(),
            reset,
            control_from,
            watch,
            activity: activity.as_ref(),
            heartbeat: *heartbeat,
            name: name.as_deref(),
            team: team.as_ref(),
            role: role.as_deref(),
            remote: *remote,
            confined,
        })
    }

    async fn prepare_session_identity(
        &self,
        harness: Harness,
        env: &mut Vec<(String, String)>,
    ) -> anyhow::Result<(u64, SessionId, String, bool)> {
        let number = self
            .sessions
            .next_number()
            .ok_or_else(|| anyhow::anyhow!("session id space exhausted"))?;
        let id = SessionId(format!("s{number}"));
        let token = tokio::task::spawn_blocking(random_hex::<32>).await??;
        env.push(("A2AMX_TOKEN".into(), token.clone()));
        env.push(("A2AMX_ADDR".into(), self.address.clone()));
        let channel = harness == Harness::Claude
            && env
                .iter()
                .any(|(key, value)| key == harness::CHANNEL_ENV && value == "1");
        env.retain(|(key, _)| key != harness::CHANNEL_ENV);
        Ok((number, id, token, channel))
    }

    fn apply_session_identity_env(
        &self,
        id: &SessionId,
        name: Option<&str>,
        role: Option<&str>,
        env: &mut Vec<(String, String)>,
    ) -> Option<String> {
        let codex_role = env
            .iter()
            .rev()
            .find_map(|(key, value)| (key == codex::ROLE_ENV).then(|| value.clone()));
        env.retain(|(key, _)| key != codex::ROLE_ENV && key != "A2AMX_SESSION");
        env.push((
            "A2AMX_SESSION".into(),
            messaging::address(name, &id.0, &self.host_name),
        ));
        env.push((codex::ROLE_ENV.into(), role.unwrap_or("").to_owned()));
        codex_role
    }

    async fn prepare_confined_and_codex(
        &self,
        prep: LaunchPrep<'_>,
    ) -> anyhow::Result<Result<(Vec<String>, Option<Plan>, Option<Arc<codex::Link>>), String>> {
        let mut confine_plan = match self
            .plan_confinement(prep.confine_request, prep.harness, prep.cwd)
            .await?
        {
            Ok(plan) => plan,
            Err(message) => return Ok(Err(message)),
        };
        if prep.harness == Harness::Codex
            && let Some(plan) = confine_plan.take()
        {
            confine_plan = Some(plan.with_codex_dir(codex::session_dir(&self.layout, &prep.id.0)));
        }
        let mut argv = prep.argv;
        let mut codex = None;
        if prep.harness == Harness::Codex {
            let prepared = match self
                .prepare_codex(
                    CodexPrep {
                        id: prep.id,
                        cwd: prep.cwd,
                        plan: confine_plan.as_ref(),
                        codex_role: prep.codex_role,
                    },
                    argv,
                    prep.env,
                )
                .await?
            {
                Ok(prepared) => prepared,
                Err(message) => return Ok(Err(message)),
            };
            argv = prepared.0;
            codex = Some(prepared.1);
        }
        Ok(Ok((argv, confine_plan, codex)))
    }

    fn register_and_start_session(
        self: &Arc<Self>,
        number: u64,
        session: Session,
        channel: bool,
    ) -> String {
        let session = Arc::new(session);
        let id = session.id().0.clone();
        self.sessions.insert(number, session.clone());
        self.start_session_tasks(session, channel);
        id
    }

    pub(super) async fn create_session(
        self: &Arc<Self>,
        request: Request,
    ) -> anyhow::Result<Response> {
        let (confine_request, mut request) = match Self::split_confine(request) {
            Ok(split) => split,
            Err(message) => return Ok(Response::Error { message }),
        };
        let heartbeat =
            match Self::validate_new_session_request(&request, confine_request.is_some()) {
                Ok(heartbeat) => heartbeat,
                Err(message) => return Ok(Response::Error { message }),
            };
        let Request::NewSession {
            argv,
            cwd,
            env,
            name,
            harness,
            team,
            role,
            ..
        } = &mut request
        else {
            anyhow::bail!("expected new_session");
        };
        let _gate = self.sessions.begin_creation().await;
        if let Some(message) = self.check_team_and_name(team.as_ref(), name.as_deref()) {
            return Ok(Response::Error { message });
        }
        let mut env = std::mem::take(env);
        let argv = std::mem::take(argv);
        let harness = *harness;
        let (number, id, token, channel) = self.prepare_session_identity(harness, &mut env).await?;
        let codex_role =
            self.apply_session_identity_env(&id, name.as_deref(), role.as_deref(), &mut env);
        let (argv, confine, codex) = match self
            .prepare_confined_and_codex(LaunchPrep {
                id: &id,
                harness,
                cwd: cwd.as_deref(),
                confine_request,
                argv,
                env: &mut env,
                codex_role: codex_role.as_deref(),
            })
            .await?
        {
            Ok(prepared) => prepared,
            Err(message) => return Ok(Response::Error { message }),
        };
        let session_spec = into_session_spec(
            request,
            PreparedSession {
                argv,
                env,
                channel,
                heartbeat,
                token,
                codex,
                confine,
            },
        )?;
        match spawn_session(id, session_spec).await? {
            Ok(session) => Ok(Response::Created {
                session: self.register_and_start_session(number, session, channel),
            }),
            Err(error) => Ok(Response::Error {
                message: error.to_string(),
            }),
        }
    }

    fn validate_watch_entries(watch: &[String], name: Option<&str>) -> Option<String> {
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
        Self::validate_watch_entries(watch, name)
    }

    fn validate_allow_entries(
        entries: &[String],
        prefix: &str,
        self_name: Option<&str>,
        self_message: &str,
    ) -> Option<String> {
        let mut seen = HashSet::with_capacity(entries.len());
        for entry in entries {
            if let Err(error) = messaging::validate_name(entry) {
                return Some(format!("{prefix} {entry:?}: {error}"));
            }
            if self_name == Some(entry.as_str()) {
                return Some(self_message.to_owned());
            }
            if !seen.insert(entry) {
                return Some(format!("{prefix} {entry:?} is repeated"));
            }
        }
        None
    }

    fn validate_team_allow_scope(scope: &messaging::TeamScope) -> Option<String> {
        if let Err(error) = messaging::validate_name(&scope.name) {
            return Some(format!("team: {error}"));
        }
        if scope.name.ends_with('-') {
            return Some("team must not end in '-'".into());
        }
        if !scope.private && !scope.allow.is_empty() {
            return Some("allow needs a private team".into());
        }
        Self::validate_allow_entries(
            &scope.allow,
            "team allow entry",
            Some(&scope.name),
            "team allow entry must not be the team itself",
        )
    }

    fn validate_session_agent_allow_scope(
        scope: &messaging::TeamScope,
        name: Option<&str>,
    ) -> Option<String> {
        if !scope.private && !scope.agents.is_empty() {
            return Some("allow needs a private team".into());
        }
        if scope.agents.len() > 16 {
            return Some("session allow has at most 16 entries".into());
        }
        Self::validate_allow_entries(
            &scope.agents,
            "session allow entry",
            name,
            "session allow entry must not name itself",
        )
    }

    fn validate_team_scope(scope: &messaging::TeamScope, name: Option<&str>) -> Option<String> {
        if let Some(message) = Self::validate_team_allow_scope(scope) {
            return Some(message);
        }
        Self::validate_session_agent_allow_scope(scope, name)
    }

    fn team_settings_conflict(&self, scope: &messaging::TeamScope) -> bool {
        self.sessions.snapshot().iter().any(|session| {
            session.exit_code().is_none()
                && session.team().is_some_and(|existing| {
                    existing.name == scope.name
                        && (existing.private != scope.private || existing.allow != scope.allow)
                })
        })
    }

    fn check_team_and_name(
        &self,
        team: Option<&messaging::TeamScope>,
        name: Option<&str>,
    ) -> Option<String> {
        if let Some(scope) = team
            && self.team_settings_conflict(scope)
        {
            return Some(format!(
                "team {} is already running with different settings",
                scope.name
            ));
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
        prep: CodexPrep<'_>,
        mut argv: Vec<String>,
        env: &mut Vec<(String, String)>,
    ) -> anyhow::Result<Result<(Vec<String>, Arc<codex::Link>), String>> {
        // shortcut: the client signals --no-authorize-peers through the
        // request environment; a wire field is deferred until a third value
        // needs this or the env path causes trouble. A field now would force
        // edits to the 29 Request::NewSession literals in tests plus
        // src/main.rs, which have no Default.
        // The Codex role instruction still comes from the A2AMX_ROLE carrier
        // read in apply_session_identity_env (`prep.codex_role`), while
        // A2AMX_ROLE in the child comes from the validated role field.
        let authorize = !env
            .iter()
            .any(|(key, value)| key == codex::NO_AUTHORIZE_ENV && value == "1");
        env.retain(|(key, _)| key != codex::NO_AUTHORIZE_ENV);
        let exe = self.exe.clone();
        if let Some(message) =
            tokio::task::spawn_blocking(move || codex::executable_error(&exe)).await?
        {
            return Ok(Err(message));
        }
        let notice = prep.plan.map(confine::notice);
        let started = codex::start(
            &argv[0],
            &self.layout,
            &prep.id.0,
            prep.cwd.map(Path::new),
            env,
            codex::server_config(&self.exe, authorize, prep.codex_role, notice.as_deref()),
            prep.plan,
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
async fn spawn_session(
    id: SessionId,
    session_spec: SessionSpec,
) -> anyhow::Result<anyhow::Result<Session>> {
    if session_spec.confine.is_some() {
        Ok(Session::spawn_confined(id, session_spec).await)
    } else {
        Ok(tokio::task::spawn_blocking(move || Session::spawn(id, session_spec)).await?)
    }
}

async fn send_channel_dialog_enter(session: Arc<Session>) {
    match tokio::task::spawn_blocking(move || session.write_input(b"\r")).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::debug!(%error, "channel dialog Enter failed"),
        Err(error) => tracing::debug!(%error, "channel dialog input task failed"),
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
            send_channel_dialog_enter(session).await;
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
