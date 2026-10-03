//! Pure team configuration and session planning.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::messaging::validate_name;
use crate::wire::SessionSummary;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamSession {
    pub name: String,
    pub command: Vec<String>,
    /// Relative paths are resolved by the caller against the team file's directory.
    pub cwd: Option<String>,
    #[serde(default)]
    pub attach: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamFile {
    #[serde(default)]
    session: Vec<toml::Table>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(TeamSession),
    AlreadyRunning { name: String, id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub name: String,
    pub id: String,
}

pub fn parse(text: &str) -> anyhow::Result<Vec<TeamSession>> {
    let file: TeamFile = toml::from_str(text).context("invalid team file")?;
    let sessions: Vec<TeamSession> = file
        .session
        .into_iter()
        .enumerate()
        .map(|(index, table)| {
            toml::Value::Table(table)
                .try_into()
                .with_context(|| format!("session {}", index + 1))
        })
        .collect::<anyhow::Result<_>>()?;
    validate_sessions(&sessions)?;
    Ok(sessions)
}

pub fn flag_sessions(specs: &[String]) -> anyhow::Result<Vec<TeamSession>> {
    let mut sessions = Vec::with_capacity(specs.len());
    for (index, spec) in specs.iter().enumerate() {
        let (name, executable) = spec
            .split_once('=')
            .with_context(|| format!("session {}: expected NAME=EXECUTABLE", index + 1))?;
        if executable.is_empty() || executable.chars().any(char::is_whitespace) {
            bail!(
                "session {name}: use a bare executable; a command with arguments needs a team file"
            );
        }
        sessions.push(TeamSession {
            name: name.to_owned(),
            command: vec![executable.to_owned()],
            cwd: None,
            attach: index == 0,
        });
    }
    validate_sessions(&sessions)?;
    Ok(sessions)
}

pub fn plan(
    wanted: &[TeamSession],
    existing: &[SessionSummary],
) -> Result<Vec<Action>, Vec<Conflict>> {
    let by_name: HashMap<_, _> = existing
        .iter()
        .filter_map(|session| session.name.as_deref().map(|name| (name, session)))
        .collect();
    let conflicts: Vec<_> = wanted
        .iter()
        .filter_map(|session| {
            let existing = by_name.get(session.name.as_str())?;
            existing.exit_code.map(|_| Conflict {
                name: session.name.clone(),
                id: existing.id.clone(),
            })
        })
        .collect();
    if !conflicts.is_empty() {
        return Err(conflicts);
    }
    Ok(wanted
        .iter()
        .map(|session| match by_name.get(session.name.as_str()) {
            Some(existing) => Action::AlreadyRunning {
                name: session.name.clone(),
                id: existing.id.clone(),
            },
            None => Action::Start(session.clone()),
        })
        .collect())
}

fn validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()> {
    if sessions.is_empty() {
        bail!("team must contain at least one session");
    }
    let mut names = HashSet::with_capacity(sessions.len());
    let mut attached = false;
    for (index, session) in sessions.iter().enumerate() {
        validate_name(&session.name).map_err(|error| {
            anyhow::anyhow!("session {} ({}): {error}", index + 1, session.name)
        })?;
        if !names.insert(session.name.as_str()) {
            bail!("session {}: duplicate name", session.name);
        }
        if session.command.is_empty() {
            bail!("session {}: command must not be empty", session.name);
        }
        if session.attach && attached {
            bail!("session {}: at most one session can attach", session.name);
        }
        attached |= session.attach;
    }
    Ok(())
}
