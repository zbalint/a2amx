use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use rustix::fs::OFlags;
use serde::Deserialize;

use crate::layout::Layout;
use crate::messaging::Limits;

const MAX_FILE_BYTES: usize = 64 * 1024;
// shortcut: ten years bounds Instant arithmetic while allowing any sensible operator lifetime.
const MAX_DURATION_SECS: u64 = 3650 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    Override,
}

#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub listen: Vec<SocketAddr>,
    pub host_name: Option<String>,
    pub owner_away_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct DaemonSettings {
    pub listen: Vec<SocketAddr>,
    pub host_name: Option<String>,
    pub limits: Limits,
}

#[derive(Debug, Clone)]
pub struct LoadedDaemon {
    pub settings: DaemonSettings,
    pub sources: Vec<(&'static str, Source)>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    listen: Option<Vec<String>>,
    host_name: Option<String>,
    owner_away: Option<String>,
    limits: Option<FileLimits>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileLimits {
    max_pending_per_recipient: Option<u32>,
    rate_per_minute: Option<u32>,
    human_inbound_per_minute: Option<u32>,
    human_outbound_per_minute: Option<u32>,
    max_stored_messages: Option<u32>,
    retention: Option<String>,
    lease: Option<String>,
    code_ttl: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct GatewayFile {
    guild_id: Option<String>,
    control_category: Option<String>,
    control_channel: Option<String>,
    max_members: Option<u64>,
    sync: Option<String>,
    idle: Option<String>,
    member_check: Option<String>,
    reconnect: Option<String>,
    api_base: Option<String>,
    gateway_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GatewaySettings {
    pub control_category: String,
    pub control_channel: String,
    pub max_members: u64,
    pub sync: Duration,
    pub idle: Duration,
    pub member_check: Duration,
    pub reconnect: Duration,
    pub api_base: String,
    pub gateway_url: String,
}

#[derive(Debug, Clone)]
pub struct LoadedGateway {
    pub guild_id: Option<String>,
    pub settings: GatewaySettings,
    pub sources: Vec<(&'static str, Source)>,
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub prefix: u8,
    pub viewer_after: Option<Duration>,
}

#[derive(Debug, Clone)]
pub struct ClientOverrides {
    pub prefix: Option<u8>,
    pub viewer_after_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct LoadedClient {
    pub settings: ClientSettings,
    pub sources: Vec<(&'static str, Source)>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ClientFile {
    prefix: Option<String>,
    viewer_after: Option<String>,
}

const DEFAULT_CLIENT_PREFIX: u8 = 0x02;
const DEFAULT_CLIENT_VIEWER_AFTER_SECS: u64 = 300;

#[derive(Debug, Clone, Copy)]
struct Key {
    name: &'static str,
    description: &'static str,
    default: DefaultValue,
    display: fn(&DaemonSettings) -> String,
}

#[derive(Debug, Clone, Copy)]
enum DefaultValue {
    Listen,
    HostName,
    OwnerAway,
    Counter(fn(&Limits) -> u32),
    Duration(fn(&Limits) -> u64),
}

fn display_listen(settings: &DaemonSettings) -> String {
    format!(
        "[{}]",
        settings
            .listen
            .iter()
            .map(|addr| format!("\"{addr}\""))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn display_host_name(settings: &DaemonSettings) -> String {
    settings
        .host_name
        .clone()
        .map_or_else(|| "(unset)".into(), |value| format!("\"{value}\""))
}
fn display_owner_away(settings: &DaemonSettings) -> String {
    format!(
        "\"{}\"",
        format_duration(Duration::from_secs(settings.limits.away_secs))
    )
}
fn display_counter(value: fn(&Limits) -> u32, settings: &DaemonSettings) -> String {
    value(&settings.limits).to_string()
}
fn display_duration(value: fn(&Limits) -> u64, settings: &DaemonSettings) -> String {
    format!(
        "\"{}\"",
        format_duration(Duration::from_secs(value(&settings.limits)))
    )
}
fn display_pending(settings: &DaemonSettings) -> String {
    display_counter(|limits| limits.max_pending_per_recipient, settings)
}
fn display_rate(settings: &DaemonSettings) -> String {
    display_counter(|limits| limits.rate_per_minute, settings)
}
fn display_inbound(settings: &DaemonSettings) -> String {
    display_counter(|limits| limits.human_inbound_per_minute, settings)
}
fn display_outbound(settings: &DaemonSettings) -> String {
    display_counter(|limits| limits.human_outbound_per_minute, settings)
}
fn display_stored(settings: &DaemonSettings) -> String {
    display_counter(|limits| limits.max_stored_messages, settings)
}
fn display_retention(settings: &DaemonSettings) -> String {
    display_duration(|limits| limits.retention_secs, settings)
}
fn display_lease(settings: &DaemonSettings) -> String {
    display_duration(|limits| limits.lease_secs, settings)
}
fn display_code_ttl(settings: &DaemonSettings) -> String {
    display_duration(|limits| limits.code_secs, settings)
}

const KEYS: &[Key] = &[
    Key {
        name: "listen",
        description: "addresses to bind.",
        default: DefaultValue::Listen,
        display: display_listen,
    },
    Key {
        name: "host_name",
        description: "optional name shown in addresses.",
        default: DefaultValue::HostName,
        display: display_host_name,
    },
    Key {
        name: "owner_away",
        description: "grace after interactive attachment ends.",
        default: DefaultValue::OwnerAway,
        display: display_owner_away,
    },
    Key {
        name: "limits.max_pending_per_recipient",
        description: "pending message cap.",
        default: DefaultValue::Counter(|limits| limits.max_pending_per_recipient),
        display: display_pending,
    },
    Key {
        name: "limits.rate_per_minute",
        description: "agent send rate.",
        default: DefaultValue::Counter(|limits| limits.rate_per_minute),
        display: display_rate,
    },
    Key {
        name: "limits.human_inbound_per_minute",
        description: "human inbound rate.",
        default: DefaultValue::Counter(|limits| limits.human_inbound_per_minute),
        display: display_inbound,
    },
    Key {
        name: "limits.human_outbound_per_minute",
        description: "human outbound rate.",
        default: DefaultValue::Counter(|limits| limits.human_outbound_per_minute),
        display: display_outbound,
    },
    Key {
        name: "limits.max_stored_messages",
        description: "stored message cap.",
        default: DefaultValue::Counter(|limits| limits.max_stored_messages),
        display: display_stored,
    },
    Key {
        name: "limits.retention",
        description: "message retention.",
        default: DefaultValue::Duration(|limits| limits.retention_secs),
        display: display_retention,
    },
    Key {
        name: "limits.lease",
        description: "remote lease duration.",
        default: DefaultValue::Duration(|limits| limits.lease_secs),
        display: display_lease,
    },
    Key {
        name: "limits.code_ttl",
        description: "pairing code lifetime.",
        default: DefaultValue::Duration(|limits| limits.code_secs),
        display: display_code_ttl,
    },
];

fn resolve<T: Clone>(default: T, file: Option<T>, override_value: Option<T>) -> (T, Source) {
    if let Some(value) = override_value {
        (value, Source::Override)
    } else if let Some(value) = file {
        (value, Source::File)
    } else {
        (default, Source::Default)
    }
}

pub(crate) fn file_error(path: &Path, key: &str, message: impl std::fmt::Display) -> anyhow::Error {
    anyhow!("{}: key {}: {}", path.display(), key, message)
}

fn read_config(path: &Path) -> anyhow::Result<Option<String>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(OFlags::NONBLOCK.bits() as i32)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
    };
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("configuration");
    let metadata = file
        .metadata()
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{}: {file_name} is not a regular file", path.display());
    }
    let mut bytes = Vec::with_capacity(MAX_FILE_BYTES + 1);
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.len() > MAX_FILE_BYTES {
        bail!("{}: {file_name} exceeds 65536 bytes", path.display());
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("{}: {file_name} is not UTF-8", path.display()))?;
    Ok(Some(text))
}

fn parse_file(path: &Path, text: &str) -> anyhow::Result<FileConfig> {
    toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display()))
}

fn parse_counter<T>(path: &Path, key: &'static str, value: T) -> anyhow::Result<T>
where
    T: PartialEq + From<u8>,
{
    if value == T::from(0) {
        bail!(
            "{}: key {}: 0 is not allowed; use 1 or more",
            path.display(),
            key
        );
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy)]
enum DurationRule {
    WholeSeconds,
    WholeSecondsOrZero,
    Millis,
}

impl DurationRule {
    fn bounds(self) -> (Duration, bool, &'static str) {
        match self {
            Self::WholeSeconds => (Duration::from_secs(1), true, "1s"),
            Self::WholeSecondsOrZero => (Duration::ZERO, true, "0s"),
            Self::Millis => (Duration::from_millis(1), false, "1ms"),
        }
    }
}

fn validate_duration(
    path: &Path,
    key: &'static str,
    duration: Duration,
    rule: DurationRule,
) -> anyhow::Result<Duration> {
    let (minimum, whole_seconds, minimum_text) = rule.bounds();
    let range_error = || {
        anyhow!(
            "{}: key {}: duration must be between {} and 3650d",
            path.display(),
            key,
            minimum_text
        )
    };
    if duration > Duration::from_secs(MAX_DURATION_SECS) {
        return Err(range_error());
    }
    if whole_seconds && duration.subsec_nanos() != 0 {
        bail!(
            "{}: key {}: duration must be a whole number of seconds",
            path.display(),
            key
        );
    }
    if duration < minimum {
        return Err(range_error());
    }
    Ok(duration)
}

fn parse_limited_duration(
    path: &Path,
    key: &'static str,
    text: &str,
    rule: DurationRule,
) -> anyhow::Result<Duration> {
    let duration = parse_duration(text).map_err(|error| file_error(path, key, error))?;
    validate_duration(path, key, duration, rule)
}

fn default_listen() -> Vec<SocketAddr> {
    vec![SocketAddr::from(([127, 0, 0, 1], 0))]
}

fn parse_listen(
    path: &Path,
    values: Option<Vec<String>>,
) -> anyhow::Result<Option<Vec<SocketAddr>>> {
    values
        .map(|values| {
            if values.is_empty() {
                bail!(
                    "{}: key listen: must contain at least one address",
                    path.display()
                );
            }
            values
                .into_iter()
                .map(|value| {
                    value
                        .parse::<SocketAddr>()
                        .map_err(|error| file_error(path, "listen", error))
                })
                .collect()
        })
        .transpose()
}

fn validate_host(path: &Path, value: Option<String>) -> anyhow::Result<Option<String>> {
    if let Some(value) = &value
        && value.is_empty()
    {
        bail!("{}: key host_name: must not be empty", path.display());
    }
    Ok(value)
}

#[derive(Clone, Copy)]
struct KeySources {
    listen: Source,
    host_name: Source,
    owner_away: Source,
    max_pending_per_recipient: Source,
    rate_per_minute: Source,
    human_inbound_per_minute: Source,
    human_outbound_per_minute: Source,
    max_stored_messages: Source,
    retention: Source,
    lease: Source,
    code_ttl: Source,
}

impl KeySources {
    fn as_array(self) -> [Source; 11] {
        [
            self.listen,
            self.host_name,
            self.owner_away,
            self.max_pending_per_recipient,
            self.rate_per_minute,
            self.human_inbound_per_minute,
            self.human_outbound_per_minute,
            self.max_stored_messages,
            self.retention,
            self.lease,
            self.code_ttl,
        ]
    }
}

pub fn load_daemon(layout: &Layout, overrides: &Overrides) -> anyhow::Result<LoadedDaemon> {
    let path = layout.daemon_config();
    let file = read_config(&path)?;
    let parsed = file
        .as_deref()
        .map(|text| parse_file(&path, text))
        .transpose()?
        .unwrap_or_default();
    let FileConfig {
        listen: file_listen,
        host_name: file_host_name,
        owner_away: file_owner_away,
        limits: file_limits,
    } = parsed;
    let file_limits = file_limits.unwrap_or_default();

    let file_listen = parse_listen(&path, file_listen)?;
    let file_host_name = validate_host(&path, file_host_name)?;
    let file_owner_away = file_owner_away
        .map(|value| {
            parse_limited_duration(
                &path,
                "owner_away",
                &value,
                DurationRule::WholeSecondsOrZero,
            )
        })
        .transpose()?
        .map(|value| value.as_secs());
    let override_host_name = validate_host(&path, overrides.host_name.clone())?;
    let file_max_pending = file_limits
        .max_pending_per_recipient
        .map(|value| parse_counter(&path, "limits.max_pending_per_recipient", value))
        .transpose()?;
    let file_rate = file_limits
        .rate_per_minute
        .map(|value| parse_counter(&path, "limits.rate_per_minute", value))
        .transpose()?;
    let file_human_inbound = file_limits
        .human_inbound_per_minute
        .map(|value| parse_counter(&path, "limits.human_inbound_per_minute", value))
        .transpose()?;
    let file_human_outbound = file_limits
        .human_outbound_per_minute
        .map(|value| parse_counter(&path, "limits.human_outbound_per_minute", value))
        .transpose()?;
    let file_max_stored = file_limits
        .max_stored_messages
        .map(|value| parse_counter(&path, "limits.max_stored_messages", value))
        .transpose()?;
    let file_retention = file_limits
        .retention
        .as_deref()
        .map(|value| {
            parse_limited_duration(&path, "limits.retention", value, DurationRule::WholeSeconds)
        })
        .transpose()?
        .map(|value| value.as_secs());
    let file_lease = file_limits
        .lease
        .as_deref()
        .map(|value| {
            parse_limited_duration(&path, "limits.lease", value, DurationRule::WholeSeconds)
        })
        .transpose()?
        .map(|value| value.as_secs());
    let file_code_ttl = file_limits
        .code_ttl
        .as_deref()
        .map(|value| {
            parse_limited_duration(&path, "limits.code_ttl", value, DurationRule::WholeSeconds)
        })
        .transpose()?
        .map(|value| value.as_secs());
    let defaults = Limits::default();
    let (listen, listen_source) = resolve(
        default_listen(),
        file_listen,
        (!overrides.listen.is_empty()).then(|| overrides.listen.clone()),
    );
    let (host_name, host_source) =
        resolve(None, file_host_name.map(Some), override_host_name.map(Some));
    let (away_secs, away_source) = resolve(
        defaults.away_secs,
        file_owner_away,
        overrides
            .owner_away_secs
            .map(|value| {
                validate_duration(
                    &path,
                    "owner_away",
                    Duration::from_secs(value),
                    DurationRule::WholeSecondsOrZero,
                )
                .map(|value| value.as_secs())
            })
            .transpose()?,
    );
    let (max_pending_per_recipient, pending_source) =
        resolve(defaults.max_pending_per_recipient, file_max_pending, None);
    let (rate_per_minute, rate_source) = resolve(defaults.rate_per_minute, file_rate, None);
    let (human_inbound_per_minute, inbound_source) =
        resolve(defaults.human_inbound_per_minute, file_human_inbound, None);
    let (human_outbound_per_minute, outbound_source) = resolve(
        defaults.human_outbound_per_minute,
        file_human_outbound,
        None,
    );
    let (max_stored_messages, stored_source) =
        resolve(defaults.max_stored_messages, file_max_stored, None);
    let (retention_secs, retention_source) = resolve(defaults.retention_secs, file_retention, None);
    let (lease_secs, lease_source) = resolve(defaults.lease_secs, file_lease, None);
    let (code_secs, code_source) = resolve(defaults.code_secs, file_code_ttl, None);
    let limits = Limits {
        max_pending_per_recipient,
        rate_per_minute,
        human_inbound_per_minute,
        human_outbound_per_minute,
        max_stored_messages,
        retention_secs,
        lease_secs,
        code_secs,
        away_secs,
    };
    let key_sources = KeySources {
        listen: listen_source,
        host_name: host_source,
        owner_away: away_source,
        max_pending_per_recipient: pending_source,
        rate_per_minute: rate_source,
        human_inbound_per_minute: inbound_source,
        human_outbound_per_minute: outbound_source,
        max_stored_messages: stored_source,
        retention: retention_source,
        lease: lease_source,
        code_ttl: code_source,
    };
    let sources = KEYS
        .iter()
        .zip(key_sources.as_array())
        .map(|(key, source)| (key.name, source))
        .collect();
    Ok(LoadedDaemon {
        settings: DaemonSettings {
            listen,
            host_name,
            limits,
        },
        sources,
    })
}

fn ensure_file(path: &Path, text: &str) -> anyhow::Result<()> {
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot create {}", path.display()));
        }
    };
    if let Err(error) = file
        .write_all(text.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(path);
        return Err(error).with_context(|| format!("cannot write {}", path.display()));
    }
    Ok(())
}

pub fn ensure_daemon_file(layout: &Layout) -> anyhow::Result<()> {
    ensure_file(&layout.daemon_config(), &daemon_template())
}

fn client_display_prefix(loaded: &LoadedClient) -> String {
    format!(
        "\"{}\"",
        crate::prefix::format_prefix(loaded.settings.prefix)
    )
}

fn client_display_viewer_after(loaded: &LoadedClient) -> String {
    format!(
        "\"{}\"",
        format_duration(loaded.settings.viewer_after.unwrap_or(Duration::ZERO))
    )
}

#[derive(Clone, Copy)]
struct ClientKey {
    name: &'static str,
    description: &'static str,
    example: &'static str,
    display: fn(&LoadedClient) -> String,
}

const CLIENT_KEYS: &[ClientKey] = &[
    ClientKey {
        name: "prefix",
        description: "prefix key used by an attached client.",
        example: "\"C-b\"",
        display: client_display_prefix,
    },
    ClientKey {
        name: "viewer_after",
        description: "seconds without owner input before an attachment becomes a viewer.",
        example: "\"5m\"",
        display: client_display_viewer_after,
    },
];

pub fn load_client(layout: &Layout, overrides: &ClientOverrides) -> anyhow::Result<LoadedClient> {
    let path = layout.client_config();
    let file = read_config(&path)?;
    let parsed: ClientFile = file
        .as_deref()
        .map(|text| toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display())))
        .transpose()?
        .unwrap_or_default();
    let file_prefix = parsed
        .prefix
        .as_deref()
        .map(crate::prefix::parse_prefix)
        .transpose()
        .map_err(|error| file_error(&path, "prefix", error))?;
    let file_viewer_after = parsed
        .viewer_after
        .as_deref()
        .map(|value| {
            parse_limited_duration(
                &path,
                "viewer_after",
                value,
                DurationRule::WholeSecondsOrZero,
            )
        })
        .transpose()?;
    let (prefix, prefix_source) = resolve(DEFAULT_CLIENT_PREFIX, file_prefix, overrides.prefix);
    let (viewer_after, viewer_source) = resolve(
        Some(Duration::from_secs(DEFAULT_CLIENT_VIEWER_AFTER_SECS)),
        file_viewer_after.map(|value| (value != Duration::ZERO).then_some(value)),
        overrides
            .viewer_after_secs
            .map(|value| (value != 0).then_some(Duration::from_secs(value))),
    );
    Ok(LoadedClient {
        settings: ClientSettings {
            prefix,
            viewer_after,
        },
        sources: vec![("prefix", prefix_source), ("viewer_after", viewer_source)],
    })
}

fn append_template_keys(
    text: &mut String,
    keys: impl IntoIterator<Item = (&'static str, &'static str, &'static str)>,
) {
    for (name, description, example) in keys {
        text.push_str(&format!("# {name}: {description}\n# {name} = {example}\n"));
    }
}

pub fn client_template() -> String {
    let mut text = String::from(
        "# A2AMX client settings. Every a2amx command reads this file, so a change applies to the next command. The daemon writes this file once and never rewrites it.\n",
    );
    append_template_keys(
        &mut text,
        CLIENT_KEYS
            .iter()
            .map(|key| (key.name, key.description, key.example)),
    );
    text
}

pub fn ensure_client_file(layout: &Layout) -> anyhow::Result<()> {
    ensure_file(&layout.client_config(), &client_template())
}

pub fn show_client(layout: &Layout, overrides: &ClientOverrides) -> anyhow::Result<String> {
    let loaded = load_client(layout, overrides)?;
    let path = layout.client_config();
    let mut out = format!(
        "client.toml: {} ({})\n",
        path.display(),
        if path.exists() { "present" } else { "absent" }
    );
    for key in CLIENT_KEYS {
        let source = loaded
            .sources
            .iter()
            .find(|(name, _)| *name == key.name)
            .map(|(_, source)| source_label(*source))
            .unwrap_or("default");
        out.push_str(&format!(
            "{} = {}  ({source})\n",
            key.name,
            (key.display)(&loaded)
        ));
    }
    Ok(out)
}

pub fn show_daemon(layout: &Layout, env_owner_away_secs: Option<u64>) -> anyhow::Result<String> {
    let loaded = load_daemon(
        layout,
        &Overrides {
            owner_away_secs: env_owner_away_secs,
            ..Overrides::default()
        },
    )?;
    let present = layout.daemon_config().exists();
    let mut out = format!(
        "daemon.toml: {} ({})\n",
        layout.daemon_config().display(),
        if present { "present" } else { "absent" }
    );
    for (key, (_, source)) in KEYS.iter().zip(&loaded.sources) {
        let source = match source {
            Source::Default => "default",
            Source::File => "file",
            // Config show has only the environment override entry point.
            Source::Override => "env",
        };
        out.push_str(&format!(
            "{} = {}  ({source})\n",
            key.name,
            (key.display)(&loaded.settings)
        ));
    }
    Ok(out)
}
#[derive(Clone, Copy)]
struct GatewayKey {
    name: &'static str,
    description: &'static str,
    example: &'static str,
    display: fn(&LoadedGateway) -> String,
}

fn source_label(source: Source) -> &'static str {
    match source {
        Source::Default => "default",
        Source::File => "file",
        Source::Override => "override",
    }
}

fn gateway_display_guild(loaded: &LoadedGateway) -> String {
    loaded
        .guild_id
        .as_deref()
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "(unset)".into())
}
fn gateway_display_category(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", loaded.settings.control_category)
}
fn gateway_display_channel(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", loaded.settings.control_channel)
}
fn gateway_display_members(loaded: &LoadedGateway) -> String {
    loaded.settings.max_members.to_string()
}
fn gateway_display_sync(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", format_duration(loaded.settings.sync))
}
fn gateway_display_idle(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", format_duration(loaded.settings.idle))
}
fn gateway_display_member_check(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", format_duration(loaded.settings.member_check))
}
fn gateway_display_reconnect(loaded: &LoadedGateway) -> String {
    format!("\"{}\"", format_duration(loaded.settings.reconnect))
}

const GATEWAY_KEYS: &[GatewayKey] = &[
    GatewayKey {
        name: "guild_id",
        description: "Discord server ID (required to run the gateway).",
        example: "\"123456789012345678\"",
        display: gateway_display_guild,
    },
    GatewayKey {
        name: "control_category",
        description: "Discord category for the control channel.",
        example: "\"a2amx\"",
        display: gateway_display_category,
    },
    GatewayKey {
        name: "control_channel",
        description: "Discord channel used for control messages.",
        example: "\"control\"",
        display: gateway_display_channel,
    },
    GatewayKey {
        name: "max_members",
        description: "Pause the gateway when the server exceeds this member count.",
        example: "2",
        display: gateway_display_members,
    },
    GatewayKey {
        name: "sync",
        description: "Interval between Discord synchronization checks.",
        example: "\"15s\"",
        display: gateway_display_sync,
    },
    GatewayKey {
        name: "idle",
        description: "Interval between idle gateway checks.",
        example: "\"1s\"",
        display: gateway_display_idle,
    },
    GatewayKey {
        name: "member_check",
        description: "Interval between member-count checks.",
        example: "\"1m\"",
        display: gateway_display_member_check,
    },
    GatewayKey {
        name: "reconnect",
        description: "Delay before reconnecting after a gateway failure.",
        example: "\"5s\"",
        display: gateway_display_reconnect,
    },
];

pub fn load_gateway(layout: &Layout) -> anyhow::Result<LoadedGateway> {
    let path = layout.gateway_config();
    let text = match fs::symlink_metadata(&path) {
        Ok(_) => {
            let bytes = crate::relay_files::read_secure(&path)?;
            Some(
                String::from_utf8(bytes)
                    .map_err(|_| anyhow!("{}: gateway.toml is not UTF-8", path.display()))?,
            )
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("cannot inspect {}", path.display()));
        }
    };
    let file: GatewayFile = text
        .as_deref()
        .map(|text| toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display())))
        .transpose()?
        .unwrap_or_default();
    if let Some(value) = &file.guild_id
        && !crate::relay_files::valid_id(value)
    {
        bail!(
            "{}",
            file_error(&path, "guild_id", "must be 1 to 20 digits")
        );
    }
    for (key, value) in [
        ("control_category", file.control_category.as_deref()),
        ("control_channel", file.control_channel.as_deref()),
    ] {
        if let Some(value) = value
            && !crate::relay_files::valid_name(value)
        {
            bail!(
                "{}",
                file_error(
                    &path,
                    key,
                    "must start with a lowercase letter or digit and contain only lowercase letters, digits, or '-'"
                )
            );
        }
    }
    let max_members = file
        .max_members
        .map(|value| parse_counter(&path, "max_members", value))
        .transpose()?;
    let sync = file
        .sync
        .as_deref()
        .map(|value| parse_limited_duration(&path, "sync", value, DurationRule::Millis))
        .transpose()?;
    let idle = file
        .idle
        .as_deref()
        .map(|value| parse_limited_duration(&path, "idle", value, DurationRule::Millis))
        .transpose()?;
    let member_check = file
        .member_check
        .as_deref()
        .map(|value| parse_limited_duration(&path, "member_check", value, DurationRule::Millis))
        .transpose()?;
    let reconnect = file
        .reconnect
        .as_deref()
        .map(|value| parse_limited_duration(&path, "reconnect", value, DurationRule::Millis))
        .transpose()?;
    let guild_source = if file.guild_id.is_some() {
        Source::File
    } else {
        Source::Default
    };
    let guild_id = file.guild_id;
    let (control_category, category_source) =
        resolve("a2amx".to_owned(), file.control_category, None);
    let (control_channel, channel_source) =
        resolve("control".to_owned(), file.control_channel, None);
    let (max_members, members_source) = resolve(2, max_members, None);
    let (sync, sync_source) = resolve(Duration::from_secs(15), sync, None);
    let (idle, idle_source) = resolve(Duration::from_secs(1), idle, None);
    let (member_check, member_source) = resolve(Duration::from_secs(60), member_check, None);
    let (reconnect, reconnect_source) = resolve(Duration::from_secs(5), reconnect, None);
    let (api_base, _) = resolve(
        "https://discord.com/api/v10".to_owned(),
        file.api_base,
        None,
    );
    let (gateway_url, _) = resolve(
        "wss://gateway.discord.gg".to_owned(),
        file.gateway_url,
        None,
    );
    let settings = GatewaySettings {
        control_category,
        control_channel,
        max_members,
        sync,
        idle,
        member_check,
        reconnect,
        api_base,
        gateway_url,
    };
    Ok(LoadedGateway {
        guild_id,
        settings,
        sources: vec![
            ("guild_id", guild_source),
            ("control_category", category_source),
            ("control_channel", channel_source),
            ("max_members", members_source),
            ("sync", sync_source),
            ("idle", idle_source),
            ("member_check", member_source),
            ("reconnect", reconnect_source),
        ],
    })
}

pub fn gateway_template() -> String {
    let mut text = String::from(
        "# A2AMX gateway settings. Restart the gateway to apply changes. a2amx writes this file once and never rewrites it.\n",
    );
    append_template_keys(
        &mut text,
        GATEWAY_KEYS
            .iter()
            .map(|key| (key.name, key.description, key.example)),
    );
    text
}

pub fn ensure_gateway_file(layout: &Layout) -> anyhow::Result<()> {
    ensure_file(&layout.gateway_config(), &gateway_template())
}

pub fn show_gateway(layout: &Layout) -> anyhow::Result<String> {
    let loaded = load_gateway(layout)?;
    let path = layout.gateway_config();
    let mut out = format!(
        "gateway.toml: {} ({})\n",
        path.display(),
        if path.exists() { "present" } else { "absent" }
    );
    for key in GATEWAY_KEYS {
        let source = loaded
            .sources
            .iter()
            .find(|(name, _)| *name == key.name)
            .map(|(_, source)| source_label(*source))
            .unwrap_or("default");
        out.push_str(&format!(
            "{} = {}  ({source})\n",
            key.name,
            (key.display)(&loaded)
        ));
    }
    Ok(out)
}

fn default_text(key: &Key, limits: &Limits) -> String {
    match key.default {
        DefaultValue::Listen => format!(
            "[{}]",
            default_listen()
                .iter()
                .map(|addr| format!("\"{addr}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        DefaultValue::HostName => "\"host-a\"".into(),
        DefaultValue::OwnerAway => {
            format!(
                "\"{}\"",
                format_duration(Duration::from_secs(limits.away_secs))
            )
        }
        DefaultValue::Counter(value) => value(limits).to_string(),
        DefaultValue::Duration(value) => {
            format!(
                "\"{}\"",
                format_duration(Duration::from_secs(value(limits)))
            )
        }
    }
}

pub fn daemon_template() -> String {
    let defaults = Limits::default();
    let mut template = String::from(
        "# A2AMX daemon policy. Restart the daemon to apply changes. The daemon writes this file once and never rewrites it.\n",
    );
    for key in KEYS {
        if key.name == "limits.max_pending_per_recipient" {
            template.push_str("# [limits]\n");
        }
        let short_name = key.name.rsplit('.').next().unwrap_or(key.name);
        template.push_str(&format!(
            "# {short_name}: {}\n# {short_name} = {}\n",
            key.description,
            default_text(key, &defaults)
        ));
    }
    template
}

pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let (digits, unit) = ["ms", "s", "m", "h", "d"]
        .iter()
        .find_map(|unit| text.strip_suffix(unit).map(|digits| (digits, *unit)))
        .unwrap_or((text, ""));
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "{text:?} is not a duration; use digits followed by ms, s, m, h or d"
        ));
    }
    let multiplier = match unit {
        "ms" => 1,
        "s" => 1000,
        "m" => 60 * 1000,
        "h" => 3600 * 1000,
        "d" => 86400 * 1000,
        _ => {
            return Err(format!(
                "{text:?} is not a duration; use digits followed by ms, s, m, h or d"
            ));
        }
    };
    let value = digits
        .parse::<u64>()
        .map_err(|_| format!("{text:?} overflows u64"))?;
    let milliseconds = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("{text:?} overflows u64"))?;
    Ok(Duration::from_millis(milliseconds))
}

pub fn format_duration(duration: Duration) -> String {
    if duration.subsec_nanos() != 0 {
        return format!("{}ms", duration.as_millis());
    }
    let seconds = duration.as_secs();
    if seconds == 0 {
        return "0s".into();
    }
    for (unit, size) in [("d", 86400), ("h", 3600), ("m", 60)] {
        if seconds.is_multiple_of(size) {
            return format!("{}{}", seconds / size, unit);
        }
    }
    format!("{seconds}s")
}
