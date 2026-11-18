#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use a2amx::discord::{ChannelInfo, Discord, DiscordConfig, DiscordError, Fatal};
use a2amx::gateway::{Event, MAX_CHAT_CHARS};
use common::fake_discord::{
    HttpReply, discord_for_http, discord_for_ws, hello, http_fake, next_close_code, next_json,
    send_json, ws_fake,
};
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::timeout;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, protocol::CloseFrame};

mod config;
mod gateway_queue;
mod gateway_reconnect;
mod gateway_session;
mod rest;
