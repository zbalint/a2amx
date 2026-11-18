#![allow(clippy::expect_used, clippy::unwrap_used)]

#[path = "../common/mod.rs"]
mod common;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, TryRecvError};
use std::thread::{self, JoinHandle};

use a2amx::client::Client;
use a2amx::harness::{Deliver, Harness};
use a2amx::wire::{MAX_FRAME_LEN, MessageInfo, Request, Response, encode_frame};
use serde_json::{Value, json};

struct McpProcess {
    child: Child,
    stdin: Option<ChildStdin>,
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
            stdin: Some(stdin),
            stdout,
        }
    }

    fn send(&mut self, request: Value) -> Value {
        self.send_raw(&serde_json::to_string(&request).expect("json request"));
        self.read()
    }

    fn send_raw(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().expect("mcp stdin open");
        writeln!(stdin, "{line}").expect("write mcp request");
        stdin.flush().expect("flush mcp request");
    }

    fn send_bytes(&mut self, bytes: &[u8]) {
        let stdin = self.stdin.as_mut().expect("mcp stdin open");
        stdin.write_all(bytes).expect("write mcp bytes");
        stdin.flush().expect("flush mcp bytes");
    }

    fn close_stdin(&mut self) {
        self.stdin.take();
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

mod line_limits;
mod process_environment;
mod tools_and_messages;
mod transport_retry;
