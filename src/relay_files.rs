use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, bail};
use rustix::fs::OFlags;
use serde::{Deserialize, Serialize};

const MAX_FILE_BYTES: usize = 64 * 1024;
const DEFAULT_CONTROL_CATEGORY: &str = "a2amx";
const DEFAULT_CONTROL_CHANNEL: &str = "control";
const DEFAULT_MAX_MEMBERS: u64 = 2;
const DEFAULT_SYNC_MS: u64 = 15_000;
const DEFAULT_IDLE_MS: u64 = 1_000;
const DEFAULT_MEMBER_CHECK_MS: u64 = 60_000;
const DEFAULT_RECONNECT_MS: u64 = 5_000;
const DEFAULT_API_BASE: &str = "https://discord.com/api/v10";
const DEFAULT_GATEWAY_URL: &str = "wss://gateway.discord.gg";

#[derive(Debug, Clone)]
pub struct Loaded {
    pub layout: crate::layout::Layout,
    pub token: String,
    pub config: GatewayConfig,
    pub pairing: Option<Pairing>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    pub guild_id: String,
    #[serde(default = "default_control_category")]
    pub control_category: String,
    #[serde(default = "default_control_channel")]
    pub control_channel: String,
    #[serde(default = "default_max_members")]
    pub max_members: u64,
    #[serde(default = "default_sync_ms")]
    pub sync_ms: u64,
    #[serde(default = "default_idle_ms")]
    pub idle_ms: u64,
    #[serde(default = "default_member_check_ms")]
    pub member_check_ms: u64,
    #[serde(default = "default_reconnect_ms")]
    pub reconnect_ms: u64,
    #[serde(default = "default_api_base")]
    pub api_base: String,
    #[serde(default = "default_gateway_url")]
    pub gateway_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pairing {
    pub guild_id: String,
    pub user_id: String,
}

fn default_control_category() -> String {
    DEFAULT_CONTROL_CATEGORY.into()
}

fn default_control_channel() -> String {
    DEFAULT_CONTROL_CHANNEL.into()
}

fn default_max_members() -> u64 {
    DEFAULT_MAX_MEMBERS
}

fn default_sync_ms() -> u64 {
    DEFAULT_SYNC_MS
}

fn default_idle_ms() -> u64 {
    DEFAULT_IDLE_MS
}

fn default_member_check_ms() -> u64 {
    DEFAULT_MEMBER_CHECK_MS
}

fn default_reconnect_ms() -> u64 {
    DEFAULT_RECONNECT_MS
}

fn default_api_base() -> String {
    DEFAULT_API_BASE.into()
}

fn default_gateway_url() -> String {
    DEFAULT_GATEWAY_URL.into()
}

pub fn load(home: &Path) -> anyhow::Result<Loaded> {
    let layout = crate::layout::Layout::new(home.to_owned());
    let gateway_dir = layout.gateway_state_dir();
    let metadata = fs::symlink_metadata(&gateway_dir)
        .with_context(|| format!("cannot inspect {}", gateway_dir.display()))?;
    if !metadata.file_type().is_dir() {
        bail!(
            "gateway state path is not a directory: {}",
            gateway_dir.display()
        );
    }
    let token_path = layout.discord_token();
    let config_path = layout.gateway_config();
    let token = read_secure(&token_path)?;
    let token = String::from_utf8(token)
        .with_context(|| format!("{} is not UTF-8", token_path.display()))?
        .trim()
        .to_owned();
    if token.is_empty() {
        bail!("{} is empty", token_path.display());
    }
    let config_bytes = read_secure(&config_path)?;
    let config_text = std::str::from_utf8(&config_bytes)
        .with_context(|| format!("{} is not UTF-8", config_path.display()))?;
    let config: GatewayConfig = toml::from_str(config_text).map_err(|error| {
        anyhow::anyhow!("invalid gateway config {}: {error}", config_path.display())
    })?;
    validate_config(&config)?;

    let pairing_path = layout.pairing();
    let pairing = match fs::symlink_metadata(&pairing_path) {
        Ok(_) => {
            let bytes = read_secure(&pairing_path)?;
            let pairing: Pairing = serde_json::from_slice(&bytes)
                .with_context(|| format!("invalid pairing file {}", pairing_path.display()))?;
            validate_pairing(&pairing, &config.guild_id, &pairing_path)?;
            Some(pairing)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("cannot inspect {}", pairing_path.display()));
        }
    };

    let layout = crate::layout::Layout::new(home.to_owned());
    Ok(Loaded {
        layout,
        token,
        config,
        pairing,
    })
}

pub fn write_token(home: &Path, token: &str) -> anyhow::Result<()> {
    if token.is_empty() {
        bail!("gateway token is empty");
    }
    let layout = crate::layout::Layout::new(home.to_owned());
    atomic_write(&layout.discord_token(), token.as_bytes(), 0o600)
}

pub fn write_pairing(home: &Path, pairing: &Pairing) -> anyhow::Result<()> {
    if !valid_id(&pairing.guild_id) || !valid_id(&pairing.user_id) {
        bail!("pairing ids must be digit strings");
    }
    let layout = crate::layout::Layout::new(home.to_owned());
    let payload = serde_json::to_vec(pairing)?;
    atomic_write(&layout.pairing(), &payload, 0o600)
}

fn validate_config(config: &GatewayConfig) -> anyhow::Result<()> {
    if !valid_id(&config.guild_id) {
        bail!("invalid guild_id in config.toml");
    }
    for (name, value) in [
        ("control_category", config.control_category.as_str()),
        ("control_channel", config.control_channel.as_str()),
    ] {
        if !valid_name(value) {
            bail!("invalid {name} in config.toml");
        }
    }
    for (name, value) in [
        ("sync_ms", config.sync_ms),
        ("idle_ms", config.idle_ms),
        ("member_check_ms", config.member_check_ms),
        ("reconnect_ms", config.reconnect_ms),
    ] {
        if value == 0 {
            bail!("{name} must be at least 1");
        }
    }
    Ok(())
}

fn validate_pairing(pairing: &Pairing, guild_id: &str, path: &Path) -> anyhow::Result<()> {
    if !valid_id(&pairing.guild_id) || !valid_id(&pairing.user_id) {
        bail!("invalid ids in {}", path.display());
    }
    if pairing.guild_id != guild_id {
        bail!("pairing guild_id differs from config guild_id");
    }
    Ok(())
}

pub(crate) fn valid_id(value: &str) -> bool {
    (1..=20).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_name(value: &str) -> bool {
    if value.is_empty() || value.len() > 100 {
        return false;
    }
    let bytes = value.as_bytes();
    (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn read_secure(path: &Path) -> anyhow::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags((OFlags::NOFOLLOW | OFlags::CLOEXEC).bits() as i32)
        .open(path)
        .with_context(|| format!("cannot open secure file {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("cannot stat secure file {}", path.display()))?;
    let mode = metadata.mode();
    let uid = rustix::process::geteuid().as_raw();
    if !metadata.file_type().is_file() {
        bail!("secure file is not regular: {}", path.display());
    }
    if metadata.uid() != uid {
        bail!("secure file is not owned by this user: {}", path.display());
    }
    if mode & 0o077 != 0 {
        bail!("secure file permissions are too broad: {}", path.display());
    }
    let mut bytes = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read secure file {}", path.display()))?;
    if bytes.len() > MAX_FILE_BYTES {
        bail!("secure file exceeds 64 KiB: {}", path.display());
    }
    Ok(bytes)
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> anyhow::Result<()> {
    let random = crate::daemon::random_hex::<4>()?;
    let temp = path.with_file_name(format!(
        ".{}.tmp-{random}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let result = (|| -> anyhow::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(OFlags::CLOEXEC.bits() as i32)
            .mode(mode)
            .open(&temp)
            .with_context(|| format!("cannot create temporary file for {}", path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(mode))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path).with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
