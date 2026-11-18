use super::*;

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
