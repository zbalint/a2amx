use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::bail;

use super::TeamSession;
use crate::harness::Harness;
use crate::messaging::{parse_interval, validate_name, validate_reset_steps, validate_role};

fn validate_activity(session: &TeamSession) -> anyhow::Result<()> {
    let Some(activity) = &session.activity else {
        return Ok(());
    };
    if session.watch.is_empty() {
        bail!("session {}: activity needs a non-empty watch", session.name);
    }
    if activity.working_after.is_none() && activity.idle_after.is_none() {
        bail!(
            "session {}: activity needs at least one duration",
            session.name
        );
    }
    if let Some(value) = &activity.working_after {
        parse_interval(value, "activity working_after")
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    if let Some(value) = &activity.idle_after {
        parse_interval(value, "activity idle_after")
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    Ok(())
}

fn activity_cycle(sessions: &[TeamSession]) -> Option<Vec<&str>> {
    let names = sessions
        .iter()
        .map(|session| session.name.as_str())
        .collect::<HashSet<_>>();
    let mut edges = HashMap::<&str, Vec<&str>>::new();
    for session in sessions.iter().filter(|session| session.activity.is_some()) {
        for watched in &session.watch {
            if names.contains(watched.as_str()) {
                edges
                    .entry(watched.as_str())
                    .or_default()
                    .push(session.name.as_str());
            }
        }
    }
    fn visit<'a>(
        node: &'a str,
        edges: &HashMap<&'a str, Vec<&'a str>>,
        state: &mut HashMap<&'a str, u8>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<&'a str>> {
        match state.get(node).copied() {
            Some(1) => {
                let start = stack.iter().position(|entry| *entry == node)?;
                Some(stack[start..].to_vec())
            }
            Some(2) => None,
            _ => {
                state.insert(node, 1);
                stack.push(node);
                for next in edges.get(node).into_iter().flatten() {
                    if let Some(cycle) = visit(next, edges, state, stack) {
                        return Some(cycle);
                    }
                }
                stack.pop();
                state.insert(node, 2);
                None
            }
        }
    }
    let mut state = HashMap::new();
    let mut stack = Vec::new();
    let mut cycle = None;
    for session in sessions {
        if let Some(found) = visit(session.name.as_str(), &edges, &mut state, &mut stack) {
            cycle = Some(found);
            break;
        }
    }
    let mut cycle = cycle?;
    cycle.sort_by_key(|name| {
        sessions
            .iter()
            .position(|session| session.name == *name)
            .unwrap_or(usize::MAX)
    });
    Some(cycle)
}

pub(super) fn validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()> {
    if sessions.is_empty() {
        bail!("team must contain at least one session");
    }
    let mut names = HashSet::with_capacity(sessions.len());
    let mut attached = false;
    for (index, session) in sessions.iter().enumerate() {
        validate_session_name(index, session)?;
        if !names.insert(session.name.as_str()) {
            bail!("session {}: duplicate name", session.name);
        }
        validate_session_command(session)?;
        if session.attach && attached {
            bail!("session {}: at most one session can attach", session.name);
        }
        attached |= session.attach;
        validate_session_reset(session)?;
        validate_session_links(session)?;
        validate_session_heartbeat(session)?;
        validate_activity(session)?;
        validate_session_confine_options(session)?;
        validate_session_remote_options(session)?;
        validate_session_role(session)?;
    }
    if let Some(cycle) = activity_cycle(sessions) {
        bail!(
            "activity notices would loop between {}",
            join_with_and(&cycle)
        );
    }
    Ok(())
}

fn validate_session_confine_options(session: &TeamSession) -> anyhow::Result<()> {
    if session.confine.is_some() {
        let harness = Harness::infer(&session.command);
        if harness == Harness::Claude
            && session
                .command
                .iter()
                .take_while(|argument| argument.as_str() != "--")
                .any(|argument| {
                    argument == "--append-system-prompt-file"
                        || argument.starts_with("--append-system-prompt-file=")
                })
        {
            bail!(
                "session {}: confinement cannot be combined with --append-system-prompt-file",
                session.name
            );
        }
    }
    Ok(())
}

fn validate_session_remote_options(session: &TeamSession) -> anyhow::Result<()> {
    if session.remote.is_some() && session.confine.is_none() {
        bail!(
            "session {}: remote messaging needs a confined session",
            session.name
        );
    }
    Ok(())
}

/// Validate confinement options that depend on the session's resolved working directory.
pub fn validate_session_confine(session: &TeamSession, cwd: &Path) -> anyhow::Result<()> {
    validate_session_confine_options(session)?;
    if session.confine.is_some() {
        let harness = Harness::infer(&session.command);
        crate::harness::validate_prompt_options(&session.command, harness, cwd)
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    Ok(())
}

fn validate_session_name(index: usize, session: &TeamSession) -> anyhow::Result<()> {
    validate_name(&session.name)
        .map_err(|error| anyhow::anyhow!("session {} ({}): {error}", index + 1, session.name))
}

fn validate_session_command(session: &TeamSession) -> anyhow::Result<()> {
    if session.command.is_empty() {
        bail!("session {}: command must not be empty", session.name);
    }
    if session.command[0].chars().any(char::is_whitespace) {
        bail!(
            "session {}: command[0] {:?} contains whitespace; give each argument as its own array element",
            session.name,
            session.command[0]
        );
    }
    Ok(())
}

fn validate_session_reset(session: &TeamSession) -> anyhow::Result<()> {
    if let Some(reset) = &session.reset {
        if reset.is_empty() {
            bail!(
                "session {}: reset must contain at least one step",
                session.name
            );
        }
        validate_reset_steps(reset)
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    Ok(())
}

fn validate_session_links(session: &TeamSession) -> anyhow::Result<()> {
    for name in &session.control_from {
        validate_name(name).map_err(|error| {
            anyhow::anyhow!(
                "session {} control_from entry {name:?}: {error}",
                session.name
            )
        })?;
    }
    for name in &session.watch {
        validate_name(name).map_err(|error| {
            anyhow::anyhow!("session {} watch entry {name:?}: {error}", session.name)
        })?;
        if name == &session.name {
            bail!("session {} watches itself", session.name);
        }
    }
    Ok(())
}

fn validate_session_heartbeat(session: &TeamSession) -> anyhow::Result<()> {
    if let Some(value) = &session.heartbeat {
        if session.watch.is_empty() {
            bail!(
                "session {}: heartbeat needs a non-empty watch",
                session.name
            );
        }
        parse_interval(value, "heartbeat")
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    Ok(())
}

fn validate_session_role(session: &TeamSession) -> anyhow::Result<()> {
    if let Some(role) = &session.role {
        validate_role(role)
            .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
    }
    Ok(())
}

fn join_with_and(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
