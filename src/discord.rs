use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

mod watch;
pub use watch::Watch;

const RESPONSE_LIMIT: u64 = 1024 * 1024;
const MAX_RETRIES: u8 = 3;

#[derive(Clone)]
pub struct DiscordConfig {
    pub token: String,
    pub guild_id: String,
    pub api_base: String,
    pub gateway_url: String,
    pub reconnect_base: Duration,
    pub reconnect_max: Duration,
    pub queue: usize,
    pub request_timeout: Duration,
    pub max_retry_wait: Duration,
    pub identify_gap: Duration,
    pub stable_after: Duration,
}

impl DiscordConfig {
    pub fn new(token: String, guild_id: String) -> Self {
        Self {
            token,
            guild_id,
            api_base: "https://discord.com/api/v10".into(),
            gateway_url: "wss://gateway.discord.gg".into(),
            reconnect_base: Duration::from_secs(1),
            reconnect_max: Duration::from_secs(30),
            queue: 1024,
            request_timeout: Duration::from_secs(30),
            max_retry_wait: Duration::from_secs(30),
            identify_gap: Duration::from_secs(5),
            stable_after: Duration::from_secs(60),
        }
    }
}

impl fmt::Debug for DiscordConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DiscordConfig")
            .field("token", &"<redacted>")
            .field("guild_id", &self.guild_id)
            .field("api_base", &self.api_base)
            .field("gateway_url", &self.gateway_url)
            .field("reconnect_base", &self.reconnect_base)
            .field("reconnect_max", &self.reconnect_max)
            .field("queue", &self.queue)
            .field("request_timeout", &self.request_timeout)
            .field("max_retry_wait", &self.max_retry_wait)
            .field("identify_gap", &self.identify_gap)
            .field("stable_after", &self.stable_after)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordError {
    Status(u16),
    Transport(&'static str),
    Invalid(&'static str),
}

impl fmt::Display for DiscordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Status(code) => write!(f, "Discord HTTP status {code}"),
            Self::Transport(kind) => write!(f, "Discord transport failure ({kind})"),
            Self::Invalid(what) => write!(f, "invalid Discord input ({what})"),
        }
    }
}

impl std::error::Error for DiscordError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInfo {
    pub id: String,
    pub kind: u8,
    pub name: String,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fatal(pub String);

impl fmt::Display for Fatal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Fatal {}

#[derive(Clone)]
pub struct Discord {
    config: Arc<DiscordConfig>,
    agent: ureq::Agent,
}

impl Discord {
    pub fn new(config: DiscordConfig) -> Self {
        let timeout = config.request_timeout;
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(timeout))
            .build()
            .into();
        Self {
            config: Arc::new(config),
            agent,
        }
    }

    pub async fn ensure_category(&self, name: &str) -> Result<String, DiscordError> {
        let this = self.clone();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || this.ensure_category_blocking(&name))
            .await
            .map_err(|_| DiscordError::Transport("task"))?
    }

    pub async fn ensure_text_channel(
        &self,
        parent_id: &str,
        name: &str,
    ) -> Result<String, DiscordError> {
        if !valid_id(parent_id) || !valid_channel_name(name) {
            return Err(DiscordError::Invalid("channel"));
        }
        let this = self.clone();
        let parent_id = parent_id.to_owned();
        let name = name.to_owned();
        tokio::task::spawn_blocking(move || this.ensure_text_channel_blocking(&parent_id, &name))
            .await
            .map_err(|_| DiscordError::Transport("task"))?
    }

    pub async fn send(&self, channel_id: &str, text: &str) -> Result<(), DiscordError> {
        if !valid_id(channel_id) {
            return Err(DiscordError::Invalid("channel"));
        }
        if text.is_empty() || text.chars().count() > crate::gateway::MAX_CHAT_CHARS {
            return Err(DiscordError::Invalid("text"));
        }
        let this = self.clone();
        let channel_id = channel_id.to_owned();
        let text = text.to_owned();
        tokio::task::spawn_blocking(move || this.send_blocking(&channel_id, &text))
            .await
            .map_err(|_| DiscordError::Transport("task"))?
    }

    pub async fn list_channels(&self) -> Result<Vec<ChannelInfo>, DiscordError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.list_channels_blocking())
            .await
            .map_err(|_| DiscordError::Transport("task"))?
    }

    pub async fn member_count(&self) -> Result<u64, DiscordError> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || this.member_count_blocking())
            .await
            .map_err(|_| DiscordError::Transport("task"))?
    }

    pub fn watch(&self) -> Watch {
        Watch::start(Arc::clone(&self.config))
    }

    fn ensure_category_blocking(&self, name: &str) -> Result<String, DiscordError> {
        self.validate_guild()?;
        let value = self.request(
            "GET",
            &format!("/guilds/{}/channels", self.config.guild_id),
            None,
        )?;
        if let Some(id) = find_channel_id(&value, 4, name, None) {
            return Ok(id);
        }
        let value = self.request(
            "POST",
            &format!("/guilds/{}/channels", self.config.guild_id),
            Some(&json!({"name": name, "type": 4})),
        )?;
        response_id(&value)
    }

    fn ensure_text_channel_blocking(
        &self,
        parent_id: &str,
        name: &str,
    ) -> Result<String, DiscordError> {
        self.validate_guild()?;
        let value = self.request(
            "GET",
            &format!("/guilds/{}/channels", self.config.guild_id),
            None,
        )?;
        if let Some(id) = find_channel_id(&value, 0, name, Some(parent_id)) {
            return Ok(id);
        }
        let value = self.request(
            "POST",
            &format!("/guilds/{}/channels", self.config.guild_id),
            Some(&json!({"name": name, "type": 0, "parent_id": parent_id})),
        )?;
        response_id(&value)
    }

    fn list_channels_blocking(&self) -> Result<Vec<ChannelInfo>, DiscordError> {
        self.validate_guild()?;
        let value = self.request(
            "GET",
            &format!("/guilds/{}/channels", self.config.guild_id),
            None,
        )?;
        let Some(items) = value.as_array() else {
            return Err(DiscordError::Invalid("response"));
        };
        Ok(items.iter().filter_map(parse_channel).collect())
    }

    fn member_count_blocking(&self) -> Result<u64, DiscordError> {
        self.validate_guild()?;
        let value = self.request(
            "GET",
            &format!("/guilds/{}?with_counts=true", self.config.guild_id),
            None,
        )?;
        value
            .get("approximate_member_count")
            .and_then(Value::as_u64)
            .ok_or(DiscordError::Invalid("response"))
    }

    fn send_blocking(&self, channel_id: &str, text: &str) -> Result<(), DiscordError> {
        let _ = self.request(
            "POST",
            &format!("/channels/{channel_id}/messages"),
            Some(&json!({
                "content": text,
                "allowed_mentions": {"parse": []}
            })),
        )?;
        Ok(())
    }

    fn validate_guild(&self) -> Result<(), DiscordError> {
        valid_id(&self.config.guild_id)
            .then_some(())
            .ok_or(DiscordError::Invalid("guild"))
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value, DiscordError> {
        let url = format!("{}{}", self.config.api_base.trim_end_matches('/'), path);
        let mut retries = 0_u8;
        loop {
            let response = self.call_once(method, &url, body)?;
            if response.status == 429 {
                if retries >= MAX_RETRIES {
                    return Err(DiscordError::Status(429));
                }
                let wait = retry_wait(&response, self.config.max_retry_wait);
                std::thread::sleep(wait);
                retries = retries.saturating_add(1);
                continue;
            }
            if !(200..300).contains(&response.status) {
                return Err(DiscordError::Status(response.status));
            }
            if response.body.is_empty() {
                return Ok(Value::Null);
            }
            return serde_json::from_str(&response.body)
                .map_err(|_| DiscordError::Transport("response json"));
        }
    }

    fn call_once(
        &self,
        method: &str,
        url: &str,
        body: Option<&Value>,
    ) -> Result<HttpResponse, DiscordError> {
        let authorization = format!("Bot {}", self.config.token);
        let user_agent = format!(
            "DiscordBot (https://github.com/zbalint/a2amx, {})",
            env!("CARGO_PKG_VERSION")
        );
        let result = match (method, body) {
            ("GET", None) => self
                .agent
                .get(url)
                .header("Authorization", authorization)
                .header("User-Agent", user_agent)
                .call(),
            ("POST", Some(value)) => {
                let encoded = serde_json::to_vec(value)
                    .map_err(|_| DiscordError::Transport("request json"))?;
                self.agent
                    .post(url)
                    .header("Authorization", authorization)
                    .header("User-Agent", user_agent)
                    .content_type("application/json")
                    .send(encoded)
            }
            _ => return Err(DiscordError::Invalid("request")),
        };
        let mut response = result.map_err(map_ureq_error)?;
        let status = response.status().as_u16();
        let retry_after = response
            .headers()
            .get("Retry-After")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response
            .body_mut()
            .with_config()
            .limit(RESPONSE_LIMIT)
            .read_to_string()
            .map_err(map_ureq_error)?;
        Ok(HttpResponse {
            status,
            retry_after,
            body,
        })
    }
}

struct HttpResponse {
    status: u16,
    retry_after: Option<String>,
    body: String,
}

fn map_ureq_error(error: ureq::Error) -> DiscordError {
    use ureq::Error;
    let kind = match error {
        Error::Timeout(_) => "timeout",
        Error::Io(_) => "io",
        Error::HostNotFound => "host not found",
        Error::Protocol(_) => "protocol",
        Error::Tls(_) | Error::Rustls(_) => "tls",
        Error::BadUri(_) => "uri",
        Error::Http(_) => "http",
        Error::ConnectionFailed => "connection",
        Error::BodyExceedsLimit(_) => "response too large",
        Error::LargeResponseHeader(_, _) => "response header",
        _ => "request",
    };
    DiscordError::Transport(kind)
}

fn retry_wait(response: &HttpResponse, cap: Duration) -> Duration {
    let parse = |value: f64| {
        if !value.is_finite() || value < 0.0 {
            return None;
        }
        if value >= cap.as_secs_f64() {
            return Some(cap);
        }
        Duration::try_from_secs_f64(value).ok()
    };
    let from_body = serde_json::from_str::<Value>(&response.body)
        .ok()
        .and_then(|value| value.get("retry_after").and_then(Value::as_f64))
        .and_then(parse);
    let from_header = response
        .retry_after
        .as_deref()
        .and_then(|value| value.parse::<f64>().ok())
        .and_then(parse);
    from_body
        .or(from_header)
        .unwrap_or_else(|| Duration::from_secs(1).min(cap))
}

fn valid_channel_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=100).contains(&bytes.len())
        && (bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit())
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn response_id(value: &Value) -> Result<String, DiscordError> {
    value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(DiscordError::Invalid("response"))
}

fn find_channel_id(value: &Value, kind: u8, name: &str, parent_id: Option<&str>) -> Option<String> {
    value.as_array()?.iter().find_map(|entry| {
        let channel = parse_channel(entry)?;
        (channel.kind == kind
            && channel.name == name
            && parent_id.is_none_or(|parent| channel.parent_id.as_deref() == Some(parent)))
        .then_some(channel.id)
    })
}

fn parse_channel(value: &Value) -> Option<ChannelInfo> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.to_owned();
    let kind = u8::try_from(object.get("type")?.as_u64()?).ok()?;
    let name = object.get("name")?.as_str()?.to_owned();
    let parent_id = match object.get("parent_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => return None,
    };
    Some(ChannelInfo {
        id,
        kind,
        name,
        parent_id,
    })
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|byte| byte.is_ascii_digit())
}
