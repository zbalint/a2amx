//! Pure team configuration and session planning.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::harness::Harness;
use crate::messaging::{
    TeamScope, parse_interval, validate_name, validate_reset_steps, validate_role,
};
use crate::wire::SessionSummary;

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
    pub allow: Vec<String>,
    #[serde(default)]
    pub confine: bool,
    #[serde(default)]
    pub confine_write: Vec<String>,
    #[serde(default)]
    pub confine_hide: Vec<String>,
    #[serde(default)]
    pub confine_unhide: Vec<String>,
    #[serde(default)]
    pub confine_no_default_hides: bool,
    #[serde(default)]
    pub confine_allow_containers: bool,
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
    confine: Option<bool>,
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
    let reason = home_reason(cwd, user_home)
        .or_else(|| system_reason(cwd, state_home))
        .or_else(|| container_reason(cwd));
    if let Some(reason) = reason {
        bail!(
            "refusing the global team file in {}: {reason}; create an a2amx.toml here, or pass --file, to start a team in this directory",
            cwd.display()
        );
    }
    Ok(())
}

fn home_reason(cwd: &std::path::Path, user_home: Option<&std::path::Path>) -> Option<&'static str> {
    if cwd.parent().is_none() {
        return Some("the filesystem root");
    }
    let home = user_home.filter(|home| home.is_absolute())?;
    if cwd == home || home.starts_with(cwd) {
        return Some("your home directory");
    }
    let below = cwd.strip_prefix(home).ok()?;
    let first = below
        .components()
        .next()
        .map(std::path::Component::as_os_str);
    if first.is_some_and(|component| component.to_string_lossy().starts_with('.')) {
        return Some("a hidden directory under your home directory");
    }
    if [
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
        return Some("a user-data folder");
    }
    None
}

fn system_reason(cwd: &std::path::Path, state_home: &std::path::Path) -> Option<&'static str> {
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
    if components.len() >= 3 && is_mnt_drive(&components) {
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
    None
}

fn container_reason(cwd: &std::path::Path) -> Option<&'static str> {
    fn is_direct_child(path: &std::path::Path, parent: &std::path::Path) -> bool {
        path.parent() == Some(parent)
    }
    let components: Vec<_> = cwd.components().collect();
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
    if components.len() == 3 && is_mnt_drive(&components) {
        return Some("a top-level container directory");
    }
    if components.len() >= 4
        && is_mnt_drive(&components)
        && components[3]
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case("Users")
        && (components.len() == 4 || components.len() == 5)
    {
        return Some("a Windows home directory");
    }
    None
}

fn is_mnt_drive(components: &[std::path::Component<'_>]) -> bool {
    components[1]
        .as_os_str()
        .to_string_lossy()
        .eq_ignore_ascii_case("mnt")
        && components[2].as_os_str().to_string_lossy().len() == 1
        && components[2].as_os_str().to_string_lossy().as_bytes()[0].is_ascii_alphabetic()
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
        confine,
        session,
    } = file;
    if team.is_some() && default_team.is_some() {
        bail!("the global team file must not set team; the team is named after the directory");
    }
    let team = team.or_else(|| default_team.map(str::to_owned));
    if prefix.is_some() {
        bail!("prefix was renamed to team; see the README");
    }
    let scope = build_scope(team, private, allow)?;
    let confine_default = confine.unwrap_or(false);
    let mut sessions: Vec<TeamSession> = session
        .into_iter()
        .enumerate()
        .map(|(index, mut table)| {
            if !table.contains_key("confine") {
                table.insert("confine".into(), toml::Value::Boolean(confine_default));
            }
            session_from_table(index, table)
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

fn build_scope(
    team: Option<String>,
    private: Option<bool>,
    allow: Option<Vec<String>>,
) -> anyhow::Result<Option<TeamScope>> {
    let allow_given = allow.is_some();
    let allow = allow.unwrap_or_default();
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
        if private.is_some() {
            bail!("private requires team");
        }
        if allow_given {
            bail!("allow requires team");
        }
        None
    };
    Ok(scope)
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
            activity: None,
            heartbeat: None,
            role: None,
            allow: Vec::new(),
            confine: false,
            confine_write: Vec::new(),
            confine_hide: Vec::new(),
            confine_unhide: Vec::new(),
            confine_no_default_hides: false,
            confine_allow_containers: false,
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

fn validate_sessions(sessions: &[TeamSession]) -> anyhow::Result<()> {
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
    let lists = [
        ("confine_write", &session.confine_write),
        ("confine_hide", &session.confine_hide),
        ("confine_unhide", &session.confine_unhide),
    ];
    let has_options = session.confine_no_default_hides
        || session.confine_allow_containers
        || lists.iter().any(|(_, values)| !values.is_empty());
    if has_options && !session.confine {
        bail!(
            "session {}: confinement options require confine = true",
            session.name
        );
    }
    for (label, values) in lists {
        if values.len() > 64 {
            bail!("session {}: {label} has at most 64 entries", session.name);
        }
        if let Some(value) = values.iter().find(|value| value.len() > 4096) {
            bail!(
                "session {}: {label} entry exceeds 4096 bytes: {value:?}",
                session.name
            );
        }
    }
    if session.confine {
        let harness = Harness::infer(&session.command);
        if harness == Harness::Codex {
            bail!(
                "session {}: confinement is not supported for Codex sessions yet: its tools run outside the session process",
                session.name
            );
        }
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

/// Validate confinement options that depend on the session's resolved working directory.
pub fn validate_session_confine(session: &TeamSession, cwd: &Path) -> anyhow::Result<()> {
    validate_session_confine_options(session)?;
    if session.confine {
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
