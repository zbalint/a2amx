//! Pure team configuration and session planning.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::messaging::{parse_interval, validate_name, validate_reset_steps};
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
    #[serde(default)]
    pub reset: Option<Vec<String>>,
    #[serde(default)]
    pub control_from: Vec<String>,
    #[serde(default)]
    pub watch: Vec<String>,
    #[serde(default)]
    pub heartbeat: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamFile {
    #[serde(default)]
    prefix: Option<String>,
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
    let TeamFile { prefix, session } = file;
    let mut sessions: Vec<TeamSession> = session
        .into_iter()
        .enumerate()
        .map(|(index, table)| {
            toml::Value::Table(table)
                .try_into()
                .with_context(|| format!("session {}", index + 1))
        })
        .collect::<anyhow::Result<_>>()?;
    if let Some(prefix) = prefix {
        if prefix.is_empty() {
            bail!("prefix must not be empty");
        }
        if prefix.ends_with('-') {
            bail!("prefix {prefix:?} must not end in '-'; the dash is added for you");
        }
        let base_names: HashSet<String> = sessions
            .iter()
            .map(|session| session.name.clone())
            .collect();
        for session in &mut sessions {
            session.name = format!("{prefix}-{}", session.name);
            for name in &mut session.watch {
                if base_names.contains(name.as_str()) {
                    *name = format!("{prefix}-{name}");
                }
            }
            for name in &mut session.control_from {
                if base_names.contains(name.as_str()) {
                    *name = format!("{prefix}-{name}");
                }
            }
        }
    }
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
            reset: None,
            control_from: Vec::new(),
            watch: Vec::new(),
            heartbeat: None,
            role: None,
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
        if session.command[0].chars().any(char::is_whitespace) {
            bail!(
                "session {}: command[0] {:?} contains whitespace; give each argument as its own array element",
                session.name,
                session.command[0]
            );
        }
        if session.attach && attached {
            bail!("session {}: at most one session can attach", session.name);
        }
        attached |= session.attach;
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
        if let Some(value) = &session.heartbeat {
            if session.watch.is_empty() {
                bail!(
                    "session {}: heartbeat needs a non-empty watch",
                    session.name
                );
            }
            parse_interval(value)
                .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
        }
        if let Some(role) = &session.role {
            if role.is_empty() {
                bail!("session {}: role must not be empty", session.name);
            }
            if role.chars().count() > 64 {
                bail!(
                    "session {}: role must be at most 64 characters",
                    session.name
                );
            }
            if role.chars().any(char::is_control) {
                bail!(
                    "session {}: role must not contain control characters",
                    session.name
                );
            }
            if role.trim() != role {
                bail!(
                    "session {}: role must not have leading or trailing whitespace",
                    session.name
                );
            }
        }
    }
    Ok(())
}
