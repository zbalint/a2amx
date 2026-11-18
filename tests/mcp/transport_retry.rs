use super::*;

fn write_raw_frame(stream: &mut TcpStream, payload: &[u8]) {
    let frame = encode_frame(payload).expect("frame scripted response");
    stream.write_all(&frame).expect("write scripted response");
    stream.flush().expect("flush scripted response");
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
        write_response(
            &mut second,
            &Response::Agents {
                agents: Vec::new(),
                owner: None,
            },
        );
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
