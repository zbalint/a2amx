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

fn file_error(path: &Path, key: &str, message: impl std::fmt::Display) -> anyhow::Error {
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
    let metadata = file
        .metadata()
        .with_context(|| format!("cannot inspect {}", path.display()))?;
    if !metadata.file_type().is_file() {
        bail!("{}: daemon.toml is not a regular file", path.display());
    }
    let mut bytes = Vec::with_capacity(MAX_FILE_BYTES + 1);
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read {}", path.display()))?;
    if bytes.len() > MAX_FILE_BYTES {
        bail!("{}: daemon.toml exceeds 65536 bytes", path.display());
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| anyhow!("{}: daemon.toml is not UTF-8", path.display()))?;
    Ok(Some(text))
}

fn parse_file(path: &Path, text: &str) -> anyhow::Result<FileConfig> {
    toml::from_str(text).map_err(|error| anyhow!("{}: {error}", path.display()))
}

fn parse_counter(path: &Path, key: &'static str, value: u32) -> anyhow::Result<u32> {
    if value == 0 {
        bail!(
            "{}: key {}: 0 is not allowed; use 1 or more",
            path.display(),
            key
        );
    }
    Ok(value)
}

fn validate_duration_secs(
    path: &Path,
    key: &'static str,
    seconds: u64,
    allow_zero: bool,
) -> anyhow::Result<u64> {
    if (!allow_zero && seconds == 0) || seconds > MAX_DURATION_SECS {
        let minimum = if allow_zero { "0s" } else { "1s" };
        bail!(
            "{}: key {}: duration must be between {} and 3650d",
            path.display(),
            key,
            minimum
        );
    }
    Ok(seconds)
}

fn parse_limited_duration(
    path: &Path,
    key: &'static str,
    text: &str,
    allow_zero: bool,
) -> anyhow::Result<u64> {
    let seconds = parse_duration(text)
        .map_err(|error| file_error(path, key, error))?
        .as_secs();
    validate_duration_secs(path, key, seconds, allow_zero)
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
        .map(|value| parse_limited_duration(&path, "owner_away", &value, true))
        .transpose()?;
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
        .map(|value| parse_limited_duration(&path, "limits.retention", value, false))
        .transpose()?;
    let file_lease = file_limits
        .lease
        .as_deref()
        .map(|value| parse_limited_duration(&path, "limits.lease", value, false))
        .transpose()?;
    let file_code_ttl = file_limits
        .code_ttl
        .as_deref()
        .map(|value| parse_limited_duration(&path, "limits.code_ttl", value, false))
        .transpose()?;
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
            .map(|value| validate_duration_secs(&path, "owner_away", value, true))
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

pub fn ensure_daemon_file(layout: &Layout) -> anyhow::Result<()> {
    let path = layout.daemon_config();
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot create {}", path.display()));
        }
    };
    if let Err(error) = file
        .write_all(daemon_template().as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(&path);
        return Err(error).with_context(|| format!("cannot write {}", path.display()));
    }
    Ok(())
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
    let (digits, unit) = ["s", "m", "h", "d"]
        .iter()
        .find_map(|unit| text.strip_suffix(unit).map(|digits| (digits, *unit)))
        .unwrap_or((text, ""));
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "{text:?} is not a duration; use digits followed by s, m, h or d"
        ));
    }
    let multiplier = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3600,
        "d" => 86400,
        _ => {
            return Err(format!(
                "{text:?} is not a duration; use digits followed by s, m, h or d"
            ));
        }
    };
    let value = digits
        .parse::<u64>()
        .map_err(|_| format!("{text:?} overflows u64"))?;
    let seconds = value
        .checked_mul(multiplier)
        .ok_or_else(|| format!("{text:?} overflows u64"))?;
    Ok(Duration::from_secs(seconds))
}

pub fn format_duration(duration: Duration) -> String {
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
