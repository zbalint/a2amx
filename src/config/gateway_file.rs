use std::fs;
use std::time::Duration;

use anyhow::{Context, anyhow, bail};
use serde::Deserialize;

use super::{
    DurationRule, Source, append_template_keys, ensure_file, file_error, format_duration,
    parse_counter, parse_limited_duration, resolve, source_label,
};
use crate::layout::Layout;

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

#[derive(Clone, Copy)]
struct GatewayKey {
    name: &'static str,
    description: &'static str,
    example: &'static str,
    display: fn(&LoadedGateway) -> String,
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
