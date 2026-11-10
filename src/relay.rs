use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow, bail};
use tokio::time::{self, MissedTickBehavior};

use crate::client::Client;
use crate::daemon::random_hex;
use crate::discord::{Discord, DiscordConfig, Watch};
use crate::gateway::{Event, Gateway, Next, UNREACHABLE_LINE};
use crate::relay_files::{self, Loaded, Pairing};
use crate::wire::GatewayAgent;

pub struct Io {
    pub stop: Pin<Box<dyn Future<Output = ()> + Send>>,
    pub pair: Option<PairIo>,
}

pub struct PairIo {
    pub announce: Box<dyn Fn(&str) + Send>,
    pub confirm: Box<dyn FnMut(&str) -> bool + Send>,
}
pub fn load(home: &std::path::Path) -> anyhow::Result<Loaded> {
    relay_files::load(home)
}

pub async fn run(loaded: Loaded, discord_config: DiscordConfig, mut io: Io) -> anyhow::Result<()> {
    let daemon = Client::connect_gateway(&loaded.link_dir).await?;
    let discord = Discord::new(discord_config);
    let members = discord.member_count().await?;
    if members > loaded.config.max_members {
        bail!(
            "server has {} members, the limit is {}",
            members,
            loaded.config.max_members
        );
    }

    let control_category = discord
        .ensure_category(&loaded.config.control_category)
        .await
        .context("ensure control category")?;
    let control_channel = discord
        .ensure_text_channel(&control_category, &loaded.config.control_channel)
        .await
        .context("ensure control channel")?;
    let mut watch = discord.watch();
    let paired_author = if let Some(mut pair_io) = io.pair.take() {
        pair(&loaded, &control_channel, &mut watch, &mut pair_io)
            .await?
            .user_id
    } else {
        loaded
            .pairing
            .clone()
            .ok_or_else(|| anyhow!("not paired; run with --pair"))?
            .user_id
    };

    let state_path = loaded.gateway_dir.join("seen.json");
    let mut relay = Relay {
        link_dir: loaded.link_dir.clone(),
        paired_author: paired_author.clone(),
        daemon,
        discord,
        gateway: Gateway::new(state_path, paired_author, control_channel)?,
        config: loaded.config,
        category_ids: HashMap::new(),
        channel_ids: HashMap::new(),
        paused: false,
        broken: false,
        next_reconnect: Instant::now(),
    };
    if let Err(error) = relay.sync().await {
        // Section 2.5: a daemon outage during startup leaves the relay alive and broken,
        // rebuilt by the reconnect path at the next tick; only the local pre-daemon
        // setup above ends the run.
        tracing::warn!(%error, "initial gateway sync failed");
        relay.mark_broken();
    }

    let mut sync_tick = time::interval(Duration::from_millis(relay.config.sync_ms));
    let mut idle_tick = time::interval(Duration::from_millis(relay.config.idle_ms));
    let mut member_tick = time::interval(Duration::from_millis(relay.config.member_check_ms));
    sync_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    idle_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);
    member_tick.set_missed_tick_behavior(MissedTickBehavior::Delay);

    enum Wake {
        Event(Result<Event, crate::discord::Fatal>),
        Sync,
        Idle,
        Member,
        Stop,
    }

    loop {
        let wake = tokio::select! {
            event = watch.recv() => Wake::Event(event),
            _ = sync_tick.tick() => Wake::Sync,
            _ = idle_tick.tick() => Wake::Idle,
            _ = member_tick.tick() => Wake::Member,
            _ = io.stop.as_mut() => Wake::Stop,
        };
        match wake {
            Wake::Stop => return Ok(()),
            Wake::Event(result) => match result {
                Ok(event) => relay.event(event).await,
                Err(error) => bail!("Discord gateway stopped: {error}"),
            },
            Wake::Sync => {
                relay.reconnect_if_due().await;
                if !relay.paused && !relay.broken {
                    // A failed daemon call marks the connection broken; the loop
                    // keeps running and reconnects at the next tick.
                    if let Err(error) = relay.sync().await {
                        relay.mark_broken();
                        tracing::warn!(%error, "gw sync failed; marking daemon broken");
                    }
                }
            }
            Wake::Idle => {
                relay.reconnect_if_due().await;
                if !relay.paused
                    && !relay.broken
                    && let Err(error) = relay.pump().await
                {
                    relay.mark_broken();
                    tracing::warn!(%error, "gw pump failed; marking daemon broken");
                }
            }
            Wake::Member => relay.member_check().await,
        }
    }
}

struct Relay {
    link_dir: PathBuf,
    paired_author: String,
    daemon: Client,
    discord: Discord,
    gateway: Gateway,
    config: relay_files::GatewayConfig,
    category_ids: HashMap<String, String>,
    channel_ids: HashMap<String, String>,
    paused: bool,
    broken: bool,
    next_reconnect: Instant,
}

impl Relay {
    fn mark_broken(&mut self) {
        self.broken = true;
        self.next_reconnect = Instant::now() + Duration::from_millis(self.config.reconnect_ms);
    }

    async fn reconnect_if_due(&mut self) {
        if !self.broken || Instant::now() < self.next_reconnect {
            return;
        }
        match Client::connect_gateway(&self.link_dir).await {
            Ok(client) => {
                self.daemon = client;
                self.broken = false;
            }
            Err(error) => {
                tracing::warn!(%error, "a2amx daemon reconnect failed");
                self.next_reconnect =
                    Instant::now() + Duration::from_millis(self.config.reconnect_ms);
            }
        }
    }

    async fn event(&mut self, event: Event) {
        if self.paused {
            return;
        }
        if self.broken {
            if event.author == self.paired_author {
                self.post_line(&event.channel, UNREACHABLE_LINE).await;
            }
            return;
        }
        match self.gateway.inbound(&mut self.daemon, event.clone()).await {
            Ok(Some(line)) => self.post_line(&event.channel, &line).await,
            Ok(None) => {}
            Err(error) => {
                self.mark_broken();
                // Defense-in-depth; Gateway::inbound applies the same author check first today.
                if event.author == self.paired_author {
                    self.post_line(&event.channel, UNREACHABLE_LINE).await;
                }
                tracing::warn!(%error, "daemon inbound request failed");
            }
        }
    }

    async fn post_line(&self, channel: &str, line: &str) {
        if let Err(error) = self.discord.send(channel, line).await {
            tracing::warn!(%error, "Discord reply failed");
        }
    }

    async fn sync(&mut self) -> anyhow::Result<()> {
        let agents = self.gateway.agents(&mut self.daemon).await?;
        let mut listed = HashSet::new();
        for agent in &agents {
            listed.insert(agent.address.clone());
            let category = self.ensure_category(&agent.team).await?;
            let channel = self.ensure_channel(agent, &category).await?;
            self.gateway.bind_channel(channel, agent.address.clone())?;
        }
        let stale: Vec<_> = self
            .channel_ids
            .iter()
            .filter(|(address, _)| !listed.contains(*address))
            .map(|(_, channel)| channel.clone())
            .collect();
        for channel in stale {
            self.gateway.unbind_channel(&channel);
        }
        self.channel_ids
            .retain(|address, _| listed.contains(address));
        Ok(())
    }

    async fn ensure_category(&mut self, team: &str) -> anyhow::Result<String> {
        if let Some(id) = self.category_ids.get(team) {
            return Ok(id.clone());
        }
        let id = self.discord.ensure_category(team).await?;
        self.category_ids.insert(team.to_owned(), id.clone());
        Ok(id)
    }

    async fn ensure_channel(
        &mut self,
        agent: &GatewayAgent,
        category: &str,
    ) -> anyhow::Result<String> {
        if let Some(id) = self.channel_ids.get(&agent.address) {
            return Ok(id.clone());
        }
        let id = self
            .discord
            .ensure_text_channel(category, &agent.name)
            .await?;
        self.channel_ids.insert(agent.address.clone(), id.clone());
        Ok(id)
    }

    async fn pump(&mut self) -> anyhow::Result<()> {
        loop {
            match self.gateway.outbound(&mut self.daemon).await? {
                Next::Idle => return Ok(()),
                Next::Send(outbound) => {
                    let sent = self.send_pieces(&outbound.channel, &outbound.pieces).await;
                    self.gateway
                        .outbound_done(&mut self.daemon, &outbound.id, sent)
                        .await?;
                }
                Next::NeedChannel { id, agent } => {
                    let result = async {
                        let category = self.ensure_category(&agent.team).await?;
                        let channel = self.ensure_channel(&agent, &category).await?;
                        self.gateway.bind_channel(channel, agent.address.clone())?;
                        Ok::<(), anyhow::Error>(())
                    }
                    .await;
                    if result.is_err() {
                        self.gateway
                            .outbound_done(&mut self.daemon, &id, false)
                            .await?;
                    }
                }
            }
        }
    }

    async fn send_pieces(&self, channel: &str, pieces: &[String]) -> bool {
        for piece in pieces {
            if let Err(error) = self.discord.send(channel, piece).await {
                tracing::warn!(%error, "Discord outbound post failed");
                return false;
            }
        }
        true
    }

    async fn member_check(&mut self) {
        let Ok(count) = self.discord.member_count().await else {
            return;
        };
        if count > self.config.max_members {
            if !self.paused {
                eprintln!(
                    "server has {} members, the limit is {}; pausing",
                    count, self.config.max_members
                );
            }
            self.paused = true;
        } else {
            self.paused = false;
        }
    }
}

async fn pair(
    loaded: &Loaded,
    control_channel: &str,
    watch: &mut Watch,
    io: &mut PairIo,
) -> anyhow::Result<Pairing> {
    if let Some(old) = &loaded.pairing {
        (io.announce)(&format!(
            "warning: this gateway is already paired with Discord account {} in server {}; confirming a new pairing replaces it, and the old account can no longer message your agents",
            old.user_id, old.guild_id
        ));
    }
    let code = random_hex::<6>()?;
    (io.announce)(&format!("post this code in the control channel: {code}"));
    let event = time::timeout(Duration::from_secs(10 * 60), async {
        loop {
            match watch.recv().await {
                Ok(event) if event.channel == control_channel && event.text.trim() == code => {
                    break Ok(event);
                }
                Ok(_) => continue,
                Err(error) => break Err(anyhow!("Discord gateway stopped: {error}")),
            }
        }
    })
    .await
    .map_err(|_| anyhow!("pairing timed out"))??;
    if !relay_files::valid_id(&event.author) {
        bail!("pairing refused: the message author is not a Discord user ID");
    }
    let prompt = match &loaded.pairing {
        Some(old) => format!(
            "replace Discord account {} with {}? [y/N]",
            old.user_id, event.author
        ),
        None => format!("bind Discord account {}? [y/N]", event.author),
    };
    let mut confirm = std::mem::replace(&mut io.confirm, Box::new(|_| false));
    let accepted = tokio::task::spawn_blocking(move || confirm(&prompt)).await?;
    if !accepted {
        bail!("pairing not confirmed");
    }
    let pairing = Pairing {
        guild_id: loaded.config.guild_id.clone(),
        user_id: event.author,
    };
    relay_files::write_pairing(&loaded.home, &pairing)?;
    (io.announce)(&format!(
        "paired with Discord account {}; starting the gateway (Ctrl-C stops it)",
        pairing.user_id
    ));
    Ok(pairing)
}
