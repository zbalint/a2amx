use super::*;

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
