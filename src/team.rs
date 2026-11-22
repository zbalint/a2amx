//! Pure team configuration and session planning.

use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::grant::{self, Grant};
use crate::messaging::{RemoteMode, TeamScope, validate_name};
use crate::wire::SessionSummary;

mod directory;
mod validate;

pub use directory::{check_global_allowed, team_name_for};
pub use validate::validate_session_confine;
use validate::validate_sessions;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamActivity {
    #[serde(default)]
    pub working_after: Option<String>,
    #[serde(default)]
    pub idle_after: Option<String>,
}

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
    pub activity: Option<TeamActivity>,
    #[serde(default)]
    pub heartbeat: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub remote: Option<RemoteMode>,
    #[serde(default)]
    pub allow_agents: Vec<String>,
    #[serde(skip)]
    pub confine: Option<crate::confine::ConfineRequest>,
    #[serde(skip)]
    pub auto: bool,
    #[serde(skip)]
    pub team: Option<TeamScope>,
}

impl TeamSession {
    pub fn auto_confine(&self) -> Option<&crate::confine::ConfineRequest> {
        self.auto.then_some(self.confine.as_ref()).flatten()
    }

    pub fn auto_confine_mut(&mut self) -> Option<&mut crate::confine::ConfineRequest> {
        self.auto.then_some(self.confine.as_mut()).flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfineTable {
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    write: Vec<String>,
    #[serde(default)]
    hide: Vec<String>,
    #[serde(default)]
    unhide: Vec<String>,
    #[serde(default)]
    auto: Option<bool>,
    #[serde(default)]
    grants: Vec<String>,
    #[serde(default)]
    keep_env: Vec<String>,
    #[serde(default)]
    default_hides: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamFile {
    #[serde(default)]
    team: Option<String>,
    #[serde(default)]
    private: Option<bool>,
    #[serde(default)]
    allow_teams: Option<Vec<String>>,
    #[serde(default)]
    confine: Option<ConfineTable>,
    #[serde(default)]
    session: Vec<toml::Table>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Start(Box<TeamSession>),
    AlreadyRunning { name: String, id: String },
}

/// Collect newly detected auto grants in action order, keeping their first reason.
pub fn new_auto_grants(actions: &[Action], accepted: &BTreeSet<String>) -> Vec<(String, String)> {
    let mut new_grants = Vec::new();
    let mut seen = BTreeSet::new();
    for action in actions {
        let Action::Start(session) = action else {
            continue;
        };
        let Some(confine) = session.auto_confine() else {
            continue;
        };
        for reason in &confine.reasons {
            if !accepted.contains(&reason.grant) && seen.insert(reason.grant.clone()) {
                new_grants.push((reason.grant.clone(), reason.reason.clone()));
            }
        }
    }
    new_grants
}

/// Render one newline-terminated escape-notice entry for each started session.
/// One block per session is intentional to keep one stderr write per session (same bytes as before).
pub fn escape_notices(actions: &[Action], rows: &[Grant]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|action| {
            let Action::Start(session) = action else {
                return None;
            };
            let confine = session.confine.as_ref()?;
            let notices = grant::escape_notice(&session.name, &confine.explicit_grants(), rows);
            (!notices.is_empty()).then(|| {
                notices
                    .into_iter()
                    .map(|notice| format!("{notice}\n"))
                    .collect::<String>()
            })
        })
        .collect()
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

pub fn parse(text: &str, dir: &std::path::Path, global: bool) -> anyhow::Result<Vec<TeamSession>> {
    reject_renamed_allow(text)?;
    let file: TeamFile = toml::from_str(text).context("invalid team file")?;
    let TeamFile {
        team,
        private,
        allow_teams,
        confine,
        session,
    } = file;
    if team.is_some() && global {
        bail!("the global team file must not set team; the team is named after the directory");
    }
    let remedy = if global {
        "create an a2amx.toml here, or pass --file"
    } else {
        "set team in the file"
    };
    let team = match team {
        Some(team) => team,
        None => team_name_for(dir, remedy)?,
    };
    let scope = build_scope(team, private, allow_teams)?;
    let mut sessions = Vec::with_capacity(session.len());
    for (index, mut table) in session.into_iter().enumerate() {
        let own_confine = table
            .remove("confine")
            .map(|value| {
                value
                    .try_into()
                    .with_context(|| format!("session {}", index + 1))
            })
            .transpose()?;
        let mut session = session_from_table(index, table)?;
        session.confine = merge_confine(&session.name, confine.as_ref(), own_confine.as_ref())?;
        session.auto = confine
            .as_ref()
            .is_some_and(|table| table.auto == Some(true));
        sessions.push(session);
    }
    let base_names: HashSet<String> = sessions
        .iter()
        .map(|session| session.name.clone())
        .collect();
    for session in &mut sessions {
        let original_name = session.name.clone();
        session.name = format!("{}-{original_name}", scope.name);
        let mut session_scope = scope.clone();
        session_scope.agents = resolve_agent_allow(
            &session.allow_agents,
            &scope.name,
            &session.name,
            scope.private,
        )?;
        session.team = Some(session_scope);
        let session_scope = session.team.as_ref();
        resolve_references(
            &mut session.watch,
            &base_names,
            session_scope,
            &session.name,
            "watch",
        )?;
        resolve_references(
            &mut session.control_from,
            &base_names,
            session_scope,
            &session.name,
            "control_from",
        )?;
    }
    validate_sessions(&sessions)?;
    Ok(sessions)
}

fn reject_renamed_allow(text: &str) -> anyhow::Result<()> {
    let Ok(root) = toml::from_str::<toml::Table>(text) else {
        return Ok(());
    };
    if root.contains_key("allow") {
        bail!("allow was renamed to allow_teams");
    }
    let Some(toml::Value::Array(sessions)) = root.get("session") else {
        return Ok(());
    };
    for (index, value) in sessions.iter().enumerate() {
        if value
            .as_table()
            .is_some_and(|table| table.contains_key("allow"))
        {
            bail!("session {}: allow was renamed to allow_agents", index + 1);
        }
    }
    Ok(())
}

pub(crate) fn deduplicated_values(team: &[String], own: &[String]) -> Vec<String> {
    let mut values = Vec::with_capacity(team.len() + own.len());
    for value in team.iter().chain(own) {
        if !values.contains(value) {
            values.push(value.clone());
        }
    }
    values
}

fn validate_confine_list(session_name: &str, label: &str, values: &[String]) -> anyhow::Result<()> {
    if values.len() > crate::confine::MAX_LIST_ENTRIES {
        bail!(
            "session {session_name}: {label} has at most {} entries",
            crate::confine::MAX_LIST_ENTRIES
        );
    }
    if let Some(value) = values
        .iter()
        .find(|value| value.len() > crate::confine::MAX_PATH_BYTES)
    {
        bail!(
            "session {session_name}: {label} entry exceeds {} bytes: {value:?}",
            crate::confine::MAX_PATH_BYTES
        );
    }
    Ok(())
}
fn validate_confine_syntax(prefix: &str, table: &ConfineTable) -> anyhow::Result<()> {
    for (label, values) in [
        ("write", &table.write),
        ("hide", &table.hide),
        ("unhide", &table.unhide),
    ] {
        for value in values {
            crate::confine::check_entry_syntax(label, value)
                .map_err(|error| anyhow::anyhow!("{prefix}{error}"))?;
        }
    }
    Ok(())
}

fn merge_confine(
    session_name: &str,
    team: Option<&ConfineTable>,
    own: Option<&ConfineTable>,
) -> anyhow::Result<Option<crate::confine::ConfineRequest>> {
    if let Some(table) = team {
        validate_confine_syntax("[confine] ", table)?;
        if table.auto == Some(true) && table.enabled == Some(false) {
            bail!("[confine] auto needs confinement; remove enabled = false");
        }
    }
    if own.is_some_and(|table| table.auto.is_some()) {
        bail!(
            "session {session_name}: auto belongs in the team's [confine] table, not [session.confine]"
        );
    }
    if team.is_some_and(|table| !table.keep_env.is_empty()) {
        bail!(
            "[confine] keep_env: a credential variable is returned per session, not team-wide; put keep_env in [session.confine]"
        );
    }

    if let Some(table) = own {
        validate_confine_syntax(&format!("session {session_name}: "), table)?;
    }
    if let Some(team_grants) = team.map(|table| &table.grants) {
        for name in team_grants {
            let grant = crate::grant::lookup(name)?;
            if !grant.team_default_ok {
                bail!(
                    "[confine] grants: {name:?} exposes {}; grant it in the session that needs it, not team-wide",
                    grant.exposes
                );
            }
        }
    }
    let enabled = own
        .map(|table| table.enabled.unwrap_or(true))
        .or_else(|| team.map(|table| table.enabled.unwrap_or(true)))
        .unwrap_or(false);
    let write = deduplicated_values(
        team.map_or(&[][..], |table| &table.write),
        own.map_or(&[][..], |table| &table.write),
    );
    let hide = deduplicated_values(
        team.map_or(&[][..], |table| &table.hide),
        own.map_or(&[][..], |table| &table.hide),
    );
    let unhide = deduplicated_values(
        team.map_or(&[][..], |table| &table.unhide),
        own.map_or(&[][..], |table| &table.unhide),
    );
    let grants = deduplicated_values(
        team.map_or(&[][..], |table| &table.grants),
        own.map_or(&[][..], |table| &table.grants),
    );
    let keep_env = deduplicated_values(&[], own.map_or(&[][..], |table| &table.keep_env));
    let own_has_options = own.is_some_and(|table| {
        !table.write.is_empty()
            || !table.hide.is_empty()
            || !table.unhide.is_empty()
            || !table.grants.is_empty()
            || !table.keep_env.is_empty()
            || table.default_hides.is_some()
    });
    if own.is_some_and(|table| table.enabled == Some(false)) && own_has_options {
        bail!(
            "session {session_name}: [confine] options are set but enabled = false; remove the options or the enabled = false"
        );
    }
    if !enabled {
        return Ok(None);
    }
    validate_confine_list(session_name, "write", &write)?;
    validate_confine_list(session_name, "hide", &hide)?;
    validate_confine_list(session_name, "unhide", &unhide)?;
    validate_confine_list(session_name, "grants", &grants)?;
    validate_confine_list(session_name, "keep_env", &keep_env)?;
    crate::grant::validate(&grants)?;
    let default_hides = own
        .and_then(|table| table.default_hides)
        .or_else(|| team.and_then(|table| table.default_hides))
        .unwrap_or(true);
    Ok(Some(crate::confine::ConfineRequest {
        write,
        hide,
        unhide,
        no_default_hides: !default_hides,
        grants,
        keep_env,
        auto: false,
        reasons: Vec::new(),
        team_file: None,
    }))
}

fn build_scope(
    name: String,
    private: Option<bool>,
    allow_teams: Option<Vec<String>>,
) -> anyhow::Result<TeamScope> {
    if name.is_empty() {
        bail!("team must not be empty");
    }
    validate_name(&name).map_err(|error| anyhow::anyhow!("team {name:?}: {error}"))?;
    if name.ends_with('-') {
        bail!("team {name:?} must not end in '-'; the dash is added for you");
    }
    let private = private.unwrap_or(true);
    let allow_given = allow_teams.is_some();
    let allow = allow_teams.unwrap_or_default();
    if !private && allow_given {
        bail!("allow_teams needs a private team");
    }
    let mut seen = HashSet::with_capacity(allow.len());
    for entry in &allow {
        validate_name(entry)
            .map_err(|error| anyhow::anyhow!("allow_teams entry {entry:?}: {error}"))?;
        if entry == &name {
            bail!("allow_teams entry must not be the team itself");
        }
        if !seen.insert(entry) {
            bail!("allow_teams entry {entry:?} is repeated");
        }
    }
    Ok(TeamScope {
        name,
        private,
        allow,
        agents: Vec::new(),
    })
}

fn session_from_table(index: usize, mut table: toml::value::Table) -> anyhow::Result<TeamSession> {
    // A role-only session uses its role as the configuration name.
    if !table.contains_key("name") {
        match table.get("role") {
            Some(toml::Value::String(role)) => {
                if let Err(error) = validate_name(role) {
                    bail!(
                        "session {}: name taken from role {role:?}: {error}; set name explicitly",
                        index + 1
                    );
                }
                let name = role.clone();
                table.insert("name".into(), toml::Value::String(name));
            }
            Some(_) => {}
            None => bail!("session {}: name or role is required", index + 1),
        }
    }
    toml::Value::Table(table)
        .try_into()
        .with_context(|| format!("session {}", index + 1))
}

fn resolve_agent_allow(
    entries: &[String],
    own_team: &str,
    session_name: &str,
    private: bool,
) -> anyhow::Result<Vec<String>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }
    if !private {
        bail!("session {session_name}: allow_agents needs a private team");
    }
    if entries.len() > 16 {
        bail!("session {session_name}: allow_agents has at most 16 entries");
    }
    let mut seen = HashSet::with_capacity(entries.len());
    let mut agents = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some((team, agent)) = entry.split_once('/') else {
            bail!("session {session_name}: allow_agents entry {entry:?} must be TEAM/AGENT");
        };
        if team.is_empty() || agent.is_empty() || agent.contains('/') {
            bail!("session {session_name}: allow_agents entry {entry:?} must be TEAM/AGENT");
        }
        validate_name(team).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow_agents entry {entry:?}: {error}")
        })?;
        validate_name(agent).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow_agents entry {entry:?}: {error}")
        })?;
        if team == own_team {
            bail!("session {session_name}: allow_agents entry {entry:?} must name another team");
        }
        let resolved = format!("{team}-{agent}");
        validate_name(&resolved).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow_agents entry {entry:?}: {error}")
        })?;
        if !seen.insert(resolved.clone()) {
            bail!("session {session_name}: allow_agents entry {entry:?} is repeated");
        }
        agents.push(resolved);
    }
    Ok(agents)
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
            activity: None,
            heartbeat: None,
            role: None,
            remote: None,
            allow_agents: Vec::new(),
            confine: None,
            auto: false,
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

#[cfg(test)]
mod tests;
