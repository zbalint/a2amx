use super::*;

#[test]
fn gateway_token_cli_reads_stdin_and_writes_trimmed_secret() {
    let home = tempdir().unwrap();
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home", home.path().to_str().unwrap(), "gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"  secret-token  \n")
        .unwrap();
    let output = command.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    assert_eq!(
        fs::read_to_string(home.path().join("config/secrets/discord.token")).unwrap(),
        "secret-token"
    );
    assert_eq!(
        fs::metadata(home.path().join("config/secrets/discord.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}

#[test]
fn gateway_token_cli_rejects_oversized_stdin_without_writing() {
    let home = tempdir().unwrap();
    let mut command = ProcessCommand::new(env!("CARGO_BIN_EXE_a2amx"))
        .args(["--home", home.path().to_str().unwrap(), "gateway", "token"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .as_mut()
        .unwrap()
        .write_all(&vec![b'x'; 64 * 1024 + 1])
        .unwrap();
    let output = command.wait_with_output().unwrap();
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("gateway token exceeds 64 KiB"));
    assert!(!home.path().join("config/secrets/discord.token").exists());
}

#[tokio::test]
async fn gateway_client_uses_gateway_token_file() {
    let (home, daemon) = start_daemon().await;
    let layout = a2amx::layout::Layout::new(home.path().to_path_buf());
    let admin_token = fs::read_to_string(layout.admin_token()).unwrap();
    let gateway_token = fs::read_to_string(layout.link_gateway_token()).unwrap();
    assert_ne!(admin_token, gateway_token);
    let mut gateway = a2amx::client::Client::connect_gateway(&layout)
        .await
        .unwrap();
    assert!(matches!(
        gateway.request(Request::GatewayAgents).await.unwrap(),
        Response::GatewayAgents { .. }
    ));
    daemon.shutdown().await.unwrap();
}
