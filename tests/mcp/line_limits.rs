use super::*;

#[tokio::test]
async fn mcp_rejects_oversized_line_and_resumes() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_id, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        let mut line = vec![b'x'; MAX_FRAME_LEN + 1];
        line.push(b'\n');
        mcp.send_bytes(&line);
        let oversized = mcp.read();
        assert_eq!(oversized["error"]["code"], -32600);
        assert_eq!(oversized["id"], Value::Null);
        assert_eq!(oversized["error"]["message"], "request too large");

        let ping = mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}));
        assert_eq!(ping["id"], 1);
        assert_eq!(ping["result"], json!({}));
        mcp.close_stdin();
        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("oversized MCP line scenario");
}

#[tokio::test]
async fn mcp_discards_the_rest_of_an_oversized_line() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_id, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        let line = vec![b'x'; 2 * MAX_FRAME_LEN];
        for chunk in line.chunks(300_000) {
            mcp.send_bytes(chunk);
        }
        mcp.send_bytes(b"\n");
        let oversized = mcp.read();
        assert_eq!(oversized["error"]["code"], -32600);
        assert_eq!(oversized["id"], Value::Null);
        assert_eq!(oversized["error"]["message"], "request too large");

        let ping = mcp.send(json!({"jsonrpc":"2.0","id":1,"method":"ping"}));
        assert_eq!(ping["id"], 1);
        assert_eq!(ping["result"], json!({}));
        mcp.close_stdin();
        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("oversized MCP discard scenario");
}

#[tokio::test]
async fn mcp_accepts_a_line_at_the_inclusive_limit() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_id, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        let mut line = vec![b'x'; MAX_FRAME_LEN];
        line.push(b'\n');
        mcp.send_bytes(&line);
        let parse_error = mcp.read();
        assert_eq!(parse_error["error"]["code"], -32700);
        assert_eq!(parse_error["id"], Value::Null);
        mcp.close_stdin();
        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("inclusive MCP limit scenario");
}

#[tokio::test]
async fn mcp_reports_an_oversized_line_terminated_by_eof() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_id, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        mcp.send_bytes(&vec![b'x'; MAX_FRAME_LEN + 1]);
        mcp.close_stdin();
        let oversized = mcp.read();
        assert_eq!(oversized["error"]["code"], -32600);
        assert_eq!(oversized["id"], Value::Null);
        assert_eq!(oversized["error"]["message"], "request too large");

        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("oversized MCP EOF scenario");
}

#[tokio::test]
async fn mcp_accepts_an_unterminated_ping_at_eof() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_id, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        None,
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        mcp.send_bytes(br#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#);
        mcp.close_stdin();
        let ping = mcp.read();
        assert_eq!(ping["id"], 1);
        assert_eq!(ping["result"], json!({}));

        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("unterminated MCP ping scenario");
}

#[tokio::test]
async fn mcp_rejects_oversized_send_message_before_daemon_write() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin");
    let (_, token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    tokio::task::spawn_blocking(move || {
        let mut mcp = McpProcess::spawn(addr, &token);
        let response = mcp.send(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"send_message","arguments":{
                "to":"a".repeat(2 * 1024 * 1024),"subject":"Subject","message":"Body"
            }}
        }));
        assert_eq!(response["error"]["code"], -32600);
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["message"], "request too large");
        let response = mcp.send(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"list_agents","arguments":{}}
        }));
        assert_eq!(
            tool_result(&response),
            json!({"agents":[
                {"address":"agent-plan@host-a","state":"running","activity":"working","attached":false,"harness":"generic","you":true}
            ]})
        );
        mcp.close_stdin();
        let mut trailing = String::new();
        mcp.stdout
            .read_to_string(&mut trailing)
            .expect("read MCP stdout to EOF");
        assert!(trailing.is_empty());
        assert_eq!(mcp.child.wait().expect("wait for MCP").code(), Some(0));
    })
    .await
    .expect("prewrite boundary scenario");
}
