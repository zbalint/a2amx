#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread::{self, JoinHandle};

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{MessageInfo, Request, Response, encode_frame};
use serde_json::{Value, json};

struct McpProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

fn spawn_scripted_server<F>(script: F) -> (SocketAddr, JoinHandle<()>)
where
    F: FnOnce(TcpListener) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("scripted daemon listener");
    let address = listener.local_addr().expect("scripted daemon address");
    let thread = thread::spawn(move || script(listener));
    (address, thread)
}

fn read_frame(stream: &mut TcpStream) -> Vec<u8> {
    let mut header = [0; 4];
    stream
        .read_exact(&mut header)
        .expect("read daemon frame header");
    let length = u32::from_be_bytes(header) as usize;
    let mut payload = vec![0; length];
    stream
        .read_exact(&mut payload)
        .expect("read daemon frame body");
    payload
}

fn read_request(stream: &mut TcpStream) -> Request {
    serde_json::from_slice(&read_frame(stream)).expect("decode daemon request")
}

fn write_response(stream: &mut TcpStream, response: &Response) {
    let payload = serde_json::to_vec(response).expect("encode daemon response");
    let frame = encode_frame(&payload).expect("frame daemon response");
    stream.write_all(&frame).expect("write daemon response");
    stream.flush().expect("flush daemon response");
}

fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
    let frame = encode_frame(payload).expect("frame scripted response");
    stream.write_all(&frame).expect("write scripted response");
    stream.flush().expect("flush scripted response");
}

fn expect_hello(stream: &mut TcpStream) {
    let request = read_request(stream);
    assert!(matches!(request, Request::Hello { .. }));
    write_response(stream, &Response::Ok);
}

impl McpProcess {
    fn spawn(addr: SocketAddr, token: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_a2amx"))
            .arg("mcp")
            .env("A2AMX_ADDR", addr.to_string())
            .env("A2AMX_TOKEN", token)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn mcp");
        let stdin = child.stdin.take().expect("mcp stdin");
        let stdout = BufReader::new(child.stdout.take().expect("mcp stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, request: Value) -> Value {
        self.send_raw(&serde_json::to_string(&request).expect("json request"));
        self.read()
    }

    fn send_raw(&mut self, line: &str) {
        writeln!(self.stdin, "{line}").expect("write mcp request");
        self.stdin.flush().expect("flush mcp request");
    }

    fn read(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("read mcp response");
        assert!(!line.is_empty(), "mcp exited before replying");
        serde_json::from_str(line.trim_end()).expect("mcp response is json")
    }
}

impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn tool_result(response: &Value) -> Value {
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["result"]["isError"], false);
    serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result text"),
    )
    .expect("tool result text is compact json")
}

fn failed_tool_result(response: &Value) -> Value {
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["result"]["isError"], true);
    serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .expect("failed tool result text"),
    )
    .expect("failed tool result text is compact json")
}

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
                "description": "List the agent sessions you can message. Sessions outside the caller's visibility are not listed. \"attached\" means a human client is attached to the session; it does not mean the session is reachable, and a detached session still receives messages. Each entry's \"team\" is its team name when it has one, and \"you\" marks the caller.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "send_message",
                "description": "Send a message to another agent session. Returns a message id once the message is accepted; acceptance does not mean it was delivered, read, or acted on. Check message_status when the reply matters. If recipient_hold is present, the recipient is held and a person may need to act. If recipient_quota is present, the recipient's harness reports that quota is used up and it may not answer until the quota resets. Plain text; do not write the text of a paste-wrapper tag in a body.",
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
        mcp.stdin
            .write_all(b"\xff\n")
            .expect("malformed UTF-8 line");
        mcp.stdin.flush().expect("flush malformed line");
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
fn mcp_dead_daemon_is_a_tool_failure_and_stdout_stays_json() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("dead-daemon listener");
    let addr = listener.local_addr().expect("dead-daemon address");
    drop(listener);

    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc":"2.0", "id":1, "method":"tools/call",
        "params":{"name":"list_agents","arguments":{}}
    }));
    let failure = failed_tool_result(&response);
    assert_eq!(failure["code"], "daemon_unreachable");
    assert!(response.is_object());
}
#[test]
fn mcp_retries_read_once_after_transport_io() {
    let (addr, server) = spawn_scripted_server(|listener| {
        let (mut first, _) = listener.accept().expect("accept first read connection");
        expect_hello(&mut first);
        let first_request = read_request(&mut first);
        assert!(matches!(first_request, Request::ListAgents));
        drop(first);

        let (mut second, _) = listener.accept().expect("accept retried read connection");
        expect_hello(&mut second);
        let second_request = read_request(&mut second);
        assert!(matches!(second_request, Request::ListAgents));
        write_response(&mut second, &Response::Agents { agents: Vec::new() });
    });

    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "list_agents", "arguments": {}}
    }));
    assert_eq!(tool_result(&response), json!({"agents": []}));
    server.join().expect("read retry server");
}

#[test]
fn mcp_does_not_retry_read_parse_errors() {
    let (addr, server) = spawn_scripted_server(|listener| {
        let (mut stream, _) = listener.accept().expect("accept parse-error connection");
        expect_hello(&mut stream);
        let request = read_request(&mut stream);
        assert!(matches!(request, Request::ListAgents));
        write_raw_frame(&mut stream, b"not json");
        drop(stream);
        drop(listener);
    });

    let mut mcp = McpProcess::spawn(addr, &"a".repeat(64));
    let response = mcp.send(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {"name": "list_agents", "arguments": {}}
    }));
    let failure = failed_tool_result(&response);
    assert_eq!(failure["code"], "internal");
    server.join().expect("parse-error server");
}

#[test]
fn mcp_send_never_retries_after_transport_loss() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("send daemon listener");
    let addr = listener.local_addr().expect("send daemon address");
    listener
        .set_nonblocking(true)
        .expect("set send listener mode");
    let (stop_sender, stop_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::yield_now();
                }
                Err(_) => panic!("accept send connection"),
            }
        };
        expect_hello(&mut stream);
        let request = read_request(&mut stream);
        assert!(matches!(request, Request::SendMessage { .. }));
        drop(stream);

        loop {
            match listener.accept() {
                Ok((mut retry, _)) => {
                    expect_hello(&mut retry);
                    let request = read_request(&mut retry);
                    assert!(matches!(request, Request::SendMessage { .. }));
                    write_response(
                        &mut retry,
                        &Response::Accepted {
                            id: "m_retry".to_owned(),
                            recipient_hold: None,
                            recipient_quota: None,
                        },
                    );
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    match stop_receiver.try_recv() {
                        Ok(()) | Err(TryRecvError::Disconnected) => return,
                        Err(TryRecvError::Empty) => thread::yield_now(),
                    }
                }
                Err(_) => return,
            }
        }
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
                "message": "The daemon connection closed after the request."
            }
        }
    }));
    let _ = stop_sender.send(());
    server.join().expect("send retry server");
    let failure = failed_tool_result(&response);
    assert_eq!(failure["code"], "unknown_outcome");
    assert_eq!(
        failure["message"],
        "connection lost; the message may or may not have been accepted"
    );
}

#[tokio::test]
async fn mcp_prewrite_failure_does_not_claim_an_unknown_outcome() {
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
        assert_eq!(failed_tool_result(&response)["code"], "internal");
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
    })
    .await
    .expect("prewrite boundary scenario");
}
