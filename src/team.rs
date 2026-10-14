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
    #[serde(default)]
    pub allow: Vec<String>,
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

/// Derive the team name used by a global team file from its invoking directory.
pub fn team_name_for(cwd: &std::path::Path) -> anyhow::Result<String> {
    let component = cwd
        .file_name()
        .ok_or_else(|| {
            anyhow::anyhow!("cannot derive a team name from directory {}", cwd.display())
        })?
        .to_string_lossy();
    let mut name = String::new();
    let mut separator = false;
    for byte in component.bytes() {
        if byte.is_ascii_alphanumeric() {
            if separator && !name.is_empty() {
                name.push('-');
            }
            name.push(byte.to_ascii_lowercase() as char);
            separator = false;
        } else {
            separator = true;
        }
    }
    if name.len() > 40 {
        name.truncate(40);
        while name.ends_with('-') {
            name.pop();
        }
    }
    validate_name(&name).map_err(|error| {
        anyhow::anyhow!(
            "cannot derive a valid team name {name:?} from directory {}: {error}; create an a2amx.toml here, or pass --file",
            cwd.display()
        )
    })?;
    Ok(name)
}

/// Reject global team files in directories that are not project-like.
pub fn check_global_allowed(
    cwd: &std::path::Path,
    user_home: Option<&std::path::Path>,
    state_home: &std::path::Path,
) -> anyhow::Result<()> {
    fn is_direct_child(path: &std::path::Path, parent: &std::path::Path) -> bool {
        path.parent() == Some(parent)
    }
    let reason = if cwd.parent().is_none() {
        Some("the filesystem root")
    } else if let Some(home) = user_home.filter(|home| home.is_absolute()) {
        if cwd == home || home.starts_with(cwd) {
            Some("your home directory")
        } else if let Ok(below) = cwd.strip_prefix(home) {
            let first = below
                .components()
                .next()
                .map(|component| component.as_os_str());
            if first.is_some_and(|component| component.to_string_lossy().starts_with('.')) {
                Some("a hidden directory under your home directory")
            } else if [
                "Desktop",
                "Documents",
                "Downloads",
                "Music",
                "Pictures",
                "Videos",
                "Public",
                "Templates",
                "Library",
                "Applications",
            ]
            .iter()
            .any(|name| below == std::path::Path::new(name))
            {
                Some("a user-data folder")
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    let reason = reason.or_else(|| {
        let components: Vec<_> = cwd.components().collect();
        let mnt = components.get(1).is_some_and(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("mnt")
        });
        let wsl_system = mnt
            && components.get(2).is_some_and(|component| {
                let name = component.as_os_str().to_string_lossy();
                name.eq_ignore_ascii_case("wsl") || name.eq_ignore_ascii_case("wslg")
            });
        let system = [
            state_home,
            std::path::Path::new("/bin"),
            std::path::Path::new("/boot"),
            std::path::Path::new("/dev"),
            std::path::Path::new("/etc"),
            std::path::Path::new("/lib"),
            std::path::Path::new("/lib32"),
            std::path::Path::new("/lib64"),
            std::path::Path::new("/libx32"),
            std::path::Path::new("/proc"),
            std::path::Path::new("/run"),
            std::path::Path::new("/sbin"),
            std::path::Path::new("/sys"),
            std::path::Path::new("/usr"),
            std::path::Path::new("/snap"),
            std::path::Path::new("/nix"),
            std::path::Path::new("/lost+found"),
            std::path::Path::new("/System"),
            std::path::Path::new("/Library"),
            std::path::Path::new("/Applications"),
            std::path::Path::new("/private/etc"),
            std::path::Path::new("/var/lib"),
            std::path::Path::new("/var/log"),
            std::path::Path::new("/var/cache"),
            std::path::Path::new("/var/spool"),
            std::path::Path::new("/var/mail"),
            std::path::Path::new("/mnt/wsl"),
            std::path::Path::new("/mnt/wslg"),
        ];
        if wsl_system || system.iter().any(|prefix| cwd.starts_with(prefix)) {
            return Some("the daemon state directory or a system directory");
        }
        if components.len() >= 3 && mnt && components[2].as_os_str().to_string_lossy().len() == 1 {
            let drive = components[2].as_os_str().to_string_lossy();
            if drive.as_bytes()[0].is_ascii_alphabetic() {
                let windows = [
                    "Windows",
                    "Program Files",
                    "Program Files (x86)",
                    "ProgramData",
                ];
                if components.len() >= 4
                    && windows.iter().any(|name| {
                        components[3]
                            .as_os_str()
                            .to_string_lossy()
                            .eq_ignore_ascii_case(name)
                    })
                {
                    return Some("the daemon state directory or a system directory");
                }
            }
        }
        if [
            "/home",
            "/Users",
            "/root",
            "/tmp",
            "/var",
            "/opt",
            "/srv",
            "/mnt",
            "/media",
            "/Volumes",
            "/private",
            "/private/tmp",
            "/private/var",
        ]
        .iter()
        .any(|prefix| cwd == std::path::Path::new(prefix))
        {
            return Some("a top-level container directory");
        }
        if is_direct_child(cwd, std::path::Path::new("/home"))
            || is_direct_child(cwd, std::path::Path::new("/Users"))
        {
            return Some("another user's home directory");
        }
        if components.len() == 3
            && mnt
            && components[2].as_os_str().to_string_lossy().len() == 1
            && components[2].as_os_str().to_string_lossy().as_bytes()[0].is_ascii_alphabetic()
        {
            return Some("a top-level container directory");
        }
        if components.len() >= 4
            && mnt
            && components[2].as_os_str().to_string_lossy().len() == 1
            && components[2].as_os_str().to_string_lossy().as_bytes()[0].is_ascii_alphabetic()
            && components[3]
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("Users")
            && (components.len() == 4 || components.len() == 5)
        {
            return Some("a Windows home directory");
        }
        None
    });
    if let Some(reason) = reason {
        bail!(
            "refusing the global team file in {}: {reason}; create an a2amx.toml here, or pass --file, to start a team in this directory",
            cwd.display()
        );
    }
    Ok(())
}

pub fn parse(text: &str) -> anyhow::Result<Vec<TeamSession>> {
    parse_with(text, None)
}

pub fn parse_with(text: &str, default_team: Option<&str>) -> anyhow::Result<Vec<TeamSession>> {
    let file: TeamFile = toml::from_str(text).context("invalid team file")?;
    let TeamFile {
        team,
        private,
        allow,
        prefix,
        session,
    } = file;
    if team.is_some() && default_team.is_some() {
        bail!("the global team file must not set team; the team is named after the directory");
    }
    let team = team.or_else(|| default_team.map(str::to_owned));
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
            agents: Vec::new(),
        })
    } else {
        None
    };
    let mut sessions: Vec<TeamSession> = session
        .into_iter()
        .enumerate()
        .map(|(index, mut table)| {
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
        })
        .collect::<anyhow::Result<_>>()?;
    let base_names: HashSet<String> = sessions
        .iter()
        .map(|session| session.name.clone())
        .collect();
    for session in &mut sessions {
        let original_name = session.name.clone();
        if let Some(base_scope) = &scope {
            session.name = format!("{}-{original_name}", base_scope.name);
            let mut session_scope = base_scope.clone();
            session_scope.agents = resolve_agent_allow(
                &session.allow,
                &base_scope.name,
                &session.name,
                base_scope.private,
            )?;
            session.team = Some(session_scope);
        } else {
            if !session.allow.is_empty() {
                bail!("session {}: allow needs a private team", session.name);
            }
            session.team = None;
        }
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
        bail!("session {session_name}: allow needs a private team");
    }
    if entries.len() > 16 {
        bail!("session {session_name}: allow has at most 16 entries");
    }
    let mut seen = HashSet::with_capacity(entries.len());
    let mut agents = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some((team, agent)) = entry.split_once('/') else {
            bail!("session {session_name}: allow entry {entry:?} must be TEAM/AGENT");
        };
        if team.is_empty() || agent.is_empty() || agent.contains('/') {
            bail!("session {session_name}: allow entry {entry:?} must be TEAM/AGENT");
        }
        validate_name(team).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow entry {entry:?}: {error}")
        })?;
        validate_name(agent).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow entry {entry:?}: {error}")
        })?;
        if team == own_team {
            bail!("session {session_name}: allow entry {entry:?} must name another team");
        }
        let resolved = format!("{team}-{agent}");
        validate_name(&resolved).map_err(|error| {
            anyhow::anyhow!("session {session_name}: allow entry {entry:?}: {error}")
        })?;
        if !seen.insert(resolved.clone()) {
            bail!("session {session_name}: allow entry {entry:?} is repeated");
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
            heartbeat: None,
            role: None,
            allow: Vec::new(),
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
