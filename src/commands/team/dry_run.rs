//! Team up dry-run reporting and command-line redaction.

use super::super::launch::{resolved_confine_request, wired_command};
use super::TeamOrigin;
use crate::harness::Harness;
use crate::messaging::TeamScope;
use crate::table::session_confinement_mode;
use crate::team::{self, TeamSession};
use crate::wire::SessionSummary;
use anyhow::Context;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn team_detail(team: Option<&TeamScope>) -> Option<String> {
    team.map(|team| {
        let visibility = if team.private { "private" } else { "public" };
        let allow = if team.allow.is_empty() {
            "-".to_owned()
        } else {
            team.allow.join(",")
        };
        format!("{}, {visibility}, allow: {allow}", team.name)
    })
}

fn is_redactable_flag(value: &str) -> bool {
    if let Some(long) = value.strip_prefix("--") {
        let mut chars = long.bytes();
        return matches!(chars.next(), Some(byte) if byte.is_ascii_alphabetic())
            && chars.all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
    }
    if let Some(short) = value.strip_prefix('-') {
        return short.len() == 1 && short.as_bytes()[0].is_ascii_alphabetic();
    }
    false
}

fn redact_argv(argv: &[String]) -> String {
    let Some(first) = argv.first() else {
        return String::new();
    };
    let mut output = vec![first.clone()];
    let mut after_separator = false;
    for argument in &argv[1..] {
        if after_separator {
            output.push(format!("<{} bytes>", argument.len()));
            continue;
        }
        if argument == "--" {
            after_separator = true;
            output.push(argument.clone());
            continue;
        }
        let (flag, value) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(flag, value)| {
                (flag, Some(value))
            });
        if is_redactable_flag(flag) {
            match value {
                Some(value) => output.push(format!("{flag}=<redacted, {} bytes>", value.len())),
                None => output.push(argument.clone()),
            }
        } else {
            output.push(format!("<{} bytes>", argument.len()));
        }
    }
    output.join(" ")
}

pub(super) async fn dry_run_report(
    home: &Path,
    origin: TeamOrigin<'_>,
    actions: &[team::Action],
    existing: &[SessionSummary],
    accepted: &BTreeSet<String>,
) -> anyhow::Result<String> {
    let mut output = String::new();
    for action in actions {
        match action {
            team::Action::AlreadyRunning { name, id } => {
                let mode = existing
                    .iter()
                    .find(|session| session.id == *id)
                    .map_or("off", session_confinement_mode);
                output.push_str(&format!("already running {name} {id} (confine {mode})\n"));
            }
            team::Action::Start(session) => {
                output.push_str(&dry_run_start(home, origin, session, accepted).await?);
            }
        }
    }
    Ok(output)
}

async fn dry_run_start(
    home: &Path,
    origin: TeamOrigin<'_>,
    session: &TeamSession,
    accepted: &BTreeSet<String>,
) -> anyhow::Result<String> {
    let harness = Harness::infer(&session.command);
    let resolved_cwd = session
        .cwd
        .as_deref()
        .map_or_else(|| origin.dir.to_owned(), |path| origin.dir.join(path));
    team::validate_session_confine(session, &resolved_cwd)?;
    let confine_plan = if let Some(request) = session.confine.as_ref() {
        let host_home = PathBuf::from(std::env::var_os("HOME").context("HOME not set")?);
        let runtime_dir =
            PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR not set")?);
        let request = resolved_confine_request(request, &resolved_cwd, origin.file)?;
        let host = crate::confine::Host {
            home: host_home,
            layout: crate::layout::Layout::new(home.to_owned()),
            runtime_dir,
        };
        let preflight = crate::confine::ensure_grant_dirs(&request, harness, &host, false);
        Some(
            crate::confine::plan_with_preflight(
                &request,
                crate::confine::PlanTarget {
                    harness,
                    cwd: &resolved_cwd,
                    host: &host,
                },
                std::env::var_os("PATH"),
                preflight,
            )?
            .with_bwrap(PathBuf::from("bwrap")),
        )
    } else {
        None
    };
    let command = wired_command(
        home,
        session.command.clone(),
        harness,
        &resolved_cwd,
        session.role.as_deref(),
        false,
        false,
        false,
    )
    .await?;
    let mut output = format!(
        "would start {}\n  harness: {}\n  cwd: {}\n",
        session.name,
        harness.as_str(),
        resolved_cwd.display()
    );
    for (label, value) in session_details(session) {
        output.push_str(&format!("  {label}: {value}\n"));
    }
    if let Some(plan) = &confine_plan {
        output.push_str(&crate::confine::dry_run_lines(
            plan,
            &resolved_cwd,
            accepted,
        ));
    } else {
        output.push_str("  confinement: off\n");
    }
    output.push_str(&format!("  command: {}\n", redact_argv(&command)));
    if harness == Harness::Omp {
        output.push_str("omp extension install skipped (dry run)\n");
    }
    Ok(output)
}

fn session_details(session: &TeamSession) -> Vec<(&'static str, String)> {
    [
        team_detail(session.team.as_ref()).map(|value| ("team", value)),
        session.role.as_ref().map(|value| ("role", value.clone())),
        session.attach.then(|| ("attach", "true".to_owned())),
        (!session.watch.is_empty()).then(|| ("watch", session.watch.join(", "))),
        (!session.control_from.is_empty())
            .then(|| ("control_from", session.control_from.join(", "))),
        session
            .heartbeat
            .as_ref()
            .map(|value| ("heartbeat", value.clone())),
        session
            .activity
            .as_ref()
            .map(|value| ("activity", activity_summary(value))),
    ]
    .into_iter()
    .flatten()
    .collect()
}

fn activity_summary(activity: &team::TeamActivity) -> String {
    [
        activity
            .working_after
            .as_ref()
            .map(|value| format!("working_after {value}")),
        activity
            .idle_after
            .as_ref()
            .map(|value| format!("idle_after {value}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ")
}
