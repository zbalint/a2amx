//! Commands operating on existing sessions and messages.

use crate::attach::{self, AttachOptions};
use crate::client::Client;
use crate::lookup::{request_sessions, resolve_reference, resolve_session};
use crate::messaging;
use crate::table::{format_messages_table, format_session_table, session_confinement_mode};
use crate::terminal::confirm;
use crate::terminal::{stdin_is_terminal, terminal_size_with_default, write_stderr, write_stdout};
use crate::wire::{Activity, Request, Response};
use anyhow::{anyhow, bail};
use std::path::{Path, PathBuf};

pub(super) async fn run_list(
    home: PathBuf,
    details: bool,
    team_filter: Option<String>,
    viewer: Option<String>,
) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::List).await?;
    let mut sessions = match response {
        Response::Sessions { sessions } => sessions,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
    let viewer_info = viewer.map(|name| {
        let session = sessions
            .iter()
            .find(|session| session.name.as_deref() == Some(name.as_str()))
            .or_else(|| sessions.iter().find(|session| session.id == name))
            .ok_or_else(|| anyhow!("unknown session {name}"))?;
        Ok::<_, anyhow::Error>((
            session.id.clone(),
            session.name.clone(),
            session.team.clone(),
        ))
    });
    let viewer_info = viewer_info.transpose()?;
    sessions.retain(|session| {
        let in_team = team_filter.as_deref().is_none_or(|team| {
            session
                .team
                .as_ref()
                .is_some_and(|scope| scope.name == team)
        });
        let visible_to_viewer =
            viewer_info
                .as_ref()
                .is_none_or(|(viewer_id, viewer_name, viewer_scope)| {
                    session.id == *viewer_id
                        || messaging::visible(
                            messaging::Party {
                                name: viewer_name.as_deref(),
                                team: viewer_scope.as_ref(),
                            },
                            messaging::Party {
                                name: session.name.as_deref(),
                                team: session.team.as_ref(),
                            },
                        )
                });
        in_team && visible_to_viewer
    });
    write_stdout(format_session_table(&sessions, details).into_bytes()).await
}

pub(super) async fn run_confine_show(
    home: PathBuf,
    reference: Option<String>,
) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    if sessions.is_empty() {
        if let Some(reference) = reference {
            bail!("unknown session {reference}");
        }
        write_stdout(b"no sessions\n".to_vec()).await?;
        return Ok(());
    }
    let selected = match reference.as_deref() {
        Some(reference) => {
            let session = sessions
                .iter()
                .find(|session| {
                    session.id == reference || session.name.as_deref() == Some(reference)
                })
                .ok_or_else(|| anyhow!("unknown session {reference}"))?;
            vec![session]
        }
        None => sessions.iter().collect(),
    };
    let mut output = String::new();
    for session in selected {
        let name = session.name.as_deref().unwrap_or("-");
        let mode = session_confinement_mode(session);
        output.push_str(&format!(
            "{name} {} {} confine {mode}\n",
            session.id,
            session.harness.as_str()
        ));
        let Some(info) = &session.confine else {
            continue;
        };
        output.push_str("GRANT  SOURCE\n");
        for grant in &info.grants {
            let source = info
                .reasons
                .iter()
                .find(|reason| reason.grant == *grant)
                .map_or_else(
                    || "set".to_owned(),
                    |reason| format!("auto: {}", reason.reason),
                );
            output.push_str(&format!("{grant}  {source}\n"));
        }
        if !info.write.is_empty() {
            output.push_str(&format!("write: {}\n", info.write.join(", ")));
        }
    }
    write_stdout(output.into_bytes()).await
}

pub(super) async fn run_kill(
    home: PathBuf,
    reference: Option<String>,
    exited: bool,
    yes: bool,
    now: bool,
) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let sessions = request_sessions(&mut client).await?;
    if exited {
        let mut found = false;
        let mut failed = false;
        for summary in sessions
            .iter()
            .filter(|summary| summary.exit_code.is_some())
        {
            found = true;
            let label = summary.name.as_deref().unwrap_or(&summary.id);
            match kill_session(&mut client, summary.id.clone(), false).await {
                Ok(()) => write_stdout(format!("removed {label}\n").into_bytes()).await?,
                Err(error) => {
                    write_stderr(format!("{label}: {error}\n").into_bytes()).await?;
                    failed = true;
                }
            }
        }
        if !found {
            return write_stdout(b"no exited sessions\n".to_vec()).await;
        }
        if failed {
            return Err(anyhow!("one or more exited sessions could not be removed"));
        }
        return Ok(());
    }

    let reference = reference.ok_or_else(|| anyhow!("kill requires a session or --exited"))?;
    let session = resolve_reference(&sessions, &reference);
    let running = sessions
        .iter()
        .any(|summary| summary.id == session && summary.exit_code.is_none());
    if running && !yes {
        let prompt = format!("Killing {session} ends a running session. Continue? [y/N] ");
        if !confirm(
            &prompt,
            &format!("refusing to kill: session {session} is running; pass --yes to end it"),
        )
        .await?
        {
            return write_stdout(b"not killed\n".to_vec()).await;
        }
    }
    kill_session(&mut client, session, now).await
}

async fn kill_session(client: &mut Client, session: String, now: bool) -> anyhow::Result<()> {
    match client.request(Request::Kill { session, now }).await? {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

pub(super) async fn reset_one(client: &mut Client, session: &str) -> anyhow::Result<usize> {
    match client
        .request(Request::Reset {
            session: session.to_owned(),
        })
        .await?
    {
        Response::Reset { steps } => {
            usize::try_from(steps).map_err(|_| anyhow!("reset step count does not fit in usize"))
        }
        Response::Failed { code, message } => Err(anyhow!("{code}: {message}")),
        Response::Error { message } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

pub(super) async fn run_reset(home: PathBuf, reference: String) -> anyhow::Result<()> {
    let session = resolve_session(&home, &reference).await?;
    let mut client = Client::connect(&home).await?;
    let steps = reset_one(&mut client, &session).await?;
    let noun = if steps == 1 { "step" } else { "steps" };
    write_stdout(format!("reset {session}: {steps} {noun}\n").into_bytes()).await?;
    Ok(())
}

pub(super) async fn run_explain(home: PathBuf, reference: String) -> anyhow::Result<()> {
    let session = resolve_session(&home, &reference).await?;
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::Explain {
            session: session.clone(),
        })
        .await?;
    let info = match response {
        Response::Explain { info } => info,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected explain response: {other:?}")),
    };
    let mut output = String::new();
    if let Some(code) = info.exit_code {
        output.push_str(&format!(
            "{} ({}): {}, exited (code {code})\n",
            info.id,
            info.address,
            info.harness.as_str(),
        ));
    } else {
        output.push_str(&format!(
            "{} ({}): {}, running\n",
            info.id,
            info.address,
            info.harness.as_str()
        ));
        let activity = info.activity.map(Activity::as_str).unwrap_or("unknown");
        let cause = info.activity_cause.as_deref().unwrap_or("unknown");
        output.push_str(&format!("activity: {activity} ({cause})\n"));
        let ready = match info.ready {
            Some(true) => "yes",
            Some(false) if info.starting => "no (starting)",
            Some(false) => "no",
            None => "unknown",
        };
        output.push_str(&format!("ready: {ready}\n"));
        output.push_str(&format!(
            "needs input: {}\n",
            info.needs_input.as_deref().unwrap_or("no")
        ));
        if let Some(hold) = info.hold {
            output.push_str("hold: ");
            output.push_str(&hold.reason);
            if let Some(explanation) = hold.explanation {
                output.push_str(" - ");
                output.push_str(&explanation);
            }
            output.push('\n');
        } else {
            output.push_str("hold: none\n");
        }
        output.push_str(&format!("queued: {}\n", info.pending));
        if info.reset.allowed {
            output.push_str("reset: gate open\n");
        } else {
            output.push_str("reset: refused (");
            output.push_str(info.reset.code.as_deref().unwrap_or("unknown"));
            output.push_str("): ");
            output.push_str(info.reset.reason.as_deref().unwrap_or("unknown"));
            output.push('\n');
        }
    }

    let sessions = request_sessions(&mut client).await?;
    if let Some(summary) = sessions.into_iter().find(|summary| summary.id == session)
        && let Some(confine) = summary.confine
    {
        let default_hides = crate::confine::default_hide_names(summary.harness);
        let cwd = summary.cwd;
        let user_home = std::env::var_os("HOME").map(PathBuf::from);
        let help = tokio::task::spawn_blocking(move || {
            let exists = |path: &str| {
                if path == "~" {
                    return user_home.as_ref().is_some_and(|home| home.exists());
                }
                if let Some(relative) = path.strip_prefix("~/") {
                    return user_home
                        .as_ref()
                        .is_some_and(|home| home.join(relative).exists());
                }
                Path::new(path).exists()
            };
            crate::grant::denial_help(
                &confine,
                cwd.as_deref(),
                &default_hides,
                crate::grant::all(),
                &exists,
            )
        })
        .await?;
        output.push_str(&help);
    }
    write_stdout(output.into_bytes()).await
}

pub(super) async fn run_screen(
    home: PathBuf,
    session: String,
    rows: Option<usize>,
) -> anyhow::Result<()> {
    let session = resolve_session(&home, &session).await?;
    let mut client = Client::connect(&home).await?;
    let response = client.request(Request::Screen { session }).await?;
    let lines = match response {
        Response::Screen { lines } => lines,
        Response::Error { message } => return Err(anyhow!(message)),
        other => return Err(anyhow!("unexpected screen response: {other:?}")),
    };
    let lines = if let Some(rows) = rows {
        let start = lines.len().saturating_sub(rows);
        lines.into_iter().skip(start).collect::<Vec<_>>()
    } else {
        lines
    };
    if lines.is_empty() {
        return write_stdout(Vec::new()).await;
    }
    write_stdout(format!("{}\n", lines.join("\n")).into_bytes()).await
}

pub(super) async fn run_messages(
    home: PathBuf,
    session: Option<String>,
    state: Option<String>,
) -> anyhow::Result<()> {
    let session = match session {
        Some(reference) => Some(resolve_session(&home, &reference).await?),
        None => None,
    };
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::ListMessages { session, state })
        .await?;
    let messages = match response {
        Response::Messages { messages } => messages,
        Response::Error { message } | Response::Failed { message, .. } => {
            return Err(anyhow!(message));
        }
        other => return Err(anyhow!("unexpected daemon response: {other:?}")),
    };
    write_stdout(format_messages_table(&messages).into_bytes()).await
}

pub(super) async fn run_cancel(home: PathBuf, message: String) -> anyhow::Result<()> {
    let mut client = Client::connect(&home).await?;
    let response = client
        .request(Request::CancelMessage { id: message })
        .await?;
    match response {
        Response::Ok => Ok(()),
        Response::Error { message } | Response::Failed { message, .. } => Err(anyhow!(message)),
        other => Err(anyhow!("unexpected daemon response: {other:?}")),
    }
}

pub(super) async fn run_attach_command(
    home: PathBuf,
    attach_options: AttachOptions,
    session: String,
    force: bool,
) -> anyhow::Result<()> {
    if !stdin_is_terminal() {
        return Err(anyhow!("attach needs a terminal on stdin"));
    }
    let session = resolve_session(&home, &session).await?;
    let client = Client::connect(&home).await?;
    let (cols, rows) = terminal_size_with_default()?;
    attach::run(client, &home, attach_options, session, force, cols, rows).await
}
