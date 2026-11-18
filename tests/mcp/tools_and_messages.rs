use super::*;

#[tokio::test]
async fn mcp_initialize_tools_and_message_calls() {
    let (dir, _daemon) = common::start_daemon().await;
    let mut admin = Client::connect(dir.path()).await.expect("admin client");
    let (_, sender_token, addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-plan"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;
    let (_, _recipient_token, _recipient_addr) = common::new_agent(
        &mut admin,
        dir.path(),
        Some("agent-review"),
        Harness::Generic,
        Deliver::Hold,
    )
    .await;

    tokio::task::spawn_blocking(move || {
    let mut mcp = McpProcess::spawn(addr, &sender_token);

    let initialized = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {"protocolVersion": "2025-11-25"}
    }));
    assert_eq!(initialized["id"], 1);
    assert_eq!(initialized["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(initialized["result"]["capabilities"]["tools"], json!({}));
    assert_eq!(initialized["result"]["serverInfo"]["name"], "a2amx");
    assert_eq!(
        initialized["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );

    let unsupported = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "initialize",
        "params": {"protocolVersion": "1999-01-01"}
    }));
    assert_eq!(unsupported["result"]["protocolVersion"], "2025-11-25");
    for (offset, version) in [
        "2025-06-18",
        "2025-03-26",
        "2024-11-05",
    ]
    .into_iter()
    .enumerate()
    {
        let response = mcp.send(json!({
            "jsonrpc": "2.0",
            "id": 10 + offset,
            "method": "initialize",
            "params": {"protocolVersion": version}
        }));
        assert_eq!(response["result"]["protocolVersion"], version);
    }

    // Unknown protocol versions use the latest supported version.

    // A notification must not produce a line: the following ping is the first
    // response observed, which synchronizes the assertion without sleeping.
    mcp.send_raw(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
    mcp.send_raw(r#"{"jsonrpc":"2.0","id":"ping","method":"ping"}"#);
    let ping = mcp.read();
    assert_eq!(ping["id"], "ping");
    assert_eq!(ping["result"], json!({}));

    let tools = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/list"
    }));
    let names = tools["result"]["tools"]
        .as_array()
        .expect("tool list")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect::<Vec<_>>();
    assert_eq!(names, ["list_agents", "send_message", "message_status", "reset_session"]);
    assert_eq!(
        tools["result"]["tools"],
        json!([
            {
                "name": "list_agents",
                "description": "List the agent sessions you can message. Sessions outside the caller's visibility are not listed. \"attached\" means a human client is attached to the session; it does not mean the session is reachable, and a detached session still receives messages. \"viewer\" is true when that client is in read-only viewer mode (it holds the session but does not type, so it does not mean the owner is at the keyboard; the \"owner\" field is the presence signal for remote-enabled sessions). Each entry's \"team\" is its team name when it has one, and \"you\" marks the caller. \"quota\" gives the percent left in each window (\"five_hour_left\", \"weekly_left\"; 100 means all of it is left, 0 means used up); \"limit_reached\" means the harness reported a usage limit. A remote-enabled session also gets \"owner\", which is \"present\" or \"away\".",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "send_message",
                "description": "Send a message to another agent session or to the operator at human:owner. Returns a message id once the message is accepted; acceptance does not mean it was delivered, read, or acted on. Check message_status when the reply matters. If recipient_hold is present, the recipient is held and a person may need to act. If recipient_quota is present, the recipient's harness reports that quota is used up and it may not answer until the quota resets. Plain text; do not write the text of a paste-wrapper tag in a body. This works only for an agent the operator enabled for remote messaging. Sending to human:owner is refused with owner_present while the owner is attached to your session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "to": {
                            "type": "string",
                            "description": "Recipient address, for example agent-review@host-a."
                        },
                        "subject": {
                            "type": "string",
                            "description": "Short subject, at most 200 bytes."
                        },
                        "message": {
                            "type": "string",
                            "description": "Message body, at most 32 KiB."
                        }
                    },
                    "required": ["to", "subject", "message"],
                    "additionalProperties": false
                }
            },
            {
                "name": "message_status",
                "description": "Show the state of a message you sent. \"state\" and \"detail\" say whether it was delivered, is waiting, or was given up on; \"hold_reason\" and \"hold_explanation\" say what a waiting message needs; \"evidence\" of submission_observed means the recipient's prompt hook saw it submitted; \"accepted_at\" and \"updated_at\" are Unix seconds.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"],
                    "additionalProperties": false
                }
            },
            {
                "name": "reset_session",
                "description": "Run the target session's configured reset sequence. The target must have granted your session consent through control_from.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "to": {
                            "type": "string",
                            "description": "Target session name or address."
                        }
                    },
                    "required": ["to"],
                    "additionalProperties": false
                }
            }
        ])
    );


    let agents = tool_result(&mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {"name": "list_agents", "arguments": {}}
    })));
    assert_eq!(agents, json!({"agents":[
        {"address":"agent-plan@host-a","state":"running","activity":"working","attached":false,"harness":"generic","you":true},
        {"address":"agent-review@host-a","state":"running","activity":"working","attached":false,"harness":"generic"}
    ]}));

    let accepted = tool_result(&mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "send_message",
            "arguments": {
                "to": "agent-review@host-a",
                "subject": "Parser issue",
                "message": "I found the regression in parser.py."
            }
        }
    })));
    assert_eq!(accepted, json!({"id":"m_1","status":"accepted","recipient_hold":"deliver_hold"}));

    let status = tool_result(&mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {"name": "message_status", "arguments": {"id": "m_1"}}
    })));
    assert_eq!(status["id"], "m_1");
    assert_eq!(status["to"], "agent-review@host-a");
    assert_eq!(status["state"], "pending");
    assert_eq!(status["detail"], Value::Null);
    assert_eq!(status["hold_reason"], "deliver_hold");
    assert_eq!(
        status["hold_explanation"],
        "The recipient session only holds messages; a person must deliver by hand or restart it with automatic delivery."
    );
    assert_eq!(status["evidence"], Value::Null);
    let accepted_at = status["accepted_at"]
        .as_i64()
        .expect("accepted_at is an integer");
    let updated_at = status["updated_at"]
        .as_i64()
        .expect("updated_at is an integer");
    assert!(accepted_at > 0);
    assert!(updated_at > 0);
    assert!(accepted_at <= updated_at);

    let unknown_recipient = failed_tool_result(&mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "send_message",
            "arguments": {
                "to": "nobody@host-a",
                "subject": "Parser issue",
                "message": "This recipient does not exist."
            }
        }
    })));
    assert_eq!(unknown_recipient["code"], "unknown_recipient");
    }).await.expect("MCP pipe scenario");
}
#[test]
fn mcp_reset_session_returns_text_and_rejects_extra_keys() {
    let (addr, server) = spawn_scripted_server(|listener| {
        let (mut stream, _) = listener.accept().expect("accept reset connection");
        expect_hello(&mut stream);
        assert_eq!(
            read_request(&mut stream),
            Request::Reset {
                session: "target@host-a".into()
            }
        );
        write_response(&mut stream, &Response::Reset { steps: 2 });
    });
    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let extra = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "reset_session", "arguments": {"to": "target", "extra": true}}
    }));
    assert_eq!(extra["error"]["code"], -32602);
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {"name": "reset_session", "arguments": {"to": "target@host-a"}}
    }));
    assert_eq!(
        tool_result(&response),
        json!({"message": "reset target@host-a: 2 steps", "steps": 2})
    );
    server.join().expect("reset MCP server");
}

fn accepted_tool_result(hold: Option<&str>, quota: Option<&str>) -> Value {
    let (hold, quota) = (hold.map(str::to_owned), quota.map(str::to_owned));
    let (addr, server) = spawn_scripted_server(move |listener| {
        let (mut stream, _) = listener.accept().expect("accept accepted connection");
        expect_hello(&mut stream);
        assert!(matches!(
            read_request(&mut stream),
            Request::SendMessage { .. }
        ));
        write_response(
            &mut stream,
            &Response::Accepted {
                id: "m_1".to_owned(),
                recipient_hold: hold,
                recipient_quota: quota,
            },
        );
    });
    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "send_message",
            "arguments": {"to": "agent-review@host-a", "subject": "s", "message": "m"}
        }
    }));
    let result = tool_result(&response);
    drop(mcp);
    server.join().expect("scripted daemon thread");
    result
}

#[test]
fn mcp_accepted_reports_recipient_quota() {
    assert_eq!(
        accepted_tool_result(None, Some("5h quota exhausted")),
        json!({"id":"m_1","status":"accepted","recipient_quota":"5h quota exhausted"})
    );
}

#[test]
fn mcp_accepted_reports_hold_and_quota_together() {
    assert_eq!(
        accepted_tool_result(Some("deliver_hold"), Some("5h quota exhausted")),
        json!({
            "id":"m_1",
            "status":"accepted",
            "recipient_hold":"deliver_hold",
            "recipient_quota":"5h quota exhausted"
        })
    );
}

#[test]
fn mcp_accepted_omits_absent_recipient_hold() {
    let (addr, server) = spawn_scripted_server(|listener| {
        let (mut stream, _) = listener
            .accept()
            .expect("accept accepted response connection");
        expect_hello(&mut stream);
        assert!(matches!(
            read_request(&mut stream),
            Request::SendMessage { .. }
        ));
        write_response(
            &mut stream,
            &Response::Accepted {
                id: "m_1".to_owned(),
                recipient_hold: None,
                recipient_quota: None,
            },
        );
    });

    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "send_message",
            "arguments": {
                "to": "agent-review@host-a",
                "subject": "Parser issue",
                "message": "No recipient hold was reported.",
            }
        }
    }));
    assert_eq!(
        tool_result(&response),
        json!({"id": "m_1", "status": "accepted"})
    );
    server.join().expect("accepted response server");
}

#[test]
fn mcp_status_serializes_optional_visibility_fields_as_null() {
    let (addr, server) = spawn_scripted_server(|listener| {
        let (mut stream, _) = listener
            .accept()
            .expect("accept status response connection");
        expect_hello(&mut stream);
        assert_eq!(
            read_request(&mut stream),
            Request::MessageStatus { id: "m_1".into() }
        );
        write_response(
            &mut stream,
            &Response::Status {
                message: MessageInfo {
                    id: "m_1".into(),
                    from: "agent-plan@host-a".into(),
                    to: "agent-review@host-a".into(),
                    subject: "Parser issue".into(),
                    state: "submitted".into(),
                    detail: None,
                    hold_reason: None,
                    evidence: None,
                    hold_explanation: None,
                    accepted_at: None,
                    updated_at: None,
                },
            },
        );
    });

    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "message_status", "arguments": {"id": "m_1"}}
    }));
    assert_eq!(
        tool_result(&response),
        json!({
            "id": "m_1",
            "to": "agent-review@host-a",
            "state": "submitted",
            "detail": null,
            "hold_reason": null,
            "evidence": null,
            "hold_explanation": null,
            "accepted_at": null,
            "updated_at": null,
        })
    );
    server.join().expect("status response server");
}

#[tokio::test]
async fn mcp_protocol_errors_notifications_and_argument_validation() {
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

        let parse_error = {
            mcp.send_raw("not json");
            mcp.read()
        };
        assert_eq!(parse_error["error"]["code"], -32700);
        assert_eq!(parse_error["id"], Value::Null);
        for blank in ["", " \t\r"] {
            mcp.send_raw(blank);
            let blank_error = mcp.read();
            assert_eq!(blank_error["error"]["code"], -32700);
            assert_eq!(blank_error["id"], Value::Null);
        }
        mcp.send_bytes(b"\xff\n");
        let encoding_error = mcp.read();
        assert_eq!(encoding_error["error"]["code"], -32700);
        assert_eq!(encoding_error["id"], Value::Null);

        let array_error = mcp.send(json!([{"jsonrpc":"2.0","id":1,"method":"ping"}]));
        assert_eq!(array_error["error"]["code"], -32600);
        assert_eq!(array_error["id"], Value::Null);

        let missing_method = mcp.send(json!({"jsonrpc":"2.0","id":2}));
        assert_eq!(missing_method["error"]["code"], -32600);

        let non_string_method = mcp.send(json!({"jsonrpc":"2.0","id":3,"method":4}));
        assert_eq!(non_string_method["error"]["code"], -32600);
        let invalid_jsonrpc = mcp.send(json!({"jsonrpc":"1.0","id":30,"method":"ping"}));
        assert_eq!(invalid_jsonrpc["error"]["code"], -32600);

        let unknown_method = mcp.send(json!({"jsonrpc":"2.0","id":4,"method":"unknown"}));
        assert_eq!(unknown_method["error"]["code"], -32601);

        // Unknown notifications are silent, and the ping proves the server kept
        // processing input after the notification.
        mcp.send_raw(r#"{"jsonrpc":"2.0","method":"unknown"}"#);
        mcp.send_raw(r#"{"jsonrpc":"2.0","id":5,"method":"ping"}"#);
        assert_eq!(mcp.read()["id"], 5);

        let missing_argument = mcp.send(json!({
            "jsonrpc":"2.0", "id":6, "method":"tools/call",
            "params":{"name":"send_message","arguments":{"to":"s2@host-a","subject":"x"}}
        }));
        assert_eq!(missing_argument["error"]["code"], -32602);

        let extra_argument = mcp.send(json!({
            "jsonrpc":"2.0", "id":7, "method":"tools/call",
            "params":{"name":"list_agents","arguments":{"extra":"no"}}
        }));
        assert_eq!(extra_argument["error"]["code"], -32602);

        let non_string_argument = mcp.send(json!({
            "jsonrpc":"2.0", "id":8, "method":"tools/call",
            "params":{"name":"message_status","arguments":{"id":9}}
        }));
        assert_eq!(non_string_argument["error"]["code"], -32602);

        // MCP treats `arguments` as optional; a no-argument tool must accept its absence.
        let omitted_arguments = mcp.send(json!({
            "jsonrpc":"2.0", "id":10, "method":"tools/call",
            "params":{"name":"list_agents"}
        }));
        assert_eq!(omitted_arguments["result"]["isError"], false);

        let unknown_tool = mcp.send(json!({
            "jsonrpc":"2.0", "id":9, "method":"tools/call",
            "params":{"name":"delete_everything","arguments":{}}
        }));
        assert_eq!(unknown_tool["error"]["code"], -32602);
    })
    .await
    .expect("MCP protocol scenario");
}
