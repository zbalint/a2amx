use super::*;

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
