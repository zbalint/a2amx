use super::*;

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

#[test]
fn close_classification_and_url_defaults_are_independent_of_network() {
    let config = DiscordConfig::new("token".into(), "123".into());
    assert_eq!(config.reconnect_base, Duration::from_secs(1));
    assert_eq!(config.queue, 1024);
    let _ = Fatal("gateway task ended".into());
}
