#![allow(clippy::expect_used, clippy::unwrap_used)]

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

#[test]
fn defaults_use_discord_gateway_and_api() {
    let config = DiscordConfig::new("token".into(), "123".into());
    assert_eq!(config.api_base, "https://discord.com/api/v10");
    assert_eq!(config.gateway_url, "wss://gateway.discord.gg");
    assert_eq!(config.reconnect_base, Duration::from_secs(1));
    assert_eq!(config.reconnect_max, Duration::from_secs(30));
    assert_eq!(config.queue, 1024);
}

#[test]
fn debug_and_errors_redact_token_and_fixed_text() {
    let token = "super-secret-token";
    let config = DiscordConfig::new(token.into(), "123".into());
    let debug = format!("{config:?}");
    assert!(!debug.contains(token));
    assert!(debug.contains("<redacted>"));
    let error = DiscordError::Status(401);
    assert_eq!(error.to_string(), "Discord HTTP status 401");
    assert!(!error.to_string().contains(token));
}

#[test]
fn rustls_gateway_config_is_constructed_without_network() {
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = "ws://127.0.0.1:9".into();
    config.reconnect_base = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async move {
        let discord = Discord::new(config);
        let mut watch = discord.watch();
        let result = timeout(Duration::from_millis(100), watch.recv()).await;
        assert!(result.is_err());
    });
}

#[tokio::test]
async fn rest_finds_and_creates_channels_and_parses_lists() {
    let fake = http_fake(vec![
        HttpReply::json(
            200,
            json!([
                {"id":"4","type":4,"name":"category","parent_id":null},
                {"id":"5","type":0,"name":"agent","parent_id":"4"}
            ]),
        ),
        HttpReply::json(
            200,
            json!([
                {"id":"4","type":4,"name":"category","parent_id":null},
                {"id":"5","type":0,"name":"agent","parent_id":"4"}
            ]),
        ),
        HttpReply::json(
            200,
            json!([
                {"id":"6","type":4,"name":"other","parent_id":null},
                {"id":"7","type":0,"name":"other","parent_id":"6"}
            ]),
        ),
        HttpReply::json(200, json!({"id":"8"})),
        HttpReply::json(
            200,
            json!([
                {"id":"6","type":4,"name":"other","parent_id":null},
                {"id":"7","type":0,"name":"other","parent_id":"6"}
            ]),
        ),
        HttpReply::json(200, json!({"id":"9"})),
        HttpReply::json(
            200,
            json!([
                {"id":"4","type":4,"name":"category","parent_id":null},
                {"id":"5","type":0,"name":"agent","parent_id":"4"}
            ]),
        ),
        HttpReply::json(200, json!({"approximate_member_count":42})),
    ]);
    let discord = discord_for_http(&fake);
    assert_eq!(discord.ensure_category("category").await.unwrap(), "4");
    assert_eq!(
        discord.ensure_text_channel("4", "agent").await.unwrap(),
        "5"
    );
    assert_eq!(discord.ensure_category("missing").await.unwrap(), "8");
    assert_eq!(
        discord.ensure_text_channel("6", "new-agent").await.unwrap(),
        "9"
    );
    let channels = discord.list_channels().await.unwrap();
    assert_eq!(channels.len(), 2);
    assert_eq!(
        channels[0],
        ChannelInfo {
            id: "4".into(),
            kind: 4,
            name: "category".into(),
            parent_id: None
        }
    );
    assert_eq!(discord.member_count().await.unwrap(), 42);
    let requests = fake.requests.lock().unwrap().clone();
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].path, "/guilds/123/channels");
    assert_eq!(requests[3].method, "POST");
    assert_eq!(
        serde_json::from_str::<Value>(&requests[3].body).unwrap(),
        json!({"name":"missing","type":4})
    );
    assert_eq!(
        serde_json::from_str::<Value>(&requests[5].body).unwrap(),
        json!({"name":"new-agent","type":0,"parent_id":"6"})
    );
}

#[tokio::test]
async fn rest_send_has_headers_mentions_and_fixed_limits() {
    let fake = http_fake(vec![
        HttpReply::json(200, json!({"id":"10"})),
        HttpReply::json(200, json!({"id":"11"})),
    ]);
    let discord = discord_for_http(&fake);
    discord.send("10", "hello @everyone").await.unwrap();
    discord
        .send("10", &"x".repeat(MAX_CHAT_CHARS))
        .await
        .unwrap();
    let request = fake.requests.lock().unwrap()[0].clone();
    assert_eq!(request.path, "/channels/10/messages");
    assert_eq!(
        request.headers.get("authorization").unwrap(),
        "Bot test-token"
    );
    assert!(
        request
            .headers
            .get("user-agent")
            .unwrap()
            .starts_with("DiscordBot (https://github.com/zbalint/a2amx, ")
    );
    assert_eq!(
        request.headers.get("content-type").unwrap(),
        "application/json"
    );
    assert_eq!(
        serde_json::from_str::<Value>(&request.body).unwrap(),
        json!({"content":"hello @everyone","allowed_mentions":{"parse":[]}})
    );
    let requests = fake.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    let exact = serde_json::from_str::<Value>(&requests[1].body).unwrap();
    assert_eq!(
        exact["content"].as_str().unwrap().chars().count(),
        MAX_CHAT_CHARS
    );
}

#[tokio::test]
async fn rest_rejects_path_injection_and_oversized_text_without_requests() {
    let fake = http_fake(Vec::new());
    let discord = discord_for_http(&fake);
    assert_eq!(
        discord.send("1/../2", "hello").await,
        Err(DiscordError::Invalid("channel"))
    );
    assert_eq!(
        discord.ensure_text_channel("x y", "agent").await,
        Err(DiscordError::Invalid("channel"))
    );
    assert_eq!(
        discord.ensure_text_channel("1", "Agent").await,
        Err(DiscordError::Invalid("channel"))
    );
    assert_eq!(
        discord.ensure_text_channel("1", "bad_name").await,
        Err(DiscordError::Invalid("channel"))
    );
    assert_eq!(
        discord.send("1", "").await,
        Err(DiscordError::Invalid("text"))
    );
    assert_eq!(
        discord.send("1", &"x".repeat(MAX_CHAT_CHARS + 1)).await,
        Err(DiscordError::Invalid("text"))
    );
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rest_rejects_invalid_guild_without_requests() {
    let fake = http_fake(Vec::new());
    let mut config = DiscordConfig::new("token".into(), "guild/escape".into());
    config.api_base = fake.base.clone();
    let discord = Discord::new(config);
    assert_eq!(
        discord.ensure_category("category").await,
        Err(DiscordError::Invalid("guild"))
    );
    assert_eq!(
        discord.ensure_text_channel("1", "agent").await,
        Err(DiscordError::Invalid("guild"))
    );
    assert_eq!(
        discord.list_channels().await,
        Err(DiscordError::Invalid("guild"))
    );
    assert_eq!(
        discord.member_count().await,
        Err(DiscordError::Invalid("guild"))
    );
    assert!(fake.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn rest_status_error_does_not_echo_token_or_response_body() {
    let fake = http_fake(vec![HttpReply::text(401, "body contains test-token")]);
    let mut config = DiscordConfig::new("test-token".into(), "123".into());
    config.api_base = fake.base.clone();
    let discord = Discord::new(config);
    let error = discord.send("1", "text").await.unwrap_err();
    assert_eq!(error, DiscordError::Status(401));
    let text = error.to_string();
    assert!(!text.contains("test-token"));
    assert!(!text.contains("body contains"));
}

#[tokio::test]
async fn rest_accepts_digit_leading_channel_names_and_skips_odd_entries() {
    let fake = http_fake(vec![HttpReply::json(
        200,
        json!([
            {"id":"1","type":0,"name":"0agent","parent_id":"2"},
            {"id":"bad","type":"0","name":"ignored"},
            {"id":"3","type":0,"name":"other","parent_id":7}
        ]),
    )]);
    let discord = discord_for_http(&fake);
    assert_eq!(
        discord.ensure_text_channel("2", "0agent").await.unwrap(),
        "1"
    );
    assert_eq!(fake.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn rest_retries_rate_limits_with_body_value_and_cap_and_stops_at_three() {
    let fake = http_fake(vec![
        HttpReply::json(429, json!({"retry_after":0.05,"global":false})),
        HttpReply::json(429, json!({"retry_after":10.0,"global":false})),
        HttpReply::json(429, json!({"retry_after":1.0,"global":false})),
        HttpReply::json(429, json!({"retry_after":0.001,"global":false})),
    ]);
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.api_base = fake.base.clone();
    config.max_retry_wait = Duration::from_millis(50);
    let discord = Discord::new(config);
    let started = Instant::now();
    assert_eq!(
        discord.send("1", "text").await,
        Err(DiscordError::Status(429))
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(fake.requests.lock().unwrap().len(), 4);
}

#[tokio::test]
async fn rest_429_huge_retry_value_is_bounded_without_panic() {
    let fake = http_fake(vec![
        HttpReply::json(429, json!({"retry_after":1e300})),
        HttpReply::json(200, json!({"id":"1"})),
    ]);
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.api_base = fake.base.clone();
    config.max_retry_wait = Duration::from_millis(2);
    let discord = Discord::new(config);
    let started = Instant::now();
    discord.send("1", "text").await.unwrap();
    assert!(started.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn rest_timeout_and_response_limit_are_errors() {
    let silent = http_fake(vec![HttpReply {
        status: 200,
        body: String::new(),
        headers: Vec::new(),
        silent: true,
    }]);
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.api_base = silent.base.clone();
    config.request_timeout = Duration::from_millis(100);
    let discord = Discord::new(config);
    let started = Instant::now();
    assert_eq!(
        discord.send("1", "text").await,
        Err(DiscordError::Transport("timeout"))
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    drop(silent);

    let huge = format!(
        "[{{\"id\":\"1\",\"type\":0,\"name\":\"agent\"}}]{}",
        " ".repeat(2 * 1024 * 1024)
    );
    let fake = http_fake(vec![HttpReply::text(200, huge)]);
    let discord = discord_for_http(&fake);
    assert_eq!(
        discord.list_channels().await,
        Err(DiscordError::Transport("response too large"))
    );
}

#[tokio::test]
async fn gateway_reconnect_backoff_is_capped() {
    let times = Arc::new(Mutex::new(Vec::new()));
    let time_slot = Arc::clone(&times);
    let fake = ws_fake(3, move |index, mut socket| {
        let time_slot = Arc::clone(&time_slot);
        async move {
            hello(&mut socket, 100).await;
            let _ = next_json(&mut socket).await.unwrap();
            time_slot.lock().unwrap().push(Instant::now());
            let code = if index < 2 { 1000 } else { 4004 };
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                        code,
                    ),
                    reason: "test".into(),
                })))
                .await;
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(50);
    config.reconnect_max = Duration::from_millis(70);
    config.identify_gap = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(2), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    let times = times.lock().unwrap();
    let first_gap = times[1].duration_since(times[0]);
    let second_gap = times[2].duration_since(times[1]);
    assert!(first_gap >= Duration::from_millis(45));
    assert!(second_gap >= Duration::from_millis(60));
    assert!(second_gap < first_gap + Duration::from_millis(35));
}

#[tokio::test]
async fn gateway_silent_hello_times_out_and_reconnects() {
    let second_op = Arc::new(Mutex::new(None));
    let second_slot = Arc::clone(&second_op);
    let fake = ws_fake(2, move |index, mut socket| {
        let second_slot = Arc::clone(&second_slot);
        async move {
            if index == 0 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                return;
            }
            hello(&mut socket, 100).await;
            *second_slot.lock().unwrap() = next_json(&mut socket)
                .await
                .map(|value| value["op"].clone());
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                        4004_u16,
                    ),
                    reason: "auth".into(),
                })))
                .await;
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.request_timeout = Duration::from_millis(50);
    config.reconnect_base = Duration::from_millis(1);
    config.identify_gap = Duration::from_millis(1);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    assert_eq!(*second_op.lock().unwrap(), Some(json!(2)));
}

#[tokio::test]
async fn gateway_identifies_filters_guilds_and_heartbeats_sequence() {
    let seen_heartbeat = Arc::new(Mutex::new(Vec::new()));
    let heartbeat_slot = Arc::clone(&seen_heartbeat);
    let fake = ws_fake(1, move |_index, mut socket| {
        let heartbeat_slot = Arc::clone(&heartbeat_slot);
        async move {
            hello(&mut socket, 100).await;
            let identify = next_json(&mut socket).await.unwrap();
            assert_eq!(identify["op"], 2);
            assert_eq!(identify["d"]["token"], "bot-token");
            assert_eq!(identify["d"]["intents"], 33281);
            assert_eq!(identify["d"]["properties"], json!({"os":"linux","browser":"a2amx","device":"a2amx"}));
            let first = next_json(&mut socket).await.unwrap();
            assert_eq!(first["op"], 1);
            assert_eq!(first["d"], Value::Null);
            heartbeat_slot.lock().unwrap().push(first["d"].clone());
            send_json(&mut socket, json!({"op":11,"d":null})).await;
            send_json(&mut socket, json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}})).await;
            send_json(&mut socket, json!({"op":0,"s":2,"t":"MESSAGE_CREATE","d":{"id":"1","channel_id":"1","author":{"id":"a"},"content":"dm"}})).await;
            send_json(&mut socket, json!({"op":0,"s":3,"t":"MESSAGE_CREATE","d":{"guild_id":"other","id":"2","channel_id":"2","author":{"id":"a"},"content":"other"}})).await;
            send_json(&mut socket, json!({"op":0,"s":4,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"not-numeric","channel_id":"9","author":{"id":"a"},"content":"bad id"}})).await;
            send_json(&mut socket, json!({"op":0,"s":5,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"42","channel_id":"9","author":{"id":"a"},"content":"right"}})).await;
            loop {
                let Some(value) = next_json(&mut socket).await else { return; };
                if value["op"] == 1 {
                    assert_eq!(value["d"], 5);
                    heartbeat_slot.lock().unwrap().push(value["d"].clone());
                    send_json(&mut socket, json!({"op":11,"d":null})).await;
                    let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
                    return;
                }
            }
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let mut watch = discord.watch();
    let event = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event,
        Event {
            channel: "9".into(),
            message_id: 42,
            author: "a".into(),
            text: "right".into()
        }
    );
    let fatal = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    assert_eq!(watch.recv().await.unwrap_err(), fatal);
    let heartbeats = seen_heartbeat.lock().unwrap();
    assert_eq!(heartbeats.len(), 2);
    assert_eq!(heartbeats.last(), Some(&json!(5)));
}

#[tokio::test]
async fn gateway_resume_uses_session_and_last_delivered_sequence() {
    let resume = Arc::new(Mutex::new(None));
    let resume_slot = Arc::clone(&resume);
    let fake = ws_fake(2, move |index, mut socket| {
        let resume_slot = Arc::clone(&resume_slot);
        async move {
            hello(&mut socket, 100).await;
            let identify = next_json(&mut socket).await.unwrap();
            if index == 0 {
                assert_eq!(identify["op"], 2);
                send_json(&mut socket, json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}})).await;
                send_json(&mut socket, json!({"op":0,"s":2,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"7","channel_id":"1","author":{"id":"a"},"content":"one"}})).await;
                let _ = socket.send(Message::Close(None)).await;
            } else {
                *resume_slot.lock().unwrap() = Some(identify.clone());
                let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
            }
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let mut watch = discord.watch();
    let event = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.message_id, 7);
    let _ = timeout(Duration::from_secs(1), watch.recv()).await.unwrap();
    let resume = timeout(Duration::from_secs(1), async {
        loop {
            if let Some(value) = resume.lock().unwrap().clone() {
                break value;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resume["op"], 6);
    assert_eq!(resume["d"]["session_id"], "session");
    assert_eq!(resume["d"]["seq"], 2);
}

#[tokio::test]
async fn gateway_resume_skips_identify_gap_and_preserves_identify_timestamp() {
    let handshakes = Arc::new(Mutex::new(Vec::new()));
    let handshake_slot = Arc::clone(&handshakes);
    let fake = ws_fake(3, move |index, mut socket| {
        let handshake_slot = Arc::clone(&handshake_slot);
        async move {
            hello(&mut socket, 100).await;
            let value = next_json(&mut socket).await.unwrap();
            handshake_slot
                .lock()
                .unwrap()
                .push((value["op"].as_u64().unwrap(), Instant::now()));
            match index {
                0 => {
                    send_json(
                        &mut socket,
                        json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
                    )
                    .await;
                    let _ = socket.send(Message::Close(None)).await;
                }
                1 => {
                    send_json(&mut socket, json!({"op":9,"d":false})).await;
                }
                _ => {
                    let _ = socket
                        .send(Message::Close(Some(CloseFrame {
                            code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16),
                            reason: "auth".into(),
                        })))
                        .await;
                }
            }
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(300);
    config.identify_gap = Duration::from_millis(700);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(2), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    let handshakes = handshakes.lock().unwrap();
    assert_eq!(
        handshakes.iter().map(|(op, _)| *op).collect::<Vec<_>>(),
        vec![2, 6, 2]
    );
    let resume_gap = handshakes[1].1.duration_since(handshakes[0].1);
    assert!(resume_gap < Duration::from_millis(550));
    let identify_after_resume = handshakes[2].1.duration_since(handshakes[1].1);
    assert!(identify_after_resume >= Duration::from_millis(550));
    assert!(identify_after_resume < Duration::from_millis(680));
}

#[tokio::test]
async fn gateway_op7_reconnects_with_resume() {
    let ops = Arc::new(Mutex::new(Vec::new()));
    let op_slot = Arc::clone(&ops);
    let fake = ws_fake(2, move |index, mut socket| {
        let op_slot = Arc::clone(&op_slot);
        async move {
            hello(&mut socket, 100).await;
            let value = next_json(&mut socket).await.unwrap();
            op_slot.lock().unwrap().push(value["op"].as_u64().unwrap());
            if index == 0 {
                send_json(&mut socket, json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}})).await;
                send_json(&mut socket, json!({"op":7,"d":null})).await;
            } else {
                let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
            }
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let _watch = discord.watch();
    timeout(Duration::from_secs(1), async {
        loop {
            if ops.lock().unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*ops.lock().unwrap(), vec![2, 6]);
}

#[tokio::test]
async fn gateway_4007_and_4009_clear_session_and_grow_identify_gaps() {
    let identifies = Arc::new(Mutex::new(Vec::new()));
    let identify_times = Arc::clone(&identifies);
    let ops = Arc::new(Mutex::new(Vec::new()));
    let op_slot = Arc::clone(&ops);
    let fake = ws_fake(3, move |index, mut socket| {
        let identify_times = Arc::clone(&identify_times);
        let op_slot = Arc::clone(&op_slot);
        async move {
            hello(&mut socket, 100).await;
            let value = next_json(&mut socket).await.unwrap();
            op_slot.lock().unwrap().push(value["op"].as_u64().unwrap());
            identify_times.lock().unwrap().push(Instant::now());
            send_json(
                &mut socket,
                json!({"op":0,"s":index as u64 + 1,"t":"READY","d":{"session_id":"session"}}),
            )
            .await;
            let code = if index == 0 {
                4009
            } else if index == 1 {
                4007
            } else {
                4004
            };
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                        code,
                    ),
                    reason: "test".into(),
                })))
                .await;
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(20);
    config.identify_gap = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(2), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    assert_eq!(*ops.lock().unwrap(), vec![2, 2, 2]);
    let times = identifies.lock().unwrap();
    assert!(times[1].duration_since(times[0]) >= Duration::from_millis(20));
    assert!(
        times[2].duration_since(times[1])
            > times[1].duration_since(times[0]) + Duration::from_millis(5)
    );
}

#[tokio::test]
async fn gateway_resume_seq_tracks_filtered_message_dispatch() {
    let resume = Arc::new(Mutex::new(None));
    let resume_slot = Arc::clone(&resume);
    let fake = ws_fake(2, move |index, mut socket| {
        let resume_slot = Arc::clone(&resume_slot);
        async move {
            hello(&mut socket, 100).await;
            let handshake = next_json(&mut socket).await.unwrap();
            if index == 0 {
                assert_eq!(handshake["op"], 2);
                send_json(
                    &mut socket,
                    json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
                )
                .await;
                send_json(
                    &mut socket,
                    json!({"op":0,"s":7,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"not-numeric","channel_id":"1","author":{"id":"a"},"content":"ignored"}}),
                )
                .await;
                let _ = socket.send(Message::Close(None)).await;
            } else {
                *resume_slot.lock().unwrap() = Some(handshake);
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16),
                        reason: "auth".into(),
                    })))
                    .await;
            }
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(1);
    let discord = Discord::new(config);
    let _watch = discord.watch();
    timeout(Duration::from_secs(1), async {
        loop {
            if resume.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let resume = resume.lock().unwrap().clone().unwrap();
    assert_eq!(resume["op"], 6);
    assert_eq!(resume["d"]["seq"], 7);
}

#[tokio::test]
async fn gateway_resume_seq_tracks_other_dispatch() {
    let resume = Arc::new(Mutex::new(None));
    let resume_slot = Arc::clone(&resume);
    let fake = ws_fake(2, move |index, mut socket| {
        let resume_slot = Arc::clone(&resume_slot);
        async move {
            hello(&mut socket, 100).await;
            let handshake = next_json(&mut socket).await.unwrap();
            if index == 0 {
                assert_eq!(handshake["op"], 2);
                send_json(
                    &mut socket,
                    json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
                )
                .await;
                send_json(
                    &mut socket,
                    json!({"op":0,"s":8,"t":"CHANNEL_CREATE","d":{}}),
                )
                .await;
                let _ = socket.send(Message::Close(None)).await;
            } else {
                *resume_slot.lock().unwrap() = Some(handshake);
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16),
                        reason: "auth".into(),
                    })))
                    .await;
            }
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(1);
    let discord = Discord::new(config);
    let _watch = discord.watch();
    timeout(Duration::from_secs(1), async {
        loop {
            if resume.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    let resume = resume.lock().unwrap().clone().unwrap();
    assert_eq!(resume["op"], 6);
    assert_eq!(resume["d"]["seq"], 8);
}

#[tokio::test]
async fn gateway_invalid_session_identifies_after_gap_and_fatal_is_persistent() {
    let identifies = Arc::new(Mutex::new(Vec::new()));
    let identify_slot = Arc::clone(&identifies);
    let fake = ws_fake(3, move |_index, mut socket| {
        let identify_slot = Arc::clone(&identify_slot);
        async move {
            hello(&mut socket, 100).await;
            let value = next_json(&mut socket).await.unwrap();
            let count = {
                let mut times = identify_slot.lock().unwrap();
                times.push(Instant::now());
                times.len()
            };
            assert_eq!(value["op"], 2);
            send_json(&mut socket, json!({"op":0,"s":count as u64,"t":"READY","d":{"session_id":"session"}})).await;
            if count < 3 {
                send_json(&mut socket, json!({"op":9,"d":false})).await;
            } else {
                let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
            }
        }
    }).await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.identify_gap = Duration::from_millis(10);
    config.reconnect_base = Duration::from_millis(20);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let error = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.0.contains("4004"));
    let times = identifies.lock().unwrap();
    assert_eq!(times.len(), 3);
    let first_gap = times[1].duration_since(times[0]);
    let second_gap = times[2].duration_since(times[1]);
    assert!(first_gap >= Duration::from_millis(10));
    assert!(second_gap > first_gap + Duration::from_millis(5));
}

#[tokio::test]
async fn gateway_zombie_close_uses_code_4000() {
    let close_code = Arc::new(Mutex::new(None));
    let close_slot = Arc::clone(&close_code);
    let fake = ws_fake(1, move |_index, mut socket| {
        let close_slot = Arc::clone(&close_slot);
        async move {
            hello(&mut socket, 5).await;
            let _ = next_json(&mut socket).await.unwrap();
            send_json(
                &mut socket,
                json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
            )
            .await;
            *close_slot.lock().unwrap() = next_close_code(&mut socket).await;
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.reconnect_base = Duration::from_millis(1);
    config.identify_gap = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let _watch = discord.watch();
    timeout(Duration::from_secs(1), async {
        loop {
            if close_code.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*close_code.lock().unwrap(), Some(4000));
}

#[tokio::test]
async fn gateway_rejects_untrusted_resume_url_and_reports_fatal_once() {
    let second_op = Arc::new(Mutex::new(None));
    let second_slot = Arc::clone(&second_op);
    let fake = ws_fake(2, move |index, mut socket| {
        let second_slot = Arc::clone(&second_slot);
        async move {
            hello(&mut socket, 100).await;
            let value = next_json(&mut socket).await.unwrap();
            if index == 0 {
                send_json(&mut socket, json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session","resume_gateway_url":"ws://evil.invalid"}})).await;
                let _ = socket.send(Message::Close(None)).await;
            } else {
                *second_slot.lock().unwrap() = Some(value["op"].as_u64().unwrap());
                let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
            }
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    assert_eq!(*second_op.lock().unwrap(), Some(6));
}

#[tokio::test]
async fn gateway_reconnects_with_resume_after_unacknowledged_heartbeat() {
    let resume = Arc::new(Mutex::new(None));
    let resume_slot = Arc::clone(&resume);
    let fake = ws_fake(2, move |index, mut socket| {
        let resume_slot = Arc::clone(&resume_slot);
        async move {
            hello(&mut socket, 10).await;
            let identify = next_json(&mut socket).await.unwrap();
            if index == 0 {
                assert_eq!(identify["op"], 2);
                send_json(&mut socket, json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}})).await;
                while next_json(&mut socket).await.is_some() {}
            } else {
                *resume_slot.lock().unwrap() = Some(identify);
                let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
            }
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let mut watch = discord.watch();
    let fatal = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap_err();
    assert!(fatal.0.contains("4004"));
    let resume = resume.lock().unwrap().clone().unwrap();
    assert_eq!(resume["op"], 6);
    assert_eq!(resume["d"]["seq"], 1);
}

#[tokio::test]
async fn gateway_fatal_close_codes_are_persistent_errors() {
    for code in [4004_u16, 4010, 4011, 4012, 4013, 4014] {
        let fake = ws_fake(1, move |_index, mut socket| async move {
            hello(&mut socket, 100).await;
            let _ = next_json(&mut socket).await.unwrap();
            let _ = socket
                .send(Message::Close(Some(CloseFrame {
                    code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(
                        code,
                    ),
                    reason: "fatal".into(),
                })))
                .await;
        })
        .await;
        let discord = discord_for_ws(&fake.url, "123");
        let mut watch = discord.watch();
        let fatal = timeout(Duration::from_secs(1), watch.recv())
            .await
            .unwrap()
            .unwrap_err();
        assert!(fatal.0.contains(&code.to_string()));
        assert_eq!(watch.recv().await.unwrap_err(), fatal);
    }
}

#[tokio::test]
async fn gateway_resume_seq_tracks_last_handoff_before_reconnect() {
    let resume = Arc::new(Mutex::new(None));
    let resume_slot = Arc::clone(&resume);
    let reconnect = Arc::new(tokio::sync::Notify::new());
    let reconnect_slot = Arc::clone(&reconnect);
    let fake = ws_fake(2, move |index, mut socket| {
        let resume_slot = Arc::clone(&resume_slot);
        let reconnect_slot = Arc::clone(&reconnect_slot);
        async move {
            hello(&mut socket, 1000).await;
            let handshake = next_json(&mut socket).await.unwrap();
            if index == 0 {
                assert_eq!(handshake["op"], 2);
                send_json(
                    &mut socket,
                    json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
                )
                .await;
                send_json(
                    &mut socket,
                    json!({"op":0,"s":2,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"2","channel_id":"1","author":{"id":"a"},"content":"2"}}),
                )
                .await;
                send_json(
                    &mut socket,
                    json!({"op":0,"s":3,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"3","channel_id":"1","author":{"id":"a"},"content":"3"}}),
                )
                .await;
                reconnect_slot.notified().await;
                let _ = socket.send(Message::Close(None)).await;
            } else {
                *resume_slot.lock().unwrap() = Some(handshake);
                let _ = socket
                    .send(Message::Close(Some(CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16),
                        reason: "auth".into(),
                    })))
                    .await;
            }
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.queue = 1;
    config.reconnect_base = Duration::from_millis(1);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    let first = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first.message_id, 2);
    tokio::time::sleep(Duration::from_millis(50)).await;
    reconnect.notify_one();
    timeout(Duration::from_secs(1), async {
        loop {
            if resume.lock().unwrap().is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(resume.lock().unwrap().as_ref().unwrap()["d"]["seq"], 3);
}

async fn answer_heartbeat(
    socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ack_count: Arc<AtomicUsize>,
) -> bool {
    let mut responded = false;
    while let Ok(Some(value)) = timeout(Duration::from_millis(100), next_json(socket)).await {
        if value["op"] == 1 {
            ack_count.fetch_add(1, Ordering::SeqCst);
            send_json(socket, json!({"op":11,"d":null})).await;
            responded = true;
            break;
        }
    }
    responded
}

async fn serve_first_connection(
    socket: &mut WebSocketStream<tokio::net::TcpStream>,
    ack_count: Arc<AtomicUsize>,
) {
    hello(socket, 1000).await;
    let identify = next_json(socket).await.unwrap();
    assert_eq!(identify["op"], 2);
    send_json(
        socket,
        json!({"op":0,"s":1,"t":"READY","d":{"session_id":"session"}}),
    )
    .await;
    for sequence in 2..=4 {
        send_json(&mut *socket, json!({"op":0,"s":sequence,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":sequence.to_string(),"channel_id":"1","author":{"id":"a"},"content":sequence.to_string()}})).await;
    }
    for _ in 0..2 {
        send_json(&mut *socket, json!({"op":1,"d":null})).await;
        if !answer_heartbeat(socket, Arc::clone(&ack_count)).await {
            return;
        }
    }
    for sequence in 5..=6 {
        send_json(&mut *socket, json!({"op":0,"s":sequence,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":sequence.to_string(),"channel_id":"1","author":{"id":sequence.to_string()},"content":sequence.to_string()}})).await;
    }
    while let Some(value) = next_json(socket).await {
        if value["op"] == 1 {
            ack_count.fetch_add(1, Ordering::SeqCst);
            send_json(socket, json!({"op":11,"d":null})).await;
        }
    }
}

async fn serve_replayed_connection(
    socket: &mut WebSocketStream<tokio::net::TcpStream>,
    resume: Arc<Mutex<Option<Value>>>,
    release: Arc<tokio::sync::Notify>,
    replay_step: Arc<tokio::sync::Notify>,
) {
    hello(socket, 100).await;
    let identify = next_json(socket).await.unwrap();
    *resume.lock().unwrap() = Some(identify);
    release.notified().await;
    for sequence in 4..=6 {
        send_json(&mut *socket, json!({"op":0,"s":sequence,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":sequence.to_string(),"channel_id":"1","author":{"id":"a"},"content":sequence.to_string()}})).await;
        replay_step.notified().await;
    }
}

async fn wait_for_resume(resume: Arc<Mutex<Option<Value>>>) {
    timeout(Duration::from_secs(1), async {
        loop {
            if resume.lock().unwrap().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn gateway_queue_reads_heartbeats_while_consumer_is_slow() {
    let ack_count = Arc::new(AtomicUsize::new(0));
    let resume = Arc::new(Mutex::new(None));
    let release = Arc::new(tokio::sync::Notify::new());
    let replay_step = Arc::new(tokio::sync::Notify::new());
    let ack_slot = Arc::clone(&ack_count);
    let resume_slot = Arc::clone(&resume);
    let release_slot = Arc::clone(&release);
    let replay_step_slot = Arc::clone(&replay_step);
    let fake = ws_fake(2, move |index, mut socket| {
        let ack_slot = Arc::clone(&ack_slot);
        let resume_slot = Arc::clone(&resume_slot);
        let release_slot = Arc::clone(&release_slot);
        let replay_step_slot = Arc::clone(&replay_step_slot);
        async move {
            if index == 0 {
                serve_first_connection(&mut socket, Arc::clone(&ack_slot)).await;
            } else {
                serve_replayed_connection(
                    &mut socket,
                    Arc::clone(&resume_slot),
                    Arc::clone(&release_slot),
                    Arc::clone(&replay_step_slot),
                )
                .await;
            }
        }
    })
    .await;
    let mut config = DiscordConfig::new("token".into(), "123".into());
    config.gateway_url = fake.url.clone();
    config.queue = 2;
    config.reconnect_base = Duration::from_millis(1);
    config.identify_gap = Duration::from_millis(1);
    config.stable_after = Duration::from_secs(30);
    let discord = Discord::new(config);
    let mut watch = discord.watch();
    wait_for_resume(Arc::clone(&resume)).await;
    assert!(ack_count.load(Ordering::SeqCst) >= 2);
    let first = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    let second = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!((first.message_id, second.message_id), (2, 3));
    release.notify_one();
    let mut events = vec![first, second];
    for expected in 4..=6 {
        let event = timeout(Duration::from_secs(1), watch.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.message_id, expected);
        replay_step.notify_one();
        events.push(event);
    }
    assert_eq!(
        events
            .iter()
            .map(|event| event.message_id)
            .collect::<Vec<_>>(),
        vec![2, 3, 4, 5, 6]
    );
    assert_eq!(resume.lock().unwrap().as_ref().unwrap()["d"]["seq"], 3);
}

#[tokio::test]
async fn malformed_gateway_frames_do_not_stop_later_events() {
    let fake = ws_fake(2, move |index, mut socket| async move {
        hello(&mut socket, 100).await;
        let _ = next_json(&mut socket).await.unwrap();
        if index == 0 {
            socket.send(Message::Text("{".into())).await.unwrap();
            socket.send(Message::Text("x".repeat(2 * 1024 * 1024).into())).await.unwrap();
        } else {
            send_json(&mut socket, json!({"op":0,"s":2,"t":"MESSAGE_CREATE","d":{"guild_id":"123","id":"8","channel_id":"1","author":{"id":"a"},"content":"later"}})).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            let _ = socket.send(Message::Close(Some(CloseFrame { code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::from(4004_u16), reason: "auth".into() }))).await;
        }
    }).await;
    let discord = discord_for_ws(&fake.url, "123");
    let mut watch = discord.watch();
    let event = timeout(Duration::from_secs(1), watch.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.text, "later");
}

#[test]
fn close_classification_and_url_defaults_are_independent_of_network() {
    let config = DiscordConfig::new("token".into(), "123".into());
    assert_eq!(config.reconnect_base, Duration::from_secs(1));
    assert_eq!(config.queue, 1024);
    let _ = Fatal("gateway task ended".into());
}
