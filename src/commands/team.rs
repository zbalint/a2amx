//! Team status, up, down, reset, and team-file resolution.
mod dry_run;

use self::dry_run::dry_run_report;
use super::{
    grants::{auto_grants_for, confirm_new_grants, load_accepted_grants, project_key},
    launch::{NewOptions, create_session, parse_activity_watch, resolved_confine_request},
};
use crate::attach::{self, AttachOptions};
use crate::client::Client;
use crate::harness::Harness;
use crate::messaging::{TeamScope, display_duration};
use crate::table::session_confinement_mode;
use crate::team::{self, TeamSession};
use crate::terminal::confirm;
use crate::terminal::{stdin_is_terminal, terminal_size_with_default, write_stderr, write_stdout};
use crate::wire::SessionSummary;
use anyhow::{Context, anyhow, bail};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

async fn resolve_auto_grants(
    actions: &mut [team::Action],
    origin: TeamOrigin<'_>,
) -> anyhow::Result<()> {
    let host_home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    for action in actions {
        let team::Action::Start(session) = action else {
            continue;
        };
        if session.auto_confine().is_none() {
            continue;
        }
        let resolved_dir = session
            .cwd
            .as_deref()
            .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
        let scan_home = host_home.clone();
        let found =
            tokio::task::spawn_blocking(move || auto_grants_for(&resolved_dir, &scan_home)).await?;
        if let Some(confine) = session.auto_confine_mut() {
            confine.apply_auto(found);
        }
    }
    Ok(())
}

#[derive(Clone)]
struct TeamSource {
    path: PathBuf,
    dir: PathBuf,
    global: bool,
}

#[derive(Clone, Copy)]
struct TeamOrigin<'a> {
    dir: &'a Path,
    file: Option<&'a Path>,
}

async fn resolve_team_file(home: &Path, file: Option<PathBuf>) -> anyhow::Result<TeamSource> {
    let invoking_cwd = std::env::current_dir()?;
    if let Some(file) = file {
        return resolve_explicit_team_file(&invoking_cwd, &file).await;
    }
    let state_home = home.to_owned();
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    let result = tokio::task::spawn_blocking(move || {
        find_local_or_global_team_file(invoking_cwd, state_home, user_home)
    })
    .await??;
    if result.global {
        write_global_team_notice(&result).await?;
    }
    Ok(result)
}

async fn resolve_explicit_team_file(
    invoking_cwd: &Path,
    file: &Path,
) -> anyhow::Result<TeamSource> {
    let path = invoking_cwd.join(file);
    let parent = path.parent().unwrap_or(invoking_cwd).to_owned();
    let dir = tokio::task::spawn_blocking(move || {
        std::fs::canonicalize(&parent).with_context(|| {
            format!(
                "cannot canonicalize team file directory {}",
                parent.display()
            )
        })
    })
    .await??;
    Ok(TeamSource {
        path,
        dir,
        global: false,
    })
}

fn find_local_or_global_team_file(
    invoking_cwd: PathBuf,
    state_home: PathBuf,
    user_home: Option<PathBuf>,
) -> anyhow::Result<TeamSource> {
    let cwd = std::fs::canonicalize(&invoking_cwd).with_context(|| {
        format!(
            "cannot canonicalize invoking directory {}",
            invoking_cwd.display()
        )
    })?;
    let state_home = std::fs::canonicalize(&state_home).unwrap_or(state_home);
    let user_home = user_home.map(|path| {
        if path.is_absolute() {
            std::fs::canonicalize(&path).unwrap_or(path)
        } else {
            path
        }
    });
    let local = invoking_cwd.join("a2amx.toml");
    match std::fs::metadata(&local) {
        Ok(_) => {
            return Ok(TeamSource {
                path: local,
                dir: cwd,
                global: false,
            });
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(anyhow!(
                "cannot inspect team file {}: {error}",
                local.display()
            ));
        }
    }
    let global = crate::layout::Layout::new(state_home.clone()).team_file();
    match std::fs::metadata(&global) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!(
                "no a2amx.toml in {} and no global team file at {}",
                cwd.display(),
                global.display()
            );
        }
        Err(error) => {
            return Err(anyhow!(
                "cannot inspect global team file {}: {error}",
                global.display()
            ));
        }
    }
    team::check_global_allowed(&cwd, user_home.as_deref(), &state_home)?;
    Ok(TeamSource {
        path: global,
        dir: cwd,
        global: true,
    })
}

async fn write_global_team_notice(source: &TeamSource) -> anyhow::Result<()> {
    let name = team::team_name_for(&source.dir, "create an a2amx.toml here, or pass --file")?;
    write_stderr(
        format!(
            "using global team file {} as team {name}\n",
            source.path.display()
        )
        .into_bytes(),
    )
    .await
}

async fn read_team(source: TeamSource) -> anyhow::Result<Vec<TeamSession>> {
    tokio::task::spawn_blocking(move || {
        let text = std::fs::read_to_string(&source.path)
            .with_context(|| format!("cannot read team file {}", source.path.display()))?;
        team::parse(&text, &source.dir, source.global)
            .with_context(|| format!("team file {}", source.path.display()))
    })
    .await?
}

fn conflict_text(reason: team::ConflictReason, name: &str, id: &str) -> String {
    match reason {
        team::ConflictReason::Exited => {
            format!("session {name} has exited ({id}); run a2amx kill {name} first")
        }
        team::ConflictReason::TeamMismatch => {
            format!(
                "session {name} ({id}) is running with a different team setting; run a2amx team down first"
            )
        }
    }
}

async fn write_conflicts(conflicts: &[team::Conflict]) -> anyhow::Result<()> {
    for conflict in conflicts {
        write_stderr(
            format!(
                "{}\n",
                conflict_text(conflict.reason.clone(), &conflict.name, &conflict.id,)
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(())
}

fn team_label(team: Option<&TeamScope>) -> String {
    let Some(team) = team else {
        return "-".to_owned();
    };
    if team.private {
        format!("{} (private)", team.name)
    } else {
        team.name.clone()
    }
}

pub(super) async fn run_team_status(
    home: PathBuf,
    file: Option<PathBuf>,
) -> anyhow::Result<std::process::ExitCode> {
    let source = resolve_team_file(&home, file).await?;
    let wanted = read_team(source).await?;
    let mut client = Client::connect(&home).await?;
    let existing = client.sessions().await?;
    let entries = team::inspect(&wanted, &existing);
    let mut output = String::from("NAME TEAM STATE CONFINE NOTE\n");
    let mut has_conflict = false;
    for (session, entry) in wanted.iter().zip(entries) {
        let (state, confine, note) = match entry.state {
            team::EntryState::Missing => ("missing".to_owned(), String::new(), "-".to_owned()),
            team::EntryState::Running { id } => {
                let current = existing.iter().find(|current| current.id == id);
                let mode = current.map_or("off", session_confinement_mode).to_owned();
                let note = if current.is_some_and(|summary| summary.starting) {
                    format!("{id} (not ready: starting)")
                } else {
                    id
                };
                ("running".to_owned(), mode, note)
            }
            team::EntryState::Conflict { id, reason } => {
                has_conflict = true;
                let state = match &reason {
                    team::ConflictReason::Exited => "exited",
                    team::ConflictReason::TeamMismatch => "team-mismatch",
                };
                let full = conflict_text(reason, &entry.name, &id);
                let note = full
                    .split_once("; ")
                    .map_or(full.as_str(), |(_, note)| note)
                    .to_owned();
                (state.to_owned(), String::new(), note)
            }
        };
        output.push_str(&format!(
            "{} {} {} {} {}\n",
            session.name,
            team_label(session.team.as_ref()),
            state,
            confine,
            note
        ));
    }
    write_stdout(output.into_bytes()).await?;
    note_lost_messages(&home, &wanted).await;
    Ok(if has_conflict {
        std::process::ExitCode::from(1)
    } else {
        std::process::ExitCode::SUCCESS
    })
}

async fn note_lost_messages(home: &Path, wanted: &[TeamSession]) {
    let Ok(mut client) = Client::connect(home).await else {
        return;
    };
    let Ok(messages) = client
        .messages(None, Some("undeliverable".to_owned()))
        .await
    else {
        return;
    };
    let count = messages
        .iter()
        .filter(|message| message.detail.as_deref() == Some("daemon_restarted"))
        .filter(|message| {
            let Some(local) = message.to.split_once('@').map(|(local, _)| local) else {
                return false;
            };
            wanted.iter().any(|session| session.name == local)
        })
        .count();
    if count != 0 {
        let _ = write_stderr(
            format!(
                "note: {count} message(s) to this team were lost in a daemon restart; see a2amx messages --state undeliverable\n"
            )
            .into_bytes(),
        )
        .await;
    }
}

fn validate_team_session(
    home: &Path,
    origin: TeamOrigin<'_>,
    session: &TeamSession,
) -> anyhow::Result<()> {
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(session, &resolved_cwd)?;
    let Some(request) = session.confine.as_ref() else {
        return Ok(());
    };
    let host_home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
    let runtime_dir =
        PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?);
    let request = resolved_confine_request(request, &resolved_cwd, origin.file)?;
    let harness = Harness::infer(&session.command);
    let host = crate::confine::Host {
        home: host_home,
        layout: crate::layout::Layout::new(home.to_owned()),
        runtime_dir,
    };
    let preflight = crate::confine::ensure_grant_dirs(&request, harness, &host, false);
    crate::confine::plan_with_preflight(
        &request,
        crate::confine::PlanTarget {
            harness,
            cwd: &resolved_cwd,
            host: &host,
        },
        std::env::var_os("PATH"),
        preflight,
    )?;
    Ok(())
}

#[derive(Clone, Copy)]
struct SessionLaunchContext<'a> {
    home: &'a Path,
    origin: TeamOrigin<'a>,
    cols: u16,
    rows: u16,
}

async fn start_planned_session(
    client: &mut Client,
    session: Box<TeamSession>,
    ctx: SessionLaunchContext<'_>,
) -> anyhow::Result<(String, String)> {
    let heartbeat = session
        .heartbeat
        .as_deref()
        .map(|value| crate::messaging::parse_interval(value, "heartbeat"))
        .transpose()?
        .map(|duration| duration.as_secs());
    let activity = session
        .activity
        .as_ref()
        .map(|value| {
            parse_activity_watch(value.working_after.as_deref(), value.idle_after.as_deref())
        })
        .transpose()?
        .flatten();
    let resolved_cwd = session.cwd.as_deref().map_or_else(
        || ctx.origin.dir.to_owned(),
        |path| ctx.origin.dir.join(path),
    );
    team::validate_session_confine(&session, &resolved_cwd)?;
    let confine = session
        .confine
        .as_ref()
        .map(|request| resolved_confine_request(request, &resolved_cwd, ctx.origin.file))
        .transpose()?;
    let name = session.name;
    let result = create_session(
        ctx.home,
        client,
        NewOptions {
            name: Some(name.clone()),
            harness: None,
            role: session.role,
            remote: session.remote,
            team: session.team,
            no_authorize_peers: false,
            no_channel: false,
            confine,
            deliver: None,
            control_from: session.control_from,
            reset: session.reset.unwrap_or_default(),
            watch: session.watch,
            activity,
            heartbeat,
            command: session.command,
            cwd: resolved_cwd,
            cols: ctx.cols,
            rows: ctx.rows,
        },
    )
    .await
    .with_context(|| format!("failed {name}"))?;
    write_stdout(format!("started {name} {result}\n").into_bytes()).await?;
    Ok((name, result))
}

async fn write_auto_summaries(actions: &[team::Action]) -> anyhow::Result<()> {
    for action in actions {
        let team::Action::Start(session) = action else {
            continue;
        };
        let Some(confine) = session.confine.as_ref() else {
            continue;
        };
        if confine.reasons.is_empty() {
            continue;
        }
        let grants = confine
            .reasons
            .iter()
            .map(|reason| reason.grant.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        write_stderr(
            format!(
                "auto confinement for session {}: grants {grants}\n",
                session.name
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(())
}

async fn issue_lease_code(client: &mut Client, remote_team: Option<String>) -> anyhow::Result<()> {
    if let Some(team) = remote_team {
        match client.issue_lease_code(team).await {
            Ok(lease) => {
                write_stdout(
                    format!("remote messaging for: {}\n", lease.agents.join(", ")).into_bytes(),
                )
                .await?;
                write_stdout(
                    format!(
                        "post this code in the control channel within {}: {}\n",
                        display_duration(Duration::from_secs(lease.valid_secs)),
                        lease.code
                    )
                    .into_bytes(),
                )
                .await?;
            }
            Err(error) => {
                write_stderr(
                    format!("warning: could not obtain lease code: {error}\n").into_bytes(),
                )
                .await?;
            }
        }
    } else {
        write_stderr(
            b"remote messaging is set but the file has no team name; no lease can start\n".to_vec(),
        )
        .await?;
    }
    Ok(())
}

async fn wait_for_enter() -> anyhow::Result<()> {
    write_stderr(b"press Enter to attach to the session".to_vec()).await?;
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        io::stdin().read_line(&mut line)
    })
    .await??;
    Ok(())
}

pub(super) struct UpOptions {
    pub(super) attach_options: AttachOptions,
    pub(super) file: Option<PathBuf>,
    pub(super) detach: bool,
    pub(super) dry_run: bool,
    pub(super) yes: bool,
    pub(super) items: Vec<String>,
}

struct TeamUpInputs {
    wanted: Vec<TeamSession>,
    file_dir: PathBuf,
    team_file: Option<PathBuf>,
    attach_name: Option<String>,
    remote_requested: bool,
    remote_team: Option<String>,
}

struct AutoGrantConsent<'a> {
    home: &'a Path,
    origin: TeamOrigin<'a>,
    dry_run: bool,
    yes: bool,
}

struct TeamUpTail {
    home: PathBuf,
    attach_options: AttachOptions,
    attach_id: Option<String>,
    remote_requested: bool,
    remote_team: Option<String>,
    detach: bool,
    cols: u16,
    rows: u16,
}

pub(super) async fn run_team_up(home: PathBuf, options: UpOptions) -> anyhow::Result<()> {
    let inputs = load_team_up_inputs(&home, options.file, &options.items).await?;
    let mut client = Client::connect(&home).await?;
    let existing = client.sessions().await?;
    let mut actions = plan_team_actions(&home, &inputs.wanted, &existing).await?;
    let origin = TeamOrigin {
        dir: &inputs.file_dir,
        file: inputs.team_file.as_deref(),
    };
    validate_start_actions(&home, origin, &mut actions).await?;
    let accepted = confirm_team_auto_grants(
        AutoGrantConsent {
            home: &home,
            origin,
            dry_run: options.dry_run,
            yes: options.yes,
        },
        &actions,
    )
    .await?;
    if options.dry_run {
        let output = dry_run_report(&home, origin, &actions, &existing, &accepted).await?;
        write_stdout(output.into_bytes()).await?;
        note_lost_messages(&home, &inputs.wanted).await;
        return Ok(());
    }
    note_lost_messages(&home, &inputs.wanted).await;
    let (cols, rows) = terminal_size_with_default()?;
    let started = execute_team_actions(
        &mut client,
        SessionLaunchContext {
            home: &home,
            origin,
            cols,
            rows,
        },
        actions,
        &existing,
    )
    .await?;
    let attach_id = find_attach_id(inputs.attach_name.as_deref(), &started);
    finish_team_up(
        client,
        TeamUpTail {
            home,
            attach_options: options.attach_options,
            attach_id,
            remote_requested: inputs.remote_requested,
            remote_team: inputs.remote_team,
            detach: options.detach,
            cols,
            rows,
        },
    )
    .await
}

fn find_attach_id(attach_name: Option<&str>, started: &[(String, String)]) -> Option<String> {
    let target = attach_name?;
    started
        .iter()
        .find(|(name, _)| name == target)
        .map(|(_, id)| id.clone())
}

async fn load_team_up_inputs(
    home: &Path,
    file: Option<PathBuf>,
    items: &[String],
) -> anyhow::Result<TeamUpInputs> {
    let cwd = std::env::current_dir()?;
    let (wanted, file_dir, team_file) = if items.is_empty() {
        let source = resolve_team_file(home, file).await?;
        let file_dir = source.dir.clone();
        let team_path = source.path.clone();
        let team_file =
            Some(tokio::task::spawn_blocking(move || std::fs::canonicalize(team_path)).await??);
        (read_team(source).await?, file_dir, team_file)
    } else {
        (team::flag_sessions(items)?, cwd, None)
    };
    let attach_name = wanted
        .iter()
        .find(|session| session.attach)
        .map(|session| session.name.clone());
    let remote_requested = wanted.iter().any(|session| session.remote.is_some());
    let remote_team = wanted
        .iter()
        .find_map(|session| session.team.as_ref().map(|team| team.name.clone()));
    Ok(TeamUpInputs {
        wanted,
        file_dir,
        team_file,
        attach_name,
        remote_requested,
        remote_team,
    })
}

async fn plan_team_actions(
    home: &Path,
    wanted: &[TeamSession],
    existing: &[SessionSummary],
) -> anyhow::Result<Vec<team::Action>> {
    match team::plan(wanted, existing) {
        Ok(actions) => Ok(actions),
        Err(conflicts) => {
            note_lost_messages(home, wanted).await;
            write_conflicts(&conflicts).await?;
            Err(anyhow!("team has session conflicts"))
        }
    }
}

async fn validate_start_actions(
    home: &Path,
    origin: TeamOrigin<'_>,
    actions: &mut [team::Action],
) -> anyhow::Result<()> {
    resolve_auto_grants(actions, origin).await?;
    for action in actions.iter() {
        if let team::Action::Start(session) = action {
            validate_team_session(home, origin, session)?;
        }
    }
    Ok(())
}

async fn confirm_team_auto_grants(
    consent: AutoGrantConsent<'_>,
    actions: &[team::Action],
) -> anyhow::Result<BTreeSet<String>> {
    let has_auto = actions.iter().any(|action| {
        matches!(
            action,
            team::Action::Start(session) if session.auto_confine().is_some()
        )
    });
    let accepted_all = if has_auto {
        load_accepted_grants(consent.home).await?
    } else {
        BTreeMap::new()
    };
    let project_key = if has_auto {
        Some(project_key(consent.origin.dir)?.to_owned())
    } else {
        None
    };
    let accepted = project_key
        .as_ref()
        .and_then(|key| accepted_all.get(key))
        .cloned()
        .unwrap_or_default();
    let new_grants = team::new_auto_grants(actions, &accepted);
    for output in team::escape_notices(actions, crate::grant::all()) {
        write_stderr(output.into_bytes()).await?;
    }
    if !consent.dry_run && !new_grants.is_empty() {
        confirm_new_grants(
            consent.home,
            consent.origin.dir,
            &accepted_all,
            &new_grants,
            consent.yes,
        )
        .await?;
    }
    write_auto_summaries(actions).await?;
    Ok(accepted)
}

async fn execute_team_actions(
    client: &mut Client,
    ctx: SessionLaunchContext<'_>,
    actions: Vec<team::Action>,
    existing: &[SessionSummary],
) -> anyhow::Result<Vec<(String, String)>> {
    let mut started = Vec::new();
    // shortcut: the conflict check and the spawns are not atomic; a name taken in between
    // fails the spawn. A daemon-side batch request if concurrent clients make this matter.
    for action in actions {
        let (name, id) = match action {
            team::Action::AlreadyRunning { name, id } => {
                let mode = existing
                    .iter()
                    .find(|session| session.id == id)
                    .map_or("off", session_confinement_mode);
                write_stdout(
                    format!("already running {name} {id} (confine {mode})\n").into_bytes(),
                )
                .await?;
                (name, id)
            }
            team::Action::Start(session) => start_planned_session(client, session, ctx).await?,
        };
        started.push((name, id));
    }
    Ok(started)
}

fn should_attach(detach: bool, attach_id: Option<&str>) -> bool {
    !detach && stdin_is_terminal() && attach_id.is_some()
}

async fn finish_team_up(mut client: Client, tail: TeamUpTail) -> anyhow::Result<()> {
    if tail.remote_requested {
        issue_lease_code(&mut client, tail.remote_team).await?;
    }
    if should_attach(tail.detach, tail.attach_id.as_deref())
        && let Some(session) = tail.attach_id
    {
        let (attach_cols, attach_rows) = if tail.remote_requested {
            wait_for_enter().await?;
            terminal_size_with_default()?
        } else {
            (tail.cols, tail.rows)
        };
        attach::run(
            client,
            &tail.home,
            tail.attach_options,
            session,
            false,
            attach_cols,
            attach_rows,
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn run_team_down(
    home: PathBuf,
    file: Option<PathBuf>,
    names: Vec<String>,
    now: bool,
) -> anyhow::Result<()> {
    let explicit = !names.is_empty();
    let names = if names.is_empty() {
        let source = resolve_team_file(&home, file).await?;
        read_team(source)
            .await?
            .into_iter()
            .map(|session| session.name)
            .collect()
    } else {
        names
    };
    let mut client = Client::connect(&home).await?;
    let sessions = client.sessions().await?;
    let mut failed = false;
    let mut missing_explicit = false;
    for name in names {
        let Some(session) = sessions
            .iter()
            .find(|session| session.name.as_deref() == Some(&name))
        else {
            write_stdout(format!("no session {name}\n").into_bytes()).await?;
            if explicit {
                missing_explicit = true;
            }
            continue;
        };
        let error = match client.kill(session.id.clone(), now).await {
            Ok(()) => {
                write_stdout(format!("killed {name} {}\n", session.id).into_bytes()).await?;
                continue;
            }
            Err(error) => error.to_string(),
        };
        write_stderr(format!("failed {name}: {error}\n").into_bytes()).await?;
        failed = true;
    }
    if missing_explicit {
        write_stderr(
            b"hint: names given to team down are full session names (with any team prefix); see a2amx list\n"
                .to_vec(),
        )
        .await?;
    }
    if failed {
        return Err(anyhow!("one or more team sessions could not be killed"));
    }
    Ok(())
}

pub(super) struct ResetArgs {
    pub(super) file: Option<PathBuf>,
    pub(super) names: Vec<String>,
    pub(super) except: Vec<String>,
    pub(super) include_attached: bool,
    pub(super) yes: bool,
}

pub(super) async fn run_team_reset(home: PathBuf, args: ResetArgs) -> anyhow::Result<()> {
    let (mut client, targets, attached_ids) = load_reset_targets(&home, &args).await?;
    if !args.yes && !confirm_reset_targets(&targets, &attached_ids).await? {
        return write_stdout(b"not reset\n".to_vec()).await;
    }
    execute_team_resets(&mut client, targets).await
}

async fn load_reset_targets(
    home: &Path,
    args: &ResetArgs,
) -> anyhow::Result<(Client, Vec<team::ResetTarget>, HashSet<String>)> {
    let explicit = !args.names.is_empty();
    let names = if explicit {
        args.names.clone()
    } else {
        let source = resolve_team_file(home, args.file.clone()).await?;
        read_team(source)
            .await?
            .into_iter()
            .map(|session| session.name)
            .collect()
    };
    let mut client = Client::connect(home).await?;
    let sessions = client.sessions().await?;
    let targets = team::reset_targets(
        &names,
        team::ResetOptions {
            explicit,
            except: &args.except,
            include_attached: args.include_attached,
        },
        &sessions,
    );
    let attached_ids = sessions
        .into_iter()
        .filter(|session| session.attached)
        .map(|session| session.id)
        .collect();
    Ok((client, targets, attached_ids))
}

async fn confirm_reset_targets(
    targets: &[team::ResetTarget],
    attached_ids: &HashSet<String>,
) -> anyhow::Result<bool> {
    let running = targets
        .iter()
        .filter(|target| matches!(target.state, team::ResetState::Reset { .. }))
        .count();
    if running == 0 {
        return Ok(true);
    }
    let mut preview = String::new();
    for target in targets {
        let team::ResetState::Reset { id } = &target.state else {
            continue;
        };
        preview.push_str(&format!(
            "{} {}{}\n",
            target.name,
            id,
            if attached_ids.contains(id) {
                " (attached)"
            } else {
                ""
            }
        ));
    }
    write_stdout(preview.into_bytes()).await?;
    let prompt = format!("Reset {running} session(s)? This clears their conversations. [y/N] ");
    confirm(
        &prompt,
        &format!("refusing to reset: {running} running session(s); pass --yes to reset them"),
    )
    .await
}

async fn execute_team_resets(
    client: &mut Client,
    targets: Vec<team::ResetTarget>,
) -> anyhow::Result<()> {
    let mut failed = false;
    for target in targets {
        failed |= execute_single_reset(client, target).await?;
    }
    if failed {
        return Err(anyhow!("one or more team sessions could not be reset"));
    }
    Ok(())
}

async fn execute_single_reset(
    client: &mut Client,
    target: team::ResetTarget,
) -> anyhow::Result<bool> {
    match target.state {
        team::ResetState::Reset { id } => match client.reset(&id).await {
            Ok(steps) => {
                let noun = if steps == 1 { "step" } else { "steps" };
                write_stdout(format!("reset {} {id}: {steps} {noun}\n", target.name).into_bytes())
                    .await?;
                Ok(false)
            }
            Err(error) => {
                write_stderr(format!("failed {}: {error}\n", target.name).into_bytes()).await?;
                Ok(true)
            }
        },
        team::ResetState::Missing => {
            write_stdout(format!("no session {}\n", target.name).into_bytes()).await?;
            Ok(false)
        }
        team::ResetState::Exited { id } => {
            write_stdout(format!("skipped {} {id} (exited)\n", target.name).into_bytes()).await?;
            Ok(false)
        }
        team::ResetState::SkippedAttached { id } => {
            write_stdout(
                format!(
                    "skipped {} {id} (attached; use --include-attached or name it)\n",
                    target.name
                )
                .into_bytes(),
            )
            .await?;
            Ok(false)
        }
    }
}
