use std::time::Duration;

use anyhow::anyhow;
use serde::Deserialize;

use super::{
    DurationRule, Source, append_template_keys, ensure_file, file_error, format_duration,
    parse_limited_duration, read_config, resolve, source_label,
};
use crate::layout::Layout;

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
