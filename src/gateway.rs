use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::client::Client;
use crate::messaging::{self, code};
use crate::wire::{GatewayAgent, HumanOutboundMessage, Request, Response};

pub const UNREACHABLE_LINE: &str = "a2amx is not reachable right now";
pub const MAX_CHAT_CHARS: usize = 2000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub channel: String,
    pub message_id: u64,
    pub author: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outbound {
    pub id: String,
    pub channel: String,
    pub pieces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Next {
    Idle,
    Send(Outbound),
    NeedChannel { id: String, agent: GatewayAgent },
}

pub struct Gateway {
    state_path: PathBuf,
    paired_author: String,
    control_channel: String,
    channels: HashMap<String, String>,
    seen: BTreeMap<String, u64>,
    held: Option<HumanOutboundMessage>,
}

impl Gateway {
    pub fn new(
        state_path: PathBuf,
        paired_author: String,
        control_channel: String,
    ) -> anyhow::Result<Self> {
        let seen = read_seen(&state_path)?;
        Ok(Self {
            state_path,
            paired_author,
            control_channel,
            channels: HashMap::new(),
            seen,
            held: None,
        })
    }

    pub async fn agents(&self, daemon: &mut Client) -> anyhow::Result<Vec<GatewayAgent>> {
        match daemon.request(Request::GatewayAgents).await? {
            Response::GatewayAgents { agents } => Ok(agents),
            Response::Error { message } => bail!(message),
            response => bail!("unexpected GatewayAgents response: {response:?}"),
        }
    }

    pub fn bind_channel(&mut self, channel: String, address: String) -> anyhow::Result<()> {
        if channel == self.control_channel {
            bail!("the control channel cannot be bound to an agent")
        }
        self.channels.insert(channel, address);
        Ok(())
    }

    pub fn unbind_channel(&mut self, channel: &str) {
        self.channels.remove(channel);
    }

    pub fn channel_for(&self, address: &str) -> Option<&str> {
        self.channels
            .iter()
            .find_map(|(channel, target)| (target == address).then_some(channel.as_str()))
    }

    pub async fn inbound(
        &mut self,
        daemon: &mut Client,
        event: Event,
    ) -> anyhow::Result<Option<String>> {
        if event.author != self.paired_author
            || !self.record_id(&event.channel, event.message_id).await?
        {
            return Ok(None);
        }
        if event.channel == self.control_channel {
            return self.control_line(daemon, &event.text).await;
        }
        if let Some(address) = self.channels.get(&event.channel).cloned() {
            return self.deliver_line(daemon, &address, &event.text).await;
        }
        Ok(Some("that channel is not linked to a running agent".into()))
    }

    pub async fn outbound(&mut self, daemon: &mut Client) -> anyhow::Result<Next> {
        loop {
            if self.held.is_none() {
                let response = daemon.request(Request::HumanOutboundNext).await?;
                let message = match response {
                    Response::HumanOutbound {
                        message: Some(message),
                    } => message,
                    Response::HumanOutbound { message: None } => return Ok(Next::Idle),
                    response => {
                        bail!("unexpected HumanOutboundNext response: {response:?}");
                    }
                };
                self.held = Some(message);
            }
            if let Some(message) = self.held.as_ref() {
                if let Some(channel) = self.channel_for(&message.from) {
                    return Ok(Next::Send(render_outbound(message, channel)));
                }
                let agents = self.agents(daemon).await?;
                if let Some(agent) = agents
                    .into_iter()
                    .find(|agent| agent.address == message.from)
                {
                    return Ok(Next::NeedChannel {
                        id: message.id.clone(),
                        agent,
                    });
                }
                let id = message.id.clone();
                report_done(daemon, &id, false).await?;
                self.held = None;
            }
        }
    }

    pub async fn outbound_done(
        &mut self,
        daemon: &mut Client,
        id: &str,
        sent: bool,
    ) -> anyhow::Result<()> {
        let response = daemon
            .request(Request::HumanOutboundDone {
                id: id.to_owned(),
                sent,
            })
            .await?;
        match response {
            Response::Ok => {
                if self.held.as_ref().is_some_and(|message| message.id == id) {
                    self.held = None;
                }
                Ok(())
            }
            Response::Error { message } => bail!(message),
            response => bail!("unexpected HumanOutboundDone response: {response:?}"),
        }
    }

    async fn record_id(&mut self, channel: &str, message_id: u64) -> anyhow::Result<bool> {
        if self
            .seen
            .get(channel)
            .is_some_and(|last| message_id <= *last)
        {
            return Ok(false);
        }
        let mut updated = self.seen.clone();
        updated.insert(channel.to_owned(), message_id);
        persist_seen(self.state_path.clone(), updated.clone()).await?;
        self.seen = updated;
        Ok(true)
    }

    async fn control_line(
        &self,
        daemon: &mut Client,
        text: &str,
    ) -> anyhow::Result<Option<String>> {
        let code = text.trim();
        if code.len() != 12
            || !code
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Ok(Some("send the code that a2amx team up printed".into()));
        }
        let response = daemon
            .request(Request::RedeemLeaseCode {
                code: code.to_owned(),
            })
            .await?;
        Ok(Some(
            match response {
                Response::Ok => "remote messaging is on",
                _ => "that code is not valid",
            }
            .into(),
        ))
    }

    async fn deliver_line(
        &self,
        daemon: &mut Client,
        address: &str,
        text: &str,
    ) -> anyhow::Result<Option<String>> {
        let response = daemon
            .request(Request::HumanInbound {
                to: address.to_owned(),
                body: text.to_owned(),
            })
            .await?;
        Ok(match response {
            Response::Accepted { .. } => None,
            Response::Failed { code, .. } => Some(refusal_line(&code).into()),
            _ => Some("the message could not be delivered".into()),
        })
    }
}

fn render_outbound(message: &HumanOutboundMessage, channel: &str) -> Outbound {
    let text = if message.subject.is_empty() || message.subject == messaging::HUMAN_SUBJECT {
        message.body.clone()
    } else {
        format!("**{}**\n{}", message.subject, message.body)
    };
    Outbound {
        id: message.id.clone(),
        channel: channel.to_owned(),
        pieces: split_message(&text, MAX_CHAT_CHARS),
    }
}

async fn report_done(daemon: &mut Client, id: &str, sent: bool) -> anyhow::Result<()> {
    match daemon
        .request(Request::HumanOutboundDone {
            id: id.to_owned(),
            sent,
        })
        .await?
    {
        Response::Ok => Ok(()),
        Response::Error { message } => bail!(message),
        response => bail!("unexpected HumanOutboundDone response: {response:?}"),
    }
}

fn refusal_line(code: &str) -> &'static str {
    match code {
        code::NOT_PERMITTED => "that agent is not taking messages from you",
        code::RATE_LIMITED => "too many messages, wait a minute",
        code::TOO_LARGE => "that message is too long",
        code::UNKNOWN_RECIPIENT | code::RECIPIENT_EXITED => "that agent is not running",
        code::QUEUE_FULL => "that agent has too many unread messages",
        _ => "the message could not be delivered",
    }
}

pub fn split_message(text: &str, max_chars: usize) -> Vec<String> {
    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + max_chars).min(chars.len());
        let mut cut = end;
        if end < chars.len() {
            let window = &chars[start..end];
            if let Some(index) = window.iter().rposition(|character| *character == '\n') {
                if start + index > start {
                    cut = start + index;
                }
            } else if let Some(index) = window.iter().rposition(|character| *character == ' ')
                && start + index > start
            {
                cut = start + index;
            }
        }
        pieces.push(chars[start..cut].iter().collect());
        start = cut;
        while start < chars.len() && (chars[start] == '\n' || chars[start] == ' ') {
            start += 1;
        }
    }
    pieces
}

fn read_seen(path: &Path) -> anyhow::Result<BTreeMap<String, u64>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.into()),
    };
    if file.metadata()?.len() > 1024 * 1024 {
        bail!("gateway dedup file is too large")
    }
    let mut bytes = Vec::new();
    file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        bail!("gateway dedup file is too large")
    }
    let raw: BTreeMap<String, String> =
        serde_json::from_slice(&bytes).context("invalid gateway dedup file")?;
    raw.into_iter()
        .map(|(channel, id)| {
            id.parse::<u64>()
                .map(|id| (channel, id))
                .context("invalid gateway dedup id")
        })
        .collect()
}

async fn persist_seen(path: PathBuf, seen: BTreeMap<String, u64>) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || write_seen(&path, &seen))
        .await
        .context("gateway dedup writer task failed")??;
    Ok(())
}

fn write_seen(path: &Path, seen: &BTreeMap<String, u64>) -> anyhow::Result<()> {
    let temp = path.with_extension("tmp");
    let result = (|| {
        let values: BTreeMap<&str, String> = seen
            .iter()
            .map(|(channel, id)| (channel.as_str(), id.to_string()))
            .collect();
        let payload = serde_json::to_vec(&values)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temp)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&payload)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)?;
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        File::open(parent)?.sync_all()?;
        Ok::<(), anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
