use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};

use crate::layout::Layout;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    NotNeeded,
    Migrated(Report),
}

pub fn old_dir(xdg_state_home: Option<&OsStr>, user_home: Option<&Path>) -> Option<PathBuf> {
    if let Some(value) = xdg_state_home.filter(|v| !v.is_empty()) {
        let path = Path::new(value);
        if path.is_absolute() {
            return Some(path.join("a2amx"));
        }
    }
    user_home.map(|home| home.join(".local/state/a2amx"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    old: PathBuf,
    new: PathBuf,
    converted: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct Journal {
    entries: Vec<Entry>,
    warnings: Vec<String>,
}

struct HeldLock {
    _file: File,
}

pub fn migrate(layout: &Layout, old: &Path) -> Result<Outcome> {
    let journal_path = layout.state_dir().join(".migrating");
    if journal_path.exists() {
        return resume(layout, &journal_path);
    }
    if !old.is_dir() || !has_migration_files(old)? {
        return Ok(Outcome::NotNeeded);
    }
    refuse_new_data(layout, old, &journal_path)?;
    let locks = vec![
        lock_old(old, "daemon.lock")?,
        lock_old(old, "gateway.lock")?,
    ];
    let entries = entries(old, layout)?;
    if entries.is_empty() {
        return Ok(Outcome::NotNeeded);
    }
    let created = dirs_before_ensure(layout);
    if let Err(error) = layout.ensure() {
        let failures = rollback(layout, &journal_path, &entries, &created);
        return Err(with_rollback(error, failures));
    }
    let journal = Journal {
        entries: entries.clone(),
        warnings: Vec::new(),
    };
    if let Err(error) = write_journal(&journal_path, &journal) {
        let failures = rollback(layout, &journal_path, &entries, &created);
        return Err(with_rollback(error, failures));
    }
    let mut warnings = Vec::new();
    if let Err(error) = prepare(&entries, &mut warnings) {
        let failures = rollback(layout, &journal_path, &entries, &created);
        return Err(with_rollback(error, failures));
    }
    let mut journal = match read_journal(&journal_path) {
        Ok(journal) => journal,
        Err(error) => {
            let failures = rollback(layout, &journal_path, &entries, &created);
            return Err(with_rollback(error, failures));
        }
    };
    journal.warnings = warnings;
    if let Err(error) = write_journal_replacement(&journal_path, &journal) {
        let failures = rollback(layout, &journal_path, &entries, &created);
        return Err(with_rollback(error, failures));
    }
    let warnings = journal.warnings.clone();
    if let Err(error) = commit(&journal_path, &entries) {
        return Err(anyhow!(
            "{error}; the migration is nearly finished; start again to complete it"
        ));
    }
    drop(locks);
    let moved = entries.into_iter().map(|e| (e.old, e.new)).collect();
    Ok(Outcome::Migrated(Report { moved, warnings }))
}
fn write_journal_replacement(path: &Path, journal: &Journal) -> Result<()> {
    let tmp = migrating_path(path);
    let bytes = serde_json::to_vec(journal)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(tmp, path)?;
    sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
}

fn resume(layout: &Layout, journal_path: &Path) -> Result<Outcome> {
    let mut journal = read_journal(journal_path)
        .with_context(|| format!("cannot parse migration journal {}", journal_path.display()))?;
    layout.ensure()?;
    for entry in &journal.entries {
        if entry.converted {
            let tmp = migrating_path(&entry.new);
            if destination_file_exists(&entry.new)? {
                let _ = fs::remove_file(&tmp);
            } else if path_exists(&entry.new) {
                bail!(
                    "migration destination {} is not a regular file",
                    entry.new.display()
                );
            } else if entry.old.exists() {
                let _ = fs::remove_file(&tmp);
                write_converted(entry, &tmp)?;
                fs::rename(&tmp, &entry.new)?;
            } else {
                bail!(
                    "migration entry is missing both {} and {}",
                    entry.old.display(),
                    entry.new.display()
                );
            }
        } else if destination_file_exists(&entry.new)? {
            if entry.old.exists() {
                bail!(
                    "migration entry has both old and new files: {} and {}",
                    entry.old.display(),
                    entry.new.display()
                );
            }
        } else if path_exists(&entry.new) {
            bail!(
                "migration destination {} is not a regular file",
                entry.new.display()
            );
        } else if entry.old.exists() {
            fs::rename(&entry.old, &entry.new)?;
        } else {
            bail!(
                "migration entry is missing both {} and {}",
                entry.old.display(),
                entry.new.display()
            );
        }
    }
    if let Err(error) = commit(journal_path, &journal.entries) {
        return Err(anyhow!(
            "{error}; the migration is nearly finished; start again to complete it"
        ));
    }
    let moved = journal
        .entries
        .into_iter()
        .map(|e| (e.old, e.new))
        .collect();
    Ok(Outcome::Migrated(Report {
        moved,
        warnings: std::mem::take(&mut journal.warnings),
    }))
}

fn prepare(entries: &[Entry], warning_paths: &mut Vec<String>) -> Result<()> {
    for entry in entries {
        if entry.converted {
            let tmp = migrating_path(&entry.new);
            if destination_file_exists(&entry.new)? {
                let _ = fs::remove_file(&tmp);
                continue;
            }
            if path_exists(&entry.new) {
                bail!(
                    "migration destination {} is not a regular file",
                    entry.new.display()
                );
            }
            write_converted(entry, &tmp)?;
            fs::rename(&tmp, &entry.new)?;
            if let Some(warning) = conversion_warning(entry) {
                warning_paths.push(warning);
            }
        } else {
            // shortcut: cross-filesystem rename is unsupported; add copy-and-remove if layouts diverge.
            fs::rename(&entry.old, &entry.new).with_context(|| {
                format!(
                    "cannot move {} to {}",
                    entry.old.display(),
                    entry.new.display()
                )
            })?;
        }
    }
    Ok(())
}

fn commit(journal_path: &Path, entries: &[Entry]) -> Result<()> {
    for entry in entries.iter().filter(|e| e.converted) {
        if entry.old.exists() {
            fs::remove_file(&entry.old)?;
        }
    }
    fs::remove_file(journal_path)?;
    sync_dir(journal_path.parent().unwrap_or_else(|| Path::new(".")))?;
    if let Some(parent) = journal_path.parent().and_then(Path::parent) {
        sync_dir(parent)?;
    }
    Ok(())
}

fn rollback(
    layout: &Layout,
    journal_path: &Path,
    entries: &[Entry],
    created: &[PathBuf],
) -> Vec<PathBuf> {
    let mut failures = Vec::new();
    for entry in entries.iter().rev() {
        if entry.converted {
            if let Err(error) = fs::remove_file(&entry.new)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                failures.push(entry.new.clone());
            }
            let temporary = migrating_path(&entry.new);
            if let Err(error) = fs::remove_file(&temporary)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                failures.push(temporary);
            }
        } else if entry.new.exists()
            && !entry.old.exists()
            && fs::rename(&entry.new, &entry.old).is_err()
        {
            failures.push(entry.new.clone());
            failures.push(entry.old.clone());
        }
    }
    let temporary = migrating_path(journal_path);
    if let Err(error) = fs::remove_file(&temporary)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        failures.push(temporary);
    }
    if let Err(error) = fs::remove_file(journal_path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        failures.push(journal_path.to_owned());
    }
    for path in created.iter().rev() {
        if let Err(error) = fs::remove_dir(path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            failures.push(path.clone());
        }
    }
    let _ = layout;
    failures
}

fn with_rollback(error: anyhow::Error, failures: Vec<PathBuf>) -> anyhow::Error {
    if failures.is_empty() {
        return error;
    }
    let paths = failures
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    anyhow!("{error}; rollback failed for: {paths}")
}
fn write_journal(path: &Path, journal: &Journal) -> Result<()> {
    let bytes = serde_json::to_vec(journal)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    sync_dir(path.parent().unwrap_or_else(|| Path::new(".")))
}

fn read_journal(path: &Path) -> Result<Journal> {
    let mut bytes = Vec::new();
    File::open(path)?.read_to_end(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn write_converted(entry: &Entry, tmp: &Path) -> Result<()> {
    let text = fs::read_to_string(&entry.old)?;
    let (converted, _) = if entry.old.file_name().is_some_and(|n| n == "team.toml") {
        convert_team_checked(&text, &entry.old)
    } else {
        convert_gateway_checked(&text, &entry.old)
    };
    let metadata = fs::symlink_metadata(&entry.old)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(tmp)?;
    file.write_all(converted.as_bytes())?;
    file.set_permissions(fs::Permissions::from_mode(metadata.mode() & 0o7777))?;
    file.sync_all()?;
    Ok(())
}

fn conversion_warning(entry: &Entry) -> Option<String> {
    let name = entry.old.file_name()?.to_string_lossy();
    let suffix = if name == "team.toml" {
        "rename allow to allow_teams (file) and allow_agents (session) by hand"
    } else {
        "rename sync_ms, idle_ms, member_check_ms and reconnect_ms by hand"
    };
    let text = fs::read_to_string(&entry.old).ok()?;
    let converted = if name == "team.toml" {
        convert_team_checked(&text, &entry.old)
    } else {
        convert_gateway_checked(&text, &entry.old)
    };
    (!converted.1).then(|| {
        format!(
            "{}: could not convert keys automatically; {}",
            entry.new.display(),
            suffix
        )
    })
}

fn migrating_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.migrating", path.display()))
}

fn entries(old: &Path, layout: &Layout) -> Result<Vec<Entry>> {
    let mut result = Vec::new();
    let mut add = |relative: &str, new: PathBuf, converted: bool| -> Result<()> {
        let path = old.join(relative);
        if path_exists(&path) {
            reject_symlink(&path)?;
            result.push(Entry {
                old: path,
                new,
                converted,
            });
        }
        Ok(())
    };
    if path_exists(&old.join("gateway")) {
        reject_symlink(&old.join("gateway"))?;
    }
    if path_exists(&old.join("messages.db")) {
        add("messages.db", layout.messages_db(), false)?;
        for suffix in ["-wal", "-shm", "-journal"] {
            add(
                &format!("messages.db{suffix}"),
                layout.state_dir().join(format!("messages.db{suffix}")),
                false,
            )?;
        }
    }
    add("gateway/pairing.json", layout.pairing(), false)?;
    add("gateway/seen.json", layout.seen(), false)?;
    add("team.toml", layout.team_file(), true)?;
    add("gateway/config.toml", layout.gateway_config(), true)?;
    add("gateway/discord.token", layout.discord_token(), false)?;
    add("daemon.log", layout.daemon_log(), false)?;
    add("gateway/gateway.log", layout.gateway_log(), false)?;
    Ok(result)
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn destination_file_exists(path: &Path) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(metadata.file_type().is_file()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn has_migration_files(old: &Path) -> Result<bool> {
    if path_exists(&old.join("messages.db")) {
        return Ok(true);
    }
    let gateway = old.join("gateway");
    if fs::symlink_metadata(&gateway)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Ok(true);
    }
    for path in [
        "gateway/pairing.json",
        "gateway/seen.json",
        "team.toml",
        "gateway/config.toml",
        "gateway/discord.token",
        "daemon.log",
        "gateway/gateway.log",
    ] {
        if path_exists(&old.join(path)) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn reject_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        bail!("old file {} is a symlink", path.display());
    }
    Ok(())
}

fn refuse_new_data(layout: &Layout, old: &Path, journal: &Path) -> Result<()> {
    let root = layout.root();
    if !root.exists() {
        return Ok(());
    }
    for item in walk(root)? {
        if item == root || item == journal || item.starts_with(layout.root().join("bin")) {
            continue;
        }
        let metadata = fs::symlink_metadata(&item)?;
        if metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            bail!(
                "both {} and {} hold data; move or delete one of them and start again",
                old.display(),
                root.display()
            );
        }
    }
    Ok(())
}
fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut output = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(path)? {
            let path = entry?.path();
            output.push(path.clone());
            if fs::symlink_metadata(&path)?.is_dir() {
                stack.push(path);
            }
        }
    }
    Ok(output)
}

fn dirs_before_ensure(layout: &Layout) -> Vec<PathBuf> {
    [
        layout.root().to_owned(),
        layout.config_dir(),
        layout.secrets_dir(),
        layout.state_dir(),
        layout.gateway_state_dir(),
        layout.run_dir(),
        layout.link_dir(),
        layout.log_dir(),
    ]
    .into_iter()
    .filter(|p| !p.exists())
    .collect()
}

fn lock_old(old: &Path, name: &str) -> Result<Option<HeldLock>> {
    let path = old.join(name);
    if !path_exists(&path) {
        return Ok(None);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)?;
    if !file.metadata()?.is_file() {
        bail!("old lock file {} is not a regular file", path.display());
    }
    match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(HeldLock { _file: file })),
        Err(error) if error == rustix::io::Errno::AGAIN => {
            let pid = lock_pid(&file);
            if let Some(pid) = pid {
                bail!(
                    "a2amx is still running from {} (pid {pid}); stop it with: kill -TERM {pid}, then start again",
                    old.display()
                );
            }
            bail!(
                "a2amx is still running from {}; stop the process that holds {}, then start again",
                old.display(),
                path.display()
            );
        }
        Err(error) => Err(error.into()),
    }
}

fn lock_pid(file: &File) -> Option<u32> {
    let metadata = file.metadata().ok()?;
    let target = format!(
        "{:02x}:{:02x}:{}",
        metadata.dev() >> 8,
        metadata.dev() & 255,
        metadata.ino()
    );
    let text = fs::read_to_string("/proc/locks").ok()?;
    text.lines().find_map(|line| {
        let fields: Vec<_> = line.split_whitespace().collect();
        (fields.len() > 5 && fields[3] == "WRITE" && fields[5] == target)
            .then(|| fields[4].parse().ok())
            .flatten()
    })
}

fn sync_dir(path: &Path) -> Result<()> {
    File::open(path)?.sync_all().map_err(Into::into)
}

fn convert_team_checked(text: &str, path: &Path) -> (String, bool) {
    let Ok(old): Result<toml::Value, _> = toml::from_str(text) else {
        return (text.to_owned(), false);
    };
    let mut expected = old.clone();
    rename_team_values(&mut expected);
    let converted = convert_team(text);
    match toml::from_str::<toml::Value>(&converted) {
        Ok(value) if value == expected => (converted, true),
        _ => {
            let _ = path;
            (text.to_owned(), false)
        }
    }
}

fn rename_team_values(value: &mut toml::Value) {
    if let toml::Value::Table(table) = value {
        if let Some(v) = table.remove("allow") {
            table.insert("allow_teams".into(), v);
        }
        if let Some(toml::Value::Array(sessions)) = table.get_mut("session") {
            for session in sessions {
                if let toml::Value::Table(session) = session
                    && let Some(v) = session.remove("allow")
                {
                    session.insert("allow_agents".into(), v);
                }
            }
        }
    }
}

pub fn convert_team(text: &str) -> String {
    lexical_convert(text, |key, session, depth| {
        if depth == 0 && key == "allow" {
            Some(if session {
                "allow_agents"
            } else {
                "allow_teams"
            })
        } else {
            None
        }
    })
}

fn convert_gateway_checked(text: &str, path: &Path) -> (String, bool) {
    let Ok(old): Result<toml::Value, _> = toml::from_str(text) else {
        return (text.to_owned(), false);
    };
    let mut expected = old.clone();
    if let toml::Value::Table(table) = &mut expected {
        for (old_key, new_key) in [
            ("sync_ms", "sync"),
            ("idle_ms", "idle"),
            ("member_check_ms", "member_check"),
            ("reconnect_ms", "reconnect"),
        ] {
            if let Some(value) = table.remove(old_key)
                && let Some(number) = value.as_integer()
            {
                table.insert(new_key.into(), toml::Value::String(format!("{number}ms")));
            }
        }
    }
    let converted = convert_gateway(text);
    match toml::from_str::<toml::Value>(&converted) {
        Ok(value) if value == expected => (converted, true),
        _ => {
            let _ = path;
            (text.to_owned(), false)
        }
    }
}

pub fn convert_gateway(text: &str) -> String {
    text.split_inclusive('\n')
        .map(convert_gateway_line)
        .collect()
}

fn convert_gateway_line(line: &str) -> String {
    let body_end = line.find('#').unwrap_or(line.len());
    let body = &line[..body_end];
    let Some(eq) = body.find('=') else {
        return line.to_owned();
    };
    let key_start = body[..eq].len() - body[..eq].trim_start().len();
    let key = body[key_start..eq].trim();
    let new_key = match key {
        "sync_ms" => "sync",
        "idle_ms" => "idle",
        "member_check_ms" => "member_check",
        "reconnect_ms" => "reconnect",
        _ => return line.to_owned(),
    };
    let after_eq = &body[eq + 1..];
    let value_start = after_eq.len() - after_eq.trim_start().len();
    let value_end = after_eq.trim_end().len();
    let raw_value = &after_eq[value_start..value_end];
    let digits = raw_value.strip_prefix('+').unwrap_or(raw_value);
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '_') {
        return line.to_owned();
    }
    let number: String = digits.chars().filter(|c| *c != '_').collect();
    let mut output = String::with_capacity(line.len() + 8);
    output.push_str(&body[..key_start]);
    output.push_str(new_key);
    output.push_str(&body[key_start + key.len()..eq + 1]);
    output.push_str(&after_eq[..value_start]);
    output.push('"');
    output.push_str(&number);
    output.push_str("ms\"");
    output.push_str(&after_eq[value_end..]);
    output.push_str(&line[body_end..]);
    output
}

fn lexical_convert(
    text: &str,
    rename: impl Fn(&str, bool, usize) -> Option<&'static str>,
) -> String {
    let mut output = String::with_capacity(text.len());
    let mut session = false;
    let mut enabled = true;
    let mut multiline: Option<char> = None;
    let mut depth = 0usize;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if multiline.is_none() && depth == 0 && trimmed.starts_with("[[") {
            let header = trimmed.trim_start_matches('[').trim_start();
            session = header.starts_with("session") || header.starts_with("\"session\"");
            enabled = session;
        } else if multiline.is_none() && depth == 0 && trimmed.starts_with('[') {
            session = false;
            enabled = false;
        }
        let mut chars = line.char_indices().peekable();
        let mut quote = None;
        let mut replacement = None;
        while let Some((i, c)) = chars.next() {
            if let Some(delim) = multiline {
                if c == delim && chars.peek().is_some_and(|(_, next)| *next == delim) {
                    chars.next();
                    if chars.peek().is_some_and(|(_, next)| *next == delim) {
                        chars.next();
                        multiline = None;
                    }
                }
                continue;
            }
            if let Some(q) = quote {
                if q == '"' && c == '\\' {
                    chars.next();
                    continue;
                }
                if c == q {
                    quote = None;
                }
                continue;
            }
            if c == '#' {
                break;
            }
            if c == '"' || c == '\'' {
                if depth == 0
                    && let Some(end_rel) = line[i + 1..].find(c)
                {
                    let end = i + 1 + end_rel;
                    let key = &line[i + 1..end];
                    if enabled
                        && key == "allow"
                        && line[end + 1..].trim_start().starts_with('=')
                        && let Some(value) = rename(key, session, depth)
                    {
                        let replacement_value = if c == '"' {
                            if value == "allow_teams" {
                                "\"allow_teams\""
                            } else {
                                "\"allow_agents\""
                            }
                        } else if value == "allow_teams" {
                            "'allow_teams'"
                        } else {
                            "'allow_agents'"
                        };
                        replacement = Some((i, end + 1, replacement_value));
                    }
                }
                if chars.peek().is_some_and(|(_, next)| *next == c)
                    && chars.clone().nth(1).is_some_and(|(_, next)| next == c)
                {
                    chars.next();
                    chars.next();
                    multiline = Some(c);
                } else {
                    quote = Some(c);
                }
                continue;
            }
            if c == '[' {
                depth += 1;
            }
            if c == ']' {
                depth = depth.saturating_sub(1);
            }
            if depth == 0 && (c.is_ascii_alphabetic() || c == '_') {
                let start = i;
                let mut end = i + c.len_utf8();
                while chars
                    .peek()
                    .is_some_and(|(_, next)| next.is_ascii_alphanumeric() || *next == '_')
                {
                    end = chars.next().map(|(j, n)| j + n.len_utf8()).unwrap_or(end);
                }
                let key = &line[start..end];
                let rest = &line[end..];
                if enabled && rest.trim_start().starts_with('=') {
                    replacement = rename(key, session, depth).map(|value| (start, end, value));
                }
            }
        }
        if let Some((start, end, value)) = replacement {
            output.push_str(&line[..start]);
            output.push_str(value);
            output.push_str(&line[end..]);
        } else {
            output.push_str(line);
        }
    }
    output
}
