//! Pure team configuration and session planning.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::messaging::{
    TeamScope, parse_interval, validate_name, validate_reset_steps, validate_role,
};
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
    #[serde(skip)]
    pub team: Option<TeamScope>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamFile {
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    private: Option<bool>,
    #[serde(default)]
    allow: Option<Vec<String>>,
    #[serde(default)]
    prefix: Option<toml::Value>,
    #[serde(default)]
    session: Vec<toml::Table>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(Box<TeamSession>),
    AlreadyRunning { name: String, id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictReason {
    Exited,
    TeamMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub name: String,
    pub id: String,
    pub reason: ConflictReason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub state: EntryState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryState {
    Missing,
    Running { id: String },
    Conflict { id: String, reason: ConflictReason },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResetTarget {
    pub name: String,
    pub state: ResetState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResetState {
    Reset { id: String },
    Missing,
    Exited { id: String },
    SkippedAttached { id: String },
}

/// Classify selected session names for an ordered team reset.
pub fn reset_targets(
    names: &[String],
    explicit: bool,
    except: &[String],
    include_attached: bool,
    existing: &[SessionSummary],
) -> Vec<ResetTarget> {
    let except: HashSet<_> = except.iter().map(String::as_str).collect();
    names
        .iter()
        .filter(|name| !except.contains(name.as_str()))
        .map(|name| {
            let state = match existing
                .iter()
                .find(|session| session.name.as_deref() == Some(name.as_str()))
            {
                None => ResetState::Missing,
                Some(session) if session.exit_code.is_some() => ResetState::Exited {
                    id: session.id.clone(),
                },
                Some(session) if session.attached && !explicit && !include_attached => {
                    ResetState::SkippedAttached {
                        id: session.id.clone(),
                    }
                }
                Some(session) => ResetState::Reset {
                    id: session.id.clone(),
                },
            };
            ResetTarget {
                name: name.clone(),
                state,
            }
        })
        .collect()
}

/// Inspect wanted sessions in file order; this is the single place where
/// exited-session and team-scope conflicts are classified.
pub fn inspect(wanted: &[TeamSession], existing: &[SessionSummary]) -> Vec<Entry> {
    let by_name: HashMap<_, _> = existing
        .iter()
        .filter_map(|session| session.name.as_deref().map(|name| (name, session)))
        .collect();
    wanted
        .iter()
        .map(|session| {
            let Some(existing) = by_name.get(session.name.as_str()) else {
                return Entry {
                    name: session.name.clone(),
                    state: EntryState::Missing,
                };
            };
            let state = if existing.exit_code.is_some() {
                EntryState::Conflict {
                    id: existing.id.clone(),
                    reason: ConflictReason::Exited,
                }
            } else if existing.team.as_ref() != session.team.as_ref() {
                EntryState::Conflict {
                    id: existing.id.clone(),
                    reason: ConflictReason::TeamMismatch,
                }
            } else {
                EntryState::Running {
                    id: existing.id.clone(),
                }
            };
            Entry {
                name: session.name.clone(),
                state,
            }
        })
        .collect()
}

pub fn parse(text: &str) -> anyhow::Result<Vec<TeamSession>> {
    let file: TeamFile = toml::from_str(text).context("invalid team file")?;
    let TeamFile {
        team,
        private,
        allow,
        prefix,
        session,
    } = file;
    let allow_given = allow.is_some();
    let allow = allow.unwrap_or_default();
    if prefix.is_some() {
        bail!("prefix was renamed to team; see the README");
    }
    if team.is_none() && private.is_some() {
        bail!("private requires team");
    }
    if team.is_none() && allow_given {
        bail!("allow requires team");
    }
    let scope = if let Some(name) = team {
        if name.is_empty() {
            bail!("team must not be empty");
        }
        validate_name(&name).map_err(|error| anyhow::anyhow!("team {name:?}: {error}"))?;
        if name.ends_with('-') {
            bail!("team {name:?} must not end in '-'; the dash is added for you");
        }
        let private = private.unwrap_or(true);
        if !private && allow_given {
            bail!("allow needs a private team");
        }
        let mut seen = HashSet::with_capacity(allow.len());
        for entry in &allow {
            validate_name(entry)
                .map_err(|error| anyhow::anyhow!("allow entry {entry:?}: {error}"))?;
            if entry == &name {
                bail!("allow entry must not be the team itself");
            }
            if !seen.insert(entry) {
                bail!("allow entry {entry:?} is repeated");
            }
        }
        Some(TeamScope {
            name,
            private,
            allow,
        })
    } else {
        None
    };
    let mut sessions: Vec<TeamSession> = session
        .into_iter()
        .enumerate()
        .map(|(index, table)| {
            toml::Value::Table(table)
                .try_into()
                .with_context(|| format!("session {}", index + 1))
        })
        .collect::<anyhow::Result<_>>()?;
    let base_names: HashSet<String> = sessions
        .iter()
        .map(|session| session.name.clone())
        .collect();
    for session in &mut sessions {
        session.team = scope.clone();
    }
    for session in &mut sessions {
        let original_name = session.name.clone();
        if let Some(scope) = &scope {
            session.name = format!("{}-{original_name}", scope.name);
        }
        resolve_references(
            &mut session.watch,
            &base_names,
            scope.as_ref(),
            &session.name,
            "watch",
        )?;
        resolve_references(
            &mut session.control_from,
            &base_names,
            scope.as_ref(),
            &session.name,
            "control_from",
        )?;
    }
    validate_sessions(&sessions)?;
    Ok(sessions)
}

fn resolve_references(
    references: &mut [String],
    base_names: &HashSet<String>,
    scope: Option<&TeamScope>,
    session_name: &str,
    kind: &str,
) -> anyhow::Result<()> {
    for reference in references {
        let explicit = reference.starts_with('/') || reference.contains('/');
        let resolved = if let Some(name) = reference.strip_prefix('/') {
            if name.is_empty() || name.contains('/') {
                bail!("invalid {kind} reference {reference:?} in {session_name}");
            }
            validate_name(name)?;
            name.to_owned()
        } else if let Some((team, name)) = reference.split_once('/') {
            if team.is_empty() || name.is_empty() || name.contains('/') {
                bail!("invalid {kind} reference {reference:?} in {session_name}");
            }
            validate_name(team)?;
            validate_name(name)?;
            format!("{team}-{name}")
        } else if let Some(scope) = scope {
            if !base_names.contains(reference) {
                bail!("unknown session {reference} in {kind} of {session_name}");
            }
            format!("{}-{reference}", scope.name)
        } else {
            reference.clone()
        };
        if explicit {
            validate_name(&resolved).map_err(|error| {
                anyhow::anyhow!("session {session_name} {kind} entry {reference:?}: {error}")
            })?;
        }
        *reference = resolved;
    }
    Ok(())
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
            team: None,
        });
    }
    validate_sessions(&sessions)?;
    Ok(sessions)
}

pub fn plan(
    wanted: &[TeamSession],
    existing: &[SessionSummary],
) -> Result<Vec<Action>, Vec<Conflict>> {
    let entries = inspect(wanted, existing);
    let conflicts: Vec<_> = entries
        .iter()
        .filter_map(|entry| match &entry.state {
            EntryState::Conflict { id, reason } => Some(Conflict {
                name: entry.name.clone(),
                id: id.clone(),
                reason: reason.clone(),
            }),
            EntryState::Missing | EntryState::Running { .. } => None,
        })
        .collect();
    if !conflicts.is_empty() {
        return Err(conflicts);
    }
    Ok(wanted
        .iter()
        .zip(entries)
        .map(|(session, entry)| match entry.state {
            EntryState::Missing => Action::Start(Box::new(session.clone())),
            EntryState::Running { id } => Action::AlreadyRunning {
                name: session.name.clone(),
                id,
            },
            EntryState::Conflict { .. } => unreachable!("conflicts returned above"),
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
            validate_role(role)
                .map_err(|error| anyhow::anyhow!("session {}: {error}", session.name))?;
        }
    }
    Ok(())
}
