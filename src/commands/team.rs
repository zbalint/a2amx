//! Team status, up, down, reset, and team-file resolution.
mod dry_run;

use self::dry_run::dry_run_report;
use super::{
    grants::{auto_grants_for, confirm_new_grants, load_accepted_grants, project_key},
    launch::{NewOptions, create_session, parse_activity_watch, resolved_confine_request},
    sessions::reset_one,
};
use crate::attach::{self, AttachOptions};
use crate::client::Client;
use crate::harness::Harness;
use crate::lookup::request_sessions;
use crate::messaging::{TeamScope, display_duration};
use crate::table::session_confinement_mode;
use crate::team::{self, TeamSession};
use crate::terminal::confirm;
use crate::terminal::{stdin_is_terminal, terminal_size_with_default, write_stderr, write_stdout};
use crate::wire::{Request, Response};
use anyhow::{Context, anyhow, bail};
use std::collections::BTreeMap;
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
        let path = invoking_cwd.join(file);
        let parent = path.parent().unwrap_or(&invoking_cwd).to_owned();
        let dir = tokio::task::spawn_blocking(move || {
            std::fs::canonicalize(&parent).with_context(|| {
                format!(
                    "cannot canonicalize team file directory {}",
                    parent.display()
                )
            })
        })
        .await??;
        return Ok(TeamSource {
            path,
            dir,
            global: false,
        });
    }
    let state_home = home.to_owned();
    let user_home = std::env::var_os("HOME").map(PathBuf::from);
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<TeamSource> {
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
    })
    .await??;
    if result.global {
        let name = team::team_name_for(&result.dir, "create an a2amx.toml here, or pass --file")?;
        write_stderr(
            format!(
                "using global team file {} as team {name}\n",
                result.path.display()
            )
            .into_bytes(),
        )
        .await?;
    }
    Ok(result)
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
    let existing = request_sessions(&mut client).await?;
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
    let Ok(Response::Messages { messages }) = client
        .request(Request::ListMessages {
            session: None,
            state: Some("undeliverable".to_owned()),
        })
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
        harness,
        &resolved_cwd,
        &host,
        std::env::var_os("PATH"),
        preflight,
    )?;
    Ok(())
}

async fn start_planned_session(
    home: &Path,
    client: &mut Client,
    session: Box<TeamSession>,
    origin: TeamOrigin<'_>,
    cols: u16,
    rows: u16,
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
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(&session, &resolved_cwd)?;
    let confine = session
        .confine
        .as_ref()
        .map(|request| resolved_confine_request(request, &resolved_cwd, origin.file))
        .transpose()?;
    let name = session.name;
    let result = create_session(
        home,
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
            cols,
            rows,
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
        match client.request(Request::IssueLeaseCode { team }).await {
            Ok(Response::LeaseCode {
                code,
                agents,
                valid_secs,
            }) => {
                write_stdout(format!("remote messaging for: {}\n", agents.join(", ")).into_bytes())
                    .await?;
                write_stdout(
                    format!(
                        "post this code in the control channel within {}: {code}\n",
                        display_duration(Duration::from_secs(valid_secs))
                    )
                    .into_bytes(),
                )
                .await?;
            }
            Ok(Response::Error { message }) => {
                write_stderr(format!("warning: {message}\n").into_bytes()).await?;
            }
            Ok(other) => {
                write_stderr(
                    format!("warning: unexpected lease response: {other:?}\n").into_bytes(),
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

pub(super) async fn run_team_up(
    home: PathBuf,
    attach_options: AttachOptions,
    file: Option<PathBuf>,
    detach: bool,
    dry_run: bool,
    yes: bool,
    items: Vec<String>,
) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let (wanted, file_dir, team_file) = if items.is_empty() {
        let source = resolve_team_file(&home, file).await?;
        let file_dir = source.dir.clone();
        let team_path = source.path.clone();
        let team_file =
            Some(tokio::task::spawn_blocking(move || std::fs::canonicalize(team_path)).await??);
        (read_team(source).await?, file_dir, team_file)
    } else {
        (team::flag_sessions(&items)?, cwd.clone(), None)
    };
    let attach_name = wanted
        .iter()
        .find(|session| session.attach)
        .map(|session| session.name.as_str());
    let mut client = Client::connect(&home).await?;
    let existing = request_sessions(&mut client).await?;
    let mut actions = match team::plan(&wanted, &existing) {
        Ok(actions) => actions,
        Err(conflicts) => {
            note_lost_messages(&home, &wanted).await;
            write_conflicts(&conflicts).await?;
            return Err(anyhow!("team has session conflicts"));
        }
    };
    let origin = TeamOrigin {
        dir: &file_dir,
        file: team_file.as_deref(),
    };
    resolve_auto_grants(&mut actions, origin).await?;
    for action in &actions {
        if let team::Action::Start(session) = action {
            validate_team_session(&home, origin, session)?;
        }
    }
    let has_auto = actions.iter().any(|action| {
        matches!(
            action,
            team::Action::Start(session) if session.auto_confine().is_some()
        )
    });
    let remote_requested = wanted.iter().any(|session| session.remote.is_some());
    let remote_team = wanted
        .iter()
        .find_map(|session| session.team.as_ref().map(|team| team.name.clone()));
    let accepted_all = if has_auto {
        load_accepted_grants(&home).await?
    } else {
        BTreeMap::new()
    };
    let project_key = if has_auto {
        Some(project_key(origin.dir)?.to_owned())
    } else {
        None
    };
    let accepted = project_key
        .as_ref()
        .and_then(|key| accepted_all.get(key))
        .cloned()
        .unwrap_or_default();
    let new_grants = team::new_auto_grants(&actions, &accepted);
    for output in team::escape_notices(&actions, crate::grant::all()) {
        write_stderr(output.into_bytes()).await?;
    }
    if !dry_run && !new_grants.is_empty() {
        confirm_new_grants(&home, origin.dir, &accepted_all, &new_grants, yes).await?;
    }
    write_auto_summaries(&actions).await?;
    if dry_run {
        let output = dry_run_report(&home, origin, &actions, &existing, &accepted).await?;
        write_stdout(output.into_bytes()).await?;
        note_lost_messages(&home, &wanted).await;
        return Ok(());
    }
    note_lost_messages(&home, &wanted).await;
    let (cols, rows) = terminal_size_with_default()?;
    let mut attached_id = None;
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
            team::Action::Start(session) => {
                start_planned_session(&home, &mut client, session, origin, cols, rows).await?
            }
        };
        if attach_name == Some(name.as_str()) {
            attached_id = Some(id);
        }
    }
    if remote_requested {
        issue_lease_code(&mut client, remote_team).await?;
    }
    if !detach
        && stdin_is_terminal()
        && let Some(session) = attached_id
    {
        let (attach_cols, attach_rows) = if remote_requested {
            wait_for_enter().await?;
            terminal_size_with_default()?
        } else {
            (cols, rows)
        };
        attach::run(
            client,
            &home,
            attach_options,
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
    let sessions = request_sessions(&mut client).await?;
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
        let result = client
            .request(Request::Kill {
                session: session.id.clone(),
                now,
            })
            .await;
        let error = match result {
            Ok(Response::Ok) => {
                write_stdout(format!("killed {name} {}\n", session.id).into_bytes()).await?;
                continue;
            }
            Ok(Response::Error { message }) => message,
            Ok(other) => format!("unexpected daemon response: {other:?}"),
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

pub(super) async fn run_team_reset(
    home: PathBuf,
    file: Option<PathBuf>,
    names: Vec<String>,
    except: Vec<String>,
    include_attached: bool,
    yes: bool,
) -> anyhow::Result<()> {
    let explicit = !names.is_empty();
    let names = if explicit {
        names
    } else {
        let source = resolve_team_file(&home, file).await?;
        read_team(source)
            .await?
            .into_iter()
            .map(|session| session.name)
            .collect()
    };
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    let targets = team::reset_targets(&names, explicit, &except, include_attached, &sessions);
    let running = targets
        .iter()
        .filter(|target| matches!(target.state, team::ResetState::Reset { .. }))
        .count();
    if running > 0 && !yes {
        let mut preview = String::new();
        for target in &targets {
            let team::ResetState::Reset { id } = &target.state else {
                continue;
            };
            let attached = sessions
                .iter()
                .find(|session| session.id == *id)
                .is_some_and(|session| session.attached);
            preview.push_str(&format!(
                "{} {}{}\n",
                target.name,
                id,
                if attached { " (attached)" } else { "" }
            ));
        }
        write_stdout(preview.into_bytes()).await?;
        let prompt = format!("Reset {running} session(s)? This clears their conversations. [y/N] ");
        if !confirm(
            &prompt,
            &format!("refusing to reset: {running} running session(s); pass --yes to reset them"),
        )
        .await?
        {
            return write_stdout(b"not reset\n".to_vec()).await;
        }
    }

    let mut failed = false;
    for target in targets {
        match target.state {
            team::ResetState::Reset { id } => match reset_one(&mut client, &id).await {
                Ok(steps) => {
                    let noun = if steps == 1 { "step" } else { "steps" };
                    write_stdout(
                        format!("reset {} {id}: {steps} {noun}\n", target.name).into_bytes(),
                    )
                    .await?;
                }
                Err(error) => {
                    write_stderr(format!("failed {}: {error}\n", target.name).into_bytes()).await?;
                    failed = true;
                }
            },
            team::ResetState::Missing => {
                write_stdout(format!("no session {}\n", target.name).into_bytes()).await?;
            }
            team::ResetState::Exited { id } => {
                write_stdout(format!("skipped {} {id} (exited)\n", target.name).into_bytes())
                    .await?;
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
            }
        }
    }
    if failed {
        return Err(anyhow!("one or more team sessions could not be reset"));
    }
    Ok(())
}
