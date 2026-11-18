use super::*;

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
