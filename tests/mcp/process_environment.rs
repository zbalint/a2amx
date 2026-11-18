use super::*;

fn spawn_inert_mcp() -> McpProcess {
    let mut child = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("mcp")
        .arg("--optional")
        .env_remove("A2AMX_ADDR")
        .env_remove("A2AMX_TOKEN")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn inert mcp");
    let stdin = child.stdin.take().expect("mcp stdin");
    let stdout = BufReader::new(child.stdout.take().expect("mcp stdout"));
    McpProcess {
        child,
        stdin: Some(stdin),
        stdout,
    }
}

#[test]
fn mcp_missing_or_invalid_environment_exits_without_protocol_output() {
    let binary = env!("CARGO_BIN_EXE_a2amx");
    let cases = [
        (None, Some("a".repeat(64))),
        (Some("127.0.0.1:1".to_owned()), None),
        (Some("not-an-address".to_owned()), Some("a".repeat(64))),
        (Some("127.0.0.1:1".to_owned()), Some("short".to_owned())),
        (Some("127.0.0.1:1".to_owned()), Some("z".repeat(64))),
    ];
    for (addr, token) in cases {
        let mut command = Command::new(binary);
        command
            .arg("mcp")
            .env_remove("A2AMX_ADDR")
            .env_remove("A2AMX_TOKEN");
        if let Some(addr) = addr {
            command.env("A2AMX_ADDR", addr);
        }
        if let Some(token) = token {
            command.env("A2AMX_TOKEN", token);
        }
        let output = command.output().expect("run mcp");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "a2amx: A2AMX_ADDR and A2AMX_TOKEN must be set"
        );
    }
}

#[test]
fn mcp_optional_without_environment_is_inert() {
    let mut mcp = spawn_inert_mcp();
    let initialize = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-11-25"}
    }));
    assert_eq!(initialize["result"]["serverInfo"]["name"], "a2amx");
    mcp.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    assert_eq!(
        mcp.send(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}))["result"]["tools"],
        json!([])
    );
    assert_eq!(
        mcp.send(json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}))["result"],
        json!({})
    );
    let call = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {"name": "list_agents", "arguments": {}}
    }));
    assert_eq!(call["error"]["code"], -32602);
    assert_eq!(
        call["error"]["message"],
        "a2amx is not attached to this session"
    );
    mcp.close_stdin();
    assert_eq!(mcp.child.wait().unwrap().code(), Some(0));
}

#[test]
fn mcp_optional_partial_or_invalid_environment_stays_strict() {
    let binary = env!("CARGO_BIN_EXE_a2amx");
    let cases = [
        (None, Some("a".repeat(64))),
        (Some("127.0.0.1:1".to_owned()), None),
        (Some("not-an-address".to_owned()), Some("a".repeat(64))),
        (Some("127.0.0.1:1".to_owned()), Some("short".to_owned())),
        (Some("127.0.0.1:1".to_owned()), Some("z".repeat(64))),
    ];
    for (addr, token) in cases {
        let mut command = Command::new(binary);
        command
            .arg("mcp")
            .arg("--optional")
            .env_remove("A2AMX_ADDR")
            .env_remove("A2AMX_TOKEN");
        if let Some(addr) = addr {
            command.env("A2AMX_ADDR", addr);
        }
        if let Some(token) = token {
            command.env("A2AMX_TOKEN", token);
        }
        let output = command.output().expect("run optional mcp");
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&output.stderr).trim(),
            "a2amx: A2AMX_ADDR and A2AMX_TOKEN must be set"
        );
    }
}

#[test]
fn mcp_reports_a_stdin_read_error_and_exits_nonzero() {
    let dir = tempfile::tempdir().expect("stdin directory");
    let stdin = std::fs::File::open(dir.path()).expect("open stdin directory");
    let output = Command::new(env!("CARGO_BIN_EXE_a2amx"))
        .arg("mcp")
        .env("A2AMX_ADDR", "127.0.0.1:1")
        .env("A2AMX_TOKEN", "a".repeat(64))
        .stdin(Stdio::from(stdin))
        .output()
        .expect("run mcp");

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("reading MCP stdin"));
}
