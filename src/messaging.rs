//! Call `validate_message` before rendering untrusted content: envelope rendering
//! preserves the body rather than silently sanitizing or truncating it.

use std::fmt;
use std::time::Duration;

use anyhow::bail;
use serde::{Deserialize, Serialize};
pub const MAX_SUBJECT_BYTES: usize = 200;
pub const SYSTEM_SENDER: &str = "a2amx-daemon";
pub const HEARTBEAT_SUBJECT: &str = "heartbeat";
pub const MAX_MESSAGE_BYTES: usize = 32 * 1024;
pub const HEARTBEAT_MAX: Duration = Duration::from_secs(24 * 60 * 60);
// shortcut: delivery uses one fixed 400 ms paste gap; the threshold was not bisected below 300 ms.
pub const PASTE_GAP: Duration = Duration::from_millis(400);
pub const COOLDOWN: Duration = Duration::from_secs(1);
pub const MAX_CORRUPTED_SUBMISSIONS: u32 = 3;
pub const MAX_MESSAGE_REJECTIONS: u32 = 2;
pub const UNMATCHABLE_SUBMISSION_REASON: &str = "A2AMX blocked this prompt because it did not match the message it delivered. The message will not be retried.";
pub const MAX_RESTORE_BYTES: usize = 64 * 1024;
pub const CORRUPTED_SUBMISSION_REASON: &str = "A2AMX blocked this prompt because it mixed your text with a peer message. The message will be delivered again.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamScope {
    pub name: String,
    pub private: bool,
    pub allow: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct Party<'a> {
    pub name: Option<&'a str>,
    pub team: Option<&'a TeamScope>,
}

pub fn display_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds >= 60 * 60 {
        format!("{}h", seconds / (60 * 60))
    } else if seconds >= 60 {
        format!("{}m", seconds / 60)
    } else {
        format!("{}s", seconds.max(1))
    }
}

pub fn hold_explanation(reason: &str) -> Option<&'static str> {
    match reason {
        "deliver_hold" => Some(
            "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery.",
        ),
        "unsubmitted_envelope" => {
            Some("A message was pasted into the recipient's prompt box and not submitted yet.")
        }
        "corrupted_submissions" => Some(
            "Three submissions in a row were blocked. A person must release the recipient session (prefix, then r).",
        ),
        "human_draft" => Some(
            "A person typed in the recipient session. Delivery resumes when they submit or release it (prefix, then r).",
        ),
        "queued" => Some("An earlier message to this recipient is still open."),
        "not_ready" => {
            Some("The recipient's prompt box is not ready, for example a dialog is open.")
        }
        "cooldown" => Some("A message was submitted less than a second ago."),
        "channel_down" => Some(
            "The recipient's A2AMX extension is not connected; the message waits until it connects.",
        ),
        "channel_refused" => Some(
            "The recipient's A2AMX extension refused to connect (a version mismatch or an OMP feature it needs is missing); a person must update A2AMX or OMP.",
        ),
        "draft_present" => Some(
            "A person has unsent text in the recipient's prompt box; delivery resumes when it is sent or cleared.",
        ),
        "app_server_down" => Some(
            "The recipient's Codex app-server is not reachable; the message waits until it is.",
        ),
        "no_thread" => Some(
            "The recipient's Codex has no conversation loaded yet, for example while it shows a startup dialog; the message waits until it does.",
        ),
        "waiting_on_approval" => Some(
            "The recipient's Codex is waiting for a person to answer an approval or question dialog.",
        ),
        "thread_error" => Some("The recipient's Codex conversation is in an error state."),
        "in_flight" => Some(
            "An earlier message is queued in the recipient and has not entered its conversation yet.",
        ),
        _ => None,
    }
}

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
    pub const NOT_PERMITTED: &str = "not_permitted";
    pub const UNKNOWN_SESSION: &str = "unknown_session";
    pub const EXITED: &str = "exited";
    pub const HELD: &str = "held";
    pub const NOT_READY: &str = "not_ready";
    pub const DRAFT_PRESENT: &str = "draft_present";
    pub const BUSY: &str = "busy";
    pub const STEP_TIMEOUT: &str = "step_timeout";
    pub const WRITE_FAILED: &str = "write_failed";
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

/// Visible when both parties share a team, either side is ungrouped/public, or mutual team allow.
/// It is also visible for two named parties when each session-level agents list contains the other.
/// The result is symmetric and requires mutual consent for both allow forms.
pub fn visible(a: Party<'_>, b: Party<'_>) -> bool {
    let team_visible = match (a.team, b.team) {
        (Some(a), Some(b)) if a.name == b.name => true,
        (Some(a), Some(b)) if a.private && b.private => {
            a.allow.iter().any(|name| name == &b.name) && b.allow.iter().any(|name| name == &a.name)
        }
        _ => true,
    };
    team_visible
        || match (a.name, a.team, b.name, b.team) {
            (Some(a_name), Some(a_team), Some(b_name), Some(b_team)) => {
                a_team.agents.iter().any(|name| name == b_name)
                    && b_team.agents.iter().any(|name| name == a_name)
            }
            _ => false,
        }
}

pub fn validate_role(role: &str) -> anyhow::Result<()> {
    if role.is_empty() {
        bail!("role must not be empty");
    }
    if role.chars().count() > 64 {
        bail!("role must be at most 64 characters");
    }
    if role.chars().any(char::is_control) {
        bail!("role must not contain control characters");
    }
    if role.trim() != role {
        bail!("role must not have leading or trailing whitespace");
    }
    Ok(())
}

pub fn validate_name(name: &str) -> anyhow::Result<()> {
    if !valid_dns_label(name) {
        bail!("invalid name {name:?}: expected [a-z0-9][a-z0-9-]{{0,62}}");
    }
    let reserved_session_id = name.starts_with('s')
        && name.len() > 1
        && name.as_bytes()[1..].iter().all(u8::is_ascii_digit);
    if name == SYSTEM_SENDER {
        bail!("invalid name {name:?}: reserved for the daemon");
    }
    if reserved_session_id {
        bail!("invalid name {name:?}: reserved for session ids (s[0-9]+)");
    }
    Ok(())
}

pub fn validate_reset_steps(steps: &[String]) -> anyhow::Result<()> {
    if steps.len() > 8 {
        bail!("reset sequence has at most 8 steps");
    }
    for (index, step) in steps.iter().enumerate() {
        if !step.starts_with('/') {
            bail!("reset step {} must start with '/'", index + 1);
        }
        if step.len() > 200 {
            bail!("reset step {} is at most 200 bytes", index + 1);
        }
        if step.chars().any(char::is_control) {
            bail!(
                "reset step {} must not contain control characters",
                index + 1
            );
        }
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

pub fn parse_interval(value: &str) -> anyhow::Result<Duration> {
    let invalid = || {
        anyhow::anyhow!(
            "invalid heartbeat {value:?}: expected a number followed by s, m or h, from 1s to 24h"
        )
    };
    let (number, multiplier) = if let Some(number) = value.strip_suffix('s') {
        (number, 1)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60)
    } else if let Some(number) = value.strip_suffix('h') {
        (number, 60 * 60)
    } else {
        return Err(invalid());
    };
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    let number = number.parse::<u64>().map_err(|_| invalid())?;
    let seconds = number.checked_mul(multiplier).ok_or_else(invalid)?;
    if !(1..=HEARTBEAT_MAX.as_secs()).contains(&seconds) {
        return Err(invalid());
    }
    Ok(Duration::from_secs(seconds))
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

pub fn unwrap_channel(prompt: &str) -> Option<String> {
    let inner = prompt
        .strip_prefix("<channel source=\"a2amx\">\n")?
        .strip_suffix("\n</channel>")?;
    (!inner.is_empty()).then(|| inner.to_owned())
}

pub fn channel_view(envelope: &str) -> String {
    // shortcut: only the lowercase closing tag was probed on Claude Code 2.1.288;
    // other wrapper forms are unprobed, and a mismatch merely means no receipt.
    envelope.replace("</channel>", "<\\/channel>")
}

/// Replace every complete paste wrapper with its inner text.
pub fn unwrap_pastes(prompt: &str) -> String {
    // shortcut: wrapper shapes were observed on Claude Code 2.1.285/2.1.286 (idle submits keep
    // the surrounding blank line and newline; queued mid-turn submits have the prompt's ends
    // trimmed); update this matcher if a supported harness version changes its paste format.
    const OPEN: &str = "<pasted_content id=\"";
    let mut output = String::with_capacity(prompt.len());
    let mut cursor = 0;
    let mut search = 0;
    while let Some(offset) = prompt[search..].find(OPEN) {
        let tag = search + offset;
        search = tag + 1;
        let start = if tag == 0 {
            0
        } else if prompt[..tag].ends_with("\n\n") {
            tag - 2
        } else {
            continue;
        };
        if start < cursor {
            continue;
        }
        let id_start = tag + OPEN.len();
        let id_len = prompt.as_bytes()[id_start..]
            .iter()
            .take_while(|byte| byte.is_ascii_alphanumeric())
            .count();
        let id_end = id_start + id_len;
        if id_len == 0 || !prompt[id_end..].starts_with("\">\n") {
            continue;
        }
        let inner_start = id_end + 3;
        let closing = format!("\n</pasted_content id=\"{}\">", &prompt[id_start..id_end]);
        let Some(end) = prompt[inner_start..].find(&closing) else {
            continue;
        };
        let inner_end = inner_start + end;
        let mut after = inner_end + closing.len();
        if prompt[after..].starts_with('\n') {
            after += 1;
        } else if after != prompt.len() {
            continue;
        }
        output.push_str(&prompt[cursor..start]);
        output.push_str(&prompt[inner_start..inner_end]);
        cursor = after;
        search = after;
    }
    output.push_str(&prompt[cursor..]);
    output
}

/// Message sequence numbers of envelope tags, in first-appearance order.
pub fn envelope_ids(text: &str) -> Vec<i64> {
    const PREFIX: &str = "<a2amx-message id=\"m_";
    let mut ids = Vec::new();
    for (start, _) in text.match_indices(PREFIX) {
        let digits = &text[start + PREFIX.len()..];
        let count = digits.bytes().take_while(u8::is_ascii_digit).count();
        if (1..=18).contains(&count) && digits.as_bytes().get(count) == Some(&b'"') {
            if let Ok(seq) = digits[..count].parse::<i64>() {
                if seq > 0 && !ids.contains(&seq) {
                    ids.push(seq);
                }
            }
        }
    }
    ids
}

/// Drop control characters other than newlines and tabs, without truncating.
pub fn sanitize_draft(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect()
}

pub fn sgr_mouse_report_len(bytes: &[u8], start: usize) -> Option<usize> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_interval_accepts_units_and_rejects_invalid_ranges() {
        for (value, seconds) in [("90s", 90), ("30m", 1800), ("2h", 7200), ("24h", 86400)] {
            assert_eq!(parse_interval(value).unwrap(), Duration::from_secs(seconds));
        }
        for value in [
            "0s",
            "25h",
            "30",
            "m",
            "1d",
            "-5m",
            "+1s",
            "1.5s",
            " 1s",
            "18446744073709551615h",
            "18446744073709551616s",
        ] {
            assert_eq!(
                parse_interval(value).unwrap_err().to_string(),
                format!(
                    "invalid heartbeat {value:?}: expected a number followed by s, m or h, from 1s to 24h"
                )
            );
        }
    }
}
