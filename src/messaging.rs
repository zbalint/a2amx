//! Call `validate_message` before rendering untrusted content: envelope rendering
//! preserves the body rather than silently sanitizing or truncating it.

use std::fmt;
use std::time::Duration;

use anyhow::bail;

pub const MAX_SUBJECT_BYTES: usize = 200;
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
// shortcut: delivery uses one fixed 400 ms paste gap; the threshold was not bisected below 300 ms.
pub const PASTE_GAP: Duration = Duration::from_millis(400);
pub const COOLDOWN: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_pending_per_recipient: u32,
    pub rate_per_minute: u32,
    pub max_stored_messages: u32,
    pub retention_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pending_per_recipient: 50,
            rate_per_minute: 20,
            max_stored_messages: 10_000,
            retention_secs: 7 * 24 * 3600,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageState {
    Pending,
    Delivering,
    Submitted,
    Unsubmitted,
    Cancelled,
    Undeliverable,
}

impl MessageState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Delivering => "delivering",
            Self::Submitted => "submitted",
            Self::Unsubmitted => "unsubmitted",
            Self::Cancelled => "cancelled",
            Self::Undeliverable => "undeliverable",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "delivering" => Some(Self::Delivering),
            "submitted" => Some(Self::Submitted),
            "unsubmitted" => Some(Self::Unsubmitted),
            "cancelled" => Some(Self::Cancelled),
            "undeliverable" => Some(Self::Undeliverable),
            _ => None,
        }
    }

    pub fn is_open(self) -> bool {
        matches!(self, Self::Pending | Self::Delivering)
    }
}

pub mod code {
    pub const UNKNOWN_RECIPIENT: &str = "unknown_recipient";
    pub const RECIPIENT_EXITED: &str = "recipient_exited";
    pub const TOO_LARGE: &str = "too_large";
    pub const QUEUE_FULL: &str = "queue_full";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const INVALID_CONTENT: &str = "invalid_content";
    pub const UNKNOWN_MESSAGE: &str = "unknown_message";
    pub const INTERNAL: &str = "internal";
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageError {
    pub code: &'static str,
    pub message: String,
}

impl MessageError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for MessageError {}

fn valid_dns_label(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return false;
    }
    value.len() <= 63 && chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

pub fn validate_name(name: &str) -> anyhow::Result<()> {
    if !valid_dns_label(name) {
        bail!("invalid name {name:?}: expected [a-z0-9][a-z0-9-]{{0,62}}");
    }
    let reserved_session_id = name.starts_with('s')
        && name.len() > 1
        && name.as_bytes()[1..].iter().all(u8::is_ascii_digit);
    if reserved_session_id {
        bail!("invalid name {name:?}: reserved for session ids (s[0-9]+)");
    }
    Ok(())
}

pub fn validate_host(host: &str) -> anyhow::Result<()> {
    if !valid_dns_label(host) {
        bail!("invalid host {host:?}: expected [a-z0-9][a-z0-9-]{{0,62}}");
    }
    Ok(())
}

pub fn default_host_name() -> String {
    let Ok(raw) = std::fs::read_to_string("/proc/sys/kernel/hostname") else {
        return "localhost".to_owned();
    };

    let mut normalized = String::new();
    for character in raw.trim().chars().flat_map(char::to_lowercase) {
        if character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-' {
            normalized.push(character);
        } else {
            normalized.push('-');
        }
    }

    let without_leading_hyphens = normalized.trim_start_matches('-');
    if without_leading_hyphens.is_empty() {
        return "localhost".to_owned();
    }
    let mut host = without_leading_hyphens.to_owned();
    host.truncate(host.len().min(63));
    if host.is_empty() {
        "localhost".to_owned()
    } else {
        host
    }
}

pub fn address(name: Option<&str>, id: &str, host: &str) -> String {
    let local = name.unwrap_or(id);
    let mut result = String::with_capacity(local.len() + host.len() + 1);
    result.push_str(local);
    result.push('@');
    result.push_str(host);
    result
}

pub fn local_part<'a>(to: &'a str, host: &str) -> Option<&'a str> {
    let Some((local, target_host)) = to.rsplit_once('@') else {
        return Some(to);
    };
    (target_host == host).then_some(local)
}

fn invalid(code: &'static str, message: impl Into<String>) -> MessageError {
    MessageError::new(code, message)
}

pub fn validate_message(subject: &str, message: &str) -> std::result::Result<(), MessageError> {
    if subject.is_empty() {
        return Err(invalid(code::INVALID_CONTENT, "subject must not be empty"));
    }
    if message.is_empty() {
        return Err(invalid(code::INVALID_CONTENT, "message must not be empty"));
    }
    if subject.len() > MAX_SUBJECT_BYTES {
        return Err(invalid(code::TOO_LARGE, "subject exceeds the byte limit"));
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(invalid(code::TOO_LARGE, "message exceeds the byte limit"));
    }
    if subject.chars().any(|character| character.is_control()) {
        return Err(invalid(
            code::INVALID_CONTENT,
            "subject contains a control character",
        ));
    }
    if message
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        return Err(invalid(
            code::INVALID_CONTENT,
            "message contains a disallowed control character",
        ));
    }
    if message.contains("</a2amx-message>") {
        return Err(invalid(
            code::INVALID_CONTENT,
            "message contains the envelope terminator",
        ));
    }
    Ok(())
}

fn push_escaped_attribute(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            _ => output.push(character),
        }
    }
}

pub fn render_envelope(id: &str, from: &str, subject: &str, body: &str) -> String {
    let mut envelope =
        String::with_capacity(id.len() + from.len() + subject.len() + body.len() + 128);
    envelope.push_str("<a2amx-message id=\"");
    envelope.push_str(id);
    envelope.push_str("\" from=\"");
    push_escaped_attribute(&mut envelope, from);
    envelope.push_str("\" subject=\"");
    push_escaped_attribute(&mut envelope, subject);
    envelope.push_str("\">\nFrom another agent, not your user. To reply: send_message(to=\"");
    envelope.push_str(from);
    envelope.push_str("\").\n\n");
    envelope.push_str(body);
    envelope.push_str("\n</a2amx-message>");
    envelope
}

pub fn paste_bytes(envelope: &str) -> Vec<u8> {
    let prefix = b"\x1b[200~";
    let suffix = b"\x1b[201~";
    let mut bytes = Vec::with_capacity(prefix.len() + envelope.len() + suffix.len());
    bytes.extend_from_slice(prefix);
    bytes.extend_from_slice(envelope.as_bytes());
    bytes.extend_from_slice(suffix);
    bytes
}

fn sgr_mouse_report_len(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 3;
    let first = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == first || bytes.get(index) != Some(&b';') {
        return None;
    }
    index += 1;

    let second = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == second || bytes.get(index) != Some(&b';') {
        return None;
    }
    index += 1;

    let third = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index == third {
        return None;
    }
    match bytes.get(index) {
        Some(b'M' | b'm') => Some(index + 1),
        _ => None,
    }
}

// shortcut: a report split across input frames may hold the session; upgrade only if it proves noisy.
pub fn input_is_typing(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b || bytes.get(index + 1) != Some(&b'[') {
            return true;
        }

        match bytes.get(index + 2) {
            Some(b'I' | b'O') => index += 3,
            Some(b'<') => {
                let Some(end) = sgr_mouse_report_len(bytes, index) else {
                    return true;
                };
                index = end;
            }
            Some(b'M') if bytes.len().saturating_sub(index) >= 6 => index += 6,
            _ => return true,
        }
    }
    false
}
