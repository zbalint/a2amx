//! Session launch request construction and session creation.

use crate::attach::{self, AttachOptions};
use crate::cli::Command;
use crate::client::Client;
use crate::confine::ConfineRequest;
use crate::daemon::{self};
use crate::harness::{Deliver, Harness};
use crate::messaging::{self, TeamScope};
use crate::names;
use crate::terminal::{stdin_is_terminal, terminal_size_with_default, write_stderr, write_stdout};
use crate::wire::Request;
use anyhow::anyhow;
use std::path::{Path, PathBuf};

pub(super) fn parse_activity_watch(
    working_after: Option<&str>,
    idle_after: Option<&str>,
) -> anyhow::Result<Option<crate::wire::ActivityWatch>> {
    let working_after = working_after
        .map(|value| crate::messaging::parse_interval(value, "activity working_after"))
        .transpose()?
        .map(|duration| duration.as_secs());
    let idle_after = idle_after
        .map(|value| crate::messaging::parse_interval(value, "activity idle_after"))
        .transpose()?
        .map(|duration| duration.as_secs());
    Ok(match (working_after, idle_after) {
        (None, None) => None,
        (working_after, idle_after) => Some(crate::wire::ActivityWatch {
            working_after,
            idle_after,
        }),
    })
}

pub(super) async fn run_new(
    home: PathBuf,
    attach_options: AttachOptions,
    options: Command,
) -> anyhow::Result<()> {
    let Command::New {
        detach,
        name,
        harness,
        deliver,
        no_authorize_peers,
        no_channel,
        confine,
        remote,
        confine_write,
        confine_hide,
        confine_unhide,
        confine_default_hides,
        mut confine_grant,
        confine_keep_env,
        reset,
        control_from,
        watch,
        activity_working_after,
        activity_idle_after,
        heartbeat,
        command,
    } = options
    else {
        return Err(anyhow!("expected new session options"));
    };
    let mut unique_grants = Vec::with_capacity(confine_grant.len());
    for grant in confine_grant {
        if !unique_grants.contains(&grant) {
            unique_grants.push(grant);
        }
    }
    confine_grant = unique_grants;
    let has_confine_options = !confine_write.is_empty()
        || !confine_hide.is_empty()
        || !confine_unhide.is_empty()
        || confine_default_hides.is_some()
        || !confine_grant.is_empty()
        || !confine_keep_env.is_empty();
    if has_confine_options && !confine {
        return Err(anyhow!("--confine-* flags require --confine"));
    }
    if confine {
        crate::grant::validate(&confine_grant)?;
    }

    let remote = remote
        .map(|value| messaging::RemoteMode::parse(&value))
        .transpose()?;
    let activity = parse_activity_watch(
        activity_working_after.as_deref(),
        activity_idle_after.as_deref(),
    )?;
    if let Some(activity) = &activity {
        activity.validate(!watch.is_empty())?;
    }
    let heartbeat = heartbeat
        .as_deref()
        .map(|value| crate::messaging::parse_interval(value, "heartbeat"))
        .transpose()?
        .map(|duration| duration.as_secs());
    if heartbeat.is_some() && watch.is_empty() {
        return Err(anyhow!("heartbeat needs a non-empty watch"));
    }
    let mut client = Client::connect(&home).await?;
    let mut generated_name = None;
    let name = match name {
        Some(name) => Some(name),
        None => {
            let taken = client
                .sessions()
                .await?
                .into_iter()
                .filter_map(|session| session.name)
                .collect::<Vec<_>>();
            let random = tokio::task::spawn_blocking(daemon::random_hex::<8>).await??;
            let seed = u64::from_str_radix(&random, 16)?;
            // shortcut: ceiling is concurrent `new` on one daemon and very churned registry;
            // upgrade trigger is daemon-side generation if either is reported.
            let generated = names::pick(&taken, seed)
                .ok_or_else(|| anyhow!("no free generated session name; pass --name"))?;
            generated_name = Some(generated.clone());
            Some(generated)
        }
    };
    let (cols, rows) = terminal_size_with_default()?;
    let cwd = std::env::current_dir()?;
    let confine = if confine {
        let request = ConfineRequest {
            write: confine_write,
            hide: confine_hide,
            unhide: confine_unhide,
            no_default_hides: confine_default_hides == Some(false),
            grants: confine_grant,
            keep_env: confine_keep_env,
            auto: false,
            reasons: Vec::new(),
            team_file: None,
        };
        Some(resolved_confine_request(&request, &cwd, None)?)
    } else {
        None
    };
    let session = create_session(
        &home,
        &mut client,
        NewOptions {
            name,
            harness,
            role: None,
            remote,
            team: None,
            deliver,
            no_authorize_peers,
            no_channel,
            confine,
            reset,
            control_from,
            watch,
            activity,
            heartbeat,
            command,
            cwd,
            cols,
            rows,
        },
    )
    .await?;
    if let Some(name) = generated_name {
        write_stderr(format!("name: {name}\n").into_bytes()).await?;
    }
    if detach {
        write_stdout(format!("{session}\n").into_bytes()).await?;
        return Ok(());
    }
    if !stdin_is_terminal() {
        return Err(anyhow!("attach needs a terminal on stdin"));
    }
    attach::run(client, &home, attach_options, session, false, cols, rows).await
}

pub(super) struct NewOptions {
    pub(super) name: Option<String>,
    pub(super) role: Option<String>,
    pub(super) remote: Option<messaging::RemoteMode>,
    pub(super) team: Option<TeamScope>,
    pub(super) harness: Option<Harness>,
    pub(super) deliver: Option<Deliver>,
    pub(super) no_authorize_peers: bool,
    pub(super) no_channel: bool,
    pub(super) confine: Option<ConfineRequest>,
    pub(super) reset: Vec<String>,
    pub(super) control_from: Vec<String>,
    pub(super) watch: Vec<String>,
    pub(super) activity: Option<crate::wire::ActivityWatch>,
    pub(super) heartbeat: Option<u64>,
    pub(super) command: Vec<String>,
    pub(super) cwd: PathBuf,
    pub(super) cols: u16,
    pub(super) rows: u16,
}

/// D6 fixes this argument shape so launch wiring is shared by real and dry-run paths.
#[allow(clippy::too_many_arguments)]
pub(super) async fn wired_command(
    home: &Path,
    command: Vec<String>,
    harness: Harness,
    cwd: &Path,
    role: Option<&str>,
    no_authorize_peers: bool,
    no_channel: bool,
    install_omp: bool,
) -> anyhow::Result<Vec<String>> {
    match harness {
        Harness::Claude => {
            let exe = std::env::current_exe()?;
            let wire_cwd = cwd.to_owned();
            let wire_config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .or_else(|| {
                    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
                })
                .unwrap_or_default();
            let wire_role = role.map(str::to_owned);
            let authorize_peers = !no_authorize_peers;
            if no_channel {
                Ok(tokio::task::spawn_blocking(move || {
                    crate::harness::wire_claude_argv(
                        command,
                        &exe,
                        &wire_cwd,
                        &wire_config_dir,
                        authorize_peers,
                        wire_role.as_deref(),
                    )
                })
                .await??)
            } else {
                Ok(tokio::task::spawn_blocking(move || {
                    crate::harness::wire_claude_channel_argv(
                        command,
                        &exe,
                        &wire_cwd,
                        &wire_config_dir,
                        authorize_peers,
                        wire_role.as_deref(),
                    )
                })
                .await??)
            }
        }
        Harness::Agy => {
            let exe = std::env::current_exe()?;
            let wire_cwd = cwd.to_owned();
            let gemini_dir = std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".gemini"))
                .unwrap_or_default();
            if install_omp {
                let layout = crate::layout::Layout::new(home.to_owned());
                // shortcut: relative command[0] resolves from the launcher's directory; canonicalizing against cwd would change the install API.
                let agy_program = std::ffi::OsString::from(
                    command
                        .first()
                        .cloned()
                        .ok_or_else(|| anyhow!("empty command"))?,
                );
                tokio::task::spawn_blocking(move || crate::agy::install(&layout, &agy_program))
                    .await??;
            }
            Ok(tokio::task::spawn_blocking(move || {
                crate::harness::wire_agy_argv(command, &exe, &wire_cwd, &gemini_dir)
            })
            .await??)
        }
        Harness::Omp if !install_omp => Ok(command),
        Harness::Omp => {
            let install_layout = crate::layout::Layout::new(home.to_owned());
            let installed =
                tokio::task::spawn_blocking(move || crate::omp::install(&install_layout)).await??;
            Ok(crate::harness::wire_omp_argv(
                command,
                &installed.extension,
                &installed.overlay,
                !no_authorize_peers,
                role,
            ))
        }
        Harness::Generic | Harness::Codex => Ok(command),
    }
}

fn resolve_confine_paths(
    base: &Path,
    option: &str,
    paths: &[String],
) -> anyhow::Result<Vec<String>> {
    paths
        .iter()
        .map(|entry| {
            crate::confine::check_entry_syntax(option, entry)?;
            let path = if entry == "~" || entry.starts_with("~/") {
                PathBuf::from(entry)
            } else {
                let path = Path::new(entry);
                if path.is_absolute() {
                    path.to_owned()
                } else {
                    base.join(path)
                }
            };
            Ok(path.to_string_lossy().into_owned())
        })
        .collect()
}

/// Resolve a confinement request's relative paths and attach its team file.
pub(super) fn resolved_confine_request(
    request: &ConfineRequest,
    cwd: &Path,
    team_file: Option<&Path>,
) -> anyhow::Result<ConfineRequest> {
    Ok(ConfineRequest {
        write: resolve_confine_paths(cwd, "write", &request.write)?,
        hide: resolve_confine_paths(cwd, "hide", &request.hide)?,
        unhide: resolve_confine_paths(cwd, "unhide", &request.unhide)?,
        no_default_hides: request.no_default_hides,
        grants: request.grants.clone(),
        auto: request.auto,
        reasons: request.reasons.clone(),
        keep_env: request.keep_env.clone(),
        team_file: team_file
            .map(|path| path.to_string_lossy().into_owned())
            .or_else(|| request.team_file.clone()),
    })
}

pub(super) async fn create_session(
    home: &Path,
    client: &mut Client,
    options: NewOptions,
) -> anyhow::Result<String> {
    let NewOptions {
        name,
        harness,
        deliver,
        role,
        remote,
        team,
        no_authorize_peers,
        no_channel,
        confine,
        reset,
        control_from,
        watch,
        activity,
        heartbeat,
        command,
        cwd,
        cols,
        rows,
    } = options;
    crate::messaging::validate_reset_steps(&reset)?;
    for controller in &control_from {
        crate::messaging::validate_name(controller)?;
    }
    for name in &watch {
        crate::messaging::validate_name(name)?;
    }
    if let Some(seconds) = heartbeat {
        if watch.is_empty() {
            return Err(anyhow!("heartbeat needs a non-empty watch"));
        }
        crate::messaging::parse_interval(&format!("{seconds}s"), "heartbeat")?;
    }
    if let Some(activity) = &activity {
        activity.validate(!watch.is_empty())?;
    }
    let harness = harness.unwrap_or_else(|| Harness::infer(&command));
    let command = wired_command(
        home,
        command,
        harness,
        &cwd,
        role.as_deref(),
        no_authorize_peers,
        no_channel,
        true,
    )
    .await?;
    let cwd = cwd.to_string_lossy().into_owned();
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.retain(|(key, _)| key != crate::codex::ROLE_ENV);
    if harness == Harness::Omp {
        env.push((
            "A2AMX_BIN".to_owned(),
            std::env::current_exe()?.to_string_lossy().into_owned(),
        ));
    }
    if harness == Harness::Codex && no_authorize_peers {
        env.push((crate::codex::NO_AUTHORIZE_ENV.to_owned(), "1".to_owned()));
    }
    if harness == Harness::Codex
        && let Some(role) = &role
    {
        env.push((crate::codex::ROLE_ENV.to_owned(), role.clone()));
    }
    if harness == Harness::Claude && !no_channel {
        env.push((crate::harness::CHANNEL_ENV.to_owned(), "1".to_owned()));
    }
    let request = Request::NewSession {
        argv: command,
        cols,
        rows,
        cwd: Some(cwd),
        env,
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
        heartbeat,
    };
    let request = match confine {
        Some(confine) => Request::Confine {
            confine,
            request: Box::new(request),
        },
        None => request,
    };
    client.create_session(request).await
}
